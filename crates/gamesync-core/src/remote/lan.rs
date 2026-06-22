//! Peer-to-peer LAN transport.
//!
//! One device **hosts** (`serve`): it runs a TCP server that exposes a
//! [`FolderRemote`]-backed store. Other devices connect with [`LanRemote`],
//! which implements [`Remote`] by sending length-prefixed JSON RPCs over TCP —
//! so all the existing sync/conflict logic works unchanged, device-to-device,
//! with no cloud or shared folder.
//!
//! Pairing is a shared `token` (the host shows it; the client supplies it).
//! Object bytes are hex-encoded in the JSON frames — simple and dependency-free;
//! fine for save-sized files (streaming/base64 is a future refinement).

use std::io::{self, Read, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream, UdpSocket};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::folder::FolderRemote;
use super::{Lease, Remote};
use crate::error::{Error, Result};
use crate::model::{Head, Snapshot};
use crate::util::{atomic_write, from_hex, new_id, now_ms, to_hex};

const LOCK_STALE_MS: i64 = 120_000;
const MAX_FRAME: usize = 1 << 30; // 1 GiB ceiling per frame

#[derive(Serialize, Deserialize)]
enum Req {
    Auth {
        token: String,
    },
    HasObject {
        hash: String,
    },
    GetObject {
        hash: String,
    },
    PutObject {
        hash: String,
        data_hex: String,
    },
    GetVersion {
        game_id: String,
        version_id: String,
    },
    PutVersion {
        game_id: String,
        snapshot: Box<Snapshot>,
    },
    GetHead {
        game_id: String,
    },
    SetHead {
        game_id: String,
        head: Head,
    },
    Lock {
        game_id: String,
    },
    Unlock {
        game_id: String,
    },
}

#[derive(Serialize, Deserialize)]
enum Resp {
    Ok,
    Bool(bool),
    Bytes { data_hex: String },
    Version(Box<Snapshot>),
    Head(Option<Head>),
    Err(String),
}

// ---- framing ------------------------------------------------------------

fn write_frame<W: Write>(w: &mut W, bytes: &[u8]) -> io::Result<()> {
    w.write_all(&(bytes.len() as u32).to_be_bytes())?;
    w.write_all(bytes)?;
    w.flush()
}

fn read_frame<R: Read>(r: &mut R) -> io::Result<Vec<u8>> {
    let mut len = [0u8; 4];
    r.read_exact(&mut len)?;
    let n = u32::from_be_bytes(len) as usize;
    if n > MAX_FRAME {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "frame too large",
        ));
    }
    let mut buf = vec![0u8; n];
    r.read_exact(&mut buf)?;
    Ok(buf)
}

// ---- client -------------------------------------------------------------

pub struct LanRemote {
    addr: String,
    token: String,
}

impl LanRemote {
    /// Connect to a host. `addr` is `host:port`; `token` must match the host's.
    pub fn connect(addr: impl Into<String>, token: impl Into<String>) -> Self {
        Self {
            addr: addr.into(),
            token: token.into(),
        }
    }

    /// One request = one short-lived connection (auth frame + request frame,
    /// then read the response). Simple and avoids shared mutable stream state.
    fn call(&self, req: &Req) -> Result<Resp> {
        let mut stream = TcpStream::connect(&self.addr)
            .map_err(|e| Error::other(format!("LAN connect {} failed: {e}", self.addr)))?;
        stream.set_nodelay(true).ok();
        stream.set_read_timeout(Some(Duration::from_secs(60))).ok();
        write_frame(
            &mut stream,
            &serde_json::to_vec(&Req::Auth {
                token: self.token.clone(),
            })?,
        )?;
        write_frame(&mut stream, &serde_json::to_vec(req)?)?;
        let resp: Resp = serde_json::from_slice(&read_frame(&mut stream)?)?;
        match resp {
            Resp::Err(e) => Err(Error::other(e)),
            other => Ok(other),
        }
    }
}

impl Remote for LanRemote {
    fn lock(&self, game_id: &str) -> Result<Lease> {
        self.call(&Req::Lock {
            game_id: game_id.to_string(),
        })?;
        let addr = self.addr.clone();
        let token = self.token.clone();
        let game_id = game_id.to_string();
        Ok(Lease::new(move || {
            let _ = LanRemote::connect(addr.clone(), token.clone()).call(&Req::Unlock {
                game_id: game_id.clone(),
            });
        }))
    }

