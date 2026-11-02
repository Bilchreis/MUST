// SPDX-License-Identifier: LGPL-3.0-or-later

//! End-to-end tests of the scenarios of concurrent requests, shared reads, repeated reads
//! on one connection and concurrent polls in specs/sec-node-runtime. Handlers are held at
//! gates and their calls counted. A short pause lets a request reach the module where
//! nothing observable can be awaited.

mod common;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use common::{Client, error_class, kelvin, parse, status, status_info};
use must_server::logfiles::LogFiles;
use must_server::{
    Ctx, Discovery, HandlerResult, Module, ModuleHandler, NodeBuilder, NodeEvent, Parameter,
    RunningNode,
};
use must_types::{ErrorClass, SecopError, Value};
use tokio::runtime::Handle;
use tokio::sync::{Semaphore, broadcast};

/// A gate for the next calls of an accessible.
struct Hold {
    calls: usize,
    gate: Arc<Semaphore>,
}

#[derive(Default)]
struct Hw {
    calls: Mutex<HashMap<String, usize>>,
    holds: Mutex<HashMap<String, Hold>>,
    busy: AtomicUsize,
    /// How long each read takes.
    read_time: Duration,
}

impl Hw {
    /// Makes the next `calls` handler calls of `name` wait; returns the gate to open.
    fn hold(&self, name: &str, calls: usize) -> Arc<Semaphore> {
        let gate = Arc::new(Semaphore::new(0));
        let hold = Hold {
            calls,
            gate: Arc::clone(&gate),
        };
        self.holds.lock().unwrap().insert(name.to_owned(), hold);
        gate
    }

    fn calls(&self, name: &str) -> usize {
        self.calls.lock().unwrap().get(name).copied().unwrap_or(0)
    }

    /// Counts a call and passes its gate, if it has one.
    async fn enter(&self, name: &str) {
        *self
            .calls
            .lock()
            .unwrap()
            .entry(name.to_owned())
            .or_default() += 1;
        let gate = {
            let mut holds = self.holds.lock().unwrap();
            holds.get_mut(name).and_then(|hold| {
                (hold.calls > 0).then(|| {
                    hold.calls -= 1;
                    Arc::clone(&hold.gate)
                })
            })
        };
        if let Some(gate) = gate {
            gate.acquire().await.unwrap().forget();
        }
    }

    /// Waits until `name` was called `count` times.
    async fn called(&self, name: &str, count: usize) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while self.calls(name) < count {
            assert!(
                Instant::now() < deadline,
                "`{name}` called {} times, not {count}",
                self.calls(name)
            );
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    }
}

#[async_trait]
impl ModuleHandler for Hw {
    async fn read(&self, _ctx: &Ctx, parameter: &str) -> HandlerResult {
        // the state when the read began, as a slow hardware query would report it
        let value = match parameter {
            "status" if self.busy.load(Ordering::SeqCst) > 0 => status(300, "ramping"),
            "status" => status(100, "idle"),
            "fail" => {
                self.enter(parameter).await;
                return Err(SecopError::new(ErrorClass::CommunicationFailed, "no link"));
            }
            _ => Value::Double(1.0 + f64::from(u32::try_from(self.calls(parameter)).unwrap())),
        };
        self.enter(parameter).await;
        if !self.read_time.is_zero() {
            tokio::time::sleep(self.read_time).await;
        }
        Ok(value.into())
    }

    async fn change(&self, ctx: &Ctx, parameter: &str, value: Value) -> HandlerResult {
        self.enter("change").await;
        self.busy.fetch_add(1, Ordering::SeqCst);
        ctx.update("status", status(300, "ramping"))?;
        let _ = parameter;
        Ok(value.into())
    }
}

/// A Writable module `m` that is not polled.
fn module(hw: &Arc<Hw>, concurrent: bool) -> Module {
    let module = Module::new("m", "gated hardware")
        .interface_classes(["Writable"])
        .parameter(Parameter::new("value", kelvin(), "value"))
        .unwrap()
        .parameter(Parameter::new("status", status_info(), "status"))
        .unwrap()
        .parameter(Parameter::new("target", kelvin(), "target").writable())
        .unwrap()
        .parameter(Parameter::new("busy", kelvin(), "keeps the module busy"))
        .unwrap()
        .parameter(Parameter::new("fail", kelvin(), "fails"))
        .unwrap()
        .handler_arc(Arc::clone(hw) as Arc<dyn ModuleHandler>)
        .no_polling();
    if concurrent {
        module.concurrent_requests()
    } else {
        module
    }
}

