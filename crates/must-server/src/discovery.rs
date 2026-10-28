// SPDX-License-Identifier: LGPL-3.0-or-later

//! SECoP's UDP discovery protocol, node side.
//!
//! Every running node listens on UDP port [`DISCOVERY_PORT`] (10767) of all IPv4
//! interfaces and answers `{"SECoP":"discover"}` with a packet naming its TCP port,
//! `equipment_id`, `firmware` and `description`. At startup it broadcasts the same packet
//! once to every interface. The port is shared (`SO_REUSEPORT` on Linux, `SO_REUSEADDR`
//! on Windows, as Frappy does), so several must and Frappy nodes can run on one host.
//! On Linux a unicast request reaches only one of them, so clients broadcast.
//!
//! - A packet never exceeds 508 bytes; the description is shortened to fit. A node
//!   whose `equipment_id` and `firmware` alone are too long stays out of discovery.
//! - A node without a `firmware` property sends `must-<version>`.
//! - A node bound to a loopback address answers only local requests and announces
//!   nothing.
//! - A node sends at most 20 replies in any second.
//! - Problems are logged; discovery never stops a node.
//!
//! Discovery is on by default; [`NodeBuilder::discovery`](crate::NodeBuilder::discovery)
//! with [`Discovery::off`] or `MUST_DISCOVERY=0` turns it off. See the
//! [specification](https://github.com/SampleEnvironment/SECoP/blob/main/protocol/specification/discovery.rst).

use std::collections::VecDeque;
use std::io;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value as Json, json};
use socket2::{Domain, Protocol, Socket, Type};
use tokio::net::UdpSocket;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;
use tracing::debug;

use crate::events::LogLevel;
use crate::runtime::{Hub, NODE_LOG_TARGET};

/// The UDP port of SECoP discovery.
pub const DISCOVERY_PORT: u16 = 10767;

/// Environment variable that turns discovery off with `0`.
pub const DISCOVERY_VARIABLE: &str = "MUST_DISCOVERY";

/// Largest safe UDP payload; no discovery packet is longer.
pub const MAX_PACKET_LEN: usize = 508;

/// Requests longer than this are ignored.
const MAX_REQUEST_LEN: usize = 1024;

/// Replies a node sends at most in [`RATE_WINDOW`].
const MAX_REPLIES: usize = 20;
const RATE_WINDOW: Duration = Duration::from_secs(1);

/// `firmware` sent by nodes without that property.
const DEFAULT_FIRMWARE: &str = concat!("must-", env!("CARGO_PKG_VERSION"));

/// How a node takes part in discovery. See the [module documentation](self).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub struct Discovery {
    enabled: bool,
    port: u16,
    announce: bool,
}

impl Default for Discovery {
    /// On, on port 10767, with an announcement at startup.
    fn default() -> Self {
        Self {
            enabled: true,
            port: DISCOVERY_PORT,
            announce: true,
        }
    }
}

impl Discovery {
    /// No discovery: the node opens no UDP socket.
    pub fn off() -> Self {
        Self {
            enabled: false,
            ..Self::default()
        }
    }

    /// Listens and announces on another UDP port (for tests and special setups;
    /// standard clients use 10767).
    pub fn port(mut self, port: u16) -> Self {
        self.port = port;
        self
    }

    /// Whether the node broadcasts its packet at startup (default true).
    pub fn announce(mut self, announce: bool) -> Self {
        self.announce = announce;
        self
    }

    /// Whether discovery is on in this configuration (`MUST_DISCOVERY` aside).
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }
}

/// Whether `MUST_DISCOVERY` leaves discovery on.
fn enabled_by_environment() -> bool {
    std::env::var(DISCOVERY_VARIABLE).map_or(true, |v| v.trim() != "0")
}

/// Starts discovery for a node listening on `addr`, if it is on. Must be called within
/// the node's tokio runtime.
pub(crate) fn start(
    hub: &Arc<Hub>,
    addr: SocketAddr,
    tracker: &TaskTracker,
    stop: &CancellationToken,
) {
    let config = hub.model.options.discovery;
    if !config.enabled || !enabled_by_environment() {
        return;
    }
    let props = &hub.model.props;
    let text = |key| props.get(key).and_then(Json::as_str);
    let Some(packet) = packet(
        addr.port(),
        &hub.model.equipment_id,
        text("firmware").unwrap_or(DEFAULT_FIRMWARE),
        text("description").unwrap_or_default(),
    ) else {
        hub.log(
            LogLevel::Warn,
            None,
            format!(
                "equipment_id and firmware do not fit into a {MAX_PACKET_LEN}-byte discovery \
                 packet; the node does not take part in discovery"
            ),
        );
        return;
    };
    let socket = match open_socket(config.port).and_then(UdpSocket::from_std) {
        Ok(socket) => socket,
        Err(e) => {
            hub.log(
                LogLevel::Warn,
                None,
                format!(
                    "cannot listen for discovery on UDP port {}: {e}; the node runs without \
                     discovery",
                    config.port
                ),
            );
            return;
        }
    };
    let local_only = addr.ip().is_loopback();
    let listener = Listener {
        hub: Arc::clone(hub),
        socket,
        packet,
        port: config.port,
        local_only,
        announce: config.announce && !local_only,
    };
    tracker.spawn(listener.run(stop.clone()));
}