    fn has_object(&self, hash: &str) -> Result<bool> {
        match self.call(&Req::HasObject {
            hash: hash.to_string(),
        })? {
            Resp::Bool(b) => Ok(b),
            _ => Err(Error::other("unexpected LAN response")),
        }
    }

    fn put_object(&self, hash: &str, bytes: &[u8]) -> Result<()> {
        self.call(&Req::PutObject {
            hash: hash.to_string(),
            data_hex: to_hex(bytes),
        })?;
        Ok(())
    }

    fn get_object(&self, hash: &str) -> Result<Vec<u8>> {
        match self.call(&Req::GetObject {
            hash: hash.to_string(),
        })? {
            Resp::Bytes { data_hex } => {
                from_hex(&data_hex).ok_or_else(|| Error::other("bad hex from LAN host"))
            }
            _ => Err(Error::other("unexpected LAN response")),
        }
    }

    fn get_version(&self, game_id: &str, version_id: &str) -> Result<Snapshot> {
        match self.call(&Req::GetVersion {
            game_id: game_id.to_string(),
            version_id: version_id.to_string(),
        })? {
            Resp::Version(s) => Ok(*s),
            _ => Err(Error::other("unexpected LAN response")),
        }
    }

    fn put_version(&self, game_id: &str, snapshot: &Snapshot) -> Result<()> {
        self.call(&Req::PutVersion {
            game_id: game_id.to_string(),
            snapshot: Box::new(snapshot.clone()),
        })?;
        Ok(())
    }

    fn get_head(&self, game_id: &str) -> Result<Option<Head>> {
        match self.call(&Req::GetHead {
            game_id: game_id.to_string(),
        })? {
            Resp::Head(h) => Ok(h),
            _ => Err(Error::other("unexpected LAN response")),
        }
    }

    fn set_head(&self, game_id: &str, head: &Head) -> Result<()> {
        self.call(&Req::SetHead {
            game_id: game_id.to_string(),
            head: head.clone(),
        })?;
        Ok(())
    }
}

// ---- server -------------------------------------------------------------

/// A running LAN host. Dropping it (or calling [`stop`](Self::stop)) shuts the
/// listener down.
pub struct LanServerHandle {
    pub port: u16,
    stop: Arc<AtomicBool>,
}

impl LanServerHandle {
    pub fn stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

impl Drop for LanServerHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Start hosting `dir` over TCP at `bind` (e.g. `0.0.0.0:0` for an ephemeral
/// port). Returns once the listener is bound; serving continues on a thread.
pub fn serve(dir: PathBuf, token: String, bind: &str) -> Result<LanServerHandle> {
    let listener = TcpListener::bind(bind)?;
    let port = listener.local_addr()?.port();
    listener.set_nonblocking(true)?;
    let stop = Arc::new(AtomicBool::new(false));
    let stop_thread = stop.clone();

    std::thread::spawn(move || loop {
        if stop_thread.load(Ordering::SeqCst) {
            break;
        }
        match listener.accept() {
            Ok((stream, _)) => {
                let dir = dir.clone();
                let token = token.clone();
                std::thread::spawn(move || {
                    let _ = handle_conn(stream, &dir, &token);
                });
            }
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(_) => std::thread::sleep(Duration::from_millis(200)),
        }
    });

    Ok(LanServerHandle { port, stop })
}

fn handle_conn(mut stream: TcpStream, dir: &Path, token: &str) -> Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(60))).ok();
    let auth: Req = serde_json::from_slice(&read_frame(&mut stream)?)?;
    match auth {
        Req::Auth { token: t } if t == token => {}
        _ => {
            let _ = write_frame(
                &mut stream,
                &serde_json::to_vec(&Resp::Err("unauthorized".into()))?,
            );
            return Ok(());
        }
    }
    let req: Req = serde_json::from_slice(&read_frame(&mut stream)?)?;
    let resp = process(req, dir).unwrap_or_else(|e| Resp::Err(e.to_string()));
    write_frame(&mut stream, &serde_json::to_vec(&resp)?)?;
    Ok(())
}

