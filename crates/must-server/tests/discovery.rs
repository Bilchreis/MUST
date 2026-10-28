// SPDX-License-Identifier: LGPL-3.0-or-later

//! SECoP UDP discovery against running nodes.
//!
//! Every test uses a discovery port of its own, so that tests running in parallel and
//! other programs on the host listening on 10767 do not interfere. Tests that need
//! broadcasts skip, with a message, on hosts without a broadcast-capable interface.

mod common;

use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use common::{Sim, node};
use must_server::discovery::{DISCOVERY_VARIABLE, MAX_PACKET_LEN};
use must_server::logfiles::LogFiles;
use must_server::{Discovery, NodeBuilder, RunningNode};
use serde_json::{Value as Json, json};
use socket2::{Domain, Protocol, Socket, Type};
use tokio::net::UdpSocket;
use tokio::runtime::Handle;

const DISCOVER: &[u8] = br#"{"SECoP":"discover"}"#;
const FIRMWARE: &str = concat!("must-", env!("CARGO_PKG_VERSION"));
/// How long to wait for a packet that is expected.
const EXPECTED: Duration = Duration::from_secs(5);
/// How long to wait to be sure that no packet comes.
const QUIET: Duration = Duration::from_millis(500);

/// A UDP port that was free a moment ago.
fn free_port() -> u16 {
    std::net::UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

/// The test node, discoverable on `port`, without announcement.
fn discoverable(port: u16) -> NodeBuilder {
    node(Arc::new(Sim::default()))
        .bind(SocketAddr::from((Ipv4Addr::UNSPECIFIED, 0)))
        .monitored(false)
        .discovery(Discovery::default().port(port).announce(false))
}

fn start(builder: NodeBuilder) -> RunningNode {
    builder.build().unwrap().spawn(&Handle::current()).unwrap()
}

async fn client(ip: Ipv4Addr) -> UdpSocket {
    let socket = UdpSocket::bind((ip, 0)).await.unwrap();
    socket.set_broadcast(true).unwrap();
    socket
}

/// The next packet the socket receives within `timeout`, and its sender.
async fn receive(socket: &UdpSocket, timeout: Duration) -> Option<(Json, SocketAddr)> {
    let mut buffer = vec![0; 2048];
    let (len, from) = tokio::time::timeout(timeout, socket.recv_from(&mut buffer))
        .await
        .ok()?
        .unwrap();
    assert!(len <= MAX_PACKET_LEN, "{len}-byte packet");
    Some((serde_json::from_slice(&buffer[..len]).unwrap(), from))
}

/// Sends a discover request to `ip:port` and returns the reply, if one comes.
async fn discover(socket: &UdpSocket, ip: Ipv4Addr, port: u16) -> Option<Json> {
    socket.send_to(DISCOVER, (ip, port)).await.unwrap();
    receive(socket, EXPECTED).await.map(|(reply, _)| reply)
}

/// Sends a datagram that must not be answered, and checks that nothing comes back.
async fn ignored(socket: &UdpSocket, port: u16, datagram: &[u8]) {
    socket
        .send_to(datagram, (Ipv4Addr::LOCALHOST, port))
        .await
        .unwrap();
    assert_eq!(
        receive(socket, QUIET).await,
        None,
        "answered {:?}",
        String::from_utf8_lossy(datagram)
    );
}

/// Whether a plain socket, without port sharing, can bind the port: true if no
/// discovery socket holds it.
fn port_is_free(port: u16) -> bool {
    std::net::UdpSocket::bind((Ipv4Addr::UNSPECIFIED, port)).is_ok()
}

/// The first non-loopback IPv4 address of the host and its interface's broadcast address.
fn network() -> Option<(Ipv4Addr, Ipv4Addr)> {
    if_addrs::get_if_addrs()
        .ok()?
        .into_iter()
        .filter(|interface| !interface.is_loopback())
        .find_map(|interface| match interface.addr {
            if_addrs::IfAddr::V4(v4) => Some((v4.ip, v4.broadcast?)),
            if_addrs::IfAddr::V6(_) => None,
        })
}

/// A socket on `port` that shares it, as a node's discovery socket does.
fn sharing_listener(port: u16) -> UdpSocket {
    let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP)).unwrap();
    #[cfg(unix)]
    socket.set_reuse_port(true).unwrap();
    #[cfg(windows)]
    socket.set_reuse_address(true).unwrap();
    socket.set_nonblocking(true).unwrap();
    socket
        .bind(&SocketAddr::from((Ipv4Addr::UNSPECIFIED, port)).into())
        .unwrap();
    UdpSocket::from_std(socket.into()).unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn answers_a_discover_request() {
    let port = free_port();
    let n = start(discoverable(port));
    let c = client(Ipv4Addr::LOCALHOST).await;
    let reply = discover(&c, Ipv4Addr::LOCALHOST, port).await.unwrap();
    assert_eq!(
        reply,
        json!({
            "SECoP": "node",
            "port": n.local_addr().port(),
            "equipment_id": "must-test",
            "firmware": FIRMWARE,
            "description": "test node",
        })
    );
    // other keys in the request are fine
    c.send_to(
        br#"{"SECoP":"discover","x":1}"#,
        (Ipv4Addr::LOCALHOST, port),
    )
    .await
    .unwrap();
    let (reply, from) = receive(&c, EXPECTED).await.unwrap();
    assert_eq!(reply["SECoP"], "node");
    assert_eq!(from.port(), port, "replies come from the discovery port");
    n.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn firmware_property_is_sent() {
    let port = free_port();
    let n = start(discoverable(port).firmware("cryo-fw 2.1"));
    let c = client(Ipv4Addr::LOCALHOST).await;
    let reply = discover(&c, Ipv4Addr::LOCALHOST, port).await.unwrap();
    assert_eq!(reply["firmware"], "cryo-fw 2.1");
    n.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ignores_other_datagrams() {
    let port = free_port();
    let n = start(discoverable(port));
    let c = client(Ipv4Addr::LOCALHOST).await;
    let announcement =
        br#"{"SECoP":"node","port":10767,"equipment_id":"x","firmware":"frappy","description":""}"#;
    // a valid request, made too long with trailing spaces
    let mut long = DISCOVER.to_vec();
    long.resize(1100, b' ');
    for datagram in [&b"hello"[..], b"[1,2]", b"\xff\xfe", announcement, &long] {
        ignored(&c, port, datagram).await;
    }
    assert!(discover(&c, Ipv4Addr::LOCALHOST, port).await.is_some());
    n.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn long_description_is_shortened() {
    let port = free_port();
    let description = "Kühler ".repeat(100);
    let n = start(
        discoverable(port)
            .property("description", description.as_str())
            .unwrap(),
    );
    let c = client(Ipv4Addr::LOCALHOST).await;
    let reply = discover(&c, Ipv4Addr::LOCALHOST, port).await.unwrap();
    let shortened = reply["description"].as_str().unwrap();
    assert!(shortened.len() > 300, "{shortened:?}");
    assert!(description.starts_with(shortened));
    n.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn too_long_identity_keeps_the_node_running() {
    let port = free_port();
    let dir = log_dir("too-long");
    let n = start(
        discoverable(port)
            .firmware(&"f".repeat(450))
            .log_files(LogFiles::default().dir(&dir)),
    );
    assert!(port_is_free(port), "no discovery socket");
    let mut c = common::Client::connect(n.local_addr()).await;
    assert!(c.request("ping x").await.starts_with("pong x"));
    n.stop().await;
    assert!(
        logged(&dir).contains("do not fit into a 508-byte discovery packet"),
        "{}",
        logged(&dir)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn loopback_bound_node_answers_local_requests() {
    let port = free_port();
    let n = start(discoverable(port).bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0))));
    let c = client(Ipv4Addr::LOCALHOST).await;
    let reply = discover(&c, Ipv4Addr::LOCALHOST, port).await.unwrap();
    assert_eq!(reply["port"], n.local_addr().port());
    n.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn loopback_bound_node_ignores_other_addresses() {
    let Some((ip, _)) = network() else {
        eprintln!("skipped: the host has no non-loopback IPv4 address");
        return;
    };
    let c = client(ip).await;
    // a node on all interfaces answers a request from that address...
    let port = free_port();
    let open = start(discoverable(port));
    assert!(discover(&c, ip, port).await.is_some());
    open.stop().await;
    // ...one on loopback does not
    let local = start(discoverable(port).bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0))));
    c.send_to(DISCOVER, (ip, port)).await.unwrap();
    assert_eq!(receive(&c, QUIET).await, None);
    local.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stopping_closes_the_socket() {
    let port = free_port();
    let n = start(discoverable(port));
    assert!(!port_is_free(port), "the node holds the discovery port");
    n.stop().await;
    let c = client(Ipv4Addr::LOCALHOST).await;
    ignored(&c, port, DISCOVER).await;
    assert!(port_is_free(port));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn turned_off_in_code() {
    let port = free_port();
    let n = start(discoverable(port).discovery(Discovery::off().port(port)));
    let c = client(Ipv4Addr::LOCALHOST).await;
    ignored(&c, port, DISCOVER).await;
    assert!(port_is_free(port));
    n.stop().await;
}

/// Run by `turned_off_through_the_environment` in a child process with
/// `MUST_DISCOVERY=0`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "run in a child process by turned_off_through_the_environment"]
async fn turned_off_child() {
    let port: u16 = std::env::var("MUST_TEST_DISCOVERY_PORT")
        .unwrap()
        .parse()
        .unwrap();
    let n = start(discoverable(port));
    let c = client(Ipv4Addr::LOCALHOST).await;
    ignored(&c, port, DISCOVER).await;
    assert!(port_is_free(port));
    n.stop().await;
}

#[test]
fn turned_off_through_the_environment() {
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--ignored",
            "--exact",
            "turned_off_child",
            "--test-threads=1",
        ])
        .env(DISCOVERY_VARIABLE, "0")
        .env("MUST_TEST_DISCOVERY_PORT", free_port().to_string())
        .status()
        .unwrap();
    assert!(status.success());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn replies_are_rate_limited() {
    let port = free_port();
    let n = start(discoverable(port));
    let c = client(Ipv4Addr::LOCALHOST).await;
    for _ in 0..200 {
        c.send_to(DISCOVER, (Ipv4Addr::LOCALHOST, port))
            .await
            .unwrap();
    }
    let mut replies = 0;
    while receive(&c, QUIET).await.is_some() {
        replies += 1;
    }
    assert!((1..=20).contains(&replies), "{replies} replies");
    // answered again once the window has passed
    tokio::time::sleep(Duration::from_millis(1100)).await;
    assert!(discover(&c, Ipv4Addr::LOCALHOST, port).await.is_some());
    n.stop().await;
}

/// On Linux a port shared with `SO_REUSEPORT` cannot be joined by a socket without it,
/// as with a Frappy node of another user; on Windows `SO_REUSEADDR` always joins.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn port_in_use_keeps_the_node_running() {
    let port = free_port();
    let _holder = std::net::UdpSocket::bind((Ipv4Addr::UNSPECIFIED, port)).unwrap();
    let dir = log_dir("in-use");
    let n = start(discoverable(port).log_files(LogFiles::default().dir(&dir)));
    let mut c = common::Client::connect(n.local_addr()).await;
    assert!(c.request("ping x").await.starts_with("pong x"));
    n.stop().await;
    assert!(
        logged(&dir).contains(&format!("cannot listen for discovery on UDP port {port}")),
        "{}",
        logged(&dir)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn startup_announcement() {
    if network().is_none() {
        eprintln!("skipped: the host has no broadcast-capable interface");
        return;
    }
    let port = free_port();
    let listener = sharing_listener(port);
    let n = start(discoverable(port).discovery(Discovery::default().port(port)));
    let (packet, _) = receive(&listener, EXPECTED).await.expect("no announcement");
    assert_eq!(packet["SECoP"], "node");
    assert_eq!(packet["port"], n.local_addr().port());
    // possibly the same packet on further interfaces
    while receive(&listener, QUIET).await.is_some() {}
    n.stop().await;

    // a node on loopback announces nothing
    let local = start(
        discoverable(port)
            .bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
            .discovery(Discovery::default().port(port)),
    );
    assert_eq!(receive(&listener, QUIET).await, None);
    local.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn broadcast_reaches_two_nodes_of_one_process() {
    let Some((_, broadcast)) = network() else {
        eprintln!("skipped: the host has no broadcast-capable interface");
        return;
    };
    let port = free_port();
    let first = start(discoverable(port));
    let second = start(discoverable(port));
    let c = client(Ipv4Addr::UNSPECIFIED).await;
    c.send_to(DISCOVER, (broadcast, port)).await.unwrap();
    let mut ports = Vec::new();
    while let Some((reply, _)) = receive(&c, Duration::from_secs(1)).await {
        ports.push(reply["port"].as_u64().unwrap());
    }
    ports.sort_unstable();
    let mut expected = vec![
        u64::from(first.local_addr().port()),
        u64::from(second.local_addr().port()),
    ];
    expected.sort_unstable();
    assert_eq!(ports, expected);
    first.stop().await;
    second.stop().await;
}

fn log_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("must-discovery-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// Everything the node logged to its files.
fn logged(dir: &std::path::Path) -> String {
    let mut text = String::new();
    for entry in walk(dir) {
        text += &std::fs::read_to_string(entry).unwrap_or_default();
    }
    text
}

fn walk(dir: &std::path::Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .flat_map(|e| {
            let path = e.path();
            if path.is_dir() {
                walk(&path)
            } else {
                vec![path]
            }
        })
        .collect()
}
