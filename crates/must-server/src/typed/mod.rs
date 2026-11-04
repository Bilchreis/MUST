// SPDX-License-Identifier: LGPL-3.0-or-later

//! Typed modules: Rust values instead of SECoP [`Value`]s, parameters
//! as typed fields that publish when set, and construction from settings.
//!
//! This is the layer under the module declaration macros
//! [`#[must_server::module]`](crate::module) and [`#[must_server::methods]`](crate::methods),
//! which generate the [`TypedModule`] and [`Methods`] implementations. See
//! `docs/writing-modules.md` for writing modules with them.

mod di;
mod module;
mod param;
mod settings;
#[doc(hidden)]
pub mod support;
pub(crate) mod value;

pub use di::Di;
pub use module::{
    BoxFuture, CommandSpec, Declared, Declaring, Methods, ParamSpec, Result, TypedHandler,
    TypedModule, has, module,
};
pub use param::{AnyParam, Command, Param, ParamHandle};
pub use settings::{Settings, SettingsError, SettingsErrors};
pub use value::{Kind, STATUS_CODES, SecopType, Status, accepts, check_shape, mismatch};

#[doc(hidden)]
pub use must_types::{DataInfo, SecopError, Value};

/// What module code usually needs: `use must_server::typed::prelude::*;`.
pub mod prelude {
    pub use super::{Param, ParamHandle, Result, SecopType, Status};
    pub use crate::{SecopEnum, SecopStruct, command, methods, module, param, property};
    pub use must_types::{ErrorClass, SecopError};
}
