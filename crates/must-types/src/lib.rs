// SPDX-License-Identifier: LGPL-3.0-or-later

//! SECoP 2.0 data types for must.
//!
//! This crate models the data part of [SECoP](https://github.com/SampleEnvironment/SECoP):
//!
//! - [`DataInfo`]: the `datainfo` description of a parameter or command, parsed from JSON
//!   or built in code, always validated.
//! - [`Value`]: a value of a datainfo, decoded from and encoded to JSON. Values from
//!   `change`/`do` are decoded in [`Mode::Request`] (all limits enforced, optionally
//!   also [`DynLimits`]), values in replies and updates in [`Mode::Report`] (shape only).
//!   Failures carry the SECoP [`ErrorClass`] to report.
//! - [`identifier`]: rules for module, accessible and property names, and
//!   case-insensitive [`NameScope`]s.
//! - [`postfix`]: the predefined parameter postfixes `_min`, `_max`, `_limits` and
//!   `_enable`.
//!
//! # Example
//!
//! A `change t1:target 250` arrives for a target with static limits 0..300 K, while the
//! parameter `target_max` currently allows at most 200 K:
//!
//! ```
//! use must_types::{DataInfo, DynLimits, ErrorClass};
//! use serde_json::json;
//!
//! let target: DataInfo = r#"{"type": "double", "min": 0, "max": 300, "unit": "K"}"#.parse()?;
//! let limits = DynLimits { min: None, max: Some(200.0) };
//!
//! let error = target.decode_request(&json!(250), Some(&limits)).unwrap_err();
//! assert_eq!(error.class, ErrorClass::RangeError);
//! assert_eq!(error.to_string(), "RangeError: 250 is above the dynamic upper limit 200");
//!
//! let value = target.decode_request(&json!(150), Some(&limits)).unwrap();
//! assert_eq!(target.encode(&value).unwrap(), json!(150.0));
//! # Ok::<(), must_types::DataInfoError>(())
//! ```
//!
//! # Specification version
//!
//! must implements the SECoP 2.0 draft as of commit `a6e90eb` (2026-07-07) of
//! <https://github.com/SampleEnvironment/SECoP>. The `matrix` data type is not supported
//! yet and is rejected with [`DataInfoErrorKind::UnsupportedType`].
//!
//! Where the specification contradicts itself, must follows the prose:
//!
//! - The default `fmtstr` of `scaled` is `%.<n>f` with `n = max(0, -floor(log10(scale)))`
//!   as the `datainfo` chapter says, not `%.6g` as in `schema/datatypes.yaml`.
//! - `int` requires `min` and `max`, although two `command` examples in the `datainfo`
//!   chapter omit them.

pub mod datainfo;
pub mod error;
pub mod identifier;
pub mod path;
pub mod postfix;
pub mod report;
pub mod value;

pub use datainfo::{DataInfo, DataInfoBuilder, DataInfoError, DataInfoErrorKind};
pub use error::{ErrorClass, SecopError, UnknownErrorClass};
pub use identifier::{IdentifierError, NameClash, NameScope, is_custom, validate_identifier};
pub use path::{Path, PathSegment};
pub use postfix::{Postfix, PostfixError, split_postfix};
pub use report::{Qualifiers, data_report, error_report};
pub use value::{DynLimits, Mode, Value, ValueError};

/// Date of the SECoP 2.0 draft must implements (commit `a6e90eb` of the specification),
/// sent as third field of the `*IDN?` reply.
pub const SECOP_DRAFT_DATE: &str = "2026-07-07";
/// The SECoP version must implements, sent as fourth field of the `*IDN?` reply.
pub const SECOP_VERSION: &str = "v2.0";
