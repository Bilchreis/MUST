// SPDX-License-Identifier: LGPL-3.0-or-later

//! End-to-end tests of the scenarios in specs/sec-node-runtime (except slow clients).

mod common;

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use common::{Client, Sim, error_class, kelvin, node, parse, sensor, status};
use must_server::{Module, NodeBuilder, NodeEvent, Parameter, RunningNode, StartError};
use must_types::Value;
use serde_json::json;
use tokio::runtime::Handle;

fn start(builder: NodeBuilder) -> RunningNode {
    builder.build().unwrap().spawn(&Handle::current()).unwrap()
}

async fn activate(c: &mut Client) {
    c.send("activate").await;
    while c.line().await != "active" {}
}

// --- 4.1 module actor: handlers, cache, timeouts, panics

#[tokio::test]
async fn handler_error_reaches_the_client() {
    let n = start(node(Arc::new(Sim::default())));
    let mut c = Client::connect(n.local_addr()).await;
    let (action, spec, data) = parse(&c.request("read t1:nocomm").await);
    assert_eq!(
        (action.as_str(), spec.as_str()),
        ("error_read", "t1:nocomm")
    );
    assert_eq!(data[0], "CommunicationFailed");
    assert_eq!(data[1], "no answer from controller");
}

#[tokio::test]
async fn failed_read_becomes_error_update() {
    let n = start(node(Arc::new(Sim::default())));
    let mut watcher = Client::connect(n.local_addr()).await;
    activate(&mut watcher).await;
    let mut c = Client::connect(n.local_addr()).await;
    c.request("read t1:broken").await;
    let line = watcher.line().await;
    assert!(
        line.starts_with("error_update t1:broken [\"HardwareError\""),
        "{line}"
    );
}

