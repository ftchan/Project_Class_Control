//! Synchronous request/reply to the local student agent on 127.0.0.1.
//!
//! The GUI runs in the interactive user session and generally lacks the
//! privileges to change UWF directly; the agent (a LocalSystem service) does.
//! So the GUI asks the local agent to carry out privileged actions, and reads
//! live status the same way. Same wire format as the teacher console.

use labctl_common::protocol::{Body, Command, Envelope, Report};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

const MAX_FRAME: usize = 8 * 1024 * 1024;

pub fn send(cmd: Command) -> anyhow::Result<Report> {
    let mut s = TcpStream::connect(("127.0.0.1", labctl_common::DEFAULT_PORT))?;
    let read_timeout = if matches!(&cmd, Command::AdminSetPolicy { .. } | Command::AdminRestore { .. }) {
        Duration::from_secs(60)
    } else { Duration::from_secs(5) };
    s.set_read_timeout(Some(read_timeout))?;
    s.set_write_timeout(Some(Duration::from_secs(5)))?;

    let body = serde_json::to_vec(&Envelope::command(cmd))?;
    s.write_all(&(body.len() as u32).to_be_bytes())?;
    s.write_all(&body)?;
    s.flush()?;

    let mut lenb = [0u8; 4];
    s.read_exact(&mut lenb)?;
    let len = u32::from_be_bytes(lenb) as usize;
    anyhow::ensure!(len <= MAX_FRAME, "reply frame too large");
    let mut buf = vec![0u8; len];
    s.read_exact(&mut buf)?;

    match serde_json::from_slice::<Envelope>(&buf)?.body {
        Body::Report(r) => Ok(r),
        _ => anyhow::bail!("unexpected reply from agent"),
    }
}
