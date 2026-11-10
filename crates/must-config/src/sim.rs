// SPDX-License-Identifier: LGPL-3.0-or-later

//! Simulated module types, so that configs can be tried without hardware:
//! `must::sim::Temperature` and `must::sim::FlowController`.
//!
//! The simulations advance by the time elapsed whenever a method runs, so they need no
//! background task; polling keeps clients up to date.

use must_server::typed::prelude::*;
use tokio::time::Instant;

use crate::registry::{Registry, RegistryError};

/// Registers the simulation types.
pub fn register(registry: &mut Registry) -> Result<(), RegistryError> {
    registry.register::<Temperature>("must::sim::Temperature")?;
    registry.register::<FlowController>("must::sim::FlowController")
}

/// The seconds since `last`, which becomes now.
fn elapsed(last: &mut Option<Instant>) -> f64 {
    let now = Instant::now();
    let dt = last.map_or(0.0, |t| now.duration_since(t).as_secs_f64());
    *last = Some(now);
    dt
}

/// A temperature that ramps towards its target.
#[allow(clippy::struct_field_names)]
#[module(
    Drivable,
    description = "simulated temperature\n\nRamps towards the target at the ramp rate."
)]
pub struct Temperature {
    value: param!(
        "current temperature",
        double(min = 0.0, unit = "K", fmtstr = "%.3f")
    ),
    status: param!("current status", status(IDLE, BUSY, ERROR)),
    target: param!(
        "setpoint",
        double(min = 0.0, max = 1000.0, unit = "K", fmtstr = "%.3f"),
        writable,
        default = temperature
    ),
    ramp: param!(
        "ramp rate",
        double(min = 0.0, max = 1000.0, unit = "K/min"),
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

    temperature: property!("temperature at start", double(min = 0.0), default = 295.0),

    current: f64,
    last: Option<Instant>,
}

impl Temperature {
    fn advance(&mut self) {
        let dt = elapsed(&mut self.last);
        let max_step = self.ramp.get() / 60.0 * dt;
        let diff = self.target.get() - self.current;
        self.current += diff.clamp(-max_step, max_step);
    }

    fn state(&self) -> Status {
        Self::state_for(self.current, self.target.get())
    }

    fn state_for(current: f64, target: f64) -> Status {
        if (current - target).abs() > 1e-6 {
            Status::busy("ramping")
        } else {
            Status::idle("at target")
        }
    }
}

#[methods]
impl Temperature {
    async fn start(&mut self) -> Result<()> {
        self.current = self.temperature;
        self.last = Some(Instant::now());
        Ok(())
    }

    async fn read_value(&mut self) -> Result<f64> {
        self.advance();
        Ok(self.current)
    }

    async fn read_status(&mut self) -> Result<Status> {
        self.advance();
        Ok(self.state())
    }

    async fn write_target(&mut self, target: f64) -> Result<f64> {
        self.advance();
        // clients see the new status before the `changed` reply
        self.status.set(Self::state_for(self.current, target))?;
        Ok(target)
    }

    async fn do_stop(&mut self) -> Result<()> {
        self.advance();
        self.target.set(self.current)?;
        self.status.set(self.state())
    }
}

/// A mass flow controller: the flow follows the target as a first-order lag, settling
/// within the settling time.
#[module(Drivable, description = "simulated mass flow controller")]
pub struct FlowController {
    value: param!(
        "current flow",
        double(min = 0.0, unit = unit, fmtstr = "%.3f")
    ),
    status: param!("current status", status(IDLE, BUSY, ERROR)),
    target: param!(
        "flow setpoint",
        double(min = 0.0, max = range, unit = unit),
        writable
    ),
    pollinterval: param!(
        "interval of value and status polling",
        double(min = 0.0, max = 3600.0, unit = "s"),
        writable,
        default = 1.0
    ),
    stop: command!("hold the current flow"),

    range: property!("full scale flow", double(min = 0.0)),
    unit: property!("flow unit", string(), default = "ln/min"),
    settling_time: property!(
        "time to settle after a change",
        double(min = 0.0, unit = "s"),
        default = 2.0
    ),

    flow: f64,
    last: Option<Instant>,
}

impl FlowController {
    fn advance(&mut self) {
        let dt = elapsed(&mut self.last);
        let target = self.target.get();
        // e^-7 < 0.1 %: settled within the settling time
        let tau = self.settling_time / 7.0;
        if tau <= 0.0 {
            self.flow = target;
        } else {
            self.flow += (target - self.flow) * (1.0 - (-dt / tau).exp());
        }
        if (self.flow - target).abs() <= self.range * 1e-3 {
            self.flow = target;
        }
    }

    fn state(&self) -> Status {
        Self::state_for(self.flow, self.target.get())
    }

    fn state_for(flow: f64, target: f64) -> Status {
        if (flow - target).abs() > 0.0 {
            Status::busy("settling")
        } else {
            Status::idle("stable")
        }
    }
}

#[methods]
impl FlowController {
    async fn start(&mut self) -> Result<()> {
        self.last = Some(Instant::now());
        Ok(())
    }

    async fn read_value(&mut self) -> Result<f64> {
        self.advance();
        Ok(self.flow)
    }

    async fn read_status(&mut self) -> Result<Status> {
        self.advance();
        Ok(self.state())
    }

    async fn write_target(&mut self, target: f64) -> Result<f64> {
        self.advance();
        self.status.set(Self::state_for(self.flow, target))?;
        Ok(target)
    }

    async fn do_stop(&mut self) -> Result<()> {
        self.advance();
        self.target.set(self.flow)?;
        self.status.set(self.state())
    }
}