fn builder(module: Module) -> NodeBuilder {
    NodeBuilder::new("shared", "shared reads")
        .bind(SocketAddr::from(([127, 0, 0, 1], 0)))
        .initial_read(false)
        .timeout(30.0)
        .log_files(LogFiles::off())
        .discovery(Discovery::off())
        .module(module)
        .unwrap()
}

fn start(module: Module) -> RunningNode {
    builder(module)
        .build()
        .unwrap()
        .spawn(&Handle::current())
        .unwrap()
}

async fn pause() {
    tokio::time::sleep(Duration::from_millis(100)).await;
}

async fn activated(n: &RunningNode) -> Client {
    let mut c = Client::connect(n.local_addr()).await;
    c.send("activate m").await;
    while c.line().await != "active m" {}
    c
}

async fn client(n: &RunningNode) -> Client {
    Client::connect(n.local_addr()).await
}

/// The reply's action and data.
async fn reply(c: &mut Client) -> (String, String, serde_json::Value) {
    parse(&c.line().await)
}

fn updates_of(events: &mut broadcast::Receiver<NodeEvent>, name: &str) -> usize {
    let mut count = 0;
    while let Ok(event) = events.try_recv() {
        if matches!(event, NodeEvent::Updated { parameter, .. } if parameter == name) {
            count += 1;
        }
    }
    count
}

/// Runs a scenario on a module with one call at a time and on one with concurrent requests.
macro_rules! on_both {
    ($($name:ident),* $(,)?) => {
        mod one_at_a_time {
            $(#[tokio::test]
            async fn $name() {
                super::$name(false).await;
            })*
        }
        mod concurrent {
            $(#[tokio::test]
            async fn $name() {
                super::$name(true).await;
            })*
        }
    };
}

on_both!(
    client_joins_a_clients_read,
    two_share_a_running_read_a_third_is_updated,
    client_joins_a_poll,
    poll_joins_a_clients_read,
    first_reader_leaves,
    repeated_read_shares_the_running_read,
    another_request_in_between,
    shared_error_while_running,
);

// --- request concurrency

#[tokio::test]
async fn one_call_at_a_time_by_default() {
    let hw = Arc::new(Hw::default());
    let n = start(module(&hw, false));
    let gate = hw.hold("value", 1);
    let (mut a, mut b) = (client(&n).await, client(&n).await);
    a.send("read m:value").await;
    hw.called("value", 1).await;
    b.send("read m:status").await;
    pause().await;
    assert_eq!(hw.calls("status"), 0, "the handler is busy");
    gate.add_permits(1);
    assert_eq!(reply(&mut a).await.0, "reply");
    assert_eq!(reply(&mut b).await.0, "reply");
    assert_eq!(hw.calls("status"), 1);
}

#[tokio::test]
async fn concurrent_requests() {
    let hw = Arc::new(Hw::default());
    let n = start(module(&hw, true));
    let gate = hw.hold("value", 1);
    let (mut a, mut b) = (client(&n).await, client(&n).await);
    a.send("read m:value").await;
    hw.called("value", 1).await;
    let (action, spec, _) = parse(&b.request("read m:status").await);
    assert_eq!((action.as_str(), spec.as_str()), ("reply", "m:status"));
    gate.add_permits(1);
    assert_eq!(reply(&mut a).await.1, "m:value");
}

#[tokio::test]
async fn order_within_a_connection() {
    let hw = Arc::new(Hw::default());
    let n = start(module(&hw, true));
    let gate = hw.hold("change", 1);
    let mut c = client(&n).await;
    c.send("change m:target 5").await;
    c.send("read m:target").await;
    hw.called("change", 1).await;
    pause().await;
    assert_eq!(
        hw.calls("target"),
        0,
        "the read waits for the change's reply"
    );
    gate.add_permits(1);
    assert_eq!(reply(&mut c).await.0, "changed");
    assert_eq!(reply(&mut c).await.0, "reply");
}

