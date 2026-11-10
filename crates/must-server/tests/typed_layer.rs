// SPDX-License-Identifier: LGPL-3.0-or-later

//! The typed module layer with a `TypedModule` written by hand, without the macros.

mod common;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

use common::{Client, error_class, parse, sensor};
use must_server::logfiles::LogFiles;
use must_server::typed::{
    self, BoxFuture, CommandSpec, Declared, Declaring, Di, Methods, Param, ParamSpec, Result,
    SecopType, Settings, SettingsErrors, Status, TypedModule, has,
};
use must_server::{Discovery, Module, NodeBuilder, NodeEvent, RunningNode};
use must_types::{DataInfo, ErrorClass, SecopError, Value};
use serde_json::{Value as Json, json};
use tokio::runtime::Handle;

/// What the modules did, per module name, in order.
static RECORD: LazyLock<Mutex<HashMap<String, Vec<String>>>> = LazyLock::new(Mutex::default);
/// How many methods of a module run at the same time, and the most seen.
static ACTIVE: LazyLock<Mutex<HashMap<String, (usize, usize)>>> = LazyLock::new(Mutex::default);
static NEXT: AtomicUsize = AtomicUsize::new(0);

fn record(module: &str, event: impl Into<String>) {
    RECORD
        .lock()
        .unwrap()
        .entry(module.to_owned())
        .or_default()
        .push(event.into());
}

fn recorded(module: &str) -> Vec<String> {
    RECORD
        .lock()
        .unwrap()
        .get(module)
        .cloned()
        .unwrap_or_default()
}

fn enter(module: &str) {
    let mut active = ACTIVE.lock().unwrap();
    let entry = active.entry(module.to_owned()).or_default();
    entry.0 += 1;
    entry.1 = entry.1.max(entry.0);
}

fn leave(module: &str) {
    ACTIVE.lock().unwrap().get_mut(module).unwrap().0 -= 1;
}

fn most_active(module: &str) -> usize {
    ACTIVE.lock().unwrap().get(module).map_or(0, |e| e.1)
}

/// A unique module name per test, as the record is global.
fn unique() -> String {
    format!("T{}", NEXT.fetch_add(1, Ordering::SeqCst))
}

/// A Drivable module written by hand.
struct Hand {
    name: String,
    value: Param<f64>,
    status: Param<Status>,
    target: Param<f64>,
    curve: Param<Vec<f64>>,
    tolerance: Param<f64>,
    reading: f64,
    fail_start: bool,
    push: bool,
    slow_write: bool,
}

impl Methods for Hand {
    const READ: &'static [&'static str] = &["value", "status", "curve"];
    const WRITE: &'static [&'static str] = &["target"];

    fn read_param<'a>(
        &'a mut self,
        parameter: &str,
        info: &DataInfo,
    ) -> Option<BoxFuture<'a, Result<Value>>> {
        let info = info.clone();
        match parameter {
            "value" => Some(Box::pin(async move {
                record(&self.name, "read value");
                enter(&self.name);
                tokio::time::sleep(Duration::from_millis(20)).await;
                leave(&self.name);
                self.reading.to_value(&info)
            })),
            "status" => Some(Box::pin(async move { Status::idle("ok").to_value(&info) })),
            "curve" => Some(Box::pin(async move { vec![1.0; 20].to_value(&info) })),
            _ => None,
        }
    }

    fn write_param<'a>(
        &'a mut self,
        parameter: &str,
        value: Value,
        info: &DataInfo,
    ) -> Option<BoxFuture<'a, Result<Value>>> {
        let info = info.clone();
        match parameter {
            "target" => Some(Box::pin(async move {
                let target = f64::from_value(&value, &info)?;
                record(&self.name, format!("write target {target}"));
                if (target - 999.0).abs() < f64::EPSILON {
                    return Err(SecopError::new(
                        ErrorClass::CommunicationFailed,
                        "no answer from controller",
                    ));
                }
                enter(&self.name);
                if self.slow_write {
                    tokio::time::sleep(Duration::from_millis(300)).await;
                }
                leave(&self.name);
                self.status.set(Status::busy("target changed"))?;
                target.to_value(&info)
            })),
            _ => None,
        }
    }

    fn call_command<'a>(
        &'a mut self,
        command: &str,
        _argument: Option<Value>,
        _argument_info: Option<&DataInfo>,
        result_info: Option<&DataInfo>,
    ) -> BoxFuture<'a, Result<Option<Value>>> {
        let result_info = result_info.cloned();
        let command = command.to_owned();
        Box::pin(async move {
            match command.as_str() {
                "stop" => Ok(None),
                // the latest value of `value`, as the module sees it
                "last" => self.value.get().to_value(&result_info.unwrap()).map(Some),
                _ => Err(SecopError::new(ErrorClass::NoSuchCommand, command)),
            }
        })
    }

    fn start_hook(&mut self) -> BoxFuture<'_, Result<()>> {
        Box::pin(async move {
            tokio::time::sleep(Duration::from_millis(200)).await;
            record(&self.name, "start");
            if self.push {
                let handle = self.value.handle();
                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_millis(300)).await;
                    handle.set(300.5).unwrap();
                });
            }
            if self.fail_start {
                return Err(SecopError::new(
                    ErrorClass::HardwareError,
                    "controller not found",
                ));
            }
            Ok(())
        })
    }
}

