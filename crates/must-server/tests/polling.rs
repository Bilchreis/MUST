// SPDX-License-Identifier: LGPL-3.0-or-later

//! End-to-end tests of the polling scenarios in specs/sec-node-runtime: tiers, interval
//! precedence, the `pollinterval` parameter and the poll hook.

mod common;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use common::{Client, di, kelvin, parse, status, status_info};
use must_server::logfiles::LogFiles;
use must_server::{
    Command, Ctx, HandlerResult, Module, ModuleHandler, NodeBuilder, NodeEvent, Parameter, Reading,
    RunningNode,
};
use must_types::{DataInfo, ErrorClass, SecopError, Value};
use serde_json::json;
use tokio::runtime::Handle;
use tokio::sync::broadcast;

/// Counts reads per parameter and answers changes of `pollinterval`.
#[derive(Default)]
struct Rec {
    reads: Mutex<HashMap<String, usize>>,
    /// Parameter lists the poll hook was called with (only by [`Batch`] and [`Hang`]).
    polls: Mutex<Vec<Vec<String>>>,
    /// Smallest `pollinterval` the "hardware" accepts.
    min_interval: f64,
    /// Fail every change of `pollinterval`.
    refuse: bool,
}

impl Rec {
    fn reads(&self, parameter: &str) -> usize {
        self.reads
            .lock()
            .unwrap()
            .get(parameter)
            .copied()
            .unwrap_or(0)
    }

    fn record_poll(&self, due: &[&str]) {
        let due = due.iter().map(|&p| p.to_owned()).collect();
        self.polls.lock().unwrap().push(due);
    }
}

#[async_trait]
impl ModuleHandler for Rec {
    async fn read(&self, _ctx: &Ctx, parameter: &str) -> HandlerResult {
        *self
            .reads
            .lock()
            .unwrap()
            .entry(parameter.to_owned())
            .or_default() += 1;
        match parameter {
            "value" => Ok(Value::Double(295.0).into()),
            "status" => Ok(status(100, "idle").into()),
            _ => Ok(Value::Double(1.0).into()),
        }
    }

    async fn change(&self, _ctx: &Ctx, parameter: &str, value: Value) -> HandlerResult {
        if parameter == "pollinterval" {
            if self.refuse {
                return Err(SecopError::new(ErrorClass::ReadOnly, "fixed rate"));
            }
            let seconds = value.as_f64().unwrap().max(self.min_interval);
            return Ok(Value::Double(seconds).into());
        }
        Ok(value.into())
    }
}

/// Answers each poll with one "controller query" for `value` and `status`.
struct Batch(Arc<Rec>);

#[async_trait]
impl ModuleHandler for Batch {
    async fn read(&self, ctx: &Ctx, parameter: &str) -> HandlerResult {
        self.0.read(ctx, parameter).await
    }

    async fn poll(&self, ctx: &Ctx, due: &[&str]) {
        self.0.record_poll(due);
        for &p in due {
            match p {
                "value" => ctx.update(p, Value::Double(295.0)).unwrap(),
                "status" => ctx.update(p, status(100, "idle")).unwrap(),
                _ => ctx.update(p, Value::Double(1.0)).unwrap(),
            }
        }
    }
}

/// A poll hook that never returns.
struct Hang(Arc<Rec>);

#[async_trait]
impl ModuleHandler for Hang {
    async fn poll(&self, _ctx: &Ctx, due: &[&str]) {
        self.0.record_poll(due);
        std::future::pending::<()>().await;
    }
}

fn seconds() -> DataInfo {
    di(json!({"type": "double", "min": 0, "max": 3600, "unit": "s"}))
}

/// A Drivable module `cryo` with value, status, target and ramp.
fn cryo() -> Module {
    Module::new("cryo", "a cryostat")
        .interface_classes(["Drivable"])
        .parameter(Parameter::new("value", kelvin(), "temperature"))
        .unwrap()
        .parameter(Parameter::new("status", status_info(), "status"))
        .unwrap()
        .parameter(Parameter::new("target", kelvin(), "setpoint").writable())
        .unwrap()
        .parameter(Parameter::new("ramp", kelvin(), "ramp rate").writable())
        .unwrap()
        .command(Command::new(
            "stop",
            DataInfo::command().build().unwrap(),
            "stop",
        ))
        .unwrap()
}

