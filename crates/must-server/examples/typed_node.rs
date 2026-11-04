// SPDX-License-Identifier: LGPL-3.0-or-later

//! The simulated cryostat of `simple_node.rs`, declared with `#[must_server::module]`
//! and `#[must_server::methods]` instead of the builder API and a hand-written handler.
//!
//! `value` and `status` are polled every second (the reported temperature carries ±5 mK
//! of noise); the simulation runs in a task started by the module's `start` hook and
//! pushes `status` changes through a parameter handle.
//!
//! ```text
//! cargo run --example typed_node [ADDRESS]
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

use must_server::NodeBuilder;
use must_server::typed::prelude::*;
use must_server::typed::{self, ParamHandle};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

/// Noise of the temperature readback in K: ±5 mK.
const NOISE: f64 = 0.005;

/// The simulated hardware: a temperature that ramps towards its target.
struct Sim {
    temperature: f64,
    target: f64,
    /// Ramp rate in K/min.
    ramp: f64,
}

impl Default for Sim {
    fn default() -> Self {
        Self {
            temperature: 295.0,
            target: 295.0,
            ramp: 60.0,
        }
    }
}

impl Sim {
    fn busy(&self) -> bool {
        (self.temperature - self.target).abs() > 0.01
    }

    fn status(&self) -> Status {
        if self.busy() {
            Status::busy("ramping")
        } else {
            Status::idle("at target")
        }
    }

    /// Advances the simulation by `dt`.
    fn step(&mut self, dt: Duration) {
        let max_step = self.ramp / 60.0 * dt.as_secs_f64();
        let diff = self.target - self.temperature;
        self.temperature += diff.clamp(-max_step, max_step);
    }
}

#[module(
    Drivable,
    description = "simulated cryostat\n\nRamps the temperature towards the target."
)]
struct Cryostat {
    value: param!(
        "current temperature",
        double(min = 0.0, max = 400.0, unit = "K", fmtstr = "%.3f")
    ),
    status: param!("current status", status(IDLE, BUSY, ERROR)),
    target: param!(
        "setpoint",
        double(min = 0.0, max = 400.0, unit = "K", fmtstr = "%.3f"),
        writable,
        default = 295.0
    ),
    target_max: param!(
        "upper limit for target",
        double(min = 0.0, max = 400.0, unit = "K", fmtstr = "%.3f"),
        writable,
        default = 300.0
    ),
    ramp: param!(
        "ramp rate",
        double(min = 0.0, max = 100.0, unit = "K/min"),
        writable,
        default = 60.0
    ),
    pollinterval: param!(
        "interval of value and status polling",
        double(min = 0.0, max = 3600.0, unit = "s"),
        writable,
        default = 1.0
    ),
    stop: command!("hold the current temperature"),

    sim: Arc<Mutex<Sim>>,
}

#[methods]
impl Cryostat {
    /// Starts the simulation, which pushes `status` changes.
    async fn start(&mut self) -> Result<()> {
        tokio::spawn(simulate(Arc::clone(&self.sim), self.status.handle()));
        Ok(())
    }

    async fn read_value(&mut self) -> Result<f64> {
        // only the reported value is noisy; status and busy use the true temperature
        let temperature = self.sim.lock().unwrap().temperature;
        Ok(temperature + (fastrand::f64() * 2.0 - 1.0) * NOISE)
    }

    async fn read_status(&mut self) -> Result<Status> {
        Ok(self.sim.lock().unwrap().status())
    }

    async fn write_target(&mut self, target: f64) -> Result<f64> {
        let status = {
            let mut sim = self.sim.lock().unwrap();
            sim.target = target;
            sim.status()
        };
        // side effect: clients see the new status before the `changed` reply
        self.status.set(status)?;
        Ok(target)
    }

    async fn write_ramp(&mut self, ramp: f64) -> Result<f64> {
        self.sim.lock().unwrap().ramp = ramp;
        Ok(ramp)
    }

    async fn do_stop(&mut self) -> Result<()> {
        let (target, status) = {
            let mut sim = self.sim.lock().unwrap();
            sim.target = sim.temperature;
            (sim.target, sim.status())
        };
        self.target.set(target)?;
        self.status.set(status)
    }
}

/// Advances the simulation until the node is gone. Clients get `value` through polling;
/// a change of `status` is pushed at once.
async fn simulate(sim: Arc<Mutex<Sim>>, status: ParamHandle<Status>) {
    const DT: Duration = Duration::from_millis(500);
    let mut ticker = tokio::time::interval(DT);
    loop {
        ticker.tick().await;
        let (now, was_busy, busy) = {
            let mut sim = sim.lock().unwrap();
            let was_busy = sim.busy();
            sim.step(DT);
            (sim.status(), was_busy, sim.busy())
        };
        if was_busy != busy && status.set(now).is_err() {
            break;
        }
    }
}

#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    // log to the terminal, and to the monitoring endpoint for must-tui's log view
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new("info"))
        .with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr))
        .with(must_server::monitor::log_layer())
        .init();
    let addr: SocketAddr = std::env::args()
        .nth(1)
        .as_deref()
        .unwrap_or("0.0.0.0:2055")
        .parse()?;

    let cryo = typed::module::<Cryostat>("cryo", None, serde_json::json!({}))?;
    let node = NodeBuilder::new(
        "must-example",
        "MUST example node\n\nOne simulated cryostat.",
    )
    .firmware(concat!("must-", env!("CARGO_PKG_VERSION")))
    .bind(addr)
    .module(cryo)?
    .build()?
    .spawn(&tokio::runtime::Handle::current())?;
    println!("listening on {}", node.local_addr());

    tokio::signal::ctrl_c().await?;
    node.stop().await;
    Ok(())
}
