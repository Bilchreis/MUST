// SPDX-License-Identifier: LGPL-3.0-or-later

//! Modules declared with `#[must_server::module]` and `#[must_server::methods]`.

mod common;

use std::net::SocketAddr;
use std::time::Duration;

use common::{Client, error_class, parse};
use must_server::logfiles::LogFiles;
use must_server::typed::prelude::*;
use must_server::typed::{self, TypedModule};
use must_server::{Discovery, Module, NodeBuilder, RunningNode};
use serde_json::{Value as Json, json};
use tokio::runtime::Handle;

#[derive(Clone, Debug, PartialEq, SecopEnum)]
enum Mode {
    Ramp = 1,
    Pid = 2,
}

#[derive(Clone, Debug, PartialEq, SecopStruct)]
struct Pid {
    #[datainfo(double(min = 0.0))]
    p: f64,
    #[datainfo(double(min = 0.0, max = 100.0))]
    i: Option<f64>,
}

#[module(Drivable, description = "simulated temperature loop")]
struct Controller {
    value: param!(
        "current temperature",
        double(min = 0.0, max = 1500.0, unit = "K", fmtstr = "%.3f")
    ),
    status: param!("current status", status(IDLE, WARN, BUSY, ERROR)),
    target: param!(
        "setpoint",
        double(min = 0.0, max = range, unit = unit),
        writable
    ),
    tolerance: param!(
        "convergence criterion",
        double(min = 0.0, unit = "K"),
        writable,
        default = 0.1
    ),
    mode: param!("regulation mode", enum(Mode), writable, default = Mode::Pid),
    pid: param!("regulation coefficients", struct(Pid), writable, default = Pid { p: 1.0, i: None }),
    setpoint: param!("ramped setpoint", double()),
    pollinterval: param!(
        "polling interval",
        double(min = 0.0, max = 60.0, unit = "s"),
        writable,
        default = 0.2
    ),
    stop: command!("stop at the current temperature"),
    wait: command!(
        "wait for stability",
        argument = double(min = 0.0, unit = "s"),
        result = bool()
    ),
    hold: command!("hold the setpoint at a value", argument = double()),

    range: property!("full scale", double(min = 0.0), default = 1000.0),
    unit: property!("unit", string(), default = "K"),
    channel: property!(
        "sensor channel",
        string(minchars = 1, maxchars = 1),
        default = "A"
    ),
    heater_loop: property!("heater loop", int(min = 1, max = 2) as u8, default = 1),

    driving: bool,
    last_pid: Option<Pid>,
}

#[methods]
impl Controller {
    async fn read_value(&mut self) -> Result<f64> {
        Ok(4.2)
    }

    async fn read_status(&mut self) -> Result<Status> {
        Ok(if self.driving {
            Status::busy("ramping")
        } else {
            Status::idle("ok")
        })
    }

    async fn write_target(&mut self, target: f64) -> Result<f64> {
        self.driving = true;
        self.status.set(Status::busy("target changed"))?;
        Ok(target)
    }

    async fn write_pid(&mut self, pid: Pid) -> Result<Pid> {
        self.last_pid = Some(pid.clone());
        // a member the client left out keeps its value; 10 marks that it was missing
        Ok(Pid {
            i: Some(pid.i.unwrap_or(10.0)),
            ..pid
        })
    }

    async fn do_stop(&mut self) -> Result<()> {
        self.driving = false;
        Ok(())
    }

    async fn do_wait(&mut self, seconds: f64) -> Result<bool> {
        Ok(seconds < 10.0 && self.query("stable?") == "yes")
    }

    async fn do_hold(&mut self, at: f64) -> Result<()> {
        self.setpoint.set(at)
    }

    /// A helper, not an accessible.
    fn query(&mut self, cmd: &str) -> String {
        let _ = (cmd, &self.channel, self.heater_loop);
        "yes".to_owned()
    }
}

fn instance(settings: Json) -> Module {
    typed::module::<Controller>("T", None, settings).unwrap()
}

fn serve(modules: Vec<Module>) -> RunningNode {
    let mut builder = NodeBuilder::new("must-test", "typed macro test node")
        .bind(SocketAddr::from(([127, 0, 0, 1], 0)))
        .log_files(LogFiles::off())
        .discovery(Discovery::off());
    for m in modules {
        builder = builder.module(m).unwrap();
    }
    builder.build().unwrap().spawn(&Handle::current()).unwrap()
}