fn with_pollinterval(m: Module) -> Module {
    m.parameter(Parameter::new("pollinterval", seconds(), "poll interval").writable())
        .unwrap()
}

fn start(module: Module) -> RunningNode {
    NodeBuilder::new("polling", "polling tests")
        .log_files(LogFiles::off())
        .bind(SocketAddr::from(([127, 0, 0, 1], 0)))
        .initial_read(false)
        .timeout(2.0)
        .module(module)
        .unwrap()
        .build()
        .unwrap()
        .spawn(&Handle::current())
        .unwrap()
}

/// Lets `d` pass; with paused time this runs everything due meanwhile.
async fn run_for(d: Duration) {
    tokio::time::sleep(d).await;
}

fn drain(events: &mut broadcast::Receiver<NodeEvent>) -> Vec<NodeEvent> {
    let mut all = Vec::new();
    while let Ok(event) = events.try_recv() {
        all.push(event);
    }
    all
}

fn updates_of(events: &[NodeEvent], name: &str) -> Vec<String> {
    events
        .iter()
        .filter_map(|e| match e {
            NodeEvent::Updated {
                parameter, line, ..
            } if parameter == name => Some(line.to_string()),
            _ => None,
        })
        .collect()
}

// --- tiers and precedence

#[tokio::test(start_paused = true)]
async fn fast_and_slow_tier() {
    let rec = Arc::new(Rec::default());
    let n = start(
        cryo()
            .poll_interval(Duration::from_secs(1))
            .handler_arc(Arc::clone(&rec) as Arc<dyn ModuleHandler>),
    );
    let mut events = n.subscribe();
    run_for(Duration::from_millis(130_500)).await;
    assert_eq!(rec.reads("value"), 130);
    assert_eq!(rec.reads("status"), 130);
    assert_eq!(rec.reads("target"), 2, "slow tier every 60 s");
    assert_eq!(rec.reads("ramp"), 2);
    // the default hook publishes what it reads
    let events = drain(&mut events);
    assert_eq!(updates_of(&events, "value").len(), 130);
    assert_eq!(updates_of(&events, "target").len(), 2);
    n.stop().await;
}

#[tokio::test(start_paused = true)]
async fn no_polling_unless_configured() {
    let rec = Arc::new(Rec::default());
    let n = start(cryo().handler_arc(Arc::clone(&rec) as Arc<dyn ModuleHandler>));
    run_for(Duration::from_secs(120)).await;
    assert!(rec.reads.lock().unwrap().is_empty());
    n.stop().await;
}

#[tokio::test(start_paused = true)]
async fn parameter_interval_wins() {
    let rec = Arc::new(Rec::default());
    let module = Module::new("cryo", "a cryostat")
        .interface_classes(["Readable"])
        .parameter(Parameter::new("value", kelvin(), "temperature"))
        .unwrap()
        .parameter(Parameter::new("status", status_info(), "status"))
        .unwrap()
        .parameter(Parameter::new("ramp", kelvin(), "ramp").poll_interval(Duration::from_secs(5)))
        .unwrap()
        .poll_interval(Duration::from_secs(1))
        .handler_arc(Arc::clone(&rec) as Arc<dyn ModuleHandler>);
    let n = start(module);
    run_for(Duration::from_millis(30_500)).await;
    assert_eq!(rec.reads("ramp"), 6, "every 5 s, not every 60 s");
    assert_eq!(rec.reads("value"), 30);
    n.stop().await;
}

#[tokio::test(start_paused = true)]
async fn uniform_interval() {
    let rec = Arc::new(Rec::default());
    let n = start(
        cryo()
            .poll_all(Duration::from_secs(2))
            .handler_arc(Arc::clone(&rec) as Arc<dyn ModuleHandler>),
    );
    run_for(Duration::from_millis(10_500)).await;
    for p in ["value", "status", "target", "ramp"] {
        assert_eq!(rec.reads(p), 5, "{p}");
    }
    n.stop().await;
}

