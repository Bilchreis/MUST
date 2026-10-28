// SPDX-License-Identifier: LGPL-3.0-or-later

//! A SEC node with one simulated temperature controller.
//!
//! The module is polled: `value` and `status` every second (the reported temperature
//! carries ±5 mK of noise), `target` and `ramp` every minute. Clients can change the
//! fast rate through the `pollinterval` parameter.
//!
//! ```text
//! cargo run --example simple_node [ADDRESS]
//! ```
//!
//! ADDRESS defaults to `0.0.0.0:2055`. Try it with any SECoP client, or by hand:
//!
//! ```text
//! $ nc localhost 2055
//! describe
//! activate
//! change cryo:target 20
//! change cryo:pollinterval 0.2
//! ```

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use must_server::{
    CallResult, Command, Ctx, HandlerResult, Module, ModuleHandler, NodeBuilder, Parameter,
    Reading, RunningNode,
};
use must_types::{DataInfo, ErrorClass, SecopError, Value};

/// Noise of the temperature readback in K: ±5 mK.
const NOISE: f64 = 0.005;

/// The simulated hardware: a temperature that ramps towards its target.
struct Cryostat {
    temperature: f64,
    target: f64,
    /// Ramp rate in K/min.
    ramp: f64,
}

impl Cryostat {
    fn busy(&self) -> bool {
        (self.temperature - self.target).abs() > 0.01
    }

    fn status(&self) -> Value {
        if self.busy() {
            status(300, "ramping")
        } else {
            status(100, "at target")
        }
    }

    /// Advances the simulation by `dt`.
    fn step(&mut self, dt: Duration) {
        let max_step = self.ramp / 60.0 * dt.as_secs_f64();
        let diff = self.target - self.temperature;
        self.temperature += diff.clamp(-max_step, max_step);
    }
}

fn status(code: i64, text: &str) -> Value {
    Value::Tuple(vec![Value::Enum(code), Value::String(text.into())])
}

/// Connects SECoP requests to the simulation.
struct Handler(Arc<Mutex<Cryostat>>);

#[async_trait]
impl ModuleHandler for Handler {
    async fn read(&self, _ctx: &Ctx, parameter: &str) -> HandlerResult {
        let sim = self.0.lock().unwrap();
        let value = match parameter {
            // only the reported value is noisy; status and busy use the true temperature
            "value" => Value::Double(sim.temperature + (fastrand::f64() * 2.0 - 1.0) * NOISE),
            "status" => sim.status(),
            "target" => Value::Double(sim.target),
            "ramp" => Value::Double(sim.ramp),
            _ => return Err(SecopError::new(ErrorClass::NoSuchParameter, parameter)),
        };
        Ok(Reading::new(value))
    }

    async fn change(&self, ctx: &Ctx, parameter: &str, value: Value) -> HandlerResult {
        if parameter == "pollinterval" {
            // any rate is fine for the simulation; the node re-times its polling
            return Ok(Reading::new(value));
        }
        let new = value.as_f64().expect("checked against the datainfo");
        let status = {
            let mut sim = self.0.lock().unwrap();
            match parameter {
                "target" => sim.target = new,
                "ramp" => sim.ramp = new,
                _ => return Err(SecopError::new(ErrorClass::ReadOnly, parameter)),
            }
            sim.status()
        };
        // side effect: clients see the new status before the `changed` reply
        ctx.update("status", status)?;
        Ok(Reading::new(value))
    }

    async fn call(&self, ctx: &Ctx, command: &str, _argument: Option<Value>) -> CallResult {
        if command != "stop" {
            return Err(SecopError::new(ErrorClass::NoSuchCommand, command));
        }
        let (target, status) = {
            let mut sim = self.0.lock().unwrap();
            sim.target = sim.temperature;
            (sim.target, sim.status())
        };
        ctx.update("target", Value::Double(target))?;
        ctx.update("status", status)?;
        Ok(None)
    }
}

fn build(
    addr: SocketAddr,
    sim: &Arc<Mutex<Cryostat>>,
) -> Result<must_server::Node, Box<dyn std::error::Error>> {
    let kelvin = DataInfo::double()
        .min(0.0)
        .max(400.0)
        .unit("K")
        .fmtstr("%.3f")
        .build()?;
    let status_info = DataInfo::tuple(&[
        DataInfo::enumeration([("IDLE", 100), ("BUSY", 300), ("ERROR", 400)]).build()?,
        DataInfo::string().build()?,
    ])
    .build()?;
    let ramp = DataInfo::double()
        .min(0.0)
        .max(100.0)
        .unit("K/min")
        .build()?;
    let seconds = DataInfo::double().min(0.0).max(3600.0).unit("s").build()?;

    let cryo = Module::new(
        "cryo",
        "simulated cryostat\n\nRamps the temperature towards the target.",
    )
    .interface_classes(["Drivable"])
    .parameter(Parameter::new(
        "value",
        kelvin.clone(),
        "current temperature",
    ))?
    .parameter(Parameter::new("status", status_info, "current status"))?
    .parameter(Parameter::new("target", kelvin.clone(), "setpoint").writable())?
    .parameter(
        Parameter::new("target_max", kelvin, "upper limit for target")
            .writable()
            .cached()
            .initial_value(Value::Double(300.0)),
    )?
    .parameter(Parameter::new("ramp", ramp, "ramp rate").writable())?
    .parameter(
        Parameter::new(
            "pollinterval",
            seconds,
            "interval of value and status polling",
        )
        .writable()
        .cached()
        .initial_value(Value::Double(1.0)),
    )?
    .poll_interval(Duration::from_secs(1))
    .command(Command::new(
        "stop",
        DataInfo::command().build()?,
        "hold the current temperature",
    ))?
    .handler(Handler(Arc::clone(sim)));

    let node = NodeBuilder::new(
        "must-example",
        "must example node\n\nOne simulated cryostat.",
    )
    .firmware(concat!("must-", env!("CARGO_PKG_VERSION")))
    .bind(addr)
    .module(cryo)?
    .build()?;
    Ok(node)
}

/// Advances the simulation. Clients get `value` through polling; a change of `status`
/// is pushed at once.
async fn simulate(node: RunningNode, sim: Arc<Mutex<Cryostat>>) {
    const DT: Duration = Duration::from_millis(500);
    let mut ticker = tokio::time::interval(DT);
    loop {
        ticker.tick().await;
        let (status, was_busy, busy) = {
            let mut sim = sim.lock().unwrap();
            let was_busy = sim.busy();
            sim.step(DT);
            (sim.status(), was_busy, sim.busy())
        };
        if was_busy != busy {
            node.update("cryo", "status", status).unwrap();
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt().with_env_filter("info").init();
    let addr: SocketAddr = std::env::args()
        .nth(1)
        .as_deref()
        .unwrap_or("0.0.0.0:2055")
        .parse()?;
    let sim = Arc::new(Mutex::new(Cryostat {
        temperature: 295.0,
        target: 295.0,
        ramp: 60.0,
    }));

    let node = build(addr, &sim)?.spawn(&tokio::runtime::Handle::current())?;
    println!("listening on {}", node.local_addr());
    tokio::spawn(simulate(node.clone(), Arc::clone(&sim)));

    tokio::signal::ctrl_c().await?;
    node.stop().await;
    Ok(())
}
