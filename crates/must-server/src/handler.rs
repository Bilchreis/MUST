// SPDX-License-Identifier: LGPL-3.0-or-later

//! The interface between a SEC node and the application's hardware code.

use async_trait::async_trait;
use must_types::{ErrorClass, SecopError, Value};
use serde_json::Value as Json;

use std::sync::Arc;

use crate::events::LogLevel;
use crate::runtime::Hub;

/// A value obtained from the hardware, with optional qualifiers.
#[derive(Debug, Clone, PartialEq)]
pub struct Reading {
    /// The value; must fit the parameter's datainfo.
    pub value: Value,
    /// When the value was obtained, as UNIX time in seconds; the node uses the current
    /// time if `None`.
    pub t: Option<f64>,
    /// Uncertainty of the value, as JSON in the value's unit.
    pub e: Option<Json>,
}

impl Reading {
    /// A reading without qualifiers, timestamped by the node.
    pub fn new(value: Value) -> Self {
        Self {
            value,
            t: None,
            e: None,
        }
    }

    /// Sets the timestamp.
    #[must_use]
    pub fn at(mut self, t: f64) -> Self {
        self.t = Some(t);
        self
    }

    /// Sets the uncertainty.
    #[must_use]
    pub fn with_uncertainty(mut self, e: impl Into<Json>) -> Self {
        self.e = Some(e.into());
        self
    }
}

impl From<Value> for Reading {
    fn from(value: Value) -> Self {
        Self::new(value)
    }
}

/// The result of a handler call.
pub type HandlerResult = Result<Reading, SecopError>;

/// Context passed to handler calls. Updates sent through it reach activated clients
/// before the reply to the request being handled.
pub struct Ctx {
    pub(crate) hub: Arc<Hub>,
    pub(crate) module: String,
}

impl Ctx {
    /// Name of the module being handled.
    pub fn module(&self) -> &str {
        &self.module
    }

    /// Publishes a new value of a parameter of this module (a side effect of the current
    /// request), e.g. `status` becoming BUSY after a `change` of `target`. Fails if the
    /// parameter does not exist or the value does not fit its datainfo.
    pub fn update(&self, parameter: &str, reading: impl Into<Reading>) -> Result<(), SecopError> {
        self.hub
            .publish_value(&self.module, parameter, reading.into())
            .map(|_| ())
    }

    /// Publishes that a parameter of this module could not be obtained.
    pub fn update_error(&self, parameter: &str, error: SecopError) {
        self.hub.publish_error(&self.module, parameter, error);
    }
}

/// Hardware access for one module. Every method has a default that reports the action as
/// not implemented, so a module implements only what it supports.
///
/// The node calls the handler of a module for one request at a time.
#[async_trait]
pub trait ModuleHandler: Send + Sync {
    /// The module's `implementation` property unless the module sets one: by default the
    /// Rust type name of the handler, e.g. `"simple_node::Handler"`.
    fn implementation(&self) -> String {
        std::any::type_name::<Self>().to_owned()
    }

    /// Reads `parameter` from the hardware.
    async fn read(&self, ctx: &Ctx, parameter: &str) -> HandlerResult {
        let _ = ctx;
        Err(not_implemented("read", parameter))
    }

    /// Changes `parameter` to `value` (already checked against its datainfo and dynamic
    /// limits) and returns the value actually set.
    async fn change(&self, ctx: &Ctx, parameter: &str, value: Value) -> HandlerResult {
        let _ = (ctx, value);
        Err(not_implemented("change", parameter))
    }

    /// Executes `command` with its (already checked) argument, returning its result
    /// (`Value` of the result datainfo, or any value for a command without result).
    async fn call(&self, ctx: &Ctx, command: &str, argument: Option<Value>) -> CallResult {
        let _ = (ctx, argument);
        Err(not_implemented("do", command))
    }

    /// Polls the parameters `due` at this tick of the module's polling, in declaration
    /// order. The node sends only what is published through `ctx`: the default reads
    /// each parameter with [`read`](Self::read) and publishes the value or error.
    /// Override it to obtain several parameters in one hardware transaction.
    async fn poll(&self, ctx: &Ctx, due: &[&str]) {
        for &parameter in due {
            match self.read(ctx, parameter).await {
                Ok(reading) => {
                    // an invalid value is logged and published as an error by the node
                    let _ = ctx.update(parameter, reading);
                }
                Err(error) => {
                    ctx.hub.log(
                        LogLevel::Warn,
                        Some(&ctx.module),
                        format!("reading `{parameter}` failed: {error}"),
                    );
                    ctx.update_error(parameter, error);
                }
            }
        }
    }
}

/// The result of a command call: its result value, `None` for a command without result.
pub type CallResult = Result<Option<Reading>, SecopError>;

fn not_implemented(action: &str, accessible: &str) -> SecopError {
    SecopError::new(
        ErrorClass::NotImplemented,
        format!("{action} of `{accessible}` is not implemented"),
    )
}

/// The handler of modules that have none: everything is answered from the cache or as
/// not implemented.
pub(crate) struct NoHandler;

#[async_trait]
impl ModuleHandler for NoHandler {}
