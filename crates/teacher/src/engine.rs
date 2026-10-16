//! Console engine: discover clients on the LAN, poll their status and
//! thumbnails, and relay administrator commands to them.
//!
//! Every connection is short-lived: for each command we open a TCP connection
//! to the client, exchange one request/reply, and close. This keeps the engine
//! simple and resilient to clients coming and going.

use anyhow::{Context, Result};
use futures_util::future::join_all;
use labctl_common::net::{read_frame, write_frame};
use labctl_common::policy::Policy;
use labctl_common::protocol::{
    Body, ClientStatus, Command, Envelope, Report, ScreenThumbnail,
};
use labctl_common::{DISCOVERY_MAGIC, DISCOVERY_PORT};
use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;
use tokio::net::{TcpStream, UdpSocket};
use tokio::sync::{broadcast, RwLock};
use uuid::Uuid;

/// What the engine knows about one client.
#[derive(Clone, serde::Serialize)]
pub struct ClientRecord {
    pub machine_id: Uuid,
    pub hostname: String,
    pub addr: String,
    pub status: Option<ClientStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thumbnail: Option<ScreenThumbnail>,
    pub last_seen: chrono::DateTime<chrono::Utc>,
    pub online: bool,
}

/// An event pushed to connected UI clients over the WebSocket.
#[derive(Clone, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Event {
    Roster { clients: Vec<ClientRecord> },
    Thumbnail { machine_id: Uuid, thumbnail: ScreenThumbnail },
}

pub struct Engine {
    clients: RwLock<HashMap<Uuid, ClientRecord>>,
    events: broadcast::Sender<Event>,
    thumb_dim: u32,
}

impl Engine {
    pub fn new() -> Arc<Self> {
        let (events, _) = broadcast::channel(256);
        Arc::new(Self {
            clients: RwLock::new(HashMap::new()),
            events,
            thumb_dim: 320,
        })
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.events.subscribe()
    }

    pub async fn roster(&self) -> Vec<ClientRecord> {
        let mut v: Vec<_> = self.clients.read().await.values().cloned().collect();
        v.sort_by(|a, b| a.hostname.cmp(&b.hostname));
        v
    }

    fn addr_of(record: &ClientRecord) -> Result<SocketAddr> {
        record.addr.parse().context("bad client address")
    }

    async fn record(&self, id: Uuid) -> Option<ClientRecord> {
        self.clients.read().await.get(&id).cloned()
    }

    /// Background loop: discover clients and refresh status + thumbnails.
    pub async fn run_poller(self: Arc<Self>) {
        let mut tick = tokio::time::interval(Duration::from_secs(3));
        loop {
            tick.tick().await;
            if let Err(e) = self.discover().await {
                tracing::debug!(error = %e, "discovery round failed");
            }
            self.refresh_all().await;
        }
    }

    /// Broadcast a discovery beacon on every IPv4 adapter and absorb replies
    /// for a short window.
    ///
    /// A single send to 255.255.255.255 from an unbound socket leaves Windows
    /// on one adapter only (the lowest-metric broadcast route), which is often
    /// a Hyper-V/WSL or VMware virtual NIC, so clients on the real Wi-Fi/LAN
    /// never hear it. Instead, bind one socket per adapter address and send
    /// both the subnet broadcast and the limited broadcast from it.
    async fn discover(&self) -> Result<()> {
        let mut socks = Vec::new();
        for (local, subnet_bcast) in broadcast_targets() {
            let Ok(sock) = UdpSocket::bind((local, 0)).await else { continue };
            if sock.set_broadcast(true).is_err() {
                continue;
            }
            for dst in [subnet_bcast, Ipv4Addr::BROADCAST] {
                if let Err(e) = sock.send_to(DISCOVERY_MAGIC.as_bytes(), (dst, DISCOVERY_PORT)).await {
                    tracing::debug!(%local, %dst, error = %e, "beacon send failed");
                }
            }
            socks.push(sock);
        }
        if socks.is_empty() {
            // Adapter enumeration failed: fall back to the default route.
            let sock = UdpSocket::bind(("0.0.0.0", 0)).await?;
            sock.set_broadcast(true)?;
            sock.send_to(DISCOVERY_MAGIC.as_bytes(), (Ipv4Addr::BROADCAST, DISCOVERY_PORT))
                .await?;
            socks.push(sock);
        }

        // Collect replies from every socket until the shared deadline.
        let deadline = tokio::time::Instant::now() + Duration::from_millis(800);
        let mut readers = tokio::task::JoinSet::new();
        for sock in socks {
            readers.spawn(async move {
                let mut replies = Vec::new();
                let mut buf = [0u8; 256];
                while let Ok(Ok((n, from))) =
                    tokio::time::timeout_at(deadline, sock.recv_from(&mut buf)).await
                {
                    replies.push((buf[..n].to_vec(), from.ip()));
                }
                replies
            });
        }
        while let Some(res) = readers.join_next().await {
            for (data, ip) in res.unwrap_or_default() {
                self.ingest_beacon(&data, ip).await;
            }
        }
        Ok(())
    }