#[tokio::test]
async fn a_65th_running_request_waits() {
    let hw = Arc::new(Hw::default());
    let n = start(module(&hw, true));
    let gate = hw.hold("change", 65);
    let mut clients = Vec::new();
    for _ in 0..65 {
        let mut c = client(&n).await;
        c.send("change m:target 5").await;
        clients.push(c);
    }
    hw.called("change", 64).await;
    pause().await;
    assert_eq!(hw.calls("change"), 64);
    gate.add_permits(1);
    hw.called("change", 65).await;
    gate.add_permits(64);
    for c in &mut clients {
        assert_eq!(reply(c).await.0, "changed");
    }
}

#[tokio::test]
async fn status_update_before_changed() {
    let hw = Arc::new(Hw::default());
    let n = start(module(&hw, true));
    let mut c = activated(&n).await;
    c.send("change m:target 5").await;
    let mut lines = Vec::new();
    loop {
        let (action, spec, _) = reply(&mut c).await;
        let last = action == "changed";
        lines.push((action, spec));
        if last {
            break;
        }
    }
    let lines: Vec<_> = lines
        .iter()
        .map(|(a, s)| (a.as_str(), s.as_str()))
        .collect();
    assert_eq!(
        lines,
        [
            ("update", "m:status"),
            ("update", "m:target"),
            ("changed", "m:target"),
        ]
    );
}

// --- shared waiting reads

#[tokio::test]
async fn reads_waiting_behind_a_change() {
    let hw = Arc::new(Hw::default());
    let n = start(module(&hw, false));
    let gate = hw.hold("change", 1);
    let (mut b, mut r1, mut r2) = (client(&n).await, client(&n).await, client(&n).await);
    b.send("change m:target 5").await;
    hw.called("change", 1).await;
    r1.send("read m:value").await;
    r2.send("read m:value").await;
    pause().await;
    gate.add_permits(1);
    let (first, second) = (reply(&mut r1).await, reply(&mut r2).await);
    assert_eq!(first.0, "reply");
    assert_eq!(first.2, second.2, "same value and timestamp");
    assert_eq!(hw.calls("value"), 1);
}

#[tokio::test]
async fn two_share_a_waiting_read_a_third_is_updated() {
    let hw = Arc::new(Hw::default());
    let n = start(module(&hw, false));
    let mut watcher = activated(&n).await;
    let gate = hw.hold("busy", 1);
    let (mut other, mut r1, mut r2) = (client(&n).await, client(&n).await, client(&n).await);
    other.send("read m:busy").await;
    hw.called("busy", 1).await;
    r1.send("read m:value").await;
    r2.send("read m:value").await;
    pause().await;
    gate.add_permits(1);
    let (first, second) = (reply(&mut r1).await, reply(&mut r2).await);
    assert_eq!(first.2, second.2);
    assert_eq!(hw.calls("value"), 1);
    assert!(watcher.line().await.starts_with("update m:busy "));
    let update = parse(&watcher.line().await);
    assert_eq!(
        ("update", "m:value"),
        (update.0.as_str(), update.1.as_str())
    );
    assert_eq!(update.2, first.2);
    assert_eq!(
        watcher.try_line(Duration::from_millis(100)).await,
        None,
        "one update, no reply"
    );
}

#[tokio::test]
async fn reads_around_a_waiting_change() {
    let hw = Arc::new(Hw::default());
    let n = start(module(&hw, false));
    let mut watcher = activated(&n).await;
    let gate = hw.hold("busy", 1);
    let mut other = client(&n).await;
    let (mut a, mut b, mut c) = (client(&n).await, client(&n).await, client(&n).await);
    other.send("read m:busy").await;
    hw.called("busy", 1).await;
    a.send("read m:target").await;
    pause().await;
    b.send("change m:target 5").await;
    pause().await;
    c.send("read m:target").await;
    pause().await;
    gate.add_permits(1);
    let (ra, rc) = (reply(&mut a).await, reply(&mut c).await);
    assert_eq!(ra.0, "reply");
    assert_eq!(ra.2, rc.2);
    assert_eq!(ra.2[0], 1.0, "the read comes before the change");
    let changed = reply(&mut b).await;
    assert_eq!(
        (changed.0.as_str(), &changed.2[0]),
        ("changed", &5.0.into())
    );
    assert_eq!(hw.calls("target"), 1);
    let mut targets = Vec::new();
    while targets.len() < 2 {
        let (action, spec, data) = reply(&mut watcher).await;
        if spec == "m:target" {
            assert_eq!(action, "update");
            targets.push(data[0].as_f64().unwrap());
        }
    }
    assert_eq!(targets, [1.0, 5.0]);
}

