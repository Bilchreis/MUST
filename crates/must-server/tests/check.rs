// SPDX-License-Identifier: LGPL-3.0-or-later

//! End-to-end tests of `check`: the scenarios of "Check" and "Check replies" in
//! specs/sec-node-protocol and of "Module handlers" in specs/sec-node-runtime.

mod common;

use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use common::{Client, di, error_class, kelvin, parse, status, status_info};
use must_server::logfiles::LogFiles;
use must_server::{
    CallResult, CheckResult, Command, Ctx, Discovery, HandlerResult, Module, ModuleHandler,
    NodeBuilder, Parameter, RunningNode,
};
use must_types::{DataInfo, ErrorClass, SecopError, Value};
use serde_json::json;
use tokio::runtime::Handle;

/// Accepts every check except a target of 280 and the parameter `hang`; records what
/// it was asked and whether a `change` was running at the time.
#[derive(Default)]
struct Checker {
    checks: Mutex<Vec<(String, Option<Value>)>>,
    changes: AtomicUsize,
    changing: AtomicUsize,
    overlapped: AtomicUsize,
}

impl Checker {
    fn checks(&self) -> Vec<(String, Option<Value>)> {
        self.checks.lock().unwrap().clone()
    }
}

#[async_trait]
impl ModuleHandler for Checker {
    async fn read(&self, _ctx: &Ctx, parameter: &str) -> HandlerResult {
        match parameter {
            "status" => Ok(status(100, "idle").into()),
            _ => Ok(Value::Double(10.0).into()),
        }
    }

    async fn change(&self, _ctx: &Ctx, _parameter: &str, value: Value) -> HandlerResult {
        self.changes.fetch_add(1, Ordering::SeqCst);
        self.changing.fetch_add(1, Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(300)).await;
        self.changing.fetch_sub(1, Ordering::SeqCst);
        Ok(value.into())
    }

    async fn call(&self, _ctx: &Ctx, _command: &str, _argument: Option<Value>) -> CallResult {
        Ok(None)
    }

    async fn check(&self, _ctx: &Ctx, accessible: &str, value: Option<Value>) -> CheckResult {
        if self.changing.load(Ordering::SeqCst) > 0 {
            self.overlapped.fetch_add(1, Ordering::SeqCst);
        }
        self.checks
            .lock()
            .unwrap()
            .push((accessible.to_owned(), value.clone()));
        match (accessible, value) {
            ("hang", _) => std::future::pending().await,
            ("target", Some(Value::Double(v))) if (v - 280.0).abs() < f64::EPSILON => Err(
                SecopError::new(ErrorClass::Impossible, "heater off in this configuration"),
            ),
            _ => Ok(()),
        }
    }
}

/// Implements everything but `check`.
struct NoCheck;

#[async_trait]
impl ModuleHandler for NoCheck {
    async fn read(&self, _ctx: &Ctx, _parameter: &str) -> HandlerResult {
        Ok(Value::Double(1.0).into())
    }
}

fn checkable(p: Parameter) -> Parameter {
    p.property("checkable", true).unwrap()
}

fn start(checker: &Arc<Checker>) -> RunningNode {
    let ramp = DataInfo::command()
        .argument(&di(json!({"type": "double"})))
        .build()
        .unwrap();
    let t1 = Module::new("t1", "a checkable controller")
        .interface_classes(["Drivable"])
        .parameter(Parameter::new("value", kelvin(), "temperature"))
        .unwrap()
        .parameter(Parameter::new("status", status_info(), "status"))
        .unwrap()
        .parameter(checkable(
            Parameter::new("target", kelvin(), "setpoint").writable(),
        ))
        .unwrap()
        .parameter(
            Parameter::new("target_max", kelvin(), "dynamic upper limit")
                .writable()
                .cached()
                .initial_value(Value::Double(300.0)),
        )
        .unwrap()
        .parameter(Parameter::new("plain", kelvin(), "not checkable").writable())
        .unwrap()
        .parameter(checkable(
            Parameter::new("hang", kelvin(), "check never answers").writable(),
        ))
        .unwrap()
        .command(
            Command::new("ramp", ramp, "ramps")
                .property("checkable", true)
                .unwrap(),
        )
        .unwrap()
        .command(
            Command::new("stop", DataInfo::command().build().unwrap(), "stops")
                .property("checkable", true)
                .unwrap(),
        )
        .unwrap()
        .no_polling()
        .handler_arc(Arc::clone(checker) as Arc<dyn ModuleHandler>);
    let t2 = Module::new("t2", "checkable without check")
        .interface_classes(["Writable"])
        .parameter(Parameter::new("value", kelvin(), "temperature"))
        .unwrap()
        .parameter(Parameter::new("status", status_info(), "status"))
        .unwrap()
        .parameter(checkable(
            Parameter::new("target", kelvin(), "setpoint").writable(),
        ))
        .unwrap()
        .no_polling()
        .handler(NoCheck);
    NodeBuilder::new("must-check", "check test node")
        .bind(SocketAddr::from(([127, 0, 0, 1], 0)))
        .initial_read(false)
        .log_files(LogFiles::off())
        .discovery(Discovery::off())
        .timeout(2.0)
        .module(t1)
        .unwrap()
        .module(t2)
        .unwrap()
        .build()
        .unwrap()
        .spawn(&Handle::current())
        .unwrap()
}

