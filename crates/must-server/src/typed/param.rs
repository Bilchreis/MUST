// SPDX-License-Identifier: LGPL-3.0-or-later

//! Typed parameter fields and the request context they publish through.

use std::fmt;
use std::future::Future;
use std::marker::PhantomData;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use must_types::{DataInfo, ErrorClass, SecopError, Value};

use super::SecopType;
use crate::handler::{Ctx, Publisher};

tokio::task_local! {
    /// The context of the request a typed module's method is handling.
    static CURRENT: Ctx;
}

/// Runs `call` as part of the request `ctx`: parameters set in it publish through `ctx`,
/// so their updates reach clients before the request's reply.
pub(crate) async fn in_request<F: Future>(ctx: Ctx, call: F) -> F::Output {
    CURRENT.scope(ctx, call).await
}

struct Cell<T> {
    name: String,
    datainfo: DataInfo,
    value: Mutex<T>,
    publisher: OnceLock<Publisher>,
}

impl<T: SecopType> Cell<T> {
    fn get(&self) -> T {
        self.value
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn store(&self, value: T) {
        *self.value.lock().unwrap_or_else(PoisonError::into_inner) = value;
    }

    /// Stores and publishes `value`: through the current request if it belongs to this
    /// module (or the module is not bound yet), else through the publisher; before the
    /// node starts it is only stored.
    fn set(&self, value: T, via_request: bool) -> Result<(), SecopError> {
        let encoded = super::value::produce(&self.name, &value, &self.datainfo)?;
        self.store(value);
        if via_request {
            let bound = self.publisher.get().map(Publisher::module);
            let sent = CURRENT.try_with(|ctx| {
                (bound.is_none_or(|m| m == ctx.module()))
                    .then(|| ctx.update(&self.name, encoded.clone()))
            });
            if let Ok(Some(result)) = sent {
                return result;
            }
        }
        match self.publisher.get() {
            Some(publisher) => publisher.update(&self.name, encoded),
            None => Ok(()),
        }
    }

    fn set_error(&self, error: SecopError, via_request: bool) -> Result<(), SecopError> {
        if via_request
            && let Ok(()) = CURRENT.try_with(|ctx| ctx.update_error(&self.name, error.clone()))
        {
            return Ok(());
        }
        match self.publisher.get() {
            Some(publisher) => publisher.update_error(&self.name, error),
            None => Err(SecopError::new(
                ErrorClass::InternalError,
                "the node has not started yet",
            )),
        }
    }
}

/// A parameter field of a typed module: holds the latest value and publishes new ones.
///
/// The value is the last one read, written, set, pushed or configured; before any, the
/// parameter's `default` or [`SecopType::fallback`].
pub struct Param<T> {
    cell: Arc<Cell<T>>,
}

impl<T: SecopType> Param<T> {
    /// A parameter named `name` with `datainfo` and a first value.
    pub fn new(name: &str, datainfo: DataInfo, value: T) -> Self {
        Self {
            cell: Arc::new(Cell {
                name: name.to_owned(),
                datainfo,
                value: Mutex::new(value),
                publisher: OnceLock::new(),
            }),
        }
    }

    /// The parameter's name.
    pub fn name(&self) -> &str {
        &self.cell.name
    }

    /// The parameter's datainfo.
    pub fn datainfo(&self) -> &DataInfo {
        &self.cell.datainfo
    }

    /// The latest value.
    pub fn get(&self) -> T {
        self.cell.get()
    }

    /// Stores `value` and publishes it to activated clients. Inside a method handling a
    /// request, the update reaches clients before that request's reply; before the node
    /// has started, the value is only stored and becomes the initial value.
    pub fn set(&self, value: T) -> Result<(), SecopError> {
        self.cell.set(value, true)
    }

    /// Publishes that the parameter could not be obtained, e.g. a failed hardware read in
    /// a poll hook.
    pub fn set_error(&self, error: SecopError) -> Result<(), SecopError> {
        self.cell.set_error(error, true)
    }

