// SPDX-License-Identifier: LGPL-3.0-or-later

//! The monitoring endpoint, read with the `must-monitor` client library: one endpoint per
//! process, snapshot, events, traffic, coalesced updates, lagging monitors, log capture.
//!
//! The endpoint is a process-wide singleton, so the tests run one at a time and use
//! their own registry directory.

// the guard serializes whole tests, each on its own runtime
#![allow(clippy::await_holding_lock)]

mod common;

use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::{Arc, LazyLock, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use common::{Client, Sim, node};
use must_monitor::client::{self, Connection, Options};
use must_monitor::protocol::{
    Direction, Event, NodeState, PROTOCOL_MAJOR, ServerMessage, UpdateMode,
};
use must_monitor::registry::{EndpointDescription, EndpointStatus, Registration};
use must_monitor::transport;
use must_server::RunningNode;
use must_types::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::runtime::Handle;
use tracing_subscriber::layer::SubscriberExt;

static SERIAL: Mutex<()> = Mutex::new(());

static DIR: LazyLock<PathBuf> = LazyLock::new(|| {
    let dir = std::env::temp_dir().join(format!("must-monitor-it-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
});

fn serial() -> MutexGuard<'static, ()> {
    let guard = SERIAL.lock().unwrap_or_else(PoisonError::into_inner);
    must_server::monitor::set_registry_dir(Some(DIR.clone()));
    must_server::monitor::set_enabled(true);
    guard
}

/// The registration of this process, if any.
fn own_registration() -> Option<Registration> {
    if !DIR.exists() {
        return None;
    }
    client::list_endpoints_in(&DIR)
        .unwrap()
        .into_iter()
        .find(|l| l.registration.pid == std::process::id())
        .map(|l| {
            assert_eq!(l.status, EndpointStatus::Live);
            l.registration
        })
}

fn start(sim: Arc<Sim>) -> RunningNode {
    node(sim)
        .build()
        .unwrap()
        .spawn(&Handle::current())
        .unwrap()
}

async fn monitor(options: Options) -> Connection {
    let registration = own_registration().expect("the endpoint is registered");
    client::connect(&registration, &options).await.unwrap()
}

/// Reads messages until `pick` returns something, with a timeout.
async fn wait_for<T>(
    connection: &mut Connection,
    mut pick: impl FnMut(&ServerMessage) -> Option<T>,
) -> T {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let message = connection.next().await.unwrap().expect("connection open");
            if let Some(t) = pick(&message) {
                return t;
            }
        }
    })
    .await
    .expect("expected message")
}