// --- Check

#[tokio::test]
async fn accepted_value() {
    let checker = Arc::new(Checker::default());
    let n = start(&checker);
    let mut c = Client::connect(n.local_addr()).await;
    c.send("activate t1").await;
    while c.line().await != "active t1" {}

    let (action, spec, data) = parse(&c.request("check t1:target 250").await);
    assert_eq!((action.as_str(), spec.as_str()), ("checked", "t1:target"));
    assert_eq!(data[0], json!(250.0));
    assert!(data[1]["t"].is_f64());
    assert_eq!(
        checker.checks(),
        [("target".to_owned(), Some(Value::Double(250.0)))]
    );
    assert_eq!(checker.changes.load(Ordering::SeqCst), 0);
    assert_eq!(
        c.try_line(Duration::from_millis(300)).await,
        None,
        "no update"
    );
}

#[tokio::test]
async fn not_checkable() {
    let checker = Arc::new(Checker::default());
    let n = start(&checker);
    let mut c = Client::connect(n.local_addr()).await;
    let line = c.request("check t1:plain 5").await;
    assert!(line.starts_with("error_check t1:plain "), "{line}");
    assert_eq!(error_class(&line), "NotCheckable");
    assert_eq!(checker.checks().len(), 0);
}

#[tokio::test]
async fn rejected_by_the_handler() {
    let checker = Arc::new(Checker::default());
    let n = start(&checker);
    let mut c = Client::connect(n.local_addr()).await;
    let (action, spec, data) = parse(&c.request("check t1:target 280").await);
    assert_eq!(
        (action.as_str(), spec.as_str()),
        ("error_check", "t1:target")
    );
    assert_eq!(data[0], "Impossible");
    assert_eq!(data[1], "heater off in this configuration");
    assert!(data[2].is_object());
}

#[tokio::test]
async fn value_outside_datainfo() {
    let checker = Arc::new(Checker::default());
    let n = start(&checker);
    let mut c = Client::connect(n.local_addr()).await;
    let line = c.request("check t1:target 400").await;
    assert!(line.starts_with("error_check t1:target "), "{line}");
    assert_eq!(error_class(&line), "RangeError");
    assert_eq!(checker.checks().len(), 0, "the handler is not asked");
}

#[tokio::test]
async fn value_beyond_dynamic_limit() {
    let checker = Arc::new(Checker::default());
    let n = start(&checker);
    n.update("t1", "target_max", Value::Double(200.0)).unwrap();
    let mut c = Client::connect(n.local_addr()).await;
    let line = c.request("check t1:target 250").await;
    assert!(line.starts_with("error_check t1:target "), "{line}");
    assert_eq!(error_class(&line), "RangeError");
    assert_eq!(checker.checks().len(), 0, "the handler is not asked");
}

#[tokio::test]
async fn readonly_parameter() {
    let checker = Arc::new(Checker::default());
    let n = start(&checker);
    let mut c = Client::connect(n.local_addr()).await;
    let line = c.request("check t1:value 5").await;
    assert!(line.starts_with("error_check t1:value "), "{line}");
    assert_eq!(error_class(&line), "ReadOnly");
}