fn process(req: Req, dir: &Path) -> Result<Resp> {
    let remote = FolderRemote::open(dir.to_path_buf())?;
    Ok(match req {
        Req::Auth { .. } => Resp::Err("unexpected auth".into()),
        Req::HasObject { hash } => Resp::Bool(remote.has_object(&hash)?),
        Req::GetObject { hash } => Resp::Bytes {
            data_hex: to_hex(&remote.get_object(&hash)?),
        },
        Req::PutObject { hash, data_hex } => {
            let bytes = from_hex(&data_hex).ok_or_else(|| Error::other("bad hex"))?;
            remote.put_object(&hash, &bytes)?;
            Resp::Ok
        }
        Req::GetVersion {
            game_id,
            version_id,
        } => Resp::Version(Box::new(remote.get_version(&game_id, &version_id)?)),
        Req::PutVersion { game_id, snapshot } => {
            remote.put_version(&game_id, &snapshot)?;
            Resp::Ok
        }
        Req::GetHead { game_id } => Resp::Head(remote.get_head(&game_id)?),
        Req::SetHead { game_id, head } => {
            remote.set_head(&game_id, &head)?;
            Resp::Ok
        }
        Req::Lock { game_id } => {
            lock_acquire(dir, &game_id)?;
            Resp::Ok
        }
        Req::Unlock { game_id } => {
            let _ = std::fs::remove_file(lock_path(dir, &game_id));
            Resp::Ok
        }
    })
}

fn lock_path(dir: &Path, game_id: &str) -> PathBuf {
    dir.join("games")
        .join(game_id.replace([':', '/'], "_"))
        .join(".lock")
}

/// Server-side advisory lock as a persistent file (not the Lease-based one, so
/// it survives between RPCs until Unlock or staleness).
fn lock_acquire(dir: &Path, game_id: &str) -> Result<()> {
    let path = lock_path(dir, game_id);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if let Ok(text) = std::fs::read_to_string(&path) {
        let acquired: i64 = text
            .split(':')
            .nth(1)
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        if now_ms() - acquired < LOCK_STALE_MS {
            return Err(Error::other(
                "remote is locked by another sync in progress; try again shortly",
            ));
        }
    }
    atomic_write(&path, format!("{}:{}", new_id(), now_ms()).as_bytes())?;
    Ok(())
}

/// Best-effort primary LAN IP via the UDP "connect" trick (no packets sent).
pub fn local_ip() -> Option<String> {
    let sock = UdpSocket::bind("0.0.0.0:0").ok()?;
    sock.connect("8.8.8.8:80").ok()?;
    sock.local_addr().ok().map(|a| a.ip().to_string())
}

// ---- discovery ----------------------------------------------------------
//
// A host periodically UDP-broadcasts a small beacon advertising its name and
// TCP port; peers listen for beacons to find hosts without typing an address.
// The pairing **token is never broadcast** — discovery removes the address
// friction, but a peer still supplies the token (shown on the host) to connect.

/// Well-known UDP port the beacon is broadcast on / listened for.
const BEACON_PORT: u16 = 51900;
/// Tag on every beacon so we ignore unrelated traffic on the port.
const BEACON_MAGIC: &str = "gamesync-lan-1";

#[derive(Serialize, Deserialize)]
struct Beacon {
    magic: String,
    name: String,
    port: u16,
}

/// A LAN host found via [`discover`]. `addr` is taken from the packet's source
/// IP (not self-reported), so it's the address a peer should actually dial.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscoveredHost {
    pub name: String,
    pub addr: String,
    pub port: u16,
}

impl DiscoveredHost {
    /// The `host:port` a [`LanRemote`] connects to (the token is added by the UI).
    pub fn endpoint(&self) -> String {
        format!("{}:{}", self.addr, self.port)
    }
}

/// A running beacon broadcaster. Dropping it (or [`stop`](Self::stop)) ends it.
pub struct BeaconHandle {
    stop: Arc<AtomicBool>,
}