/// Collects the messages arriving within `duration`.
async fn collect_for(connection: &mut Connection, duration: Duration) -> Vec<ServerMessage> {
    let mut messages = Vec::new();
    let end = tokio::time::Instant::now() + duration;
    while let Ok(Ok(Some(message))) = tokio::time::timeout_at(end, connection.next()).await {
        messages.push(message);
    }
    messages
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn two_nodes_one_endpoint_and_last_node_stops() {
    let _serial = serial();
    let a = start(Arc::new(Sim::default()));
    let b = start(Arc::new(Sim::default()));
    let listed = client::list_endpoints_in(&DIR).unwrap();
    let own: Vec<_> = listed
        .iter()
        .filter(|l| l.registration.pid == std::process::id())
        .collect();
    assert_eq!(own.len(), 1, "exactly one endpoint for the process");
    #[cfg(unix)]
    {
        let EndpointDescription::Unix { path } = &own[0].registration.endpoint else {
            panic!("{:?}", own[0].registration.endpoint)
        };
        assert_eq!(path, &DIR.join(format!("{}.sock", std::process::id())));
    }
    #[cfg(windows)]
    assert!(matches!(
        own[0].registration.endpoint,
        EndpointDescription::Pipe { .. }
    ));

    let mut m = monitor(Options::new("test")).await;
    let nodes = &m.snapshot().nodes;
    assert_eq!(nodes.len(), 2);
    assert!(nodes.iter().all(|n| n.state == NodeState::Running));
    assert_eq!(m.process().pid, std::process::id());

    a.stop().await;
    let stopped = nodes[0].id;
    wait_for(&mut m, |msg| {
        matches!(msg, ServerMessage::Event(Event::NodeRemoved { node }) if *node == stopped)
            .then_some(())
    })
    .await;
    assert!(own_registration().is_some(), "one node left");

    b.stop().await;
    // the endpoint closes: the connection ends and the registration is gone
    tokio::time::timeout(Duration::from_secs(10), async {
        while m.next().await.ok().flatten().is_some() {}
    })
    .await
    .unwrap();
    assert!(own_registration().is_none());
    let leftovers: Vec<_> = std::fs::read_dir(&*DIR)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with(&std::process::id().to_string())
        })
        .collect();
    assert!(leftovers.is_empty(), "registration and socket removed");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn snapshot_content_without_hardware_reads() {
    let _serial = serial();
    let sim = Arc::new(Sim::default());
    let n = start(Arc::clone(&sim));
    let mut c = Client::connect(n.local_addr()).await;
    c.request("read t1:value").await;
    let reads = sim.reads.load(Ordering::SeqCst);

    let mut m = monitor(Options::new("test")).await;
    assert_eq!(
        sim.reads.load(Ordering::SeqCst),
        reads,
        "no reads for the snapshot"
    );
    let node = m.snapshot().nodes[0].clone();
    assert_eq!(node.equipment_id, "must-test");
    assert_eq!(node.address, Some(n.local_addr().to_string()));
    assert!(node.structure.as_ref().unwrap()["modules"]["t1"].is_object());
    let value = node
        .parameters
        .iter()
        .find(|p| p.module == "t1" && p.parameter == "value")
        .unwrap();
    assert_eq!(value.report[0], 295.0);
    assert_eq!(node.clients.len(), 1);
    assert!(node.clients[0].peer.starts_with("127.0.0.1:"));
    assert!(node.clients[0].connected_at > 1.0e9);
    assert_eq!(node.context, None);
    drop(c);
    wait_for(&mut m, |msg| {
        matches!(msg, ServerMessage::Event(Event::ClientDisconnected { .. })).then_some(())
    })
    .await;
    n.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn traffic_only_when_subscribed_and_counters_always() {
    let _serial = serial();
    let sim = Arc::new(Sim {
        big_len: 200 * 1024,
        ..Sim::default()
    });
    let n = start(sim);
    let mut m = monitor(Options::new("test")).await;
    let node = m.snapshot().nodes[0].id;
    let mut c = Client::connect(n.local_addr()).await;
    let client = wait_for(&mut m, |msg| match msg {
        ServerMessage::Event(Event::ClientConnected { client, .. }) => Some(client.id),
        _ => None,
    })
    .await;

    // not subscribed: counters, but no messages
    c.request("read t1:value").await;
    let messages = collect_for(&mut m, Duration::from_millis(1500)).await;
    assert!(
        !messages
            .iter()
            .any(|msg| matches!(msg, ServerMessage::Event(Event::Message { .. })))
    );
    assert!(messages.iter().any(|msg| matches!(
        msg,
        ServerMessage::Counters { node: n, rx, tx, .. } if *n == node && *rx >= 1 && *tx >= 1
    )));

    // subscribed: the request and its reply, naming the client
    m.subscribe_traffic(node, true).await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    c.request("read t1:value").await;
    let (direction, text) = wait_for(&mut m, |msg| match msg {
        ServerMessage::Event(Event::Message {
            client: cl,
            direction,
            text,
            ..
        }) if *cl == client => Some((*direction, text.clone())),
        _ => None,
    })
    .await;
    assert_eq!(
        (direction, text.as_str()),
        (Direction::Received, "read t1:value")
    );
    let reply = wait_for(&mut m, |msg| match msg {
        ServerMessage::Event(Event::Message {
            direction: Direction::Sent,
            text,
            ..
        }) => Some(text.clone()),
        _ => None,
    })
    .await;
    assert!(reply.starts_with("reply t1:value "), "{reply}");

    // a long reply is cut to 4 KiB
    c.request("do t1:big").await;
    let (len, truncated) = wait_for(&mut m, |msg| match msg {
        ServerMessage::Event(Event::Message {
            direction: Direction::Sent,
            text,
            truncated,
            ..
        }) if text.starts_with("done t1:big") => Some((text.len(), *truncated)),
        _ => None,
    })
    .await;
    assert!(truncated && len <= 4096 && len > 4000);

    drop(c);
    let reason = wait_for(&mut m, |msg| match msg {
        ServerMessage::Event(Event::ClientDisconnected { reason, .. }) => Some(reason.clone()),
        _ => None,
    })
    .await;
    assert_eq!(reason, "closed by client");
    n.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn coalesced_and_every_update() {
    let _serial = serial();
    let n = start(Arc::new(Sim::default()));
    let mut m = monitor(Options {
        client: "test".into(),
        update_interval: Some(0.3),
    })
    .await;
    let node = m.snapshot().nodes[0].id;

    // 50 updates within a few ms: about one update, with the last value
    for i in 0..50 {
        n.update("t1", "value", Value::Double(f64::from(i)))
            .unwrap();
    }
    let updates: Vec<f64> = collect_for(&mut m, Duration::from_millis(1000))
        .await
        .iter()
        .filter_map(|msg| match msg {
            ServerMessage::Event(Event::Update { state, .. }) if state.parameter == "value" => {
                state.report[0].as_f64()
            }
            _ => None,
        })
        .collect();
    assert!(!updates.is_empty() && updates.len() <= 2, "{updates:?}");
    assert_eq!(updates.last(), Some(&49.0), "the last value is not lost");

    // every update for this node
    m.subscribe_updates(node, UpdateMode::All).await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    for i in 100..110 {
        n.update("t1", "value", Value::Double(f64::from(i)))
            .unwrap();
    }
    let updates: Vec<f64> = collect_for(&mut m, Duration::from_millis(500))
        .await
        .iter()
        .filter_map(|msg| match msg {
            ServerMessage::Event(Event::Update { state, .. }) if state.parameter == "value" => {
                state.report[0].as_f64()
            }
            _ => None,
        })
        .collect();
    assert_eq!(updates, (100..110).map(f64::from).collect::<Vec<_>>());
    n.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stalled_monitor_does_not_delay_the_node() {
    let _serial = serial();
    let n = start(Arc::new(Sim::default()));
    let mut m = monitor(Options::new("test")).await;
    let node = m.snapshot().nodes[0].id;
    m.subscribe_updates(node, UpdateMode::All).await.unwrap();
    m.subscribe_traffic(node, true).await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;

    // the monitor reads nothing while the node publishes 20000 updates
    let mut c = Client::connect(n.local_addr()).await;
    let start_time = Instant::now();
    for i in 0..20_000 {
        n.update("t1", "value", Value::Double(f64::from(i % 300)))
            .unwrap();
    }
    let ping = Instant::now();
    c.request("ping x").await;
    assert!(
        ping.elapsed() < Duration::from_millis(500),
        "ping took {:?}",
        ping.elapsed()
    );
    assert!(start_time.elapsed() < Duration::from_secs(10));

    // later the monitor learns that events were dropped
    let dropped = wait_for(&mut m, |msg| match msg {
        ServerMessage::Lagged { dropped } => Some(*dropped),
        _ => None,
    })
    .await;
    assert!(dropped > 0);
    n.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hello_first_and_version() {
    let _serial = serial();
    let n = start(Arc::new(Sim::default()));
    let registration = own_registration().unwrap();

    // no hello: closed without any data
    let mut raw = transport::connect(&registration.endpoint, registration.pid)
        .await
        .unwrap();
    raw.write_all(b"{\"type\":\"subscribe_traffic\",\"node\":1,\"enabled\":true}\n")
        .await
        .unwrap();
    let mut buf = Vec::new();
    tokio::time::timeout(Duration::from_secs(5), raw.read_to_end(&mut buf))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(buf, b"");

    // major version 99: an error naming the supported version, then closed
    let mut raw = transport::connect(&registration.endpoint, registration.pid)
        .await
        .unwrap();
    raw.write_all(b"{\"type\":\"hello\",\"version\":\"99.0\",\"client\":\"x\"}\n")
        .await
        .unwrap();
    let mut buf = String::new();
    tokio::time::timeout(Duration::from_secs(5), raw.read_to_string(&mut buf))
        .await
        .unwrap()
        .unwrap();
    let message: ServerMessage = serde_json::from_str(buf.trim_end()).unwrap();
    let ServerMessage::Error { message } = message else {
        panic!("{buf}")
    };
    assert!(message.contains(&format!("{PROTOCOL_MAJOR}.")), "{message}");
    n.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn two_monitors_see_the_same_events() {
    let _serial = serial();
    let n = start(Arc::new(Sim::default()));
    let mut a = monitor(Options::new("a")).await;
    let mut b = monitor(Options::new("b")).await;
    let c = Client::connect(n.local_addr()).await;
    for m in [&mut a, &mut b] {
        let peer = wait_for(m, |msg| match msg {
            ServerMessage::Event(Event::ClientConnected { client, .. }) => {
                Some(client.peer.clone())
            }
            _ => None,
        })
        .await;
        assert!(peer.starts_with("127.0.0.1:"));
    }
    drop(c);
    n.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn handler_error_is_logged_with_node_and_module() {
    let _serial = serial();
    let n = start(Arc::new(Sim::default()));
    let mut m = monitor(Options::new("test")).await;
    let node = m.snapshot().nodes[0].id;
    let mut c = Client::connect(n.local_addr()).await;
    c.request("read t1:broken").await;
    let record = wait_for(&mut m, |msg| match msg {
        ServerMessage::Event(Event::Log(record)) if record.node == Some(node) => {
            Some(record.clone())
        }
        _ => None,
    })
    .await;
    assert_eq!(record.level, "WARN");
    assert_eq!(record.module.as_deref(), Some("t1"));
    // buffered for later monitors
    let later = monitor(Options::new("later")).await;
    assert!(later.snapshot().logs.contains(&record));
    n.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn host_tracing_records_are_captured_once() {
    let _serial = serial();
    // a host with its own subscriber adds the layer; the library does not replace it
    let subscriber = tracing_subscriber::registry().with(must_server::monitor::log_layer());
    let _ = tracing::subscriber::set_global_default(subscriber);
    must_server::monitor::install_log_capture();

    let n = start(Arc::new(Sim::default()));
    let mut m = monitor(Options::new("test")).await;
    tracing::info!(answer = 42, "host says hello");
    let record = wait_for(&mut m, |msg| match msg {
        ServerMessage::Event(Event::Log(record)) if record.message.contains("host says hello") => {
            Some(record.clone())
        }
        _ => None,
    })
    .await;
    assert_eq!(record.level, "INFO");
    assert!(record.message.contains("answer=42"), "{}", record.message);
    assert_eq!(record.node, None);
    // node records arrive once, with their node, not again through the layer
    let mut c = Client::connect(n.local_addr()).await;
    c.request("read t1:broken").await;
    let messages = collect_for(&mut m, Duration::from_millis(500)).await;
    let broken: Vec<_> = messages
        .iter()
        .filter(|msg| {
            matches!(msg, ServerMessage::Event(Event::Log(r)) if r.message.contains("broken"))
        })
        .collect();
    assert_eq!(broken.len(), 1, "{broken:?}");
    n.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn disabled_through_the_api() {
    let _serial = serial();
    must_server::monitor::set_enabled(false);
    let n = start(Arc::new(Sim::default()));
    assert!(own_registration().is_none());
    let mut c = Client::connect(n.local_addr()).await;
    assert!(c.request("ping x").await.starts_with("pong x"));
    n.stop().await;
    must_server::monitor::set_enabled(true);
}

/// Run by `disabled_through_the_environment` in a child process with `MUST_MONITOR=0`.
#[tokio::test]
#[ignore = "run in a child process by disabled_through_the_environment"]
async fn disabled_child() {
    let dir = PathBuf::from(std::env::var("MUST_TEST_REGISTRY").unwrap());
    must_server::monitor::set_registry_dir(Some(dir.clone()));
    let n = start(Arc::new(Sim::default()));
    let own = format!("{}.json", std::process::id());
    assert!(
        !dir.join(own).exists(),
        "no registration with MUST_MONITOR=0"
    );
    let mut c = Client::connect(n.local_addr()).await;
    assert!(c.request("ping x").await.starts_with("pong x"));
    n.stop().await;
}

#[test]
fn disabled_through_the_environment() {
    let dir = std::env::temp_dir().join(format!("must-monitor-env-{}", std::process::id()));
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--ignored", "--exact", "disabled_child", "--test-threads=1"])
        .env("MUST_MONITOR", "0")
        .env("MUST_TEST_REGISTRY", &dir)
        .status()
        .unwrap();
    assert!(status.success());
    let _ = std::fs::remove_dir_all(&dir);
}

/// The TCP and UDP ports this process listens on.
#[cfg(target_os = "linux")]
fn own_listening_ports() -> Vec<u16> {
    // inodes of our sockets
    let inodes: std::collections::HashSet<String> = std::fs::read_dir("/proc/self/fd")
        .unwrap()
        .filter_map(|e| std::fs::read_link(e.ok()?.path()).ok())
        .filter_map(|link| {
            let link = link.to_string_lossy().into_owned();
            Some(link.strip_prefix("socket:[")?.strip_suffix(']')?.to_owned())
        })
        .collect();
    let mut ports = Vec::new();
    for (table, listening) in [
        ("/proc/net/tcp", Some("0A")),
        ("/proc/net/tcp6", Some("0A")),
        ("/proc/net/udp", None),
        ("/proc/net/udp6", None),
    ] {
        let Ok(text) = std::fs::read_to_string(table) else {
            continue;
        };
        for line in text.lines().skip(1) {
            let fields: Vec<&str> = line.split_whitespace().collect();
            if fields.len() < 10 || !inodes.contains(fields[9]) {
                continue;
            }
            if listening.is_some_and(|state| fields[3] != state) {
                continue;
            }
            let port = fields[1].rsplit(':').next().unwrap();
            ports.push(u16::from_str_radix(port, 16).unwrap());
        }
    }
    ports.sort_unstable();
    ports
}

/// The TCP and UDP ports this process listens on.
#[cfg(windows)]
fn own_listening_ports() -> Vec<u16> {
    let pid = std::process::id().to_string();
    let mut ports = Vec::new();
    for protocol in ["TCP", "TCPv6", "UDP", "UDPv6"] {
        let output = std::process::Command::new("netstat")
            .args(["-ano", "-p", protocol])
            .output()
            .unwrap();
        for line in String::from_utf8_lossy(&output.stdout).lines() {
            let fields: Vec<&str> = line.split_whitespace().collect();
            let tcp = protocol.starts_with("TCP");
            let wanted = if tcp {
                fields.len() == 5 && fields[3] == "LISTENING" && fields[4] == pid
            } else {
                fields.len() == 4 && fields[3] == pid
            };
            if wanted && let Some(port) = fields[1].rsplit(':').next() {
                ports.push(port.parse().unwrap());
            }
        }
    }
    ports.sort_unstable();
    ports
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn not_on_the_network() {
    let _serial = serial();
    let n = start(Arc::new(Sim::default()));
    let m = monitor(Options::new("test")).await;
    assert_eq!(m.snapshot().nodes.len(), 1);
    assert_eq!(
        own_listening_ports(),
        [n.local_addr().port()],
        "only the SECoP port, no monitoring port"
    );
    n.stop().await;
}
