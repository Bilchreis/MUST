// SPDX-License-Identifier: LGPL-3.0-or-later

//! Node config files for MUST: declare a SEC node in a TOML file, with module instances
//! of registered module types, many modules of one type through templates, and serve it
//! with `must serve`.
//!
//! ```toml
//! [node]
//! equipment_id = "gasrack-1"
//! description = "gas mixing rack"
//! interface = "tcp://10767"
//!
//! [modules."mfc{i}"]
//! type = "must::sim::FlowController"
//! count = 5
//! range = 100.0
//! description = "mass flow controller {i}"
//! ```
//!
//! Module types are structs declared with `#[must_server::module]`, registered in a
//! [`Registry`] under a name such as `acme::Valve`, or factories for modules whose shape
//! depends on the config ([`Registry::register_fn`]). [`load_file`] checks a config
//! completely and reports every problem with its line; [`cli::main`] is the `serve`,
//! `check` and `types` runner for a driver collection's own binary. See
//! `docs/node-config.md`.

pub mod cli;
mod config;
mod load;
mod registry;
pub mod sim;
mod source;
mod template;

pub use config::{ConfigError, ConfigErrors, ModuleConfig};
pub use load::{Loaded, load_file, load_str};
pub use registry::{Factory, Registry, RegistryError};