    /// A handle for background tasks, sharing this parameter's value.
    pub fn handle(&self) -> ParamHandle<T> {
        ParamHandle {
            cell: Arc::clone(&self.cell),
        }
    }
}

impl<T: SecopType + fmt::Debug> fmt::Debug for Param<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Param")
            .field("name", &self.cell.name)
            .field("value", &self.get())
            .finish()
    }
}

/// A cloneable handle to a parameter for background tasks: values pushed through it are
/// published at once and returned by the parameter's [`Param::get`] afterwards.
pub struct ParamHandle<T> {
    cell: Arc<Cell<T>>,
}

impl<T> Clone for ParamHandle<T> {
    fn clone(&self) -> Self {
        Self {
            cell: Arc::clone(&self.cell),
        }
    }
}

impl<T: SecopType> ParamHandle<T> {
    /// The latest value.
    pub fn get(&self) -> T {
        self.cell.get()
    }

    /// Stores `value` and publishes it. Before the node has started it is only stored.
    pub fn set(&self, value: T) -> Result<(), SecopError> {
        self.cell.set(value, false)
    }

    /// Publishes that the parameter could not be obtained.
    pub fn set_error(&self, error: SecopError) -> Result<(), SecopError> {
        self.cell.set_error(error, false)
    }
}

/// The marker type of a command field of a typed module: `A` is the argument type and
/// `R` the result type, `()` for none.
pub struct Command<A = (), R = ()>(PhantomData<fn(A) -> R>);

impl<A, R> Default for Command<A, R> {
    fn default() -> Self {
        Self(PhantomData)
    }
}

impl<A, R> Clone for Command<A, R> {
    fn clone(&self) -> Self {
        Self(PhantomData)
    }
}

impl<A, R> fmt::Debug for Command<A, R> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Command")
    }
}

/// A parameter of any type, as the typed module adapter sees it.
#[doc(hidden)]
pub trait AnyParam: Send + Sync {
    /// The parameter's name.
    fn name(&self) -> &str;
    /// Stores a value that was published, e.g. a read result.
    fn store_value(&self, value: &Value) -> Result<(), SecopError>;
    /// The latest value, for a read served without a method.
    fn current_value(&self) -> Result<Value, SecopError>;
    /// Binds the parameter to its module's publisher when the node starts.
    fn bind(&self, publisher: &Publisher);
}

impl<T: SecopType> AnyParam for Param<T> {
    fn name(&self) -> &str {
        &self.cell.name
    }

    fn store_value(&self, value: &Value) -> Result<(), SecopError> {
        self.cell.store(T::from_value(value, &self.cell.datainfo)?);
        Ok(())
    }

    fn current_value(&self) -> Result<Value, SecopError> {
        self.get().to_value(&self.cell.datainfo)
    }

    fn bind(&self, publisher: &Publisher) {
        let _ = self.cell.publisher.set(publisher.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn double() -> DataInfo {
        DataInfo::from_json(&json!({"type": "double"})).unwrap()
    }

    #[test]
    fn before_start_only_stores() {
        let p = Param::new("x", double(), 1.0);
        p.set(2.0).unwrap();
        assert_eq!(p.get(), 2.0);
        // errors need a node
        assert!(
            p.set_error(SecopError::new(ErrorClass::HardwareError, "x"))
                .is_err()
        );
    }

    #[test]
    fn handle_shares_the_value() {
        let p = Param::new("x", double(), 1.0);
        let h = p.handle();
        h.set(3.5).unwrap();
        assert_eq!(p.get(), 3.5);
        p.set(4.5).unwrap();
        assert_eq!(h.clone().get(), 4.5);
    }

    #[test]
    fn stored_values_come_back() {
        let p = Param::new("x", double(), 0.0);
        AnyParam::store_value(&p, &Value::Double(7.0)).unwrap();
        assert_eq!(p.get(), 7.0);
        assert_eq!(AnyParam::current_value(&p).unwrap(), Value::Double(7.0));
    }
}
