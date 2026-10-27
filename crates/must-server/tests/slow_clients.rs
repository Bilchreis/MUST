// SPDX-License-Identifier: LGPL-3.0-or-later

//! The slow-client scenarios of specs/sec-node-runtime.
//!
//! A stuck client is simulated by a connection that is never read. Its socket receive
//! buffer is made small so that the kernel absorbs little and the node's own queue is
//! what fills up.

mod common;

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use common::{Client, Sim, node, parse};
use must_server::{Module, NodeBuilder, NodeEvent, Parameter, RunningNode};
use must_types::{DataInfo, Value};
use tokio::io::AsyncWriteExt;
use tokio::net::{TcpSocket, TcpStream};
use tokio::runtime::Handle;

const MIB: usize = 1 << 20;

/// A module `s` with a cached string parameter for large updates.
fn strings() -> Module {
    let s = DataInfo::string().build().unwrap();
    Module::new("s", "large values")
        .interface_classes([])
        .parameter(
            Parameter::new("text", s, "text")
                .cached()
                .initial_value(Value::String(String::new())),
        )
        .unwrap()
}

fn start(builder: NodeBuilder) -> RunningNode {
    builder.build().unwrap().spawn(&Handle::current()).unwrap()
}

/// A connection that is never read, with a small receive buffer.
async fn stuck_client(addr: SocketAddr, activate: bool) -> TcpStream {
    let socket = TcpSocket::new_v4().unwrap();
    socket.set_recv_buffer_size(4096).unwrap();
    let mut stream = socket.connect(addr).await.unwrap();
    if activate {
        stream.write_all(b"activate\n").await.unwrap();
    }
    stream
}

