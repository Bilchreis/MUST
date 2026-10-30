// SPDX-License-Identifier: LGPL-3.0-or-later

//! The `must-server` handler of a module built through the C API: calls the
//! application's C functions (callback mode) or queues requests for
//! `SECoP_S_getStoredCommand` (polling mode).

use std::collections::HashMap;
use std::ffi::{CString, c_int};
use std::ptr;

use async_trait::async_trait;
use must_server::{CallResult, Ctx, HandlerResult, ModuleHandler, Reading};
use must_types::datainfo::CommandInfo;
use must_types::{DataInfo, ErrorClass, Mode, SecopError, Value};
use serde_json::Value as Json;
use tokio::sync::oneshot;

use super::build::{CallFunction, GetSet};
use super::codes::{action, to_secop};
use super::state;
use crate::variant::{self, clone_handle, destroy_handle, new_handle};

/// An accessible of the module with what is needed to call the application.
pub(crate) enum Accessible {
    Parameter {
        /// `node:module:parameter`, as passed to the callbacks.
        full_name: CString,
        info: DataInfo,
        /// The value type, from which handles for the callbacks are made.
        proto: variant::Node,
        getter: Option<GetSet>,
        setter: Option<GetSet>,
    },
    Command {
        full_name: CString,
        info: CommandInfo,
        argument: Option<variant::Node>,
        result: Option<variant::Node>,
        function: Option<CallFunction>,
    },
}

/// The handler of a module built through the C API.
pub(crate) struct ShallModule {
    pub node: String,
    pub module: String,
    pub context: String,
    pub callbacks: bool,
    pub accessibles: HashMap<String, Accessible>,
}

/// The application's answer to a request, from a callback or `SECoP_S_putCommandAnswer`.
#[derive(Debug)]
pub(crate) struct Answer {
    pub code: c_int,
    pub value: Option<Json>,
    pub sigma: Option<Json>,
    pub t: f64,
}

impl Answer {
    fn check(&self, accessible: &str) -> Result<(), SecopError> {
        if self.code < 0 {
            Err(to_secop(self.code, accessible))
        } else {
            Ok(())
        }
    }

    fn timestamp(&self) -> Option<f64> {
        (self.t.is_finite() && self.t > 0.0).then_some(self.t)
    }

    fn reading(self, info: &DataInfo, accessible: &str) -> HandlerResult {
        self.check(accessible)?;
        let json = self
            .value
            .as_ref()
            .ok_or_else(|| internal(format!("{accessible}: the application returned no value")))?;
        let value = info.decode(json, Mode::Report).map_err(|e| {
            internal(format!(
                "{accessible}: the application returned {json}, which does not fit: {e}"
            ))
        })?;
        Ok(self.into_reading(value))
    }

    fn into_reading(self, value: Value) -> Reading {
        let mut reading = Reading::new(value);
        if let Some(t) = self.timestamp() {
            reading = reading.at(t);
        }
        if let Some(e) = self.sigma {
            reading = reading.with_uncertainty(e);
        }
        reading
    }
}

fn internal(message: impl Into<String>) -> SecopError {
    SecopError::new(ErrorClass::InternalError, message)
}

fn missing(what: &str, accessible: &str) -> SecopError {
    SecopError::new(
        ErrorClass::NotImplemented,
        format!("{accessible}: no {what} function was registered"),
    )
}

/// A `null` value, which SHALL passes for absent command arguments and results.
fn null() -> variant::Node {
    variant::Node::from_json(&Json::Null).expect("null is a value")
}

/// Calls a getter (`with_sigma`) or setter with a handle holding `value`.
fn call_getset(function: GetSet, name: &CString, value: variant::Node, with_sigma: bool) -> Answer {
    let handle = new_handle(value);
    let mut data = handle;
    let mut sigma = ptr::null_mut();
    let mut code: c_int = 0;
    let mut t = f64::NAN;
    let sigma_ptr = if with_sigma {
        &raw mut sigma
    } else {
        ptr::null_mut()
    };
    // SAFETY: an application function with SHALL's getter/setter signature; the name and
    // all out pointers are valid for the duration of the call
    unsafe {
        function(
            name.as_ptr(),
            &raw mut code,
            &raw mut data,
            sigma_ptr,
            &raw mut t,
        );
    };
    collect(code, handle, data, sigma, t)
}

/// Calls a command function.
fn call_function(
    function: CallFunction,
    name: &CString,
    argument: variant::Node,
    result: variant::Node,
) -> Answer {
    let argument = new_handle(argument);
    let handle = new_handle(result);
    let mut data = handle;
    let mut code: c_int = 0;
    let mut t = f64::NAN;
    // SAFETY: an application function with SHALL's command signature; the name, the
    // argument and all out pointers are valid for the duration of the call
    unsafe {
        function(
            name.as_ptr(),
            argument,
            &raw mut code,
            &raw mut data,
            &raw mut t,
        );
    };
    destroy_handle(argument);
    collect(code, handle, data, ptr::null_mut(), t)
}

