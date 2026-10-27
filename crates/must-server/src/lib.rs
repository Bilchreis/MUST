// SPDX-License-Identifier: LGPL-3.0-or-later

//! A SECoP 2.0 SEC node library.
//!
//! A SEC node offers hardware as *modules* with *parameters* and *commands* over the
//! [SECoP](https://github.com/SampleEnvironment/SECoP) protocol. With this crate an
//! application
//!
//! 1. declares the node with [`NodeBuilder`], [`Module`], [`Parameter`] and [`Command`];
//!    [`NodeBuilder::build`] checks the declaration against the SECoP 2.0 rules,
//! 2. connects each module to its hardware code with a [`ModuleHandler`],
//! 3. starts the node with [`Node::start`] (on a runtime owned by the library) or
//!    [`Node::spawn`] (on the application's tokio runtime), and
//! 4. pushes new values with [`RunningNode::update`] whenever the hardware reports them.
//!
//! The node answers `*IDN?`, `describe`, `activate`/`deactivate`, `ping`, `read`,
//! `change` and `do`, validates incoming values against their datainfo (including
//! dynamic limits from `_min`/`_max`/`_limits` parameters), sends updates to activated
//! clients, and keeps SECoP's ordering rules: updates caused by a request reach clients
//! before its reply. The optional `check` and `logging` messages are answered with
//! `ProtocolError`.
//!
//! # Example
//!
//! ```
//! use async_trait::async_trait;
//! use must_server::{Ctx, HandlerResult, Module, ModuleHandler, NodeBuilder, Parameter, Reading};
//! use must_types::{DataInfo, Value};
//!
//! struct Thermometer;
//!
//! #[async_trait]
//! impl ModuleHandler for Thermometer {
//!     async fn read(&self, _ctx: &Ctx, parameter: &str) -> HandlerResult {
//!         let value = match parameter {
//!             "value" => Value::Double(4.2), // ask the hardware here
//!             _ => Value::Tuple(vec![Value::Enum(100), Value::String("ok".into())]),
//!         };
//!         Ok(Reading::new(value))
//!     }
//! }
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let kelvin = DataInfo::double().min(0.0).unit("K").build()?;
//! let status = DataInfo::tuple(&[
//!     DataInfo::enumeration([("IDLE", 100), ("ERROR", 400)]).build()?,
//!     DataInfo::string().build()?,
//! ])
//! .build()?;
//!
//! let node = NodeBuilder::new("HZB-example", "an example node")
//!     .bind("127.0.0.1:0".parse()?)
//!     .module(
//!         Module::new("t1", "sample thermometer")
//!             .interface_classes(["Readable"])
//!             .parameter(Parameter::new("value", kelvin, "temperature"))?
//!             .parameter(Parameter::new("status", status, "status"))?
//!             .handler(Thermometer),
//!     )?
//!     .build()?;
//!
//! let running = node.start()?;
//! println!("SEC node listening on {}", running.local_addr());
//! // ... later, e.g. from a hardware callback:
//! running.update("t1", "value", Value::Double(4.25))?;
//! running.stop_blocking();
//! # Ok(())
//! # }
//! ```
//!
//! See `examples/simple_node.rs` for a complete simulated temperature controller.

pub mod events;
pub mod handler;
pub mod model;
pub mod monitor;
mod protocol;
mod report;
mod runtime;

pub use events::{ConnectionId, LogLevel, NodeEvent};

pub use handler::{CallResult, Ctx, HandlerResult, ModuleHandler, Reading};
pub use model::{
    BuildError, Command, ModelError, ModelErrorKind, ModelWarning, Module, Node, NodeBuilder,
    Parameter,
};
pub use runtime::{NODE_LOG_TARGET, RunningNode, StartError};