/// Waits for a `ClientDisconnected` event with the given reason.
async fn disconnected(
    events: &mut tokio::sync::broadcast::Receiver<NodeEvent>,
    reason: &str,
) -> bool {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            match events.recv().await {
                Ok(NodeEvent::ClientDisconnected { reason: r, .. }) if r == reason => return true,
                Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(_) => return false,
            }
        }
    })
    .await
    .unwrap_or(false)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stuck_client_does_not_delay_others() {
    let n = start(node(Arc::new(Sim::default())));
    let _stuck = stuck_client(n.local_addr(), true).await;
    let mut good = Client::connect(n.local_addr()).await;
    good.send("activate").await;
    while good.line().await != "active" {}

    for i in 0..100 {
        let pushed = Instant::now();
        n.update("t1", "value", Value::Double(f64::from(i)))
            .unwrap();
        let line = good.line().await;
        assert!(
            line.starts_with(&format!("update t1:value [{i}.0,")),
            "{line}"
        );
        assert!(
            pushed.elapsed() < Duration::from_millis(200),
            "update {i} delayed"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    good.send("change t1:target 5").await;
    loop {
        let line = good.line().await;
        if line.starts_with("changed t1:target [5.0,") {
            break;
        }
        assert!(line.starts_with("update "), "{line}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stuck_client_is_disconnected_and_can_reconnect() {
    let n = start(
        node(Arc::new(Sim::default()))
            .module(strings())
            .unwrap()
            .max_outgoing_bytes(MIB),
    );
    let mut events = n.subscribe();
    let stuck = stuck_client(n.local_addr(), true).await;

    // push 64 KiB updates until the node gives up on the stuck client
    let chunk = "x".repeat(64 * 1024);
    let pusher = n.clone();
    let push = tokio::spawn(async move {
        let mut bytes_sent = 0usize;
        loop {
            if pusher
                .update("s", "text", Value::String(chunk.clone()))
                .is_err()
            {
                break bytes_sent;
            }
            bytes_sent += chunk.len();
            tokio::time::sleep(Duration::from_millis(1)).await;
            if bytes_sent > 64 * MIB {
                break bytes_sent;
            }
        }
    });
    assert!(
        disconnected(&mut events, "too much unread data").await,
        "stuck client not disconnected"
    );
    push.abort();
    drop(stuck);

    // reconnecting and activating again gives the current values
    let mut c = Client::connect(n.local_addr()).await;
    c.send("activate").await;
    let mut initial = Vec::new();
    loop {
        let line = c.line().await;
        if line == "active" {
            break;
        }
        initial.push(parse(&line).1);
    }
    assert!(initial.contains(&"t1:value".to_owned()) && initial.contains(&"s:text".to_owned()));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn configured_limit_is_used() {
    for (limit, max_pushed) in [(MIB, 8 * MIB), (16 * MIB, 32 * MIB)] {
        let n = start(
            node(Arc::new(Sim::default()))
                .module(strings())
                .unwrap()
                .max_outgoing_bytes(limit),
        );
        let mut events = n.subscribe();
        let _stuck = stuck_client(n.local_addr(), true).await;
        tokio::time::sleep(Duration::from_millis(100)).await;
        let chunk = "x".repeat(64 * 1024);
        let mut pushed = 0usize;
        let closed = loop {
            n.update("s", "text", Value::String(chunk.clone())).unwrap();
            pushed += chunk.len();
            tokio::task::yield_now().await;
            let mut closed = false;
            while let Ok(event) = events.try_recv() {
                closed |= matches!(event, NodeEvent::ClientDisconnected { ref reason, .. } if reason == "too much unread data");
            }
            if closed || pushed > max_pushed {
                break closed;
            }
        };
        assert!(
            closed,
            "limit {limit}: not disconnected after {pushed} bytes"
        );
        assert!(
            pushed >= limit,
            "limit {limit}: disconnected after only {pushed} bytes"
        );
        n.stop().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn large_reply_while_updates_are_queued() {
    let sim = Arc::new(Sim {
        big_len: 50 * MIB,
        ..Sim::default()
    });
    let n = start(node(sim));
    let mut c = Client::connect(n.local_addr()).await;
    c.send("activate").await;
    while c.line().await != "active" {}

    // updates keep coming while the 50 MiB reply is queued and written
    let pusher = n.clone();
    let push = tokio::spawn(async move {
        for i in 0..200 {
            pusher
                .update("t1", "value", Value::Double(f64::from(i)))
                .unwrap();
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    });
    c.send("do t1:big").await;
    let done = loop {
        let line = c.line().await;
        if line.starts_with("done ") {
            break line;
        }
        assert!(
            line.starts_with("update t1:value"),
            "{}",
            &line[..line.len().min(80)]
        );
    };
    let (_, spec, data) = parse(&done);
    assert_eq!(spec, "t1:big");
    assert_eq!(data[0].as_str().unwrap().len(), 50 * MIB);
    push.await.unwrap();
    // still connected
    c.send("ping alive").await;
    loop {
        let line = c.line().await;
        if line.starts_with("pong alive ") {
            break;
        }
        assert!(line.starts_with("update "), "{line}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn requests_without_reading_replies() {
    const ROUNDS: usize = 6000;
    let sim = Arc::new(Sim::default());
    let n = start(node(Arc::clone(&sim)).max_outgoing_bytes(64 * 1024));

    // each round: a read answered with 2 KiB (counted by the handler) and a ping
    let flooder = stuck_client(n.local_addr(), false).await;
    let (mut reader, mut writer) = flooder.into_split();
    let send = tokio::spawn(async move {
        for i in 0..ROUNDS {
            writer
                .write_all(format!("read t1:chunk\nping {i}\n").as_bytes())
                .await
                .unwrap();
        }
        writer
    });

    let reads = || sim.reads.load(std::sync::atomic::Ordering::SeqCst);
    tokio::time::sleep(Duration::from_secs(1)).await;
    let first = reads();
    tokio::time::sleep(Duration::from_millis(500)).await;
    let later = reads();
    assert!(
        first > 0 && first < ROUNDS,
        "node handled {first} of {ROUNDS} reads"
    );
    assert_eq!(
        later, first,
        "node kept taking requests while the client did not read"
    );

    // other clients are unaffected, and the flooding connection stays open
    let mut other = Client::connect(n.local_addr()).await;
    assert!(other.request("ping other").await.starts_with("pong other "));

    // once the client reads, all replies arrive in order
    let mut lines = tokio::io::BufReader::new(&mut reader);
    let mut expected_ping = 0;
    let mut line = String::new();
    while expected_ping < ROUNDS {
        line.clear();
        let n_read = tokio::time::timeout(
            Duration::from_secs(10),
            tokio::io::AsyncBufReadExt::read_line(&mut lines, &mut line),
        )
        .await
        .expect("replies stopped")
        .unwrap();
        assert!(n_read > 0, "connection closed");
        if line.starts_with("pong ") {
            assert!(
                line.starts_with(&format!("pong {expected_ping} ")),
                "out of order: {line}"
            );
            expected_ping += 1;
        } else {
            assert!(line.starts_with("reply t1:chunk "), "{line}");
        }
    }
    send.await.unwrap();
    assert_eq!(
        reads(),
        ROUNDS,
        "all requests handled after the client read"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn large_describe_is_not_a_slow_client() {
    let huge = Module::new("h", &"d".repeat(2 * MIB)).interface_classes([]);
    let n = start(node(Arc::new(Sim::default())).module(huge).unwrap());
    let mut c = Client::connect(n.local_addr()).await;
    let line = c.request("describe").await;
    assert!(line.len() > 2 * MIB && line.ends_with('}'));
    assert!(c.request("ping after").await.starts_with("pong after "));
}