#[tokio::test]
async fn command_argument() {
    let checker = Arc::new(Checker::default());
    let n = start(&checker);
    let mut c = Client::connect(n.local_addr()).await;
    let line = c.request("check t1:ramp \"fast\"").await;
    assert!(line.starts_with("error_check t1:ramp "), "{line}");
    assert_eq!(error_class(&line), "WrongType");
    assert_eq!(checker.checks().len(), 0);

    let (action, _, data) = parse(&c.request("check t1:ramp 1.5").await);
    assert_eq!(action, "checked");
    assert_eq!(data[0], json!(1.5));
    let (action, _, data) = parse(&c.request("check t1:stop").await);
    assert_eq!(action, "checked");
    assert_eq!(data[0], json!(null));
    assert_eq!(
        checker.checks(),
        [
            ("ramp".to_owned(), Some(Value::Double(1.5))),
            ("stop".to_owned(), None)
        ]
    );
}

#[tokio::test]
async fn unknown_accessible() {
    let checker = Arc::new(Checker::default());
    let n = start(&checker);
    let mut c = Client::connect(n.local_addr()).await;
    let line = c.request("check t1:nope 1").await;
    assert!(line.starts_with("error_check t1:nope "), "{line}");
    assert_eq!(error_class(&line), "NoSuchParameter");
    let line = c.request("check tx:target 1").await;
    assert_eq!(error_class(&line), "NoSuchModule");
}

// --- Check replies

#[tokio::test]
async fn other_clients_see_nothing() {
    let checker = Arc::new(Checker::default());
    let n = start(&checker);
    let mut a = Client::connect(n.local_addr()).await;
    let mut b = Client::connect(n.local_addr()).await;
    b.send("activate").await;
    while b.line().await != "active" {}
    assert!(
        a.request("check t1:target 250")
            .await
            .starts_with("checked t1:target ")
    );
    assert_eq!(b.try_line(Duration::from_millis(300)).await, None);
}

#[tokio::test]
async fn hanging_check() {
    let checker = Arc::new(Checker::default());
    let n = start(&checker);
    let mut c = Client::connect(n.local_addr()).await;
    let started = Instant::now();
    let line = c.request("check t1:hang 1").await;
    let took = started.elapsed();
    assert!(line.starts_with("error_check t1:hang "), "{line}");
    assert_eq!(error_class(&line), "TimeoutError");
    assert!(
        took >= Duration::from_millis(1900) && took < Duration::from_secs(4),
        "{took:?}"
    );
}

// --- Module handlers (sec-node-runtime)

#[tokio::test]
async fn check_not_implemented_by_the_handler() {
    let checker = Arc::new(Checker::default());
    let n = start(&checker);
    let mut c = Client::connect(n.local_addr()).await;
    let line = c.request("check t2:target 5").await;
    assert!(line.starts_with("error_check t2:target "), "{line}");
    assert_eq!(error_class(&line), "NotImplemented");
}

#[tokio::test]
async fn check_counts_as_a_request_of_the_module() {
    let checker = Arc::new(Checker::default());
    let n = start(&checker);
    let mut a = Client::connect(n.local_addr()).await;
    let mut b = Client::connect(n.local_addr()).await;
    a.send("change t1:target 100").await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(
        b.request("check t1:target 250")
            .await
            .starts_with("checked t1:target ")
    );
    assert!(a.line().await.starts_with("changed t1:target "));
    assert_eq!(checker.checks().len(), 1);
    assert_eq!(
        checker.overlapped.load(Ordering::SeqCst),
        0,
        "the check waited for the change"
    );
}

#[test]
fn checkable_readonly_parameter_warns() {
    let node = NodeBuilder::new("must-check", "check test node")
        .module(
            Module::new("t1", "a sensor")
                .interface_classes(["Readable"])
                .parameter(checkable(Parameter::new("value", kelvin(), "temperature")))
                .unwrap()
                .parameter(Parameter::new("status", status_info(), "status"))
                .unwrap(),
        )
        .unwrap()
        .build()
        .unwrap();
    assert!(
        node.warnings()
            .iter()
            .any(|w| w.path == "t1:value" && w.message.contains("checkable")),
        "{:?}",
        node.warnings()
    );
}