#[tokio::test(start_paused = true)]
async fn unchanged_value_is_sent_again() {
    let rec = Arc::new(Rec::default());
    let n = start(
        cryo()
            .poll_interval(Duration::from_secs(1))
            .handler_arc(Arc::clone(&rec) as Arc<dyn ModuleHandler>),
    );
    let mut events = n.subscribe();
    run_for(Duration::from_millis(2500)).await;
    let updates = updates_of(&drain(&mut events), "value");
    assert_eq!(updates.len(), 2, "both polls of 295.0 sent");
    let (_, _, first) = parse(&updates[0]);
    let (_, _, second) = parse(&updates[1]);
    assert_eq!(first[0], second[0]);
    assert_ne!(first[1]["t"], second[1]["t"], "each with its own timestamp");
    n.stop().await;
}

// --- poll hook

#[tokio::test(start_paused = true)]
async fn parameters_due_together() {
    let rec = Arc::new(Rec::default());
    let n = start(
        cryo()
            .poll_interval(Duration::from_secs(1))
            .slow_poll_interval(Duration::from_secs(3))
            .handler(Batch(Arc::clone(&rec))),
    );
    run_for(Duration::from_millis(3500)).await;
    let polls = rec.polls.lock().unwrap().clone();
    assert_eq!(
        polls,
        [
            vec!["value", "status"],
            vec!["value", "status"],
            vec!["value", "status", "target", "ramp"],
        ]
    );
    n.stop().await;
}

#[tokio::test(start_paused = true)]
async fn one_hardware_transaction() {
    let rec = Arc::new(Rec::default());
    let n = start(
        cryo()
            .poll_interval(Duration::from_secs(1))
            .handler(Batch(Arc::clone(&rec))),
    );
    let mut events = n.subscribe();
    run_for(Duration::from_millis(1500)).await;
    let events = drain(&mut events);
    assert_eq!(updates_of(&events, "value").len(), 1);
    assert_eq!(updates_of(&events, "status").len(), 1);
    assert!(rec.reads.lock().unwrap().is_empty(), "read is not called");
    n.stop().await;
}

#[tokio::test(start_paused = true)]
async fn hook_times_out() {
    let rec = Arc::new(Rec::default());
    let n = start(
        cryo()
            .poll_interval(Duration::from_secs(1))
            .handler(Hang(Arc::clone(&rec))),
    );
    let mut events = n.subscribe();
    run_for(Duration::from_millis(9500)).await;
    let logged = drain(&mut events)
        .into_iter()
        .filter(|e| {
            matches!(e, NodeEvent::Log { module: Some(m), message, .. }
                if m == "cryo" && message.contains("polling failed"))
        })
        .count();
    assert!(logged >= 3, "each abandoned poll is logged, got {logged}");
    assert!(
        rec.polls.lock().unwrap().len() >= 4,
        "later ticks poll again"
    );
    n.stop().await;
}

// --- the pollinterval parameter (real time: clients change it over TCP)

/// Value reads during `d`.
async fn value_reads_during(rec: &Rec, d: Duration) -> usize {
    let before = rec.reads("value");
    tokio::time::sleep(d).await;
    rec.reads("value") - before
}

#[tokio::test]
async fn client_tunes_the_rate() {
    let rec = Arc::new(Rec::default());
    let n = start(
        with_pollinterval(cryo())
            .poll_interval(Duration::from_millis(500))
            .handler_arc(Arc::clone(&rec) as Arc<dyn ModuleHandler>),
    );
    let mut c = Client::connect(n.local_addr()).await;
    let reply = c.request("read cryo:pollinterval").await;
    assert!(
        reply.starts_with("reply cryo:pollinterval [1.0,"),
        "{reply}"
    );
    assert_eq!(
        value_reads_during(&rec, Duration::from_millis(300)).await,
        0
    );
    let reply = c.request("change cryo:pollinterval 0.05").await;
    assert!(
        reply.starts_with("changed cryo:pollinterval [0.05,"),
        "{reply}"
    );
    let reads = value_reads_during(&rec, Duration::from_secs(1)).await;
    assert!((12..=22).contains(&reads), "about 20 reads, got {reads}");
    assert_eq!(rec.reads("pollinterval"), 1, "only the client's read");
    n.stop().await;
}