    async fn ingest_beacon(&self, data: &[u8], ip: std::net::IpAddr) {
        // Format: "LABCTL/1 <uuid> <hostname> <port>"
        let Ok(text) = std::str::from_utf8(data) else { return };
        let mut parts = text.split_whitespace();
        if parts.next() != Some(DISCOVERY_MAGIC) {
            return;
        }
        let (Some(id), Some(host), Some(port)) = (parts.next(), parts.next(), parts.next()) else {
            return;
        };
        let (Ok(id), Ok(port)) = (id.parse::<Uuid>(), port.parse::<u16>()) else { return };
        let addr = format!("{ip}:{port}");

        let mut map = self.clients.write().await;
        let rec = map.entry(id).or_insert_with(|| ClientRecord {
            machine_id: id,
            hostname: host.to_string(),
            addr: addr.clone(),
            status: None,
            thumbnail: None,
            last_seen: chrono::Utc::now(),
            online: true,
        });
        rec.hostname = host.to_string();
        rec.addr = addr;
        rec.last_seen = chrono::Utc::now();
        rec.online = true;
    }

    async fn refresh_all(&self) {
        let records = self.roster().await;
        join_all(records.iter().map(|rec| self.refresh_one(rec.machine_id))).await;
        let _ = self.events.send(Event::Roster { clients: self.roster().await });
    }

    async fn refresh_one(&self, id: Uuid) -> Result<()> {
        let Some(rec) = self.record(id).await else { return Ok(()) };
        let addr = Self::addr_of(&rec)?;

        // Status
        match self.exchange(addr, Command::QueryStatus).await {
            Ok(Report::Status(mut st)) => {
                st.ip = addr.ip().to_string();
                let mut map = self.clients.write().await;
                if let Some(r) = map.get_mut(&id) {
                    r.status = Some(st);
                    r.online = true;
                    r.last_seen = chrono::Utc::now();
                }
            }
            _ => self.mark_offline(id).await,
        }

        // Thumbnail
        if let Ok(Report::Thumbnail(t)) = self
            .exchange(addr, Command::RequestThumbnail { max_dim: self.thumb_dim })
            .await
        {
            let mut map = self.clients.write().await;
            if let Some(r) = map.get_mut(&id) {
                r.thumbnail = Some(t.clone());
            }
            let _ = self.events.send(Event::Thumbnail { machine_id: id, thumbnail: t });
        }
        Ok(())
    }

    async fn mark_offline(&self, id: Uuid) {
        let mut map = self.clients.write().await;
        if let Some(r) = map.get_mut(&id) {
            r.online = false;
        }
    }

    /// Open a connection, send one command, return the first reply.
    async fn exchange(&self, addr: SocketAddr, cmd: Command) -> Result<Report> {
        self.exchange_to(addr, cmd, Duration::from_secs(5)).await
    }

    async fn exchange_to(&self, addr: SocketAddr, cmd: Command, timeout: Duration) -> Result<Report> {
        let fut = async {
            let stream = TcpStream::connect(addr).await?;
            let (mut rd, mut wr) = stream.into_split();
            write_frame(&mut wr, &Envelope::command(cmd)).await?;
            match read_frame(&mut rd).await? {
                Some(Envelope { body: Body::Report(r), .. }) => Ok(r),
                _ => anyhow::bail!("no reply"),
            }
        };
        tokio::time::timeout(timeout, fut)
            .await
            .context("client timed out")?
    }

    /// Public command helpers used by the HTTP API.

