// SPDX-License-Identifier: LGPL-3.0-or-later

//! Locations inside nested datainfos and values, used in error messages.

use std::fmt;

/// One step into a nested datainfo or value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathSegment {
    /// An element of an array or tuple, by index.
    Index(usize),
    /// The element type of an array datainfo.
    Element,
    /// A struct member, by name.
    Member(String),
    /// The argument of a command.
    Argument,
    /// The result of a command.
    Result,
}

/// A location inside a nested datainfo or value, rendered like `[2].x` or `argument.p`.
///
/// The empty path denotes the top level.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Path(Vec<PathSegment>);

impl Path {
    /// The top level.
    pub fn root() -> Self {
        Self::default()
    }

    /// Returns `true` for the top level.
    pub fn is_root(&self) -> bool {
        self.0.is_empty()
    }

    /// The segments from the outside in.
    pub fn segments(&self) -> &[PathSegment] {
        &self.0
    }

    /// Prepends `segment`, used while an error travels outwards.
    #[must_use]
    pub(crate) fn within(mut self, segment: PathSegment) -> Self {
        self.0.insert(0, segment);
        self
    }
}

impl fmt::Display for Path {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, segment) in self.0.iter().enumerate() {
            match segment {
                PathSegment::Index(index) => write!(f, "[{index}]")?,
                PathSegment::Element => f.write_str("[]")?,
                PathSegment::Member(name) if i == 0 => f.write_str(name)?,
                PathSegment::Member(name) => write!(f, ".{name}")?,
                PathSegment::Argument if i == 0 => f.write_str("argument")?,
                PathSegment::Argument => f.write_str(".argument")?,
                PathSegment::Result if i == 0 => f.write_str("result")?,
                PathSegment::Result => f.write_str(".result")?,
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_rendering() {
        let path = Path::root()
            .within(PathSegment::Member("x".into()))
            .within(PathSegment::Index(2));
        assert_eq!(path.to_string(), "[2].x");

        let path = Path::root()
            .within(PathSegment::Member("p".into()))
            .within(PathSegment::Argument);
        assert_eq!(path.to_string(), "argument.p");

        let path = Path::root()
            .within(PathSegment::Index(0))
            .within(PathSegment::Element);
        assert_eq!(path.to_string(), "[][0]");

        assert_eq!(Path::root().to_string(), "");
        assert!(Path::root().is_root());
    }
}