#[tokio::test]
async fn handler_adjusts_the_value() {
    let rec = Arc::new(Rec {
        min_interval: 0.1,
        ..Rec::default()
    });
    let n = start(
        with_pollinterval(cryo())
            .poll_interval(Duration::from_secs(10))
            .handler_arc(Arc::clone(&rec) as Arc<dyn ModuleHandler>),
    );
    let mut c = Client::connect(n.local_addr()).await;
    let reply = c.request("change cryo:pollinterval 0.01").await;
    assert!(
        reply.starts_with("changed cryo:pollinterval [0.1,"),
        "{reply}"
    );
    let reads = value_reads_during(&rec, Duration::from_secs(1)).await;
    assert!((6..=11).contains(&reads), "about 10 reads, got {reads}");
    n.stop().await;
}

#[tokio::test]
async fn handler_refuses() {
    let rec = Arc::new(Rec {
        refuse: true,
        ..Rec::default()
    });
    let n = start(
        with_pollinterval(cryo())
            .poll_interval(Duration::from_millis(200))
            .handler_arc(Arc::clone(&rec) as Arc<dyn ModuleHandler>),
    );
    let mut c = Client::connect(n.local_addr()).await;
    let reply = c.request("change cryo:pollinterval 0.01").await;
    assert!(
        reply.starts_with("error_change cryo:pollinterval"),
        "{reply}"
    );
    let reads = value_reads_during(&rec, Duration::from_secs(1)).await;
    assert!((3..=6).contains(&reads), "still about 5 reads, got {reads}");
    n.stop().await;
}

#[tokio::test]
async fn zero_stops_polling() {
    let rec = Arc::new(Rec::default());
    let module = Module::new("cryo", "a cryostat")
        .interface_classes(["Readable"])
        .parameter(Parameter::new("value", kelvin(), "temperature"))
        .unwrap()
        .parameter(Parameter::new("status", status_info(), "status"))
        .unwrap()
        .parameter(
            Parameter::new("ramp", kelvin(), "ramp").poll_interval(Duration::from_millis(50)),
        )
        .unwrap();
    let n = start(
        with_pollinterval(module)
            .poll_interval(Duration::from_millis(50))
            .handler_arc(Arc::clone(&rec) as Arc<dyn ModuleHandler>),
    );
    let mut c = Client::connect(n.local_addr()).await;
    assert!(value_reads_during(&rec, Duration::from_millis(300)).await > 0);
    let reply = c.request("change cryo:pollinterval 0").await;
    assert!(
        reply.starts_with("changed cryo:pollinterval [0.0,"),
        "{reply}"
    );
    tokio::time::sleep(Duration::from_millis(100)).await;
    let ramp_before = rec.reads("ramp");
    assert_eq!(
        value_reads_during(&rec, Duration::from_millis(500)).await,
        0
    );
    assert!(
        rec.reads("ramp") - ramp_before >= 5,
        "own interval keeps polling"
    );
    n.stop().await;
}

#[tokio::test]
async fn application_push_re_times() {
    let rec = Arc::new(Rec::default());
    let n = start(
        with_pollinterval(cryo())
            .poll_interval(Duration::from_secs(10))
            .handler_arc(Arc::clone(&rec) as Arc<dyn ModuleHandler>),
    );
    let mut c = Client::connect(n.local_addr()).await;
    c.send("activate").await;
    while c.line().await != "active" {}
    n.update("cryo", "pollinterval", Reading::new(Value::Double(0.05)))
        .unwrap();
    assert!(
        c.line()
            .await
            .starts_with("update cryo:pollinterval [0.05,")
    );
    let reads = value_reads_during(&rec, Duration::from_millis(600)).await;
    assert!(reads >= 6, "about 12 reads, got {reads}");
    assert_eq!(rec.reads("pollinterval"), 0);
    n.stop().await;
}

#[tokio::test(start_paused = true)]
async fn pollinterval_is_never_polled() {
    let rec = Arc::new(Rec::default());
    let n = start(
        with_pollinterval(cryo())
            .poll_interval(Duration::from_secs(1))
            .handler_arc(Arc::clone(&rec) as Arc<dyn ModuleHandler>),
    );
    run_for(Duration::from_millis(300_500)).await;
    assert_eq!(rec.reads("pollinterval"), 0);
    assert_eq!(rec.reads("target"), 5);
    n.stop().await;
}
