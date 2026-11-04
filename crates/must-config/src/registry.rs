// SPDX-License-Identifier: LGPL-3.0-or-later

//! The module types a config can name.

use std::collections::BTreeMap;
use std::fmt;

use must_server::Module;
use must_server::typed::{self, SettingsErrors, TypedModule};

use crate::config::{ConfigError, ModuleConfig};

/// A module type written as a factory: it reads its settings from the [`ModuleConfig`]
/// and returns the module.
pub type Factory =
    Box<dyn for<'a> Fn(&mut ModuleConfig<'a>) -> Result<Module, ConfigError> + Send + Sync>;

type Build =
    for<'a> fn(&str, Option<&str>, &mut ModuleConfig<'a>) -> Result<Module, SettingsErrors>;

pub(crate) enum TypeEntry {
    /// A struct declared with `#[must_server::module]`.
    Typed {
        build: Build,
        default_description: Option<&'static str>,
    },
    /// A factory.
    Factory(Factory),
}

fn build_typed<T: TypedModule>(
    name: &str,
    description: Option<&str>,
    config: &mut ModuleConfig<'_>,
) -> Result<Module, SettingsErrors> {
    typed::module::<T>(name, description, config)
}

/// The module types a config can name, compiled in: e.g. `must::sim::FlowController`.
pub struct Registry {
    types: BTreeMap<String, TypeEntry>,
}

/// A rejected registration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistryError(pub String);

impl fmt::Display for RegistryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for RegistryError {}

/// Whether `name` is identifiers joined by `::`.
fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.split("::").all(|part| {
            let mut chars = part.chars();
            chars
                .next()
                .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
                && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
        })
}

impl Registry {
    /// An empty registry.
    pub fn new() -> Self {
        Self {
            types: BTreeMap::new(),
        }
    }

    fn insert(&mut self, name: &str, entry: TypeEntry) -> Result<(), RegistryError> {
        if !valid_name(name) {
            return Err(RegistryError(format!(
                "`{name}` is not a module type name: identifiers joined by `::`, e.g. `acme::Valve`"
            )));
        }
        if self.types.contains_key(name) {
            return Err(RegistryError(format!(
                "the module type `{name}` is registered twice"
            )));
        }
        self.types.insert(name.to_owned(), entry);
        Ok(())
    }

    /// Registers a module type declared with `#[must_server::module]` as `name`.
    pub fn register<T: TypedModule>(&mut self, name: &str) -> Result<(), RegistryError> {
        self.insert(
            name,
            TypeEntry::Typed {
                build: build_typed::<T>,
                default_description: T::DEFAULT_DESCRIPTION,
            },
        )
    }

    /// Registers a module type written as a factory, for modules whose shape depends on
    /// the config.
    pub fn register_fn<F>(&mut self, name: &str, factory: F) -> Result<(), RegistryError>
    where
        F: for<'a> Fn(&mut ModuleConfig<'a>) -> Result<Module, ConfigError> + Send + Sync + 'static,
    {
        self.insert(name, TypeEntry::Factory(Box::new(factory)))
    }

    /// The registered type names, sorted.
    pub fn names(&self) -> Vec<&str> {
        self.types.keys().map(String::as_str).collect()
    }

    pub(crate) fn get(&self, name: &str) -> Option<&TypeEntry> {
        self.types.get(name)
    }
}

impl Default for Registry {
    /// The built-in types: `must::sim::Temperature` and `must::sim::FlowController`.
    fn default() -> Self {
        let mut registry = Self::new();
        crate::sim::register(&mut registry).expect("the built-in types have distinct names");
        registry
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duplicate_type_name() {
        let mut r = Registry::new();
        r.register_fn("acme::Valve", |c| Ok(c.new_module("valve")))
            .unwrap();
        let err = r
            .register_fn("acme::Valve", |c| Ok(c.new_module("valve")))
            .unwrap_err();
        assert!(err.0.contains("acme::Valve"), "{err}");
    }

    #[test]
    fn invalid_type_name() {
        let mut r = Registry::new();
        assert!(
            r.register_fn("acme:Valve", |c| Ok(c.new_module("v")))
                .is_err()
        );
        assert!(r.register_fn("acme::", |c| Ok(c.new_module("v"))).is_err());
        assert!(
            r.register_fn("must::sim::Thing2", |c| Ok(c.new_module("v")))
                .is_ok()
        );
    }

    #[test]
    fn names_are_sorted() {
        let r = Registry::default();
        assert_eq!(
            r.names(),
            ["must::sim::FlowController", "must::sim::Temperature"]
        );
    }
}
