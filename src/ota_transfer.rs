//! Bounded firmware transfer, independent of the flash and HTTP drivers.

use embedded_svc::io::{Read, Write};
use std::time::Instant;

#[derive(Debug, thiserror::Error)]
pub enum TransferError {
    #[error("Firmware download failed: {0}")]
    Read(String),
    #[error("Firmware flash write failed: {0}")]
    Write(String),
    #[error("{0}")]
    Invalid(String),
}

pub fn copy_firmware(
    source: &mut impl Read,
    destination: &mut impl Write,
    buffer: &mut [u8],
    capacity: usize,
    expected_size: Option<usize>,
    deadline: Instant,
) -> Result<usize, TransferError> {
    if buffer.is_empty() {
        return Err(TransferError::Invalid("Empty transfer buffer".into()));
    }
    let mut received = 0_usize;
    loop {
        if Instant::now() >= deadline {
            return Err(TransferError::Invalid("Firmware transfer timed out".into()));
        }
        let count = source
            .read(buffer)
            .map_err(|error| TransferError::Read(error.to_string()))?;
        if Instant::now() >= deadline {
            return Err(TransferError::Invalid("Firmware transfer timed out".into()));
        }
        if count == 0 {
            break;
        }
        let total = received
            .checked_add(count)
            .filter(|&total| total <= capacity && expected_size.is_none_or(|size| total <= size))
            .ok_or_else(|| {
                TransferError::Invalid(
                    "Firmware exceeds its declared size or the OTA slot capacity".into(),
                )
            })?;
        destination
            .write_all(&buffer[..count])
            .map_err(|error| TransferError::Write(error.to_string()))?;
        received = total;
    }
    if received == 0 || expected_size.is_some_and(|size| size != received) {
        return Err(TransferError::Invalid(
            "Firmware download is empty or truncated".into(),
        ));
    }
    Ok(received)
}