async fn describe(n: &RunningNode) -> Json {
    let mut c = Client::connect(n.local_addr()).await;
    parse(&c.request("describe").await).2
}

#[tokio::test]
async fn drivable_module_and_declarations() {
    let n = serve(vec![instance(json!({}))]);
    let d = describe(&n).await;
    let t = &d["modules"]["T"];
    assert_eq!(t["interface_classes"], json!(["Drivable"]));
    assert_eq!(t["description"], "simulated temperature loop");
    let acc = &t["accessibles"];
    // Declared parameter in describe
    assert_eq!(acc["target"]["description"], "setpoint");
    assert_eq!(acc["target"]["readonly"], false);
    assert_eq!(
        acc["target"]["datainfo"],
        json!({"type": "double", "min": 0.0, "max": 1000.0, "unit": "K"})
    );
    // Unit is published
    assert_eq!(
        acc["value"]["datainfo"],
        json!({"type": "double", "min": 0.0, "max": 1500.0, "unit": "K", "fmtstr": "%.3f"})
    );
    // Status codes
    assert_eq!(
        acc["status"]["datainfo"],
        json!({"type": "tuple", "members": [
            {"type": "enum", "members": {"IDLE": 100, "WARN": 200, "BUSY": 300, "ERROR": 400}},
            {"type": "string"}]})
    );
    // Enum datainfo
    assert_eq!(
        acc["mode"]["datainfo"],
        json!({"type": "enum", "members": {"Ramp": 1, "Pid": 2}})
    );
    // Command with argument and result
    assert_eq!(
        acc["wait"]["datainfo"],
        json!({"type": "command",
               "argument": {"type": "double", "min": 0.0, "unit": "s"},
               "result": {"type": "bool"}})
    );
    // Property is not published; state fields neither
    for name in [
        "channel",
        "range",
        "unit",
        "heater_loop",
        "driving",
        "last_pid",
    ] {
        assert!(acc.get(name).is_none(), "{name} is published");
    }
    n.stop().await;
}

#[tokio::test]
async fn limit_and_unit_from_properties() {
    let n = serve(vec![
        typed::module::<Controller>("A", None, json!({"range": 200.0, "unit": "ln/min"})).unwrap(),
        typed::module::<Controller>("B", None, json!({"range": 500.0, "unit": "ln/min"})).unwrap(),
    ]);
    let d = describe(&n).await;
    let target = |m: &str| d["modules"][m]["accessibles"]["target"]["datainfo"].clone();
    assert_eq!(target("A")["max"], 200.0);
    assert_eq!(target("B")["max"], 500.0);
    assert_eq!(target("A")["unit"], "ln/min");
    let mut c = Client::connect(n.local_addr()).await;
    assert_eq!(
        error_class(&c.request("change A:target 250").await),
        "RangeError"
    );
    assert!(
        c.request("change B:target 250")
            .await
            .starts_with("changed")
    );
    n.stop().await;
}

#[test]
fn rust_types_of_declared_accessibles() {
    let declared = Controller::declare("T", "d", &mut json!({})).ok().unwrap();
    let m = declared.instance;
    // Default type of a double, Integer as u8, State field
    let _: &Param<f64> = &m.value;
    let _: &Param<Status> = &m.status;
    let _: &Param<Mode> = &m.mode;
    let _: u8 = m.heater_loop;
    let _: &String = &m.channel;
    assert!(!m.driving);
    assert_eq!(m.last_pid, None);
    assert_eq!(m.mode.get(), Mode::Pid);
    assert_eq!(m.tolerance.get(), 0.1);
}

#[test]
fn property_out_of_range() {
    let err = typed::module::<Controller>("T", None, json!({"heater_loop": 3}))
        .err()
        .unwrap()
        .to_string();
    assert!(err.contains("heater_loop"), "{err}");
}

#[test]
fn property_value_too_long() {
    let err = typed::module::<Controller>("T", None, json!({"channel": "AB"}))
        .err()
        .unwrap()
        .to_string();
    assert!(err.contains("channel"), "{err}");
}

#[module(description = "a module with a too wide range")]
struct TooWide {
    level: param!("level", int(min = 0, max = 300) as u8),
}

#[methods]
impl TooWide {}

#[test]
fn limits_too_wide() {
    let err = typed::module::<TooWide>("W", None, json!({}))
        .err()
        .unwrap()
        .to_string();
    assert!(err.contains("level") && err.contains("0..=300"), "{err}");
}

