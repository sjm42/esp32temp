//! Persistent frame decoding: cancelling a read retains every consumed byte.

use tokio::{
    io::{self, AsyncRead, AsyncReadExt},
    time::{Duration, Instant, timeout_at},
};

pub const MAX_PAYLOAD_SIZE: usize = 4096;
const FRAME_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Default)]
enum Stage {
    #[default]
    Preamble,
    Length,
    MessageType,
    Payload,
}

#[derive(Default)]
pub struct FrameReader {
    stage: Stage,
    varint: u32,
    shift: u32,
    length: usize,
    message_type: u32,
    payload: Vec<u8>,
    received: usize,
    deadline: Option<Instant>,
}

impl FrameReader {
    pub async fn read_frame(
        &mut self,
        stream: &mut (impl AsyncRead + Unpin),
    ) -> io::Result<(u32, Vec<u8>)> {
        loop {
            match self.stage {
                Stage::Preamble => {
                    if stream.read_u8().await? != 0 {
                        return Err(invalid_data("Invalid ESPHome frame preamble"));
                    }
                    self.deadline = Some(Instant::now() + FRAME_TIMEOUT);
                    self.stage = Stage::Length;
                }
                Stage::Length => {
                    let length = self.read_varint(stream).await? as usize;
                    if length > MAX_PAYLOAD_SIZE {
                        return Err(invalid_data("ESPHome frame payload exceeds 4096 bytes"));
                    }
                    self.length = length;
                    self.stage = Stage::MessageType;
                }
                Stage::MessageType => {
                    self.message_type = self.read_varint(stream).await?;
                    self.payload.try_reserve_exact(self.length).map_err(|_| {
                        io::Error::other("Could not allocate ESPHome frame payload")
                    })?;
                    self.payload.resize(self.length, 0);
                    self.stage = Stage::Payload;
                }
                Stage::Payload => {
                    while self.received < self.length {
                        let count = Self::with_deadline(
                            self.deadline,
                            stream.read(&mut self.payload[self.received..]),
                        )
                        .await?;
                        if count == 0 {
                            return Err(io::ErrorKind::UnexpectedEof.into());
                        }
                        self.received += count;
                    }
                    let frame = (self.message_type, std::mem::take(&mut self.payload));
                    *self = Self::default();
                    return Ok(frame);
                }
            }
        }
    }

    async fn read_varint(&mut self, stream: &mut (impl AsyncRead + Unpin)) -> io::Result<u32> {
        loop {
            let byte = Self::with_deadline(self.deadline, stream.read_u8()).await?;
            if self.shift == 28 && byte > 0x0F {
                return Err(invalid_data("ESPHome frame varint overflow"));
            }
            self.varint |= u32::from(byte & 0x7F) << self.shift;
            if byte & 0x80 == 0 {
                let value = self.varint;
                self.varint = 0;
                self.shift = 0;
                return Ok(value);
            }
            self.shift += 7;
        }
    }

    async fn with_deadline<T>(
        deadline: Option<Instant>,
        read: impl std::future::Future<Output = io::Result<T>>,
    ) -> io::Result<T> {
        if let Some(deadline) = deadline {
            Box::pin(timeout_at(deadline, read))
                .await
                .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "ESPHome frame stalled"))?
        } else {
            read.await
        }
    }
}

fn invalid_data(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

pub fn read_varuint_from_slice(data: &[u8], index: &mut usize) -> Option<u64> {
    let mut value = 0_u64;
    for shift in (0..70).step_by(7) {
        let byte = *data.get(*index)?;
        *index += 1;
        if shift == 63 && byte > 1 {
            return None;
        }
        value |= u64::from(byte & 0x7F) << shift;
        if byte & 0x80 == 0 {
            return Some(value);
        }
    }
    None
}
