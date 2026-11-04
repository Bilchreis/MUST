// SPDX-License-Identifier: LGPL-3.0-or-later

//! Settings a typed module is constructed from: property values and configured initial
//! values of parameters.

use std::fmt;

use serde_json::{Map, Value as Json};

/// A source of a module's settings, keyed by name: a JSON object in code, or a module
/// table of a node config file.
pub trait Settings {
    /// Takes the setting `key`, marking it used.
    fn take(&mut self, key: &str) -> Option<Json>;

    /// The keys not taken yet, in their original order.
    fn remaining(&self) -> Vec<String>;

    /// An error about the setting `key` (or the module's settings as a whole for `None`),
    /// located where the implementation knows it, e.g. at a line of a config file.
    fn error(&self, key: Option<&str>, message: String) -> SettingsError {
        SettingsError {
            module: String::new(),
            key: key.map(str::to_owned),
            message,
            location: None,
        }
    }
}

impl Settings for Map<String, Json> {
    fn take(&mut self, key: &str) -> Option<Json> {
        self.remove(key)
    }

    fn remaining(&self) -> Vec<String> {
        self.keys().cloned().collect()
    }
}

/// A JSON object; any other JSON value counts as no settings.
impl Settings for Json {
    fn take(&mut self, key: &str) -> Option<Json> {
        self.as_object_mut().and_then(|m| m.remove(key))
    }

    fn remaining(&self) -> Vec<String> {
        self.as_object()
            .map(|m| m.keys().cloned().collect())
            .unwrap_or_default()
    }
}

impl<S: Settings + ?Sized> Settings for &mut S {
    fn take(&mut self, key: &str) -> Option<Json> {
        (**self).take(key)
    }

    fn remaining(&self) -> Vec<String> {
        (**self).remaining()
    }

    fn error(&self, key: Option<&str>, message: String) -> SettingsError {
        (**self).error(key, message)
    }
}

/// A problem with one setting of a module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingsError {
    /// The module being constructed.
    pub module: String,
    /// The setting concerned; `None` for the module's settings as a whole.
    pub key: Option<String>,
    /// What is wrong.
    pub message: String,
    /// Line and column (1-based) in the settings' source, if it has any.
    pub location: Option<(usize, usize)>,
}

impl fmt::Display for SettingsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "module `{}`", self.module)?;
        if let Some(key) = &self.key {
            write!(f, ", setting `{key}`")?;
        }
        write!(f, ": {}", self.message)
    }
}

impl std::error::Error for SettingsError {}

/// All problems found while constructing a module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingsErrors(pub Vec<SettingsError>);

impl fmt::Display for SettingsErrors {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, e) in self.0.iter().enumerate() {
            if i > 0 {
                writeln!(f)?;
            }
            write!(f, "{e}")?;
        }
        Ok(())
    }
}

impl std::error::Error for SettingsErrors {}