#[tokio::test]
async fn writable_parameter_without_methods() {
    let n = serve(vec![instance(json!({}))]);
    let mut c = Client::connect(n.local_addr()).await;
    c.send("activate").await;
    loop {
        if c.line().await == "active" {
            break;
        }
    }
    c.send("change T:tolerance 0.2").await;
    let mut lines = Vec::new();
    loop {
        let line = c.line().await;
        let done = line.starts_with("changed T:tolerance");
        lines.push(line);
        if done {
            break;
        }
    }
    assert!(
        lines
            .last()
            .unwrap()
            .starts_with("changed T:tolerance [0.2"),
        "{lines:?}"
    );
    assert!(
        lines
            .iter()
            .any(|l| l.starts_with("update T:tolerance [0.2")),
        "{lines:?}"
    );
    let (_, _, data) = parse(&c.request("read T:tolerance").await);
    assert_eq!(data[0], 0.2);
    n.stop().await;
}

#[tokio::test]
async fn parameter_without_read_method_is_not_polled() {
    let n = serve(vec![instance(json!({}))]);
    let mut c = Client::connect(n.local_addr()).await;
    assert!(c.request("do T:hold 12.5").await.starts_with("done T:hold"));
    c.send("activate").await;
    // value is polled every 0.2 s (pollinterval); setpoint, without a read method, never
    let deadline = tokio::time::Instant::now() + Duration::from_millis(1200);
    let mut value_updates = 0;
    let mut setpoint_updates = 0;
    while let Ok(Some(line)) =
        tokio::time::timeout_at(deadline, c.try_line(Duration::from_secs(2))).await
    {
        if line.starts_with("update T:value") {
            value_updates += 1;
        }
        if line.starts_with("update T:setpoint") {
            setpoint_updates += 1;
        }
    }
    assert!(value_updates >= 3, "value polled {value_updates} times");
    assert_eq!(setpoint_updates, 1, "only the initial update of activate");
    let (_, _, data) = parse(&c.request("read T:setpoint").await);
    assert_eq!(data[0], 12.5);
    n.stop().await;
}

#[tokio::test]
async fn optional_struct_member_and_commands() {
    let n = serve(vec![instance(json!({}))]);
    let mut c = Client::connect(n.local_addr()).await;
    let (action, _, data) = parse(&c.request(r#"change T:pid {"p": 40}"#).await);
    assert_eq!(action, "changed");
    // the method received `i: None`
    assert_eq!(data[0], json!({"p": 40.0, "i": 10.0}));
    let (_, _, data) = parse(&c.request("do T:wait 2").await);
    assert_eq!(data[0], true);
    assert_eq!(error_class(&c.request("do T:wait -1").await), "RangeError");
    assert!(c.request("change T:target 20").await.starts_with("changed"));
    let (_, _, data) = parse(&c.request("read T:status").await);
    assert_eq!(data[0][0], 300);
    assert!(c.request("do T:stop").await.starts_with("done T:stop"));
    let (_, _, data) = parse(&c.request("read T:status").await);
    assert_eq!(data[0][0], 100);
    n.stop().await;
}

#[test]
fn configured_values() {
    // target has a write method: written at start, not at construction
    let declared = Controller::declare("T", "d", &mut json!({"target": 300.0, "tolerance": 0.5}))
        .ok()
        .unwrap();
    assert_eq!(declared.initial_writes.len(), 1);
    assert_eq!(declared.initial_writes[0].0, "target");
    assert_eq!(declared.instance.tolerance.get(), 0.5);
    // value has a read method
    let err = Controller::declare("T", "d", &mut json!({"value": 4.2}))
        .err()
        .unwrap();
    assert!(err.to_string().contains("cannot be configured"), "{err}");
}

/// A methods block in a child module of the struct's module.
mod child {
    use must_server::typed::prelude::*;

    #[module(Readable, description = "a sensor")]
    pub struct Sensor {
        value: param!("value", double()),
        status: param!("status", status(IDLE, ERROR)),
    }

    mod behaviour {
        use must_server::typed::prelude::*;

        #[methods]
        impl super::Sensor {
            async fn read_value(&mut self) -> Result<f64> {
                Ok(1.5)
            }
        }
    }
}

#[tokio::test]
async fn methods_block_in_a_child_module() {
    let n = serve(vec![
        typed::module::<child::Sensor>("S", None, json!({})).unwrap(),
    ]);
    let mut c = Client::connect(n.local_addr()).await;
    let (_, _, data) = parse(&c.request("read S:value").await);
    assert_eq!(data[0], 1.5);
    n.stop().await;
}