impl TypedModule for Hand {
    const DEFAULT_DESCRIPTION: Option<&'static str> = Some("a typed module written by hand");

    fn declare(
        name: &str,
        description: &str,
        settings: &mut dyn Settings,
    ) -> Result<Declared<Self>, SettingsErrors> {
        let mut d = Declaring::new(name, description, &["Drivable"], settings);
        let reading: f64 = d.property("reading", Di::new("double").build(), Some(4.2));
        let fail_start: bool = d.property("fail_start", Di::new("bool").build(), Some(false));
        let push: bool = d.property("push", Di::new("bool").build(), Some(false));
        let slow_write: bool = d.property("slow_write", Di::new("bool").build(), Some(false));
        let _heater_range: u8 = d.property(
            "heater_range",
            Di::new("int").set("min", 0).set("max", 5).build(),
            None,
        );
        let kelvin = || {
            Di::new("double")
                .set("min", 0.0)
                .set("max", 1500.0)
                .set("unit", "K")
        };
        let value = d.param(ParamSpec::<f64>::new(
            "value",
            "temperature",
            kelvin().build(),
            has(Self::READ, "value"),
            false,
        ));
        let status = d.param(ParamSpec::<Status>::new(
            "status",
            "status",
            Di::status(&["IDLE", "BUSY", "ERROR"]).build(),
            true,
            false,
        ));
        let target = d.param(
            ParamSpec::<f64>::new(
                "target",
                "setpoint",
                kelvin().build(),
                has(Self::READ, "target"),
                has(Self::WRITE, "target"),
            )
            .writable(),
        );
        let curve = d.param(ParamSpec::<Vec<f64>>::new(
            "curve",
            "calibration curve",
            Di::new("array")
                .members(vec![Di::new("double")])
                .set("maxlen", 10)
                .build(),
            true,
            false,
        ));
        let tolerance = d.param(
            ParamSpec::<f64>::new(
                "tolerance",
                "convergence criterion",
                Di::new("double").set("min", 0.0).build(),
                has(Self::READ, "tolerance"),
                has(Self::WRITE, "tolerance"),
            )
            .writable()
            .default(0.1),
        );
        d.command(CommandSpec::new("stop", "stop"));
        d.command(CommandSpec::new("last", "the latest value").result::<f64>(kelvin().build()));
        let instance = Self {
            name: name.to_owned(),
            value,
            status,
            target,
            curve,
            tolerance,
            reading,
            fail_start,
            push,
            slow_write,
        };
        d.finish(instance)
    }

    fn params(&self) -> Vec<&dyn typed::AnyParam> {
        vec![
            &self.value,
            &self.status,
            &self.target,
            &self.curve,
            &self.tolerance,
        ]
    }
}

fn hand(name: &str, settings: Json) -> std::result::Result<Module, SettingsErrors> {
    let mut settings = settings;
    settings
        .as_object_mut()
        .unwrap()
        .entry("heater_range")
        .or_insert(json!(3));
    typed::module::<Hand>(name, None, settings)
}

fn serve(modules: Vec<Module>) -> RunningNode {
    let mut builder = NodeBuilder::new("must-test", "typed layer test node")
        .bind(SocketAddr::from(([127, 0, 0, 1], 0)))
        .log_files(LogFiles::off())
        .discovery(Discovery::off());
    for m in modules {
        builder = builder.module(m.no_polling()).unwrap();
    }
    builder.build().unwrap().spawn(&Handle::current()).unwrap()
}

