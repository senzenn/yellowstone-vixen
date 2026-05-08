//! Stream record + replay.
//!
//! On-disk format:
//!
//! ```text
//! magic    : 8 bytes  = b"MEVRADR1"
//! frames   : repeated
//!     len  : 4 bytes  little-endian u32 (size of the encoded SubscribeUpdate)
//!     body : `len` bytes (prost-encoded yellowstone_grpc_proto::geyser::SubscribeUpdate)
//! ```
//!
//! Designed to be drop-in compatible with
//! `yellowstone-vixen-mock`'s captured-fixture format so the same files
//! can drive both the mev-radar replay path and Vixen test fixtures.

use std::path::Path;

use prost::Message;
use thiserror::Error;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use yellowstone_grpc_proto::geyser::SubscribeUpdate;

const MAGIC: &[u8; 8] = b"MEVRADR1";

/// Hard cap on a single frame body. A real `SubscribeUpdate` is at most
/// a few MiB (a fat full-block message), so 16 MiB leaves plenty of
/// headroom while preventing a corrupted or hostile file from making us
/// allocate 4 GiB on a `vec![0; len]`.
pub const MAX_FRAME_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug, Error)]
pub enum Error {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("decode: {0}")]
    Decode(#[from] prost::DecodeError),
    #[error("bad magic; expected {expected:?}, got {got:?}")]
    BadMagic { expected: [u8; 8], got: [u8; 8] },
    #[error("frame too large: {size} bytes (max {MAX_FRAME_BYTES})")]
    FrameTooLarge { size: usize },
}

pub type Result<T> = std::result::Result<T, Error>;

pub struct Recorder<W: AsyncWriteExt + Unpin> {
    w: W,
    written_frames: u64,
}

impl<W: AsyncWriteExt + Unpin> Recorder<W> {
    pub async fn new(mut w: W) -> Result<Self> {
        w.write_all(MAGIC).await?;
        Ok(Self { w, written_frames: 0 })
    }

    pub async fn write(&mut self, msg: &SubscribeUpdate) -> Result<()> {
        let len = u32::try_from(msg.encoded_len()).map_err(|_| {
            Error::Io(std::io::Error::other("frame too large"))
        })?;

        self.w.write_all(&len.to_le_bytes()).await?;
        let mut buf = Vec::with_capacity(len as usize);
        msg.encode(&mut buf).map_err(|e| Error::Io(std::io::Error::other(e.to_string())))?;
        self.w.write_all(&buf).await?;
        self.written_frames += 1;
        Ok(())
    }

    pub async fn finish(mut self) -> Result<u64> {
        self.w.flush().await?;
        self.w.shutdown().await?;
        Ok(self.written_frames)
    }
}

pub struct Player<R: AsyncReadExt + Unpin> {
    r: R,
}

impl<R: AsyncReadExt + Unpin> Player<R> {
    pub async fn new(mut r: R) -> Result<Self> {
        let mut magic = [0u8; 8];
        r.read_exact(&mut magic).await?;
        if &magic != MAGIC {
            return Err(Error::BadMagic { expected: *MAGIC, got: magic });
        }
        Ok(Self { r })
    }

    /// Read the next frame. Returns `Ok(None)` at clean EOF.
    pub async fn next(&mut self) -> Result<Option<SubscribeUpdate>> {
        let mut len_buf = [0u8; 4];
        match self.r.read_exact(&mut len_buf).await {
            Ok(_) => {},
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
            Err(e) => return Err(Error::Io(e)),
        }

        let len = u32::from_le_bytes(len_buf) as usize;
        if len > MAX_FRAME_BYTES {
            return Err(Error::FrameTooLarge { size: len });
        }

        let mut body = vec![0u8; len];
        self.r.read_exact(&mut body).await?;
        let msg = SubscribeUpdate::decode(body.as_slice())?;
        Ok(Some(msg))
    }
}

/// Convenience: open a recording file for writing.
pub async fn open_record(path: &Path) -> Result<Recorder<tokio::io::BufWriter<tokio::fs::File>>> {
    let f = tokio::fs::File::create(path).await?;
    let buf = tokio::io::BufWriter::new(f);
    Recorder::new(buf).await
}

/// Convenience: open a recording file for reading.
pub async fn open_play(path: &Path) -> Result<Player<tokio::io::BufReader<tokio::fs::File>>> {
    let f = tokio::fs::File::open(path).await?;
    let buf = tokio::io::BufReader::new(f);
    Player::new(buf).await
}

#[cfg(test)]
mod tests {
    use yellowstone_grpc_proto::geyser::{subscribe_update::UpdateOneof, SubscribeUpdatePong};

    use super::*;

    #[tokio::test]
    async fn rejects_oversized_frame() {
        // Magic + a u32 length larger than MAX_FRAME_BYTES.
        let mut buf: Vec<u8> = Vec::new();
        buf.extend_from_slice(MAGIC);
        buf.extend_from_slice(&u32::MAX.to_le_bytes());
        // No body needed — the guard fires before we read it.

        let mut player = Player::new(buf.as_slice()).await.unwrap();
        let err = player.next().await.unwrap_err();
        assert!(matches!(err, Error::FrameTooLarge { .. }));
    }

    #[tokio::test]
    async fn round_trips_a_pong() {
        let mut buf: Vec<u8> = Vec::new();
        let mut rec = Recorder::new(&mut buf).await.unwrap();

        let msg = SubscribeUpdate {
            filters: vec![],
            update_oneof: Some(UpdateOneof::Pong(SubscribeUpdatePong { id: 7 })),
            created_at: None,
        };
        rec.write(&msg).await.unwrap();
        let n = rec.finish().await.unwrap();
        assert_eq!(n, 1);

        let mut player = Player::new(buf.as_slice()).await.unwrap();
        let got = player.next().await.unwrap().unwrap();
        assert_eq!(got, msg);
        assert!(player.next().await.unwrap().is_none());
    }
}
