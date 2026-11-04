// SPDX-License-Identifier: LGPL-3.0-or-later

//! Errors located in a config file, and one module's settings as module types see them.

use std::fmt;

use must_server::Module;
use must_server::typed::{Settings, SettingsError};
use serde::de::DeserializeOwned;
use serde_json::Value as Json;

use crate::source::{Lines, Node, Span};

/// A problem in a config file, located at a line and column.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigError {
    /// The config file, as given.
    pub file: String,
    /// 1-based line.
    pub line: usize,
    /// 1-based column.
    pub column: usize,
    /// The module concerned, if any.
    pub module: Option<String>,
    /// The template key and the 1-based instance number of an expanded module.
    pub origin: Option<(String, usize)>,
    /// What is wrong.
    pub message: String,
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}:{}: ", self.file, self.line, self.column)?;
        if let Some(module) = &self.module {
            write!(f, "module `{module}`")?;
            if let Some((template, instance)) = &self.origin {
                write!(f, " (template `{template}`, instance {instance})")?;
            }
            f.write_str(": ")?;
        }
        f.write_str(&self.message)
    }
}

impl std::error::Error for ConfigError {}

/// All problems found in a config file, in file order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigErrors(pub Vec<ConfigError>);

impl fmt::Display for ConfigErrors {
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

impl std::error::Error for ConfigErrors {}

/// Locates spans of one config file.
#[derive(Debug, Clone)]
pub struct Locator {
    pub(crate) file: String,
    pub(crate) lines: Lines,
}

impl Locator {
    pub(crate) fn new(file: &str, text: &str) -> Self {
        Self {
            file: file.to_owned(),
            lines: Lines::new(text),
        }
    }

    /// An error at `span`.
    pub(crate) fn error(
        &self,
        span: &Span,
        module: Option<&str>,
        origin: Option<&(String, usize)>,
        message: impl Into<String>,
    ) -> ConfigError {
        let (line, column) = self.lines.locate(span.start);
        ConfigError {
            file: self.file.clone(),
            line,
            column,
            module: module.map(str::to_owned),
            origin: origin.cloned(),
            message: message.into(),
        }
    }
}

/// One setting of a module.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Setting {
    pub key: String,
    pub key_span: Span,
    pub node: Node,
}

/// One module's settings, as its type sees them: the keys of its module table (or of
/// its template and instance) without the ones the loader handles (`type`, `description`,
/// `group`, `visibility`, custom `_` properties and the template keys).
pub struct ModuleConfig<'a> {
    pub(crate) name: String,
    pub(crate) description: Option<String>,
    pub(crate) settings: Vec<(Setting, bool)>,
    /// Instance keys used as template placeholders: not unknown if no one reads them.
    pub(crate) vars: Vec<String>,
    pub(crate) table_span: Span,
    pub(crate) origin: Option<(String, usize)>,
    pub(crate) locator: &'a Locator,
}

/// The kind of value a Rust type expects, for error messages.
fn expected<T>() -> String {
    let name = std::any::type_name::<T>();
    match name {
        "f64" | "f32" => "a number".to_owned(),
        "i8" | "i16" | "i32" | "i64" | "u8" | "u16" | "u32" | "u64" | "usize" | "isize" => {
            "an integer".to_owned()
        }
        "bool" => "a boolean".to_owned(),
        "alloc::string::String" => "a string".to_owned(),
        _ => format!("a `{name}`"),
    }
}

impl ModuleConfig<'_> {
    /// The module's name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The description from the config, if it gives one.
    pub fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }

    /// A module with this name and the config's description, or `default` if the config
    /// gives none.
    pub fn new_module(&self, default: &str) -> Module {
        Module::new(&self.name, self.description().unwrap_or(default))
    }

    fn find(&mut self, key: &str) -> Option<&mut (Setting, bool)> {
        self.settings.iter_mut().find(|(s, _)| s.key == key)
    }

    fn decode<T: DeserializeOwned>(&self, setting: &Setting) -> Result<T, ConfigError> {
        serde_json::from_value(setting.node.value.to_json()).map_err(|e| {
            self.error_at(
                &setting.node.span,
                format!(
                    "`{}` must be {}, not {} ({e})",
                    setting.key,
                    expected::<T>(),
                    setting.node.value.type_name()
                ),
            )
        })
    }

    /// The required setting `key` as a `T`.
    pub fn get<T: DeserializeOwned>(&mut self, key: &str) -> Result<T, ConfigError> {
        match self.get_opt(key)? {
            Some(v) => Ok(v),
            None => Err(self.error_at(
                &self.table_span.clone(),
                format!("the setting `{key}` is missing"),
            )),
        }
    }

    /// The setting `key` as a `T`, or `default` if it is not given.
    pub fn get_or<T: DeserializeOwned>(&mut self, key: &str, default: T) -> Result<T, ConfigError> {
        Ok(self.get_opt(key)?.unwrap_or(default))
    }

    /// The setting `key` as a `T`, if it is given.
    pub fn get_opt<T: DeserializeOwned>(&mut self, key: &str) -> Result<Option<T>, ConfigError> {
        let Some(entry) = self.find(key) else {
            return Ok(None);
        };
        entry.1 = true;
        let setting = entry.0.clone();
        self.decode(&setting).map(Some)
    }

    /// An error about the setting `key`, located at its value (or at the module if it is
    /// not given).
    pub fn error(&self, key: &str, message: impl fmt::Display) -> ConfigError {
        let span = self
            .settings
            .iter()
            .find(|(s, _)| s.key == key)
            .map_or_else(|| self.table_span.clone(), |(s, _)| s.node.span.clone());
        self.error_at(&span, format!("`{key}`: {message}"))
    }

    fn error_at(&self, span: &Span, message: String) -> ConfigError {
        self.locator
            .error(span, Some(&self.name), self.origin.as_ref(), message)
    }

    /// The settings no one read, apart from template variables.
    pub(crate) fn unread(&self) -> Vec<&Setting> {
        self.settings
            .iter()
            .filter(|(s, taken)| !taken && !self.vars.contains(&s.key))
            .map(|(s, _)| s)
            .collect()
    }

    /// A located error for a `SettingsError` of a typed module.
    pub(crate) fn locate(&self, error: &SettingsError) -> ConfigError {
        let span = error
            .key
            .as_ref()
            .and_then(|k| self.settings.iter().find(|(s, _)| &s.key == k))
            .map_or_else(|| self.table_span.clone(), |(s, _)| s.node.span.clone());
        let message = match &error.key {
            Some(key) if !error.message.contains(&format!("`{key}`")) => {
                format!("`{key}`: {}", error.message)
            }
            _ => error.message.clone(),
        };
        let mut located = self.error_at(&span, message);
        if let Some((line, column)) = error.location {
            located.line = line;
            located.column = column;
        }
        located
    }
}

impl Settings for ModuleConfig<'_> {
    fn take(&mut self, key: &str) -> Option<Json> {
        let entry = self.find(key)?;
        entry.1 = true;
        Some(entry.0.node.value.to_json())
    }

    fn remaining(&self) -> Vec<String> {
        self.unread().into_iter().map(|s| s.key.clone()).collect()
    }

    fn error(&self, key: Option<&str>, message: String) -> SettingsError {
        let located = key.map_or_else(
            || self.error_at(&self.table_span.clone(), message.clone()),
            |k| ModuleConfig::error(self, k, &message),
        );
        SettingsError {
            module: self.name.clone(),
            key: key.map(str::to_owned),
            message,
            location: Some((located.line, located.column)),
        }
    }
}
