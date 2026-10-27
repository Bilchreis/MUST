// SPDX-License-Identifier: LGPL-3.0-or-later

//! SECoP 2.0 predefined parameter postfixes: `_min`, `_max`, `_limits` and `_enable`.
//!
//! A parameter `<p>_min`, `<p>_max` or `<p>_limits` restricts the values accepted for
//! parameter `<p>` at runtime (dynamic limits); `<p>_enable` switches the effect of `<p>`
//! on and off. This module covers the type rules; whether the parent exists and which
//! postfixes may coexist is decided where the module's parameters are known.

use std::fmt;

use thiserror::Error;

use crate::datainfo::DataInfo;
use crate::identifier::validate_identifier;
use crate::path::{Path, PathSegment};

/// A predefined parameter postfix.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Postfix {
    /// `_min`: dynamic lower limit.
    Min,
    /// `_max`: dynamic upper limit.
    Max,
    /// `_limits`: dynamic lower and upper limit as a tuple.
    Limits,
    /// `_enable`: enables or disables the effect of the parent.
    Enable,
}

impl Postfix {
    /// All predefined postfixes.
    pub const ALL: [Postfix; 4] = [Self::Min, Self::Max, Self::Limits, Self::Enable];

    /// The postfix as it appears in parameter names, e.g. `"_limits"`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Min => "_min",
            Self::Max => "_max",
            Self::Limits => "_limits",
            Self::Enable => "_enable",
        }
    }

    /// Whether the postfix defines dynamic limits, which need a numeric parent.
    pub fn is_limit(self) -> bool {
        !matches!(self, Self::Enable)
    }

    /// Checks that `parent` may have this postfix.
    pub fn check_parent(self, parent: &DataInfo) -> Result<(), PostfixError> {
        if self.is_limit()
            && !matches!(
                parent,
                DataInfo::Double(_) | DataInfo::Scaled(_) | DataInfo::Int(_)
            )
        {
            return Err(PostfixError::NonNumericParent {
                postfix: self,
                parent_type: parent.type_name(),
            });
        }
        Ok(())
    }

    /// The datainfo a parameter with this postfix must have, given its parent's.
    // cannot panic: a tuple of two valid datainfos and a plain bool are always valid
    #[allow(clippy::missing_panics_doc)]
    pub fn expected_datainfo(self, parent: &DataInfo) -> Result<DataInfo, PostfixError> {
        self.check_parent(parent)?;
        let info = match self {
            Self::Min | Self::Max => Ok(parent.clone()),
            Self::Limits => DataInfo::tuple([parent, parent]).build(),
            Self::Enable => DataInfo::bool().build(),
        };
        Ok(info.expect("derived from a valid datainfo"))
    }

    /// Checks the declared datainfo of a parameter with this postfix against its parent.
    pub fn check_declared(
        self,
        parent: &DataInfo,
        declared: &DataInfo,
    ) -> Result<(), PostfixError> {
        let expected = self.expected_datainfo(parent)?;
        match mismatch(&expected, declared) {
            None => Ok(()),
            Some(path) => Err(PostfixError::Mismatch {
                postfix: self,
                path,
                expected: expected.to_string(),
                declared: declared.to_string(),
            }),
        }
    }
}