impl BeaconHandle {
    pub fn stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

impl Drop for BeaconHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Sleep up to `total` in short slices, returning early if `stop` is set.
fn interruptible_sleep(stop: &AtomicBool, total: Duration) {
    let slice = Duration::from_millis(100);
    let mut left = total;
    while left > Duration::ZERO {
        if stop.load(Ordering::SeqCst) {
            return;
        }
        let nap = slice.min(left);
        std::thread::sleep(nap);
        left = left.saturating_sub(nap);
    }
}

/// Start broadcasting a beacon for a host named `name` serving on TCP `port`.
/// Broadcasts on the well-known beacon port until the handle is dropped.
pub fn announce(name: String, port: u16) -> Result<BeaconHandle> {
    announce_on(name, port, BEACON_PORT)
}

fn announce_on(name: String, port: u16, beacon_port: u16) -> Result<BeaconHandle> {
    let sock = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))?;
    sock.set_broadcast(true)?;
    let payload = serde_json::to_vec(&Beacon {
        magic: BEACON_MAGIC.to_string(),
        name,
        port,
    })?;
    let stop = Arc::new(AtomicBool::new(false));
    let stop_thread = stop.clone();
    std::thread::spawn(move || {
        while !stop_thread.load(Ordering::SeqCst) {
            let _ = sock.send_to(&payload, (Ipv4Addr::BROADCAST, beacon_port));
            interruptible_sleep(&stop_thread, Duration::from_millis(800));
        }
    });
    Ok(BeaconHandle { stop })
}

/// Listen for host beacons for `timeout`, returning each unique host once.
pub fn discover(timeout: Duration) -> Result<Vec<DiscoveredHost>> {
    discover_on(BEACON_PORT, timeout)
}

fn discover_on(beacon_port: u16, timeout: Duration) -> Result<Vec<DiscoveredHost>> {
    let sock = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, beacon_port))?;
    sock.set_broadcast(true).ok();
    sock.set_read_timeout(Some(Duration::from_millis(250)))?;
    let deadline = std::time::Instant::now() + timeout;
    let mut found: Vec<DiscoveredHost> = Vec::new();
    let mut buf = [0u8; 2048];
    while std::time::Instant::now() < deadline {
        match sock.recv_from(&mut buf) {
            Ok((n, src)) => {
                if let Ok(b) = serde_json::from_slice::<Beacon>(&buf[..n]) {
                    if b.magic == BEACON_MAGIC {
                        let host = DiscoveredHost {
                            name: b.name,
                            addr: src.ip().to_string(),
                            port: b.port,
                        };
                        if !found
                            .iter()
                            .any(|h| h.addr == host.addr && h.port == host.port)
                        {
                            found.push(host);
                        }
                    }
                }
            }
            Err(ref e)
                if e.kind() == io::ErrorKind::WouldBlock || e.kind() == io::ErrorKind::TimedOut => {
            }
            Err(_) => break,
        }
    }
    Ok(found)
}

/// This device's hostname (a friendly label for the beacon), or a fallback.
pub fn hostname() -> String {
    std::process::Command::new("hostname")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().trim_end_matches(".local").to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "GameSync host".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn beacon_round_trips_and_rejects_foreign_packets() {
        let bytes = serde_json::to_vec(&Beacon {
            magic: BEACON_MAGIC.to_string(),
            name: "Test-PC".into(),
            port: 5000,
        })
        .unwrap();
        let b: Beacon = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(b.magic, BEACON_MAGIC);
        assert_eq!(b.port, 5000);
        // Random/foreign UDP payloads must not parse as a beacon.
        assert!(serde_json::from_slice::<Beacon>(b"hello world").is_err());
    }

    #[test]
    fn hostname_is_nonempty() {
        assert!(!hostname().is_empty());
    }

    #[test]
    fn discover_finds_an_announced_host() {
        // Use an unusual beacon port so this doesn't clash with the real one or
        // other tests. Broadcasts loop back to local listeners on the same host.
        let port = 51987;
        let _beacon = announce_on("Loopback-Host".into(), 4321, port).unwrap();
        // Poll for up to ~3s; the first broadcast goes out immediately.
        let hosts = discover_on(port, Duration::from_millis(3000)).unwrap();
        if hosts.is_empty() {
            // Some sandboxes block UDP broadcast entirely — don't false-fail.
            eprintln!("skip: UDP broadcast not delivered in this environment");
            return;
        }
        let h = hosts.iter().find(|h| h.port == 4321).expect("our host");
        assert_eq!(h.name, "Loopback-Host");
        assert!(!h.addr.is_empty());
        assert!(h.endpoint().ends_with(":4321"));
    }
}