/// Reads lines until one starts with `prefix`, returning everything read.
async fn until(c: &mut Client, prefix: &str) -> Vec<String> {
    let mut lines = Vec::new();
    loop {
        let line = c.line().await;
        let done = line.starts_with(prefix);
        lines.push(line);
        if done {
            return lines;
        }
    }
}

#[tokio::test]
async fn value_within_limits_reaches_the_method() {
    let name = unique();
    let n = serve(vec![hand(&name, json!({})).unwrap()]);
    let mut c = Client::connect(n.local_addr()).await;
    let reply = c.request(&format!("change {name}:target 2000")).await;
    assert_eq!(error_class(&reply), "RangeError");
    assert!(
        !recorded(&name)
            .iter()
            .any(|e| e.starts_with("write target"))
    );
    n.stop().await;
}

#[tokio::test]
async fn measured_value_out_of_range() {
    let name = unique();
    let n = serve(vec![hand(&name, json!({"reading": 1600.0})).unwrap()]);
    let mut c = Client::connect(n.local_addr()).await;
    let (action, _, data) = parse(&c.request(&format!("read {name}:value")).await);
    assert_eq!((action.as_str(), data[0].as_f64()), ("reply", Some(1600.0)));
    n.stop().await;
}

#[tokio::test]
async fn array_too_long() {
    let name = unique();
    let n = serve(vec![hand(&name, json!({})).unwrap()]);
    let mut c = Client::connect(n.local_addr()).await;
    let reply = c.request(&format!("read {name}:curve")).await;
    assert!(
        reply.starts_with(&format!("error_read {name}:curve")),
        "{reply}"
    );
    assert_eq!(error_class(&reply), "InternalError");
    n.stop().await;
}

#[tokio::test]
async fn latest_read_value() {
    let name = unique();
    let n = serve(vec![hand(&name, json!({})).unwrap()]);
    let mut c = Client::connect(n.local_addr()).await;
    c.request(&format!("read {name}:value")).await;
    let (_, _, data) = parse(&c.request(&format!("do {name}:last")).await);
    assert_eq!(data[0].as_f64(), Some(4.2));
    n.stop().await;
}

#[tokio::test]
async fn status_set_while_writing_the_target() {
    let name = unique();
    let n = serve(vec![hand(&name, json!({})).unwrap()]);
    let mut c = Client::connect(n.local_addr()).await;
    c.send("activate").await;
    until(&mut c, "active").await;
    c.send(&format!("change {name}:target 10")).await;
    let lines = until(&mut c, "changed").await;
    let status = lines
        .iter()
        .position(|l| l.starts_with(&format!("update {name}:status [[300,")));
    assert!(status.is_some(), "{lines:?}");
    n.stop().await;
}

#[tokio::test]
async fn simulation_task_pushes_the_value() {
    let name = unique();
    let n = serve(vec![hand(&name, json!({"push": true})).unwrap()]);
    let mut c = Client::connect(n.local_addr()).await;
    c.send("activate").await;
    until(&mut c, "active").await;
    until(&mut c, &format!("update {name}:value [300.5")).await;
    let (_, _, data) = parse(&c.request(&format!("do {name}:last")).await);
    assert_eq!(data[0].as_f64(), Some(300.5));
    n.stop().await;
}

#[tokio::test]
async fn overlapping_requests() {
    let name = unique();
    let n = serve(vec![hand(&name, json!({"slow_write": true})).unwrap()]);
    let mut a = Client::connect(n.local_addr()).await;
    let mut b = Client::connect(n.local_addr()).await;
    a.send(&format!("change {name}:target 10")).await;
    b.send(&format!("read {name}:value")).await;
    assert!(a.line().await.starts_with("changed"));
    assert!(b.line().await.starts_with("reply"));
    assert_eq!(most_active(&name), 1);
    n.stop().await;
}

#[tokio::test]
async fn connection_opened_in_the_start_hook() {
    let name = unique();
    let n = serve(vec![hand(&name, json!({})).unwrap()]);
    let mut c = Client::connect(n.local_addr()).await;
    c.request(&format!("read {name}:value")).await;
    let events = recorded(&name);
    assert_eq!(
        events.first().map(String::as_str),
        Some("start"),
        "{events:?}"
    );
    n.stop().await;
}

