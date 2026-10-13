#![cfg(test)]

#[path = "../../src/esphome_frame.rs"]
mod esphome_frame;
#[path = "../../src/ota_transfer.rs"]
mod ota_transfer;

use embedded_svc::io::{ErrorType, Read, Write};
use esphome_frame::{FrameReader, read_varuint_from_slice};
use ota_transfer::{TransferError, copy_firmware};
use tokio::{
    io::AsyncWriteExt,
    time::{Duration, timeout},
};

#[tokio::test(start_paused = true)]
async fn partial_frame_survives_repeated_poll_timeouts() {
    let (mut server, mut client) = tokio::io::duplex(128);
    let mut decoder = FrameReader::default();
    // Preamble, a partial two-byte length, message type, then partial payload.
    client.write_all(&[0, 0x80]).await.unwrap();
    assert!(
        timeout(Duration::from_secs(1), decoder.read_frame(&mut server))
            .await
            .is_err()
    );
    client.write_all(&[1, 7, 1, 2, 3]).await.unwrap();
    assert!(
        timeout(Duration::from_secs(1), decoder.read_frame(&mut server))
            .await
            .is_err()
    );
    client.write_all(&[9; 125]).await.unwrap();
    let (kind, payload) = decoder.read_frame(&mut server).await.unwrap();
    assert_eq!(kind, 7);
    assert_eq!(payload.len(), 128);
    assert_eq!(&payload[..3], &[1, 2, 3]);
    assert!(payload[3..].iter().all(|&byte| byte == 9));
    // Subsequent frames must not inherit the previous decoder state.
    client.write_all(&[0, 0, 9, 0, 1, 7, 42]).await.unwrap();
    assert_eq!(decoder.read_frame(&mut server).await.unwrap(), (9, vec![]));
    assert_eq!(
        decoder.read_frame(&mut server).await.unwrap(),
        (7, vec![42])
    );
}

#[tokio::test(start_paused = true)]
async fn partial_message_type_and_frame_deadline_survive_cancellation() {
    let (mut server, mut client) = tokio::io::duplex(32);
    let mut decoder = FrameReader::default();
    client.write_all(&[0, 0, 0x80]).await.unwrap();
    assert!(
        timeout(Duration::from_secs(1), decoder.read_frame(&mut server))
            .await
            .is_err()
    );
    client.write_all(&[1]).await.unwrap();
    assert_eq!(
        decoder.read_frame(&mut server).await.unwrap(),
        (128, vec![])
    );
    client.write_all(&[0, 1, 7]).await.unwrap();
    assert!(
        timeout(Duration::from_secs(5), decoder.read_frame(&mut server))
            .await
            .is_err()
    );
    assert!(
        timeout(Duration::from_secs(5), decoder.read_frame(&mut server))
            .await
            .is_err()
    );
    let error = decoder.read_frame(&mut server).await.unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
}

#[tokio::test]
async fn malformed_frames_and_oversized_payloads_are_rejected() {
    for bytes in [
        vec![1],
        vec![0, 0x81, 0x20],
        vec![0, 0xFF, 0xFF, 0xFF, 0xFF, 0x10],
        vec![0, 0, 0x80, 0x80, 0x80, 0x80, 0x80],
    ] {
        let error = FrameReader::default()
            .read_frame(&mut bytes.as_slice())
            .await
            .unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
    }
    let error = FrameReader::default()
        .read_frame(&mut [0, 2, 7, 42].as_slice())
        .await
        .unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::UnexpectedEof);
}

#[test]
fn protobuf_varints_reject_overflow_and_truncation() {
    assert_eq!(
        read_varuint_from_slice(
            &[0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 1],
            &mut 0
        ),
        Some(u64::MAX)
    );
    assert_eq!(
        read_varuint_from_slice(
            &[0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 2],
            &mut 0
        ),
        None
    );
    assert_eq!(read_varuint_from_slice(&[0x80], &mut 0), None);
}