#[tokio::test]
async fn poll_tick_across_a_change() {
    let hw = Arc::new(Hw::default());
    let target_polled = Module::new("m", "gated hardware")
        .interface_classes(["Writable"])
        .parameter(Parameter::new("value", kelvin(), "value"))
        .unwrap()
        .parameter(Parameter::new("status", status_info(), "status"))
        .unwrap()
        .parameter(
            Parameter::new("target", kelvin(), "target")
                .writable()
                .poll_interval(Duration::from_millis(500)),
        )
        .unwrap()
        .parameter(Parameter::new("busy", kelvin(), "keeps the module busy"))
        .unwrap()
        .handler_arc(Arc::clone(&hw) as Arc<dyn ModuleHandler>)
        .no_polling();
    let n = start(target_polled);
    let mut events = n.subscribe();
    let gate = hw.hold("busy", 1);
    let mut other = client(&n).await;
    let (mut a, mut b) = (client(&n).await, client(&n).await);
    other.send("read m:busy").await;
    hw.called("busy", 1).await;
    a.send("read m:target").await;
    pause().await;
    b.send("change m:target 5").await;
    // the tick falls due now, behind the change
    tokio::time::sleep(Duration::from_millis(450)).await;
    gate.add_permits(1);
    assert_eq!(reply(&mut a).await.0, "reply");
    assert_eq!(reply(&mut b).await.0, "changed");
    pause().await;
    assert_eq!(hw.calls("target"), 1, "the poll makes no read of its own");
    assert_eq!(updates_of(&mut events, "target"), 2);
}

async fn shared_error_while_running(concurrent: bool) {
    let hw = Arc::new(Hw::default());
    let n = start(module(&hw, concurrent));
    let gate = hw.hold("fail", 1);
    let (mut a, mut b) = (client(&n).await, client(&n).await);
    a.send("read m:fail").await;
    hw.called("fail", 1).await;
    b.send("read m:fail").await;
    pause().await;
    gate.add_permits(1);
    assert_eq!(error_class(&a.line().await), "CommunicationFailed");
    assert_eq!(error_class(&b.line().await), "CommunicationFailed");
    assert_eq!(hw.calls("fail"), 1);
}

#[tokio::test]
async fn shared_error_while_waiting() {
    let hw = Arc::new(Hw::default());
    let n = start(module(&hw, false));
    let gate = hw.hold("busy", 1);
    let mut other = client(&n).await;
    let (mut a, mut b) = (client(&n).await, client(&n).await);
    other.send("read m:busy").await;
    hw.called("busy", 1).await;
    a.send("read m:fail").await;
    b.send("read m:fail").await;
    pause().await;
    gate.add_permits(1);
    assert_eq!(error_class(&a.line().await), "CommunicationFailed");
    assert_eq!(error_class(&b.line().await), "CommunicationFailed");
    assert_eq!(hw.calls("fail"), 1);
}

// --- shared running reads

async fn client_joins_a_clients_read(concurrent: bool) {
    let hw = Arc::new(Hw::default());
    let n = start(module(&hw, concurrent));
    let gate = hw.hold("value", 1);
    let (mut a, mut b) = (client(&n).await, client(&n).await);
    a.send("read m:value").await;
    hw.called("value", 1).await;
    b.send("read m:value").await;
    pause().await;
    gate.add_permits(1);
    let (first, second) = (reply(&mut a).await, reply(&mut b).await);
    assert_eq!(first.0, "reply");
    assert_eq!(first.2, second.2);
    assert_eq!(hw.calls("value"), 1);
}

async fn two_share_a_running_read_a_third_is_updated(concurrent: bool) {
    let hw = Arc::new(Hw::default());
    let n = start(module(&hw, concurrent));
    let mut watcher = activated(&n).await;
    let gate = hw.hold("value", 1);
    let (mut a, mut b) = (client(&n).await, client(&n).await);
    a.send("read m:value").await;
    hw.called("value", 1).await;
    b.send("read m:value").await;
    pause().await;
    gate.add_permits(1);
    let (first, second) = (reply(&mut a).await, reply(&mut b).await);
    assert_eq!(first.2, second.2);
    assert_eq!(hw.calls("value"), 1);
    let update = parse(&watcher.line().await);
    assert_eq!(
        ("update", "m:value"),
        (update.0.as_str(), update.1.as_str())
    );
    assert_eq!(update.2, first.2);
    assert_eq!(watcher.try_line(Duration::from_millis(100)).await, None);
}