#[tokio::test]
async fn activation_does_not_hit_the_hardware() {
    let sim = Arc::new(Sim::default());
    let n = start(node(Arc::clone(&sim)));
    let mut c = Client::connect(n.local_addr()).await;
    activate(&mut c).await;
    assert_eq!(sim.reads.load(Ordering::SeqCst), 0);
    // a read of a normal parameter does call the handler, and updates the cache
    c.request("read t1:value").await;
    assert_eq!(sim.reads.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn cached_and_constant_parameters() {
    let sim = Arc::new(Sim::default());
    let n = start(node(Arc::clone(&sim)));
    let mut c = Client::connect(n.local_addr()).await;
    let (action, _, data) = parse(&c.request("read t1:serial").await);
    assert_eq!((action.as_str(), &data[0]), ("reply", &json!("SN-1")));
    let (_, _, data) = parse(&c.request("read t1:target_max").await);
    assert_eq!(data[0], json!(200.0));
    let (_, _, data) = parse(&c.request("read t2:value").await);
    assert_eq!(data[0], json!(4.2));
    assert_eq!(
        sim.reads.load(Ordering::SeqCst),
        0,
        "no handler call for constant or cached"
    );
}

#[tokio::test]
async fn startup_reads_fill_the_cache() {
    let sim = Arc::new(Sim::default());
    let module = Module::new("m", "x")
        .interface_classes(["Readable"])
        .parameter(Parameter::new("value", kelvin(), "v"))
        .unwrap()
        .parameter(Parameter::new("status", common::status_info(), "s"))
        .unwrap()
        .handler_arc(Arc::clone(&sim) as Arc<dyn must_server::ModuleHandler>);
    let n = start(
        NodeBuilder::new("x", "y")
            .log_files(must_server::logfiles::LogFiles::off())
            .bind(SocketAddr::from(([127, 0, 0, 1], 0)))
            .module(module)
            .unwrap(),
    );
    let mut events = n.subscribe();
    let mut seen = 0;
    while seen < 2 {
        if let NodeEvent::Updated { .. } = events.recv().await.unwrap() {
            seen += 1;
        }
    }
    let mut c = Client::connect(n.local_addr()).await;
    c.send("activate").await;
    assert!(c.line().await.starts_with("update m:value [295.0,{\"t\":"));
    assert!(
        c.line()
            .await
            .starts_with("update m:status [[100,\"idle\"],")
    );
    assert_eq!(c.line().await, "active");
}

#[tokio::test]
async fn hanging_handler_times_out() {
    let n = start(node(Arc::new(Sim::default())).timeout(0.5));
    let mut c = Client::connect(n.local_addr()).await;
    let mut other = Client::connect(n.local_addr()).await;
    let started = Instant::now();
    c.send("read t1:hang").await;
    // a concurrent ping on another connection is answered immediately
    other.send("ping 1").await;
    assert!(other.line().await.starts_with("pong 1 "));
    assert!(started.elapsed() < Duration::from_millis(200));
    let line = c.line().await;
    assert_eq!(error_class(&line), "TimeoutError");
    let elapsed = started.elapsed();
    assert!(
        elapsed >= Duration::from_millis(450) && elapsed < Duration::from_secs(2),
        "{elapsed:?}"
    );
    // the module works again afterwards
    assert!(
        c.request("read t1:value")
            .await
            .starts_with("reply t1:value")
    );
}

#[tokio::test]
async fn panicking_handler_becomes_internal_error() {
    let n = start(node(Arc::new(Sim::default())));
    let mut c = Client::connect(n.local_addr()).await;
    assert_eq!(
        error_class(&c.request("read t1:panic").await),
        "InternalError"
    );
    assert!(
        c.request("read t1:value")
            .await
            .starts_with("reply t1:value")
    );
}

#[tokio::test]
async fn two_modules_in_parallel() {
    let sim = Arc::new(Sim::default());
    let slow_module = Module::new("a", "slow")
        .interface_classes([])
        .parameter(Parameter::new("slow", kelvin(), "x"))
        .unwrap()
        .handler_arc(Arc::clone(&sim) as Arc<dyn must_server::ModuleHandler>);
    let n = start(
        NodeBuilder::new("x", "y")
            .log_files(must_server::logfiles::LogFiles::off())
            .bind(SocketAddr::from(([127, 0, 0, 1], 0)))
            .initial_read(false)
            .module(slow_module)
            .unwrap()
            .module(sensor("b"))
            .unwrap(),
    );
    let mut c1 = Client::connect(n.local_addr()).await;
    let mut c2 = Client::connect(n.local_addr()).await;
    c1.send("read a:slow").await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    let started = Instant::now();
    assert!(
        c2.request("read b:value")
            .await
            .starts_with("reply b:value")
    );
    assert!(
        started.elapsed() < Duration::from_millis(200),
        "b waited for a"
    );
    assert!(c1.line().await.starts_with("reply a:slow"));
}

#[tokio::test]
async fn requests_of_one_connection_in_order() {
    let n = start(node(Arc::new(Sim::default())));
    let mut c = Client::connect(n.local_addr()).await;
    c.send("read t1:slow").await;
    c.send("ping 2").await;
    assert!(c.line().await.starts_with("reply t1:slow"));
    assert!(c.line().await.starts_with("pong 2 "));
}

// --- 4.2 ordering

#[tokio::test]
async fn status_update_before_changed() {
    let n = start(node(Arc::new(Sim::default())));
    let mut watcher = Client::connect(n.local_addr()).await;
    activate(&mut watcher).await;
    let mut c = Client::connect(n.local_addr()).await;
    activate(&mut c).await;
    c.send("change t1:target 5").await;
    for client in [&mut c, &mut watcher] {
        assert!(
            client
                .line()
                .await
                .starts_with("update t1:status [[300,\"ramping\"],")
        );
        assert!(client.line().await.starts_with("update t1:target [5.0,"));
    }
    assert!(c.line().await.starts_with("changed t1:target [5.0,"));
}

#[tokio::test]
async fn command_side_effects_before_done() {
    let n = start(node(Arc::new(Sim::default())));
    let mut c = Client::connect(n.local_addr()).await;
    activate(&mut c).await;
    c.send("do t1:stop").await;
    assert!(
        c.line()
            .await
            .starts_with("update t1:status [[100,\"stopped\"],")
    );
    assert!(c.line().await.starts_with("done t1:stop [null,"));
}

#[tokio::test]
async fn initial_updates_before_active() {
    let n = start(node(Arc::new(Sim::default())));
    let pusher = n.clone();
    // keep pushing while activating; `active` must still come after all initial updates
    let push = tokio::spawn(async move {
        for i in 0..200 {
            pusher
                .update("t2", "value", Value::Double(f64::from(i)))
                .unwrap();
            tokio::task::yield_now().await;
        }
    });
    let mut c = Client::connect(n.local_addr()).await;
    c.send("activate").await;
    let mut seen_before_active = std::collections::HashSet::new();
    loop {
        let line = c.line().await;
        if line == "active" {
            break;
        }
        let (_, spec, _) = parse(&line);
        seen_before_active.insert(spec);
    }
    for spec in [
        "t1:value",
        "t1:status",
        "t1:target",
        "t1:target_max",
        "t2:value",
        "t2:status",
    ] {
        assert!(
            seen_before_active.contains(spec),
            "{spec} missing before active"
        );
    }
    push.await.unwrap();
}

// --- 4.5 polling and pushes

#[tokio::test]
async fn application_push_reaches_only_activated_clients() {
    let n = start(node(Arc::new(Sim::default())));
    let mut a = Client::connect(n.local_addr()).await;
    let mut b = Client::connect(n.local_addr()).await;
    let mut idle = Client::connect(n.local_addr()).await;
    activate(&mut a).await;
    activate(&mut b).await;
    n.update(
        "t1",
        "value",
        must_server::Reading::new(Value::Double(295.2)),
    )
    .unwrap();
    for client in [&mut a, &mut b] {
        assert!(client.line().await.starts_with("update t1:value [295.2,"));
    }
    assert_eq!(idle.try_line(Duration::from_millis(300)).await, None);
    // pushes are checked against the datainfo
    assert!(
        n.update("t1", "value", Value::String("hot".into()))
            .is_err()
    );
    assert!(n.update("t1", "nope", Value::Double(1.0)).is_err());
}

#[tokio::test(start_paused = true)]
async fn periodic_poll() {
    let sim = Arc::new(Sim::default());
    // only value and status: the controller's `hang` parameter would block the module
    let module = Module::new("p", "polled sensor")
        .interface_classes(["Readable"])
        .parameter(Parameter::new("value", kelvin(), "v"))
        .unwrap()
        .parameter(Parameter::new("status", common::status_info(), "s"))
        .unwrap()
        .poll_interval(Duration::from_secs(1))
        .handler_arc(Arc::clone(&sim) as Arc<dyn must_server::ModuleHandler>);
    let n = start(
        NodeBuilder::new("x", "y")
            .log_files(must_server::logfiles::LogFiles::off())
            .bind(SocketAddr::from(([127, 0, 0, 1], 0)))
            .initial_read(false)
            .module(module)
            .unwrap(),
    );
    let mut events = n.subscribe();
    let start = tokio::time::Instant::now();
    let mut value_updates = 0;
    while start.elapsed() < Duration::from_millis(5500) {
        tokio::select! {
            event = events.recv() => {
                if let Ok(NodeEvent::Updated { parameter, .. }) = event && parameter == "value" {
                    value_updates += 1;
                }
            }
            () = tokio::time::sleep(Duration::from_millis(100)) => {}
        }
    }
    assert_eq!(value_updates, 5, "one poll per second");
    assert_eq!(
        sim.reads.load(Ordering::SeqCst),
        10,
        "value and status, five times each"
    );
    n.stop().await;
}

#[tokio::test]
async fn polled_values_reach_activated_clients() {
    let sim = Arc::new(Sim::default());
    let module = common::controller(Arc::clone(&sim));
    let fast = Module::new("p", "polled")
        .interface_classes([])
        .parameter(Parameter::new("value", kelvin(), "v").poll_interval(Duration::from_millis(100)))
        .unwrap()
        .handler_arc(sim as Arc<dyn must_server::ModuleHandler>);
    let n = start(
        NodeBuilder::new("x", "y")
            .log_files(must_server::logfiles::LogFiles::off())
            .bind(SocketAddr::from(([127, 0, 0, 1], 0)))
            .initial_read(false)
            .module(module)
            .unwrap()
            .module(fast)
            .unwrap(),
    );
    let mut c = Client::connect(n.local_addr()).await;
    c.send("activate p").await;
    assert_eq!(c.line().await, "active p");
    assert!(c.line().await.starts_with("update p:value [295.0,"));
    assert!(c.line().await.starts_with("update p:value [295.0,"));
}

// --- 4.6 lifecycle

#[tokio::test]
async fn port_in_use() {
    let n = start(node(Arc::new(Sim::default())));
    let taken = n.local_addr();
    let err = node(Arc::new(Sim::default()))
        .bind(taken)
        .build()
        .unwrap()
        .spawn(&Handle::current());
    let Err(StartError::Bind { addr, .. }) = err else {
        panic!("second node started")
    };
    assert_eq!(addr, taken);
    assert!(err.unwrap_err().to_string().contains(&taken.to_string()));
}

#[tokio::test]
async fn stop_one_of_two_nodes() {
    let first = start(node(Arc::new(Sim::default())));
    let second = start(node(Arc::new(Sim::default())));
    let mut c1 = Client::connect(first.local_addr()).await;
    let mut c2 = Client::connect(second.local_addr()).await;
    assert!(c1.request("ping a").await.starts_with("pong a"));

    let mut events = first.subscribe();
    first.stop().await;
    assert!(
        c1.is_closed(Duration::from_secs(2)).await,
        "client of the stopped node disconnected"
    );
    assert!(
        tokio::net::TcpStream::connect(first.local_addr())
            .await
            .is_err(),
        "port closed"
    );
    let mut saw_stopped = false;
    while let Ok(event) = events.try_recv() {
        saw_stopped |= event == NodeEvent::Stopped;
    }
    assert!(saw_stopped);

    assert!(
        c2.request("ping b").await.starts_with("pong b"),
        "the other node keeps working"
    );
    assert!(
        c2.request("read t1:value")
            .await
            .starts_with("reply t1:value")
    );
}

#[test]
fn library_owned_runtime() {
    // no tokio runtime here: `start` uses the library's own
    let n = node(Arc::new(Sim::default()))
        .build()
        .unwrap()
        .start()
        .unwrap();
    let addr = n.local_addr();
    let reply = std::thread::spawn(move || {
        use std::io::{BufRead, BufReader, Write};
        let mut s = std::net::TcpStream::connect(addr).unwrap();
        s.write_all(b"read t1:value\n").unwrap();
        let mut line = String::new();
        BufReader::new(s).read_line(&mut line).unwrap();
        line
    })
    .join()
    .unwrap();
    assert!(reply.starts_with("reply t1:value"), "{reply}");
    n.stop_blocking();
    assert!(std::net::TcpStream::connect(addr).is_err());
}

#[tokio::test]
async fn status_helper_is_valid() {
    // the status values used by the simulation fit the status datainfo
    assert!(common::status_info().encode(&status(300, "x")).is_ok());
}