struct Source<'a> {
    bytes: &'a [u8],
    fail_at_end: bool,
}
impl ErrorType for Source<'_> {
    type Error = std::io::Error;
}
impl Read for Source<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> Result<usize, Self::Error> {
        if self.bytes.is_empty() && self.fail_at_end {
            return Err(std::io::ErrorKind::ConnectionReset.into());
        }
        let count = self.bytes.len().min(buffer.len());
        buffer[..count].copy_from_slice(&self.bytes[..count]);
        self.bytes = &self.bytes[count..];
        Ok(count)
    }
}
#[derive(Default)]
struct Flash {
    bytes: Vec<u8>,
    fail: bool,
}
impl ErrorType for Flash {
    type Error = std::io::Error;
}
impl Write for Flash {
    fn write(&mut self, buffer: &[u8]) -> Result<usize, Self::Error> {
        if self.fail {
            return Err(std::io::Error::other("Flash failure"));
        }
        self.bytes.extend_from_slice(buffer);
        Ok(buffer.len())
    }
    fn flush(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
}

#[test]
fn firmware_transfers_enforce_slot_and_declared_size_before_writing() {
    let image = vec![42; 17];
    for declared in [None, Some(image.len())] {
        let mut flash = Flash::default();
        let size = copy_firmware(
            &mut Source {
                bytes: &image,
                fail_at_end: false,
            },
            &mut flash,
            &mut [0; 4],
            17,
            declared,
            std::time::Instant::now() + Duration::from_secs(60),
        )
        .unwrap();
        assert_eq!(size, image.len());
        assert_eq!(flash.bytes, image);
    }
    for (capacity, expected) in [(16, None), (20, Some(16)), (20, Some(18))] {
        let mut flash = Flash::default();
        assert!(matches!(
            copy_firmware(
                &mut Source {
                    bytes: &image,
                    fail_at_end: false
                },
                &mut flash,
                &mut [0; 4],
                capacity,
                expected,
                std::time::Instant::now() + Duration::from_secs(60),
            ),
            Err(TransferError::Invalid(_))
        ));
        assert!(flash.bytes.len() <= capacity);
        assert!(expected.is_none_or(|size| flash.bytes.len() <= size));
    }
}

#[test]
fn empty_interrupted_and_failed_flash_transfers_are_errors() {
    assert!(matches!(
        copy_firmware(
            &mut Source {
                bytes: &[],
                fail_at_end: false
            },
            &mut Flash::default(),
            &mut [0; 4],
            10,
            None,
            std::time::Instant::now() + Duration::from_secs(60),
        ),
        Err(TransferError::Invalid(_))
    ));
    assert!(matches!(
        copy_firmware(
            &mut Source {
                bytes: &[1, 2],
                fail_at_end: true
            },
            &mut Flash::default(),
            &mut [0; 4],
            10,
            None,
            std::time::Instant::now() + Duration::from_secs(60),
        ),
        Err(TransferError::Read(_))
    ));
    assert!(matches!(
        copy_firmware(
            &mut Source {
                bytes: &[1, 2],
                fail_at_end: false
            },
            &mut Flash {
                fail: true,
                ..Default::default()
            },
            &mut [0; 4],
            10,
            None,
            std::time::Instant::now() + Duration::from_secs(60),
        ),
        Err(TransferError::Write(_))
    ));
}

#[test]
fn expired_firmware_deadline_leaves_flash_untouched() {
    let mut flash = Flash::default();
    let error = copy_firmware(
        &mut Source {
            bytes: &[1, 2],
            fail_at_end: false,
        },
        &mut flash,
        &mut [0; 4],
        10,
        None,
        std::time::Instant::now() - Duration::from_secs(1),
    )
    .unwrap_err();
    assert!(matches!(error, TransferError::Invalid(_)));
    assert!(flash.bytes.is_empty());
}