/// [`module`] with `value` polled at `interval`.
fn value_polled(hw: &Arc<Hw>, concurrent: bool, interval: Duration) -> Module {
    let module = Module::new("m", "gated hardware")
        .interface_classes(["Readable"])
        .parameter(Parameter::new("value", kelvin(), "value").poll_interval(interval))
        .unwrap()
        .parameter(Parameter::new("status", status_info(), "status"))
        .unwrap()
        .handler_arc(Arc::clone(hw) as Arc<dyn ModuleHandler>)
        .no_polling();
    if concurrent {
        module.concurrent_requests()
    } else {
        module
    }
}

async fn client_joins_a_poll(concurrent: bool) {
    let hw = Arc::new(Hw::default());
    let gate = hw.hold("value", 1);
    let n = start(value_polled(&hw, concurrent, Duration::from_millis(200)));
    let mut events = n.subscribe();
    hw.called("value", 1).await;
    let mut c = client(&n).await;
    c.send("read m:value").await;
    pause().await;
    gate.add_permits(1);
    assert_eq!(reply(&mut c).await.0, "reply");
    assert_eq!(hw.calls("value"), 1, "the handler is not called again");
    assert_eq!(updates_of(&mut events, "value"), 1);
}

async fn poll_joins_a_clients_read(concurrent: bool) {
    let hw = Arc::new(Hw::default());
    let gate = hw.hold("value", 1);
    let n = start(value_polled(&hw, concurrent, Duration::from_millis(300)));
    let mut events = n.subscribe();
    let mut c = client(&n).await;
    c.send("read m:value").await;
    hw.called("value", 1).await;
    // the tick falls due now
    tokio::time::sleep(Duration::from_millis(400)).await;
    gate.add_permits(1);
    assert_eq!(reply(&mut c).await.0, "reply");
    assert_eq!(
        hw.calls("value"),
        1,
        "the handler is not called for the poll"
    );
    assert_eq!(updates_of(&mut events, "value"), 1);
}

#[tokio::test]
async fn read_after_changed_sees_the_change() {
    let hw = Arc::new(Hw::default());
    let n = start(module(&hw, true));
    let gate = hw.hold("status", 1);
    let (mut a, mut b) = (client(&n).await, client(&n).await);
    a.send("read m:status").await;
    hw.called("status", 1).await;
    assert_eq!(parse(&b.request("change m:target 5").await).0, "changed");
    let (action, _, data) = parse(&b.request("read m:status").await);
    assert_eq!(action, "reply");
    assert_eq!(data[0][0], 300, "B receives the BUSY status");
    assert_eq!(hw.calls("status"), 2);
    gate.add_permits(1);
    assert_eq!(reply(&mut a).await.2[0][0], 100);
}

async fn first_reader_leaves(concurrent: bool) {
    let hw = Arc::new(Hw::default());
    let n = start(module(&hw, concurrent));
    let gate = hw.hold("value", 1);
    let (mut a, mut b) = (client(&n).await, client(&n).await);
    a.send("read m:value").await;
    hw.called("value", 1).await;
    b.send("read m:value").await;
    pause().await;
    drop(a);
    pause().await;
    gate.add_permits(1);
    assert_eq!(reply(&mut b).await.0, "reply");
    assert_eq!(hw.calls("value"), 1);
}

// --- repeated reads on one connection

async fn repeated_read_shares_the_running_read(concurrent: bool) {
    let hw = Arc::new(Hw::default());
    let n = start(module(&hw, concurrent));
    let gate = hw.hold("value", 1);
    let mut a = client(&n).await;
    a.send("read m:value").await;
    hw.called("value", 1).await;
    a.send("read m:value").await;
    pause().await;
    gate.add_permits(1);
    let (first, second) = (reply(&mut a).await, reply(&mut a).await);
    assert_eq!((first.0.as_str(), second.0.as_str()), ("reply", "reply"));
    assert_eq!(first.2, second.2, "same value and timestamp");
    assert_eq!(hw.calls("value"), 1);
}