/// Reads the results of a call and frees the handles: the one passed in, and the ones the
/// application may have put in its place (the library owns them, as in SHALL).
fn collect(
    code: c_int,
    passed: *mut variant::CSECoPbaseType,
    data: *mut variant::CSECoPbaseType,
    sigma: *mut variant::CSECoPbaseType,
    t: f64,
) -> Answer {
    let answer = Answer {
        code,
        value: clone_handle(data).map(|n| n.export_value()),
        sigma: clone_handle(sigma).map(|n| n.export_value()),
        t,
    };
    destroy_handle(passed);
    destroy_handle(data);
    destroy_handle(sigma);
    answer
}

/// Runs a call of the application on tokio's blocking pool, so a slow function does not
/// stall the node.
async fn blocking(call: impl FnOnce() -> Answer + Send + 'static) -> Result<Answer, SecopError> {
    tokio::task::spawn_blocking(call)
        .await
        .map_err(|e| internal(format!("the application's function failed: {e}")))
}

impl ShallModule {
    /// Queues a request for `SECoP_S_getStoredCommand` and waits for its answer.
    async fn request(
        &self,
        action: c_int,
        accessible: &str,
        value: Option<variant::Node>,
    ) -> Result<Answer, SecopError> {
        let (reply, answer) = oneshot::channel();
        let id = state::enqueue(
            &self.context,
            state::Pending {
                node: self.node.clone(),
                module: self.module.clone(),
                accessible: accessible.to_owned(),
                action,
                value,
                reply,
            },
        )
        .ok_or_else(|| internal("the application has finished using the library"))?;
        // also when the node's timeout drops this future first
        let _queued = state::Queued(id);
        let timeout = state::poll_timeout();
        match tokio::time::timeout(timeout, answer).await {
            Ok(Ok(answer)) => Ok(answer),
            Ok(Err(_)) => Err(internal(format!("{accessible}: the request was dropped"))),
            Err(_) => Err(SecopError::new(
                ErrorClass::TimeoutError,
                format!(
                    "{accessible}: no answer from the application within {} s",
                    timeout.as_secs_f64()
                ),
            )),
        }
    }
}

#[async_trait]
impl ModuleHandler for ShallModule {
    fn implementation(&self) -> String {
        "must SHALL-compatible C API".to_owned()
    }

    async fn read(&self, _ctx: &Ctx, parameter: &str) -> HandlerResult {
        let Some(Accessible::Parameter {
            full_name,
            info,
            proto,
            getter,
            ..
        }) = self.accessibles.get(parameter)
        else {
            return Err(SecopError::new(ErrorClass::NoSuchParameter, parameter));
        };
        let answer = if self.callbacks {
            let getter = getter.ok_or_else(|| missing("getter", parameter))?;
            let (name, value) = (full_name.clone(), proto.clone());
            blocking(move || call_getset(getter, &name, value, true)).await?
        } else {
            self.request(action::READ, parameter, None).await?
        };
        answer.reading(info, parameter)
    }

    async fn change(&self, _ctx: &Ctx, parameter: &str, value: Value) -> HandlerResult {
        let Some(Accessible::Parameter {
            full_name,
            info,
            proto,
            setter,
            ..
        }) = self.accessibles.get(parameter)
        else {
            return Err(SecopError::new(ErrorClass::NoSuchParameter, parameter));
        };
        let json = info.encode(&value).map_err(|e| internal(e.to_string()))?;
        let mut node = proto.clone();
        if !node.import(&json, false) {
            return Err(internal(format!("{parameter}: cannot pass {json} on")));
        }
        let answer = if self.callbacks {
            let setter = setter.ok_or_else(|| missing("setter", parameter))?;
            let name = full_name.clone();
            blocking(move || call_getset(setter, &name, node, false)).await?
        } else {
            let mut answer = self.request(action::CHANGE, parameter, Some(node)).await?;
            // an answer without value confirms the requested one
            answer.value.get_or_insert(json);
            answer
        };
        answer.reading(info, parameter)
    }

    async fn call(&self, _ctx: &Ctx, command: &str, argument: Option<Value>) -> CallResult {
        let Some(Accessible::Command {
            full_name,
            info,
            argument: argument_type,
            result: result_type,
            function,
        }) = self.accessibles.get(command)
        else {
            return Err(SecopError::new(ErrorClass::NoSuchCommand, command));
        };
        let argument = match (argument_type, &argument) {
            (Some(proto), Some(value)) => {
                let json = info
                    .encode_argument(Some(value))
                    .map_err(|e| internal(e.to_string()))?;
                let mut node = proto.clone();
                if !node.import(&json, false) {
                    return Err(internal(format!("{command}: cannot pass {json} on")));
                }
                Some(node)
            }
            _ => None,
        };
        let answer = if self.callbacks {
            let function = function.ok_or_else(|| missing("command", command))?;
            let name = full_name.clone();
            let result = result_type.clone().unwrap_or_else(null);
            let argument = argument.unwrap_or_else(null);
            blocking(move || call_function(function, &name, argument, result)).await?
        } else {
            self.request(action::DO, command, argument).await?
        };
        answer.check(command)?;
        let json = answer.value.clone().unwrap_or(Json::Null);
        let result = info.decode_result(&json).map_err(|e| {
            internal(format!(
                "{command}: the application returned {json}, which does not fit: {e}"
            ))
        })?;
        Ok(result.map(|value| answer.into_reading(value)))
    }
}
