// SPDX-License-Identifier: LGPL-3.0-or-later

//! The interface between a SEC node and the application's hardware code.

use async_trait::async_trait;
use must_types::{ErrorClass, SecopError, Value};
use serde_json::Value as Json;

use std::sync::{Arc, Weak};

use crate::runtime::{Hub, ModuleShared};

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
///
/// A `Ctx` keeps its node alive. Do not store one in the handler (or in a task the handler
/// owns): the node owns its handlers, so the node would never be freed. To publish values
/// from a background task, store a [`Publisher`] from [`Ctx::publisher`] instead.
#[derive(Clone)]
pub struct Ctx {
    pub(crate) hub: Arc<Hub>,
    pub(crate) module: String,
    pub(crate) shared: Arc<ModuleShared>,
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

    /// Reads a parameter of this module the way a poll does and publishes the result: it
    /// shares a running read of the parameter if there is one, otherwise it calls
    /// [`ModuleHandler::read`] once, also for every request waiting to read the parameter.
    ///
    /// The read goes on when this future is dropped. Do not await it from the handler's
    /// read of the same parameter, which would wait for itself until the node's timeout.
    pub async fn read(&self, parameter: &str) -> Result<Reading, SecopError> {
        self.shared.read_for_poll(parameter).await
    }

    /// Whether the module's handler accepts overlapping calls, see
    /// [`Module::concurrent_requests`](crate::Module::concurrent_requests).
    pub fn concurrent_requests(&self) -> bool {
        self.shared.concurrent()
    }

    /// A handle for publishing values of this module from outside handler calls, e.g. from
    /// a background task, that does not keep the node alive.
    pub fn publisher(&self) -> Publisher {
        Publisher {
            hub: Arc::downgrade(&self.hub),
            module: self.module.clone(),
        }
    }
}

/// Publishes values of one module from anywhere, e.g. a background task reading the
/// hardware. Unlike [`Ctx`], it does not keep the node alive, so a handler may store it;
/// once the node is gone, publishing fails.
#[derive(Clone, Debug)]
pub struct Publisher {
    hub: Weak<Hub>,
    module: String,
}

impl Publisher {
    /// Name of the module.
    pub fn module(&self) -> &str {
        &self.module
    }

    /// Publishes a new value of a parameter of the module, see [`Ctx::update`]. Also fails
    /// if the node is gone.
    pub fn update(&self, parameter: &str, reading: impl Into<Reading>) -> Result<(), SecopError> {
        self.hub()?
            .publish_value(&self.module, parameter, reading.into())
            .map(|_| ())
    }

    /// Publishes that a parameter of the module could not be obtained, see
    /// [`Ctx::update_error`]. Fails if the node is gone.
    pub fn update_error(&self, parameter: &str, error: SecopError) -> Result<(), SecopError> {
        self.hub()?.publish_error(&self.module, parameter, error);
        Ok(())
    }

    fn hub(&self) -> Result<Arc<Hub>, SecopError> {
        self.hub
            .upgrade()
            .ok_or_else(|| SecopError::new(ErrorClass::InternalError, "the node is gone"))
    }
}

/// Hardware access for one module. Every method has a default that reports the action as
/// not implemented, so a module implements only what it supports.
///
/// The node calls the handler of a module for one request at a time, unless the module
/// is declared with [`Module::concurrent_requests`](crate::Module::concurrent_requests).
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
    /// order. The node sends only what is published through `ctx`: the default reads each
    /// parameter with [`Ctx::read`], which publishes the value or error, one after another
    /// or, on a module with concurrent requests, all at once. Override it to obtain several
    /// parameters in one hardware transaction.
    async fn poll(&self, ctx: &Ctx, due: &[&str]) {
        if ctx.concurrent_requests() {
            let mut reads = tokio::task::JoinSet::new();
            for &parameter in due {
                let (ctx, parameter) = (ctx.clone(), parameter.to_owned());
                reads.spawn(async move {
                    let _ = ctx.read(&parameter).await;
                });
            }
            while reads.join_next().await.is_some() {}
        } else {
            for &parameter in due {
                let _ = ctx.read(parameter).await;
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