impl fmt::Display for Postfix {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Where `declared` first differs from `expected`, or `None` if they match.
fn mismatch(expected: &DataInfo, declared: &DataInfo) -> Option<Path> {
    if expected.matches(declared) {
        return None;
    }
    if let (DataInfo::Tuple(e), DataInfo::Tuple(d)) = (expected, declared)
        && e.members().len() == d.members().len()
    {
        for (i, (e, d)) in e.members().iter().zip(d.members()).enumerate() {
            if let Some(path) = mismatch(e, d) {
                return Some(path.within(PathSegment::Index(i)));
            }
        }
    }
    Some(Path::root())
}

/// Splits a parameter name into parent name and predefined postfix.
///
/// Returns `None` if the name has no predefined postfix or the remaining parent name is
/// not an identifier (e.g. `_max` alone is an ordinary custom name). Custom parents are
/// allowed: `_ramp_max` is the `_max` of `_ramp`.
pub fn split_postfix(name: &str) -> Option<(&str, Postfix)> {
    Postfix::ALL.into_iter().find_map(|postfix| {
        let parent = name.strip_suffix(postfix.as_str())?;
        validate_identifier(parent).ok().map(|()| (parent, postfix))
    })
}

/// A postfixed parameter does not fit its parent.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum PostfixError {
    /// Dynamic limits were declared for a parameter that is not `double`, `scaled` or `int`.
    #[error(
        "`{postfix}` defines dynamic limits, which need a scalar numeric parameter, not `{parent_type}`"
    )]
    NonNumericParent {
        /// The postfix.
        postfix: Postfix,
        /// The parent's type name.
        parent_type: &'static str,
    },
    /// The declared datainfo differs from the one derived from the parent.
    #[error("datainfo of the `{postfix}` parameter does not match its parent{}: expected {expected}, declared {declared}",
        if path.is_root() { String::new() } else { format!(" at member {path}") })]
    Mismatch {
        /// The postfix.
        postfix: Postfix,
        /// Where the datainfos first differ (e.g. `[1]` for the upper limit of `_limits`).
        path: Path,
        /// The expected datainfo as JSON.
        expected: String,
        /// The declared datainfo as JSON.
        declared: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value as Json, json};

    #[allow(clippy::needless_pass_by_value)]
    fn info(json: Json) -> DataInfo {
        DataInfo::from_json(&json).unwrap()
    }

    // --- 5.1 names

    #[test]
    fn split_names() {
        assert_eq!(
            split_postfix("target_limits"),
            Some(("target", Postfix::Limits))
        );
        assert_eq!(split_postfix("target_min"), Some(("target", Postfix::Min)));
        assert_eq!(
            split_postfix("heater_enable"),
            Some(("heater", Postfix::Enable))
        );
        assert_eq!(split_postfix("_ramp_max"), Some(("_ramp", Postfix::Max)));
        assert_eq!(split_postfix("_max"), None);
        assert_eq!(split_postfix("target_maximum"), None);
        assert_eq!(split_postfix("target"), None);
        assert_eq!(split_postfix("1x_max"), None);
    }

    // --- 5.2 datainfo rules

    #[test]
    fn numeric_parent_required() {
        let double = info(json!({"type": "double", "min": 0, "max": 300}));
        assert_eq!(Postfix::Limits.check_parent(&double), Ok(()));

        let string = info(json!({"type": "string"}));
        let e = Postfix::Max.check_parent(&string).unwrap_err();
        assert_eq!(
            e,
            PostfixError::NonNumericParent {
                postfix: Postfix::Max,
                parent_type: "string"
            }
        );
        assert!(e.to_string().contains("scalar numeric"), "{e}");
        assert_eq!(Postfix::Enable.check_parent(&string), Ok(()));
    }

    #[test]
    fn derived_datainfos() {
        let int = info(json!({"type": "int", "min": 0, "max": 10}));
        assert_eq!(
            Postfix::Limits.expected_datainfo(&int).unwrap().to_json(),
            json!({"type": "tuple", "members": [
                {"type": "int", "min": 0, "max": 10}, {"type": "int", "min": 0, "max": 10}]})
        );
        assert_eq!(Postfix::Max.expected_datainfo(&int).unwrap(), int);

        let string = info(json!({"type": "string"}));
        assert_eq!(
            Postfix::Enable
                .expected_datainfo(&string)
                .unwrap()
                .to_json(),
            json!({"type": "bool"})
        );
    }

    #[test]
    fn matching_limits() {
        let parent = info(json!({"type": "double", "min": 0, "max": 300, "unit": "K"}));
        let limits = info(json!({"type": "tuple", "members": [
            {"type": "double", "min": 0, "max": 300, "unit": "K"},
            {"type": "double", "min": 0, "max": 300, "unit": "K", "_note": "custom is ignored"}]}));
        assert_eq!(Postfix::Limits.check_declared(&parent, &limits), Ok(()));
    }

    #[test]
    fn mismatching_member_type() {
        let parent = info(json!({"type": "double"}));
        let limits = info(json!({"type": "tuple", "members": [
            {"type": "double"}, {"type": "int", "min": 0, "max": 9}]}));
        let e = Postfix::Limits
            .check_declared(&parent, &limits)
            .unwrap_err();
        let PostfixError::Mismatch { path, .. } = &e else {
            panic!("{e}")
        };
        assert_eq!(path.to_string(), "[1]");
        assert!(e.to_string().contains("at member [1]"), "{e}");
    }

    #[test]
    fn mismatching_unit() {
        let parent = info(json!({"type": "double", "unit": "K"}));
        let max = info(json!({"type": "double", "unit": "mK"}));
        assert!(matches!(
            Postfix::Max.check_declared(&parent, &max),
            Err(PostfixError::Mismatch { .. })
        ));
    }

    #[test]
    fn matches_ignores_custom_properties_only() {
        let a = info(json!({"type": "int", "min": 0, "max": 5, "_x": 1}));
        let b = info(json!({"type": "int", "min": 0, "max": 5}));
        let c = info(json!({"type": "int", "min": 0, "max": 6}));
        assert!(a.matches(&b));
        assert!(!a.matches(&c));
        assert_ne!(a, b, "PartialEq stays strict");
    }
}
