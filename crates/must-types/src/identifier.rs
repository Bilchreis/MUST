// SPDX-License-Identifier: LGPL-3.0-or-later

//! SECoP identifiers and name scopes.
//!
//! Module, accessible and property names must be identifiers: 1 to 63 ASCII letters,
//! digits and underscores, not starting with a digit. Names starting with an underscore
//! are custom names. Within a scope (modules of a node, accessibles of a module, members
//! of a struct or enum, ...) names are stored case sensitive but must be unique when
//! lowercased.

use indexmap::IndexMap;
use thiserror::Error;

/// Maximum length of a SECoP identifier in characters.
pub const MAX_IDENTIFIER_LEN: usize = 63;

/// Reason why a name is not a valid SECoP identifier.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum IdentifierError {
    /// The name is empty.
    #[error("identifier is empty")]
    Empty,
    /// The name is longer than [`MAX_IDENTIFIER_LEN`] characters.
    #[error("identifier `{name}` has {len} characters, at most {MAX_IDENTIFIER_LEN} are allowed")]
    TooLong {
        /// The rejected name.
        name: String,
        /// Its length in characters.
        len: usize,
    },
    /// The name starts with a character that may not start an identifier.
    #[error("identifier `{name}` must not start with `{ch}`")]
    InvalidFirstChar {
        /// The rejected name.
        name: String,
        /// The offending first character.
        ch: char,
    },
    /// The name contains a character other than ASCII letters, digits and underscores.
    #[error("identifier `{name}` contains invalid character `{ch}` at position {position}")]
    InvalidChar {
        /// The rejected name.
        name: String,
        /// The offending character.
        ch: char,
        /// Zero-based character position of `ch`.
        position: usize,
    },
}

/// Checks that `name` is a valid SECoP identifier for a module, accessible or property.
///
/// The full name counts, including any parameter postfix such as `_limits`.
pub fn validate_identifier(name: &str) -> Result<(), IdentifierError> {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return Err(IdentifierError::Empty);
    };
    if !(first.is_ascii_alphabetic() || first == '_') {
        return Err(IdentifierError::InvalidFirstChar {
            name: name.to_owned(),
            ch: first,
        });
    }
    for (position, ch) in name.chars().enumerate().skip(1) {
        if !(ch.is_ascii_alphanumeric() || ch == '_') {
            return Err(IdentifierError::InvalidChar {
                name: name.to_owned(),
                ch,
                position,
            });
        }
    }
    // all characters are ASCII here, so the byte length is the character count
    if name.len() > MAX_IDENTIFIER_LEN {
        return Err(IdentifierError::TooLong {
            name: name.to_owned(),
            len: name.len(),
        });
    }
    Ok(())
}

/// Returns `true` for a custom name, i.e. one starting with an underscore.
pub fn is_custom(name: &str) -> bool {
    name.starts_with('_')
}

/// A name was added to a [`NameScope`] that already holds a name equal to it after
/// lowercasing.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("name `{name}` clashes with existing name `{existing}`")]
pub struct NameClash {
    /// The name that could not be added.
    pub name: String,
    /// The name already in the scope.
    pub existing: String,
}

/// A set of names that are unique when lowercased, keeping their original spelling and
/// insertion order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NameScope {
    // lowercased name -> name as given
    names: IndexMap<String, String>,
}

impl NameScope {
    /// Creates an empty scope.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds `name`, failing if a name equal to it after lowercasing is already present.
    pub fn insert(&mut self, name: &str) -> Result<(), NameClash> {
        let key = name.to_lowercase();
        if let Some(existing) = self.names.get(&key) {
            return Err(NameClash {
                name: name.to_owned(),
                existing: existing.clone(),
            });
        }
        self.names.insert(key, name.to_owned());
        Ok(())
    }

    /// Returns `true` if exactly `name` (case sensitive) is in the scope.
    pub fn contains(&self, name: &str) -> bool {
        self.get(name).is_some()
    }

    /// Looks up exactly `name` (case sensitive) and returns it as stored.
    pub fn get(&self, name: &str) -> Option<&str> {
        self.names
            .get(&name.to_lowercase())
            .map(String::as_str)
            .filter(|stored| *stored == name)
    }

    /// Number of names in the scope.
    pub fn len(&self) -> usize {
        self.names.len()
    }

    /// Returns `true` if the scope holds no names.
    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }

    /// Iterates over the names in insertion order, as given.
    pub fn iter(&self) -> impl Iterator<Item = &str> {
        self.names.values().map(String::as_str)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifier_valid_names() {
        for name in ["temp", "T1", "_custom", "target_limits"] {
            assert_eq!(validate_identifier(name), Ok(()), "{name}");
        }
    }

    #[test]
    fn identifier_leading_digit() {
        assert_eq!(
            validate_identifier("1temp"),
            Err(IdentifierError::InvalidFirstChar {
                name: "1temp".into(),
                ch: '1'
            })
        );
    }

    #[test]
    fn identifier_invalid_characters() {
        for name in ["temp-1", "temp value", "tempé"] {
            assert!(
                matches!(
                    validate_identifier(name),
                    Err(IdentifierError::InvalidChar { .. })
                ),
                "{name}"
            );
        }
    }

    #[test]
    fn identifier_length_limits() {
        assert_eq!(validate_identifier(""), Err(IdentifierError::Empty));
        assert_eq!(validate_identifier(&"a".repeat(63)), Ok(()));
        assert!(matches!(
            validate_identifier(&"a".repeat(64)),
            Err(IdentifierError::TooLong { len: 64, .. })
        ));
    }

    #[test]
    fn identifier_postfix_counts_in_full() {
        let name = format!("{}_limits", "p".repeat(57));
        assert_eq!(name.len(), 64);
        assert!(matches!(
            validate_identifier(&name),
            Err(IdentifierError::TooLong { .. })
        ));
    }

    #[test]
    fn identifier_custom_names() {
        assert!(is_custom("_my_param"));
        assert!(!is_custom("my_param"));
    }

    #[test]
    fn scope_clash_differing_only_in_case() {
        let mut scope = NameScope::new();
        scope.insert("Temp").unwrap();
        assert_eq!(
            scope.insert("temp"),
            Err(NameClash {
                name: "temp".into(),
                existing: "Temp".into()
            })
        );
        assert_eq!(scope.len(), 1);
    }

    #[test]
    fn scope_preserves_case() {
        let mut scope = NameScope::new();
        scope.insert("Temp").unwrap();
        assert_eq!(scope.get("Temp"), Some("Temp"));
        assert_eq!(scope.get("temp"), None);
        assert!(!scope.contains("temp"));
    }

    #[test]
    fn scope_non_ascii_member_names() {
        let mut scope = NameScope::new();
        scope.insert("Größe").unwrap();
        assert!(
            scope.insert("GRÖSSE").is_ok(),
            "lowercasing does not fold ß and SS"
        );
        assert!(scope.insert("größe").is_err());
        assert_eq!(scope.iter().collect::<Vec<_>>(), ["Größe", "GRÖSSE"]);
    }
}