    pub async fn push_policy(&self, id: Uuid, policy: Policy) -> Result<Report> {
        let rec = self.record(id).await.context("unknown client")?;
        // Applying runs netsh and registry writes on the client; UWF work
        // continues there in the background.
        self.exchange_to(Self::addr_of(&rec)?, Command::PushPolicy { policy }, Duration::from_secs(15))
            .await
    }

    /// Push to every client at once, so the whole class takes as long as the
    /// slowest machine rather than the sum of all of them.
    pub async fn push_policy_all(&self, policy: Policy) -> Vec<(Uuid, Result<Report>)> {
        let records = self.roster().await;
        join_all(records.iter().map(|rec| {
            let policy = policy.clone();
            async move { (rec.machine_id, self.push_policy(rec.machine_id, policy).await) }
        }))
        .await
    }

    pub async fn launch_ui(&self, id: Uuid) -> Result<Report> {
        let rec = self.record(id).await.context("unknown client")?;
        self.exchange(Self::addr_of(&rec)?, Command::LaunchClientUi).await
    }

    pub async fn reboot(&self, id: Uuid, restore: bool, delay: u32) -> Result<Report> {
        let rec = self.record(id).await.context("unknown client")?;
        self.exchange(Self::addr_of(&rec)?, Command::RebootAndRestore { restore, delay_secs: delay })
            .await
    }

    /// Push a new client executable to every client and update it seamlessly.
    pub async fn update_client_all(
        &self,
        filename: String,
        target_path: Option<String>,
        data_base64: String,
        run_after: bool,
    ) -> Vec<(Uuid, Result<Report>)> {
        let records = self.roster().await;
        join_all(records.iter().map(|rec| {
            let cmd = Command::UpdateClient {
                filename: filename.clone(),
                target_path: target_path.clone(),
                data_base64: data_base64.clone(),
                run_after,
            };
            async move {
                let r = match Self::addr_of(rec) {
                    Ok(addr) => self.exchange_to(addr, cmd, Duration::from_secs(90)).await,
                    Err(e) => Err(e),
                };
                (rec.machine_id, r)
            }
        }))
        .await
    }

    /// Broadcast the one-click "restore desktop defaults" to every client.
    pub async fn apply_defaults_all(
        &self,
        wallpaper_data_base64: Option<String>,
        wallpaper_name: Option<String>,
    ) -> Vec<(Uuid, Result<Report>)> {
        let records = self.roster().await;
        join_all(records.iter().map(|rec| {
            let cmd = Command::ApplyDefaults {
                wallpaper_data_base64: wallpaper_data_base64.clone(),
                wallpaper_name: wallpaper_name.clone(),
            };
            async move {
                let r = match Self::addr_of(rec) {
                    Ok(addr) => self.exchange_to(addr, cmd, Duration::from_secs(30)).await,
                    Err(e) => Err(e),
                };
                (rec.machine_id, r)
            }
        }))
        .await
    }
}

/// Every non-loopback IPv4 adapter address paired with its subnet broadcast.
fn broadcast_targets() -> Vec<(Ipv4Addr, Ipv4Addr)> {
    let ifaces = match if_addrs::get_if_addrs() {
        Ok(v) => v,
        Err(e) => {
            tracing::debug!(error = %e, "could not list network adapters");
            return Vec::new();
        }
    };
    ifaces
        .into_iter()
        .filter_map(|iface| match iface.addr {
            if_addrs::IfAddr::V4(v4) if !v4.ip.is_loopback() => {
                Some((v4.ip, subnet_broadcast(v4.ip, v4.netmask)))
            }
            _ => None,
        })
        .collect()
}

fn subnet_broadcast(ip: Ipv4Addr, netmask: Ipv4Addr) -> Ipv4Addr {
    Ipv4Addr::from(u32::from(ip) | !u32::from(netmask))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subnet_broadcast_per_prefix() {
        let ip = Ipv4Addr::new(192, 168, 31, 123);
        assert_eq!(subnet_broadcast(ip, Ipv4Addr::new(255, 255, 255, 0)), Ipv4Addr::new(192, 168, 31, 255));
        let ip = Ipv4Addr::new(172, 25, 208, 1);
        assert_eq!(subnet_broadcast(ip, Ipv4Addr::new(255, 255, 240, 0)), Ipv4Addr::new(172, 25, 223, 255));
    }
}