#[tokio::test]
async fn typed_start_hook_fails() {
    let name = unique();
    let n = serve(vec![hand(&name, json!({"fail_start": true})).unwrap()]);
    let mut events = n.subscribe();
    let message = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let NodeEvent::Log {
                module, message, ..
            } = events.recv().await.unwrap()
                && module.as_deref() == Some(name.as_str())
            {
                return message;
            }
        }
    })
    .await
    .unwrap();
    assert!(message.contains("controller not found"), "{message}");
    let mut c = Client::connect(n.local_addr()).await;
    assert!(
        c.request(&format!("read {name}:value"))
            .await
            .starts_with("reply")
    );
    n.stop().await;
}

#[test]
fn missing_property() {
    let err = typed::module::<Hand>("T", None, json!({})).err().unwrap();
    let text = err.to_string();
    assert!(
        text.contains("module `T`") && text.contains("heater_range"),
        "{text}"
    );
}

#[test]
fn unknown_setting() {
    let err = hand("T", json!({"chanel": "A"})).err().unwrap();
    let text = err.to_string();
    assert!(text.contains("chanel"), "{text}");
}

#[tokio::test]
async fn target_written_at_start() {
    let name = unique();
    let n = serve(vec![hand(&name, json!({"target": 300.0})).unwrap()]);
    let mut c = Client::connect(n.local_addr()).await;
    let (_, _, data) = parse(&c.request(&format!("read {name}:target")).await);
    assert_eq!(data[0].as_f64(), Some(300.0));
    let events = recorded(&name);
    let writes: Vec<_> = events.iter().filter(|e| e.starts_with("write")).collect();
    assert_eq!(writes, ["write target 300"]);
    let pos = |e: &str| events.iter().position(|x| x == e).unwrap();
    assert!(pos("start") < pos("write target 300"));
    assert!(pos("write target 300") < pos("read value"), "{events:?}");
    n.stop().await;
}

#[test]
fn configured_readonly_parameter() {
    let err = hand("T", json!({"value": 4.2})).err().unwrap();
    let text = err.to_string();
    assert!(
        text.contains("`value`") && text.contains("cannot be configured"),
        "{text}"
    );
}

#[tokio::test]
async fn initial_write_fails() {
    let name = unique();
    let n = serve(vec![hand(&name, json!({"target": 999.0})).unwrap()]);
    let mut events = n.subscribe();
    let message = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let NodeEvent::Log {
                module, message, ..
            } = events.recv().await.unwrap()
                && module.as_deref() == Some(name.as_str())
                && message.contains("initial write")
            {
                return message;
            }
        }
    })
    .await
    .unwrap();
    assert!(message.contains("target"), "{message}");
    let mut c = Client::connect(n.local_addr()).await;
    let reply = c.request(&format!("read {name}:status")).await;
    assert!(
        reply.starts_with("reply"),
        "the node keeps serving: {reply}"
    );
    c.send("activate").await;
    let lines = until(&mut c, "active").await;
    assert!(
        lines
            .iter()
            .any(|l| l.starts_with(&format!("error_update {name}:target"))),
        "{lines:?}"
    );
    n.stop().await;
}

#[tokio::test]
async fn typed_and_hand_written_modules_together() {
    let name = unique();
    let n = serve(vec![hand(&name, json!({})).unwrap(), sensor("t2")]);
    let mut c = Client::connect(n.local_addr()).await;
    let (_, _, data) = parse(&c.request("describe").await);
    let modules = data["modules"].as_object().unwrap();
    assert!(modules.contains_key(&name) && modules.contains_key("t2"));
    assert!(
        c.request(&format!("read {name}:value"))
            .await
            .starts_with("reply")
    );
    assert!(c.request("read t2:value").await.starts_with("reply"));
    n.stop().await;
}

#[tokio::test]
async fn writable_parameter_without_a_write_method_stores() {
    let name = unique();
    let n = serve(vec![hand(&name, json!({"tolerance": 0.3})).unwrap()]);
    let mut c = Client::connect(n.local_addr()).await;
    let (_, _, data) = parse(&c.request(&format!("read {name}:tolerance")).await);
    assert_eq!(
        data[0].as_f64(),
        Some(0.3),
        "configured value without a write method"
    );
    let (action, _, data) = parse(&c.request(&format!("change {name}:tolerance 0.2")).await);
    assert_eq!((action.as_str(), data[0].as_f64()), ("changed", Some(0.2)));
    let (_, _, data) = parse(&c.request(&format!("read {name}:tolerance")).await);
    assert_eq!(data[0].as_f64(), Some(0.2));
    n.stop().await;
}
