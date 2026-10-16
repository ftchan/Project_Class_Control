//! Async read/write of length-prefixed JSON `Envelope`s.

use crate::protocol::Envelope;
use anyhow::Context;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Largest frame we will accept, to bound memory on malformed/hostile input.
/// Large enough to carry a base64-encoded client executable for push updates.
pub const MAX_FRAME: usize = 96 * 1024 * 1024;

/// Write one envelope as a 4-byte big-endian length prefix + JSON body.
pub async fn write_frame<W>(w: &mut W, env: &Envelope) -> anyhow::Result<()>
where
    W: AsyncWriteExt + Unpin,
{
    let body = serde_json::to_vec(env).context("serialize envelope")?;
    anyhow::ensure!(body.len() <= MAX_FRAME, "frame too large: {}", body.len());
    w.write_u32(body.len() as u32).await?;
    w.write_all(&body).await?;
    w.flush().await?;
    Ok(())
}

/// Read one envelope. Returns `Ok(None)` on a clean EOF at a frame boundary.
pub async fn read_frame<R>(r: &mut R) -> anyhow::Result<Option<Envelope>>
where
    R: AsyncReadExt + Unpin,
{
    let len = match r.read_u32().await {
        Ok(n) => n as usize,
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    anyhow::ensure!(len <= MAX_FRAME, "declared frame too large: {len}");
    let mut buf = vec![0u8; len];
    r.read_exact(&mut buf).await.context("read frame body")?;
    let env = serde_json::from_slice(&buf).context("parse envelope")?;
    Ok(Some(env))
}