/// A UDP socket bound to `0.0.0.0:<port>` that shares the port with other discovery
/// sockets, as Frappy's do.
fn open_socket(port: u16) -> io::Result<std::net::UdpSocket> {
    let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
    #[cfg(unix)]
    socket.set_reuse_port(true)?;
    #[cfg(windows)]
    socket.set_reuse_address(true)?;
    socket.set_broadcast(true)?;
    socket.set_nonblocking(true)?;
    socket.bind(&SocketAddr::from((Ipv4Addr::UNSPECIFIED, port)).into())?;
    Ok(socket.into())
}

/// The reply and announcement packet, with the description shortened to fit
/// [`MAX_PACKET_LEN`]; `None` if it does not fit even with an empty description.
pub(crate) fn packet(
    port: u16,
    equipment_id: &str,
    firmware: &str,
    description: &str,
) -> Option<Vec<u8>> {
    let encode = |description: &str| {
        let packet = json!({
            "SECoP": "node",
            "port": port,
            "equipment_id": equipment_id,
            "firmware": firmware,
            "description": description,
        });
        serde_json::to_vec(&packet).expect("a JSON value always serializes")
    };
    let fits = |packet: &Vec<u8>| packet.len() <= MAX_PACKET_LEN;
    let full = encode(description);
    if fits(&full) {
        return Some(full);
    }
    if !fits(&encode("")) {
        return None;
    }
    // escapes make the encoded length differ from the text's, so measure the packet;
    // a longer prefix never gives a shorter packet
    let cuts: Vec<usize> = description.char_indices().map(|(i, _)| i).collect();
    let fitting = cuts.partition_point(|&cut| fits(&encode(&description[..cut])));
    Some(encode(&description[..cuts[fitting - 1]]))
}

/// Whether a datagram is a discover request.
fn is_discover(datagram: &[u8]) -> bool {
    serde_json::from_slice::<Json>(datagram)
        .is_ok_and(|request| request.get("SECoP").and_then(Json::as_str) == Some("discover"))
}

/// Allows at most [`MAX_REPLIES`] replies in any [`RATE_WINDOW`].
#[derive(Default)]
struct RateLimit {
    sent: VecDeque<Instant>,
}

impl RateLimit {
    /// Whether a reply may be sent at `now`; if so, it is counted.
    fn allow(&mut self, now: Instant) -> bool {
        while self
            .sent
            .front()
            .is_some_and(|&at| now.duration_since(at) >= RATE_WINDOW)
        {
            self.sent.pop_front();
        }
        if self.sent.len() >= MAX_REPLIES {
            return false;
        }
        self.sent.push_back(now);
        true
    }
}

/// Errors after which the socket can still receive: an ICMP port unreachable reported
/// for an earlier reply, and (Windows) a datagram larger than the buffer.
fn is_transient(e: &io::Error) -> bool {
    const WSAEMSGSIZE: i32 = 10040;
    matches!(
        e.kind(),
        io::ErrorKind::ConnectionReset | io::ErrorKind::ConnectionRefused
    ) || (cfg!(windows) && e.raw_os_error() == Some(WSAEMSGSIZE))
}

/// The broadcast addresses of all IPv4 interfaces except loopback, or the limited
/// broadcast address if there are none.
pub(crate) fn broadcast_addresses() -> Vec<Ipv4Addr> {
    let mut addresses: Vec<Ipv4Addr> = if_addrs::get_if_addrs()
        .unwrap_or_default()
        .into_iter()
        .filter(|interface| !interface.is_loopback())
        .filter_map(|interface| match interface.addr {
            if_addrs::IfAddr::V4(v4) => v4.broadcast,
            if_addrs::IfAddr::V6(_) => None,
        })
        .collect();
    addresses.sort_unstable();
    addresses.dedup();
    if addresses.is_empty() {
        addresses.push(Ipv4Addr::BROADCAST);
    }
    addresses
}

struct Listener {
    hub: Arc<Hub>,
    socket: UdpSocket,
    packet: Vec<u8>,
    port: u16,
    local_only: bool,
    announce: bool,
}