async fn another_request_in_between(concurrent: bool) {
    let hw = Arc::new(Hw::default());
    let n = start(module(&hw, concurrent));
    let gate = hw.hold("value", 1);
    let mut a = client(&n).await;
    a.send("read m:value").await;
    hw.called("value", 1).await;
    a.send("read m:status").await;
    a.send("read m:value").await;
    pause().await;
    assert_eq!(hw.calls("status"), 0, "handled after the first reply");
    gate.add_permits(1);
    let specs = [
        reply(&mut a).await.1,
        reply(&mut a).await.1,
        reply(&mut a).await.1,
    ];
    assert_eq!(specs, ["m:value", "m:status", "m:value"]);
    assert_eq!(hw.calls("value"), 2);
}

#[tokio::test]
async fn read_after_another_clients_change() {
    let hw = Arc::new(Hw::default());
    let n = start(module(&hw, true));
    let gate = hw.hold("status", 1);
    let (mut a, mut b) = (client(&n).await, client(&n).await);
    a.send("read m:status").await;
    hw.called("status", 1).await;
    assert_eq!(parse(&b.request("change m:target 5").await).0, "changed");
    a.send("read m:status").await;
    hw.called("status", 2).await;
    gate.add_permits(1);
    assert_eq!(reply(&mut a).await.2[0][0], 100, "from before the change");
    assert_eq!(reply(&mut a).await.2[0][0], 300, "from after the change");
}

// --- concurrent polls

fn eleven_polled(hw: &Arc<Hw>) -> Module {
    let mut module = Module::new("m", "many parameters")
        .interface_classes(["Readable"])
        .parameter(Parameter::new("value", kelvin(), "value"))
        .unwrap()
        .parameter(Parameter::new("status", status_info(), "status"))
        .unwrap();
    for i in 0..9 {
        module = module
            .parameter(Parameter::new(&format!("p{i}"), kelvin(), "parameter"))
            .unwrap();
    }
    module
        .poll_all(Duration::from_secs(1))
        .handler_arc(Arc::clone(hw) as Arc<dyn ModuleHandler>)
        .concurrent_requests()
}

#[tokio::test(start_paused = true)]
async fn reads_started_together() {
    let hw = Arc::new(Hw {
        read_time: Duration::from_millis(100),
        ..Hw::default()
    });
    let n = start(eleven_polled(&hw));
    let mut events = n.subscribe();
    tokio::time::sleep(Duration::from_millis(1150)).await;
    let mut updated = 0;
    while let Ok(event) = events.try_recv() {
        if matches!(event, NodeEvent::Updated { .. }) {
            updated += 1;
        }
    }
    assert_eq!(updated, 11, "all updates about 100 ms after the tick");
    n.stop().await;
}

#[tokio::test(start_paused = true)]
async fn hanging_read() {
    let hw = Arc::new(Hw::default());
    let _gate = hw.hold("p0", 1);
    let n = start(eleven_polled(&hw));
    let mut events = n.subscribe();
    tokio::time::sleep(Duration::from_millis(2500)).await;
    assert_eq!(hw.calls("p0"), 1, "not called again while it hangs");
    assert_eq!(hw.calls("p1"), 2);
    assert_eq!(updates_of(&mut events, "p1"), 2);
}

#[tokio::test(start_paused = true)]
async fn one_at_a_time_polls_one_after_another() {
    let hw = Arc::new(Hw {
        read_time: Duration::from_millis(50),
        ..Hw::default()
    });
    let module = Module::new("m", "polled")
        .interface_classes(["Readable"])
        .parameter(Parameter::new("value", kelvin(), "value"))
        .unwrap()
        .parameter(Parameter::new("status", status_info(), "status"))
        .unwrap()
        .poll_all(Duration::from_secs(1))
        .handler_arc(Arc::clone(&hw) as Arc<dyn ModuleHandler>);
    let n = start(module);
    tokio::time::sleep(Duration::from_millis(1025)).await;
    assert_eq!((hw.calls("value"), hw.calls("status")), (1, 0));
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!((hw.calls("value"), hw.calls("status")), (1, 1));
    n.stop().await;
}