impl Listener {
    async fn run(self, stop: CancellationToken) {
        let node = self.hub.model.equipment_id.as_str();
        if self.announce {
            for address in broadcast_addresses() {
                let target = SocketAddr::from((address, self.port));
                match self.socket.send_to(&self.packet, target).await {
                    Ok(_) => {
                        debug!(target: NODE_LOG_TARGET, node, %target, "discovery announcement sent");
                    }
                    Err(e) => {
                        debug!(target: NODE_LOG_TARGET, node, %target, "cannot send the discovery announcement: {e}");
                    }
                }
            }
        }
        // one byte more than allowed, so that a longer datagram shows as too long
        let mut buffer = vec![0; MAX_REQUEST_LEN + 1];
        let mut limit = RateLimit::default();
        loop {
            let received = tokio::select! {
                () = stop.cancelled() => break,
                received = self.socket.recv_from(&mut buffer) => received,
            };
            let (len, peer) = match received {
                Ok(received) => received,
                Err(e) if is_transient(&e) => continue,
                Err(e) => {
                    self.hub.log(
                        LogLevel::Warn,
                        None,
                        format!("discovery stopped: receiving failed: {e}"),
                    );
                    break;
                }
            };
            if len > MAX_REQUEST_LEN
                || !is_discover(&buffer[..len])
                || (self.local_only && !peer.ip().is_loopback())
                || !limit.allow(Instant::now())
            {
                continue;
            }
            debug!(target: NODE_LOG_TARGET, node, %peer, "answering discovery request");
            if let Err(e) = self.socket.send_to(&self.packet, peer).await {
                debug!(target: NODE_LOG_TARGET, node, %peer, "cannot answer the discovery request: {e}");
            }
        }
        // the socket closes when the listener is dropped here
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(packet: &[u8]) -> Json {
        serde_json::from_slice(packet).unwrap()
    }

    fn description_of(packet: &[u8]) -> String {
        text(packet)["description"].as_str().unwrap().to_owned()
    }

    #[test]
    fn layout() {
        let packet = packet(41234, "HZB-cryo", "must-0.1.0", "a cryostat").unwrap();
        assert_eq!(
            String::from_utf8(packet).unwrap(),
            r#"{"SECoP":"node","port":41234,"equipment_id":"HZB-cryo","firmware":"must-0.1.0","description":"a cryostat"}"#
        );
    }

    #[test]
    fn non_ascii_is_utf8() {
        let packet = packet(2055, "eq", "fw", "Kühler 4 K").unwrap();
        assert!(String::from_utf8(packet).unwrap().contains("Kühler 4 K"));
    }

    #[test]
    fn empty_overhead() {
        // as in Frappy's test: the most a packet adds to its three strings
        assert_eq!(packet(65535, "", "", "").unwrap().len(), 78);
    }

    #[test]
    fn ascii_truncation() {
        let description = "a".repeat(600);
        let packet = packet(65535, "eq", "must-0.1.0", &description).unwrap();
        assert_eq!(packet.len(), MAX_PACKET_LEN);
        assert!(description.starts_with(&description_of(&packet)));
    }

    #[test]
    fn unicode_truncation() {
        let description = "\u{1f604}".repeat(400);
        let packet = packet(65535, "eq", "must-0.1.0", &description).unwrap();
        assert!(packet.len() <= MAX_PACKET_LEN);
        let shortened = description_of(&packet);
        assert!(description.starts_with(&shortened));
        // 4 bytes per character: at most 3 bytes of room are left unused
        assert!(MAX_PACKET_LEN - packet.len() < 4);
        assert_eq!(shortened.len() % 4, 0);
    }

    #[test]
    fn escaped_characters_count() {
        // each `"` and newline takes two bytes in the packet
        let description = "\"\n".repeat(300);
        let packet = packet(65535, "eq", "must-0.1.0", &description).unwrap();
        assert!(packet.len() <= MAX_PACKET_LEN);
        assert!(description.starts_with(&description_of(&packet)));
    }

    #[test]
    fn equipment_id_too_long() {
        assert_eq!(packet(2055, &"e".repeat(500), "must-0.1.0", ""), None);
    }

    #[test]
    fn discover_requests() {
        assert!(is_discover(br#"{"SECoP":"discover"}"#));
        assert!(is_discover(br#"{"SECoP":"discover","x":1}"#));
        for other in [
            &b"hello"[..],
            b"[1,2]",
            b"\xff\xfe",
            br#"{"SECoP":"node","port":10767}"#,
            br#"{"secop":"discover"}"#,
            br#"{"SECoP":["discover"]}"#,
        ] {
            assert!(!is_discover(other), "{other:?}");
        }
    }

    #[tokio::test(start_paused = true)]
    async fn rate_limit_window_slides() {
        let mut limit = RateLimit::default();
        let start = Instant::now();
        for _ in 0..MAX_REPLIES {
            assert!(limit.allow(Instant::now()));
            tokio::time::advance(Duration::from_millis(10)).await;
        }
        assert!(!limit.allow(Instant::now()), "21st reply within one second");
        // the first reply leaves the window one second after it was sent
        tokio::time::sleep_until(start + RATE_WINDOW).await;
        assert!(limit.allow(Instant::now()));
        assert!(
            !limit.allow(Instant::now()),
            "only one reply left the window"
        );
    }

    #[test]
    fn defaults() {
        let discovery = Discovery::default();
        assert!(discovery.is_enabled());
        assert_eq!(discovery.port, DISCOVERY_PORT);
        assert!(discovery.announce);
        assert!(!Discovery::off().is_enabled());
    }
}
