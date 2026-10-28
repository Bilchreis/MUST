// SPDX-License-Identifier: LGPL-3.0-or-later

//! Declaring a SEC node: node, modules, parameters, commands and their properties.
//!
//! Build a node with [`NodeBuilder`], [`Module`], [`Parameter`] and [`Command`];
//! [`NodeBuilder::build`] checks the whole declaration against the SECoP 2.0 rules and
//! returns a [`Node`] whose structure can no longer change.

use std::collections::HashSet;
use std::fmt;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use indexmap::IndexMap;
use must_types::datainfo::CommandInfo;
use must_types::{
    DataInfo, DynLimits, IdentifierError, Mode, NameClash, NameScope, Postfix, PostfixError, Value,
    split_postfix, validate_identifier,
};
use serde_json::Value as Json;
use thiserror::Error;

use crate::discovery::Discovery;
use crate::handler::{ModuleHandler, NoHandler};
use crate::logfiles::LogFiles;

/// Default reply timeout of a node (the `timeout` node property), in seconds.
pub const DEFAULT_TIMEOUT: f64 = 10.0;
/// Default limit for the length of an incoming message.
pub const DEFAULT_MAX_LINE_LENGTH: usize = 1 << 20;
/// Default limit for data buffered for one connection; see [`NodeBuilder::max_outgoing_bytes`].
pub const DEFAULT_MAX_OUTGOING_BYTES: usize = 16 << 20;

/// Node properties defined by SECoP 2.0.
pub const NODE_PROPERTIES: &[&str] = &[
    "equipment_id",
    "description",
    "firmware",
    "implementor",
    "timeout",
    "systems",
    "schemata",
];
/// Module properties defined by SECoP 2.0.
pub const MODULE_PROPERTIES: &[&str] = &[
    "description",
    "interface_classes",
    "features",
    "visibility",
    "group",
    "meaning",
    "implementor",
    "implementation",
    "acquisition_channels",
];
/// Parameter properties defined by SECoP 2.0.
pub const PARAMETER_PROPERTIES: &[&str] = &[
    "description",
    "readonly",
    "group",
    "visibility",
    "meaning",
    "checkable",
    "constant",
];
/// Command properties defined by SECoP 2.0.
pub const COMMAND_PROPERTIES: &[&str] =
    &["description", "group", "visibility", "meaning", "checkable"];
/// Keys set by the structure itself, not as properties.
const STRUCTURAL_PROPERTIES: &[&str] = &["modules", "accessibles", "datainfo"];

/// The interface classes defined by SECoP 2.0.
pub const INTERFACE_CLASSES: &[&str] = &[
    "Readable",
    "Writable",
    "Drivable",
    "Communicator",
    "AcquisitionController",
    "AcquisitionChannel",
    "Acquisition",
];

const VISIBILITIES: &[&str] = &[
    "www", "wwr", "ww-", "wrr", "wr-", "w--", "rrr", "rr-", "r--", "---", "user", "advanced",
    "expert",
];

/// A problem in a node declaration.
#[derive(Debug, Clone, PartialEq, Error)]
#[error("{}{kind}", if path.is_empty() { String::new() } else { format!("{path}: ") })]
pub struct ModelError {
    /// Where the problem is: empty for the node, `module`, `module:accessible`, with
    /// `.property` appended for property problems.
    pub path: String,
    /// What is wrong.
    pub kind: ModelErrorKind,
}

/// The kinds of problems in a node declaration.
#[derive(Debug, Clone, PartialEq, Error)]
#[non_exhaustive]
pub enum ModelErrorKind {
    /// A name is not a valid SECoP identifier.
    #[error(transparent)]
    InvalidName(IdentifierError),
    /// A name differs only in case from one already in the same scope.
    #[error(transparent)]
    NameClash(NameClash),
    /// A property name is neither defined by SECoP nor custom.
    #[error(
        "`{name}` is not a SECoP property; custom properties start with an underscore, e.g. `_{name}`"
    )]
    UnknownProperty {
        /// The rejected property name.
        name: String,
    },
    /// A property cannot be set directly, as it is part of the structure.
    #[error("`{0}` is set by the node structure, not as a property")]
    StructuralProperty(String),
    /// A mandatory property is missing or empty.
    #[error("missing mandatory property `{0}`")]
    MissingProperty(&'static str),
    /// A property has an invalid value.
    #[error("property `{property}` {reason}")]
    InvalidProperty {
        /// The property name.
        property: String,
        /// Why the value is invalid.
        reason: String,
    },
    /// A parameter was given a `command` datainfo, or a command another datainfo.
    #[error("{0}")]
    WrongDatainfoKind(&'static str),
    /// A component of a `group` property equals a module or accessible name.
    #[error("group component `{component}` clashes with the name `{existing}`")]
    GroupClash {
        /// The group component.
        component: String,
        /// The module or accessible name it clashes with.
        existing: String,
    },
    /// A module lacks an accessible required by its interface class.
    #[error("interface class `{class}` requires {what}")]
    InterfaceClass {
        /// The base interface class.
        class: String,
        /// The missing or wrongly declared accessible.
        what: String,
    },
    /// The `status` parameter does not have the required shape.
    #[error("`status` must be a tuple of an enum with codes 0 to 499 and a string: {0}")]
    StatusShape(String),
    /// A postfixed parameter does not fit its parent.
    #[error(transparent)]
    Postfix(PostfixError),
    /// `<p>_limits` and `<p>_min`/`<p>_max` are declared for the same parent.
    #[error("`{limits}` and `{other}` cannot both be declared")]
    ConflictingLimits {
        /// The `_limits` parameter.
        limits: String,
        /// The `_min` or `_max` parameter.
        other: String,
    },
    /// An interface class is neither defined by SECoP nor a custom `_` name.
    #[error(
        "`{0}` is not a SECoP interface class; custom classes start with an underscore, e.g. `_{0}`"
    )]
    UnknownInterfaceClass(String),
    /// The module's or a parameter's poll settings are invalid.
    #[error("{0}")]
    Polling(&'static str),
}

impl ModelError {
    fn new(path: impl Into<String>, kind: ModelErrorKind) -> Self {
        Self {
            path: path.into(),
            kind,
        }
    }

    fn property(path: &str, property: &str, reason: impl Into<String>) -> Self {
        Self::new(
            format!("{path}.{property}").trim_start_matches('.'),
            ModelErrorKind::InvalidProperty {
                property: property.to_owned(),
                reason: reason.into(),
            },
        )
    }
}

/// [`NodeBuilder::build`] found problems; all of them are listed.
#[derive(Debug, Clone, PartialEq, Error)]
pub struct BuildError(pub Vec<ModelError>);

impl fmt::Display for BuildError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid node declaration:")?;
        for e in &self.0 {
            write!(f, "\n  {e}")?;
        }
        Ok(())
    }
}

/// A non-fatal remark about a node declaration, e.g. a non-standard property.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelWarning {
    /// Where the remark applies, like [`ModelError::path`].
    pub path: String,
    /// The remark.
    pub message: String,
}

impl fmt::Display for ModelWarning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.path, self.message)
    }
}

// ---------------------------------------------------------------------------------------
// properties

/// The properties of a node, module or accessible, in declaration order.
#[derive(Debug, Clone, Default)]
struct Properties {
    values: IndexMap<String, Json>,
    non_standard: Vec<String>,
}

impl Properties {
    fn set(&mut self, key: &str, value: Json) {
        self.values.insert(key.to_owned(), value);
    }

    /// Sets a SECoP-defined or custom property, rejecting anything else.
    fn set_checked(
        &mut self,
        known: &[&str],
        key: &str,
        value: Json,
    ) -> Result<(), ModelErrorKind> {
        if STRUCTURAL_PROPERTIES.contains(&key) {
            return Err(ModelErrorKind::StructuralProperty(key.to_owned()));
        }
        validate_identifier(key).map_err(ModelErrorKind::InvalidName)?;
        if !known.contains(&key) && !key.starts_with('_') {
            return Err(ModelErrorKind::UnknownProperty {
                name: key.to_owned(),
            });
        }
        self.set(key, value);
        Ok(())
    }

    fn set_non_standard(&mut self, key: &str, value: Json) {
        self.set(key, value);
        if !self.non_standard.iter().any(|k| k == key) {
            self.non_standard.push(key.to_owned());
        }
    }

    fn get(&self, key: &str) -> Option<&Json> {
        self.values.get(key)
    }

    fn str(&self, key: &str) -> Option<&str> {
        self.get(key).and_then(Json::as_str)
    }
}

// ---------------------------------------------------------------------------------------
// builders

/// Network and resource options of a node.
#[derive(Debug, Clone)]
pub(crate) struct NodeOptions {
    pub bind: SocketAddr,
    pub max_line_length: usize,
    pub max_outgoing_bytes: usize,
    pub initial_read: bool,
    pub monitored: bool,
    /// `None`: from the environment ([`LogFiles::from_env`]).
    pub log_files: Option<LogFiles>,
    pub discovery: Discovery,
}

/// Declares a SEC node. See the [module documentation](self).
#[must_use]
pub struct NodeBuilder {
    props: Properties,
    modules: IndexMap<String, Module>,
    module_names: NameScope,
    options: NodeOptions,
}

impl NodeBuilder {
    /// Starts a node with its mandatory properties `equipment_id` (worldwide unique, e.g.
    /// `"HZB-vm4"`) and `description`. It listens on all interfaces, port 2055, unless
    /// [`bind`](Self::bind) says otherwise.
    pub fn new(equipment_id: &str, description: &str) -> Self {
        let mut props = Properties::default();
        props.set("equipment_id", equipment_id.into());
        props.set("description", description.into());
        Self {
            props,
            modules: IndexMap::new(),
            module_names: NameScope::new(),
            options: NodeOptions {
                bind: SocketAddr::from(([0, 0, 0, 0], 2055)),
                max_line_length: DEFAULT_MAX_LINE_LENGTH,
                max_outgoing_bytes: DEFAULT_MAX_OUTGOING_BYTES,
                initial_read: true,
                monitored: true,
                log_files: None,
                discovery: Discovery::default(),
            },
        }
    }

    /// Sets the `firmware` property, e.g. `"must-0.1.0"`.
    pub fn firmware(mut self, firmware: &str) -> Self {
        self.props.set("firmware", firmware.into());
        self
    }

    /// Sets the `implementor` property (globally unique, e.g. `"hzb.de"`).
    pub fn implementor(mut self, implementor: &str) -> Self {
        self.props.set("implementor", implementor.into());
        self
    }

    /// Sets the `timeout` property in seconds; handlers taking longer are answered with
    /// `TimeoutError`. Default 10 s.
    pub fn timeout(mut self, seconds: f64) -> Self {
        self.props.set("timeout", seconds.into());
        self
    }

    /// Sets a SECoP-defined or custom (`_name`) node property.
    pub fn property(mut self, key: &str, value: impl Into<Json>) -> Result<Self, ModelError> {
        self.props
            .set_checked(NODE_PROPERTIES, key, value.into())
            .map_err(|kind| ModelError::new("", kind))?;
        Ok(self)
    }

    /// Sets a property that is neither SECoP-defined nor custom; it is published
    /// unchanged and reported as a warning by [`build`](Self::build).
    pub fn non_standard_property(mut self, key: &str, value: impl Into<Json>) -> Self {
        self.props.set_non_standard(key, value.into());
        self
    }

    /// Adds a module. Fails if its name is invalid or clashes with another module.
    pub fn module(mut self, module: Module) -> Result<Self, ModelError> {
        let err = |kind| ModelError::new(module.name.clone(), kind);
        validate_identifier(&module.name).map_err(|e| err(ModelErrorKind::InvalidName(e)))?;
        self.module_names
            .insert(&module.name)
            .map_err(|e| err(ModelErrorKind::NameClash(e)))?;
        self.modules.insert(module.name.clone(), module);
        Ok(self)
    }

    /// Sets the address to listen on (default `0.0.0.0:2055`).
    pub fn bind(mut self, addr: SocketAddr) -> Self {
        self.options.bind = addr;
        self
    }

    /// Sets the maximum length of an incoming message (default 1 MiB); longer messages
    /// are answered with `ProtocolError`.
    pub fn max_line_length(mut self, bytes: usize) -> Self {
        self.options.max_line_length = bytes;
        self
    }

    /// Sets how much outgoing data may be buffered for one connection (default 16 MiB).
    /// Above the limit the node stops reading the client's requests; an update that
    /// would exceed it closes the connection.
    pub fn max_outgoing_bytes(mut self, bytes: usize) -> Self {
        self.options.max_outgoing_bytes = bytes;
        self
    }

    /// Whether to read every parameter once in the background when the node starts, so
    /// that `activate` has values to send (default true).
    pub fn initial_read(mut self, enabled: bool) -> Self {
        self.options.initial_read = enabled;
        self
    }

    /// Whether the node registers with the process's monitoring endpoint when it starts
    /// (default true; see [`crate::monitor`]). Code that registers the node itself, like
    /// the SHALL C ABI with [`crate::monitor::ExternalNode`], turns this off.
    pub fn monitored(mut self, enabled: bool) -> Self {
        self.options.monitored = enabled;
        self
    }

    /// Sets where and how long the node logs to disk, overriding the environment
    /// variables (see [`crate::logfiles`]; default: [`LogFiles::from_env`]).
    pub fn log_files(mut self, log_files: LogFiles) -> Self {
        self.options.log_files = Some(log_files);
        self
    }

    /// Sets how the node takes part in SECoP's UDP discovery (default: on, port 10767,
    /// announcing at startup; see [`crate::discovery`]). `MUST_DISCOVERY=0` turns it off
    /// regardless.
    pub fn discovery(mut self, discovery: Discovery) -> Self {
        self.options.discovery = discovery;
        self
    }
}

/// Declares a module.
#[must_use]
pub struct Module {
    name: String,
    props: Properties,
    accessibles: IndexMap<String, Accessible>,
    accessible_names: NameScope,
    handler: Option<Arc<dyn ModuleHandler>>,
    tiered_poll: Option<Duration>,
    uniform_poll: Option<Duration>,
    slow_poll: Option<Duration>,
    polling_off: bool,
}

enum Accessible {
    Parameter(Parameter),
    Command(Command),
}

impl Module {
    /// Starts a module with its mandatory `description`. `interface_classes` must be set
    /// too, see [`interface_classes`](Self::interface_classes).
    pub fn new(name: &str, description: &str) -> Self {
        let mut props = Properties::default();
        props.set("description", description.into());
        Self {
            name: name.to_owned(),
            props,
            accessibles: IndexMap::new(),
            accessible_names: NameScope::new(),
            handler: None,
            tiered_poll: None,
            uniform_poll: None,
            slow_poll: None,
            polling_off: false,
        }
    }

    /// Sets `interface_classes`, highest class first, ending with a SECoP base class,
    /// e.g. `["Drivable"]` or `["_MyController", "Drivable"]`. Names other than the SECoP
    /// classes ([`INTERFACE_CLASSES`]) must start with an underscore.
    pub fn interface_classes<'a>(mut self, classes: impl IntoIterator<Item = &'a str>) -> Self {
        let classes: Vec<Json> = classes.into_iter().map(Json::from).collect();
        self.props.set("interface_classes", classes.into());
        self
    }

    /// Sets `features`; published as `[]` if not set.
    pub fn features<'a>(mut self, features: impl IntoIterator<Item = &'a str>) -> Self {
        let features: Vec<Json> = features.into_iter().map(Json::from).collect();
        self.props.set("features", features.into());
        self
    }

    /// Sets `implementation`, e.g. a driver name; published as the handler's
    /// [`ModuleHandler::implementation`] if not set.
    pub fn implementation(mut self, implementation: &str) -> Self {
        self.props.set("implementation", implementation.into());
        self
    }

    /// Sets `visibility`, e.g. `"www"` or `"expert"`.
    pub fn visibility(mut self, visibility: &str) -> Self {
        self.props.set("visibility", visibility.into());
        self
    }

    /// Sets `group`, components separated by `:`.
    pub fn group(mut self, group: &str) -> Self {
        self.props.set("group", group.into());
        self
    }

    /// Sets a SECoP-defined or custom (`_name`) module property.
    pub fn property(mut self, key: &str, value: impl Into<Json>) -> Result<Self, ModelError> {
        self.props
            .set_checked(MODULE_PROPERTIES, key, value.into())
            .map_err(|kind| ModelError::new(self.name.clone(), kind))?;
        Ok(self)
    }

    /// Sets a property that is neither SECoP-defined nor custom; see
    /// [`NodeBuilder::non_standard_property`].
    pub fn non_standard_property(mut self, key: &str, value: impl Into<Json>) -> Self {
        self.props.set_non_standard(key, value.into());
        self
    }

    /// Adds a parameter. Fails on an invalid or clashing name or a `command` datainfo.
    pub fn parameter(self, parameter: Parameter) -> Result<Self, ModelError> {
        let path = format!("{}:{}", self.name, parameter.name);
        if matches!(parameter.datainfo, DataInfo::Command(_)) {
            return Err(ModelError::new(
                path,
                ModelErrorKind::WrongDatainfoKind("a parameter cannot have a `command` datainfo"),
            ));
        }
        self.add_accessible(
            path,
            parameter.name.clone(),
            Accessible::Parameter(parameter),
        )
    }

    /// Adds a command. Fails on an invalid or clashing name.
    pub fn command(self, command: Command) -> Result<Self, ModelError> {
        let path = format!("{}:{}", self.name, command.name);
        self.add_accessible(path, command.name.clone(), Accessible::Command(command))
    }

    fn add_accessible(
        mut self,
        path: String,
        name: String,
        accessible: Accessible,
    ) -> Result<Self, ModelError> {
        validate_identifier(&name)
            .map_err(|e| ModelError::new(path.clone(), ModelErrorKind::InvalidName(e)))?;
        self.accessible_names
            .insert(&name)
            .map_err(|e| ModelError::new(path, ModelErrorKind::NameClash(e)))?;
        self.accessibles.insert(name, accessible);
        Ok(self)
    }

    /// Sets the handler that reads, changes and calls on the hardware. Without one,
    /// parameters are served from the cache only and changes and commands fail with
    /// `NotImplemented`.
    pub fn handler(mut self, handler: impl ModuleHandler + 'static) -> Self {
        self.handler = Some(Arc::new(handler));
        self
    }

    /// Sets the handler from an existing `Arc`.
    pub fn handler_arc(mut self, handler: Arc<dyn ModuleHandler>) -> Self {
        self.handler = Some(handler);
        self
    }

    /// Sets the fast tier of the module's polling, 5 s by default. Without other poll
    /// settings a module polls in two tiers: `value` and `status` of a `Readable`,
    /// `Writable` or `Drivable` module at the fast interval, and every other parameter
    /// that is not constant and not cached every 60 s (see
    /// [`slow_poll_interval`](Self::slow_poll_interval)). Parameters with an interval of
    /// their own keep it. A `pollinterval` parameter of the module replaces the fast
    /// interval while the node runs.
    pub fn poll_interval(mut self, interval: Duration) -> Self {
        self.tiered_poll = Some(interval);
        self
    }

    /// Interval of the slow poll tier (see [`poll_interval`](Self::poll_interval)), 60 s
    /// if not set.
    pub fn slow_poll_interval(mut self, interval: Duration) -> Self {
        self.slow_poll = Some(interval);
        self
    }

    /// Polls all parameters of the module that have no interval of their own, are not
    /// constant and not cached, at the same `interval`. A `pollinterval` parameter of the
    /// module replaces `interval` while the node runs. Cannot be combined with
    /// [`poll_interval`](Self::poll_interval) or [`no_polling`](Self::no_polling).
    pub fn poll_all(mut self, interval: Duration) -> Self {
        self.uniform_poll = Some(interval);
        self
    }

    /// Turns the module's polling off: only parameters with an interval of their own
    /// ([`Parameter::poll_interval`]) are polled. Cannot be combined with
    /// [`poll_interval`](Self::poll_interval) or [`poll_all`](Self::poll_all).
    pub fn no_polling(mut self) -> Self {
        self.polling_off = true;
        self
    }
}

/// Declares a parameter.
#[must_use]
pub struct Parameter {
    name: String,
    datainfo: DataInfo,
    props: Properties,
    cached: bool,
    initial: Option<Value>,
    poll_interval: Option<Duration>,
}

impl Parameter {
    /// Starts a readonly parameter with its mandatory `datainfo` and `description`.
    pub fn new(name: &str, datainfo: DataInfo, description: &str) -> Self {
        let mut props = Properties::default();
        props.set("description", description.into());
        props.set("readonly", true.into());
        Self {
            name: name.to_owned(),
            datainfo,
            props,
            cached: false,
            initial: None,
            poll_interval: None,
        }
    }

    /// Makes the parameter writable (`readonly: false`).
    pub fn writable(mut self) -> Self {
        self.props.set("readonly", false.into());
        self
    }

    /// Sets `visibility`, e.g. `"www"` or `"expert"`.
    pub fn visibility(mut self, visibility: &str) -> Self {
        self.props.set("visibility", visibility.into());
        self
    }

    /// Sets `group`, components separated by `:`.
    pub fn group(mut self, group: &str) -> Self {
        self.props.set("group", group.into());
        self
    }

    /// Declares the parameter constant with the given value (JSON, checked against the
    /// datainfo). It is answered without calling the handler and never changes.
    pub fn constant(mut self, value: impl Into<Json>) -> Self {
        self.props.set("constant", value.into());
        self
    }

    /// Serves reads from the cache without calling the handler; the application keeps
    /// the value current by pushing updates.
    pub fn cached(mut self) -> Self {
        self.cached = true;
        self
    }

    /// Value in the cache when the node starts.
    pub fn initial_value(mut self, value: Value) -> Self {
        self.initial = Some(value);
        self
    }

    /// Polls this parameter at `interval`, overriding the module's polling. Ignored, with a
    /// warning, for constant and cached parameters and for `pollinterval`.
    pub fn poll_interval(mut self, interval: Duration) -> Self {
        self.poll_interval = Some(interval);
        self
    }

    /// Sets a SECoP-defined or custom (`_name`) property.
    pub fn property(mut self, key: &str, value: impl Into<Json>) -> Result<Self, ModelError> {
        self.props
            .set_checked(PARAMETER_PROPERTIES, key, value.into())
            .map_err(|kind| ModelError::new(self.name.clone(), kind))?;
        Ok(self)
    }

    /// Sets a property that is neither SECoP-defined nor custom; see
    /// [`NodeBuilder::non_standard_property`].
    pub fn non_standard_property(mut self, key: &str, value: impl Into<Json>) -> Self {
        self.props.set_non_standard(key, value.into());
        self
    }
}

/// Declares a command.
#[must_use]
pub struct Command {
    name: String,
    datainfo: DataInfo,
    props: Properties,
}

impl Command {
    /// Starts a command. `datainfo` must be a `command` datainfo; use
    /// `DataInfo::command().build()` for a command without argument and result.
    pub fn new(name: &str, datainfo: DataInfo, description: &str) -> Self {
        let mut props = Properties::default();
        props.set("description", description.into());
        Self {
            name: name.to_owned(),
            datainfo,
            props,
        }
    }

    /// Sets `visibility`.
    pub fn visibility(mut self, visibility: &str) -> Self {
        self.props.set("visibility", visibility.into());
        self
    }

    /// Sets `group`.
    pub fn group(mut self, group: &str) -> Self {
        self.props.set("group", group.into());
        self
    }

    /// Sets a SECoP-defined or custom (`_name`) property.
    pub fn property(mut self, key: &str, value: impl Into<Json>) -> Result<Self, ModelError> {
        self.props
            .set_checked(COMMAND_PROPERTIES, key, value.into())
            .map_err(|kind| ModelError::new(self.name.clone(), kind))?;
        Ok(self)
    }

    /// Sets a property that is neither SECoP-defined nor custom.
    pub fn non_standard_property(mut self, key: &str, value: impl Into<Json>) -> Self {
        self.props.set_non_standard(key, value.into());
        self
    }
}

// ---------------------------------------------------------------------------------------
// the built node

/// A completed, validated node declaration, ready to be served. Its structure cannot
/// change any more: a `Node` has no methods to add modules or accessibles.
///
/// ```compile_fail,E0599
/// use must_server::{Module, NodeBuilder};
///
/// let node = NodeBuilder::new("HZB-vm4", "test node").build().unwrap();
/// let node = node.module(Module::new("t1", "too late")); // no such method on Node
/// ```
#[derive(Clone)]
pub struct Node {
    pub(crate) model: Arc<NodeModel>,
    warnings: Vec<ModelWarning>,
}

pub(crate) struct NodeModel {
    pub equipment_id: String,
    pub props: IndexMap<String, Json>,
    pub modules: IndexMap<String, ModuleModel>,
    pub options: NodeOptions,
    pub timeout: Duration,
}

pub(crate) struct ModuleModel {
    pub name: String,
    pub props: IndexMap<String, Json>,
    pub accessibles: IndexMap<String, AccessibleModel>,
    pub handler: Arc<dyn ModuleHandler>,
    pub poll: ModulePoll,
}

/// How a module polls, resolved from its declaration.
pub(crate) struct ModulePoll {
    /// Interval of the [`PollSource::Fast`] parameters as declared.
    pub fast: Option<Duration>,
    /// Interval of the [`PollSource::Slow`] parameters.
    pub slow: Duration,
    /// Whether the module's `pollinterval` parameter drives the fast interval.
    pub driven: bool,
}

/// Where a parameter's poll interval comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PollSource {
    /// Not polled.
    None,
    /// Its own interval.
    Own(Duration),
    /// The module's fast (or uniform) interval, or its `pollinterval` parameter.
    Fast,
    /// The module's slow interval.
    Slow,
}

/// Name of the parameter that drives a module's polling.
pub(crate) const POLLINTERVAL: &str = "pollinterval";

/// Interval of the fast poll tier unless the module sets one.
pub(crate) const DEFAULT_FAST_POLL: Duration = Duration::from_secs(5);

/// Interval of the slow poll tier unless the module sets one.
const DEFAULT_SLOW_POLL: Duration = Duration::from_secs(60);

/// Base classes whose `value` and `status` are in the fast poll tier.
const FAST_TIER_CLASSES: &[&str] = &["Readable", "Writable", "Drivable"];

pub(crate) enum AccessibleModel {
    Parameter(ParameterModel),
    Command(CommandModel),
}

pub(crate) struct ParameterModel {
    pub datainfo: DataInfo,
    pub props: IndexMap<String, Json>,
    pub readonly: bool,
    pub constant: Option<Value>,
    pub cached: bool,
    pub initial: Option<Value>,
    pub poll: PollSource,
    /// Names of this parameter's `_min`, `_max` and `_limits` parameters, if declared.
    pub limit_params: Vec<(Postfix, String)>,
}

pub(crate) struct CommandModel {
    pub datainfo: DataInfo,
    pub props: IndexMap<String, Json>,
}

impl CommandModel {
    pub fn info(&self) -> &CommandInfo {
        match &self.datainfo {
            DataInfo::Command(c) => c,
            _ => unreachable!("checked when the node was built"),
        }
    }
}

impl Node {
    /// The node's `equipment_id`.
    pub fn equipment_id(&self) -> &str {
        &self.model.equipment_id
    }

    /// Remarks about the declaration that did not prevent building it.
    pub fn warnings(&self) -> &[ModelWarning] {
        &self.warnings
    }

    /// The structure report sent in reply to `describe`.
    pub fn structure_report(&self) -> Json {
        crate::report::structure_report(&self.model)
    }

    /// The address the node will listen on.
    pub fn bind_address(&self) -> SocketAddr {
        self.model.options.bind
    }
}

impl ModuleModel {
    pub fn parameter(&self, name: &str) -> Option<&ParameterModel> {
        match self.accessibles.get(name) {
            Some(AccessibleModel::Parameter(p)) => Some(p),
            _ => None,
        }
    }

    pub fn command(&self, name: &str) -> Option<&CommandModel> {
        match self.accessibles.get(name) {
            Some(AccessibleModel::Command(c)) => Some(c),
            _ => None,
        }
    }

    pub fn parameters(&self) -> impl Iterator<Item = (&String, &ParameterModel)> {
        self.accessibles.iter().filter_map(|(name, a)| match a {
            AccessibleModel::Parameter(p) => Some((name, p)),
            AccessibleModel::Command(_) => None,
        })
    }
}

impl ParameterModel {
    /// Current dynamic limits from the cached values of the limit parameters.
    pub fn dyn_limits(&self, lookup: impl Fn(&str) -> Option<Value>) -> Option<DynLimits> {
        if self.limit_params.is_empty() {
            return None;
        }
        let mut limits = DynLimits::default();
        for (postfix, name) in &self.limit_params {
            let Some(value) = lookup(name) else { continue };
            match postfix {
                Postfix::Min => limits.min = value.as_f64(),
                Postfix::Max => limits.max = value.as_f64(),
                Postfix::Limits => {
                    if let Some(l) = DynLimits::from_limits(&value) {
                        limits = l;
                    }
                }
                Postfix::Enable => {}
            }
        }
        Some(limits)
    }
}

// ---------------------------------------------------------------------------------------
// validation

impl NodeBuilder {
    /// Checks the whole declaration and returns the node, or every problem found.
    pub fn build(self) -> Result<Node, BuildError> {
        let mut v = Validator::default();
        v.check_node(&self);
        if !v.errors.is_empty() {
            return Err(BuildError(v.errors));
        }

        let timeout = self
            .props
            .get("timeout")
            .and_then(Json::as_f64)
            .unwrap_or(DEFAULT_TIMEOUT);
        let modules = self
            .modules
            .into_iter()
            .map(|(name, m)| (name, finish_module(m)))
            .collect();
        let model = NodeModel {
            equipment_id: self
                .props
                .str("equipment_id")
                .unwrap_or_default()
                .to_owned(),
            props: self.props.values,
            modules,
            options: self.options,
            timeout: Duration::from_secs_f64(timeout),
        };
        Ok(Node {
            model: Arc::new(model),
            warnings: v.warnings,
        })
    }
}

fn finish_module(m: Module) -> ModuleModel {
    let base = base_class(&m.props).map(str::to_owned);
    let fast_names: &[&str] = if base
        .as_deref()
        .is_some_and(|b| FAST_TIER_CLASSES.contains(&b))
    {
        &["value", "status"]
    } else {
        &[]
    };
    let off = m.polling_off;
    let tiered = !off && m.uniform_poll.is_none();
    let driven = !off && drives_polling(&m);
    let fast = if off {
        None
    } else {
        m.uniform_poll
            .or(Some(m.tiered_poll.unwrap_or(DEFAULT_FAST_POLL)))
    };
    let names: HashSet<String> = m.accessibles.keys().cloned().collect();
    let accessibles = m
        .accessibles
        .into_iter()
        .map(|(name, a)| {
            let model = match a {
                Accessible::Parameter(p) => {
                    let limit_params = [Postfix::Min, Postfix::Max, Postfix::Limits]
                        .into_iter()
                        .map(|pf| (pf, format!("{name}{pf}")))
                        .filter(|(_, n)| names.contains(n))
                        .collect();
                    let constant = p
                        .props
                        .get("constant")
                        .and_then(|c| p.datainfo.decode(c, Mode::Report).ok());
                    let poll = if p.cached || constant.is_some() || name == POLLINTERVAL {
                        PollSource::None
                    } else if let Some(own) = p.poll_interval {
                        PollSource::Own(own)
                    } else if off {
                        PollSource::None
                    } else if !tiered || fast_names.contains(&name.as_str()) {
                        PollSource::Fast
                    } else {
                        PollSource::Slow
                    };
                    let mut initial = p.initial;
                    if driven && name == POLLINTERVAL && initial.is_none() && constant.is_none() {
                        let seconds = fast.map_or(0.0, |d| d.as_secs_f64());
                        initial = Some(Value::Double(seconds));
                    }
                    AccessibleModel::Parameter(ParameterModel {
                        readonly: p.props.get("readonly") != Some(&Json::Bool(false)),
                        datainfo: p.datainfo,
                        props: p.props.values,
                        constant,
                        cached: p.cached,
                        initial,
                        poll,
                        limit_params,
                    })
                }
                Accessible::Command(c) => AccessibleModel::Command(CommandModel {
                    datainfo: c.datainfo,
                    props: c.props.values,
                }),
            };
            (name, model)
        })
        .collect();
    let handler = m.handler.unwrap_or_else(|| Arc::new(NoHandler));
    // required by the spec's machine-readable schema, optional in its prose: always publish
    let mut props = m.props.values;
    props
        .entry("features".into())
        .or_insert_with(|| Json::Array(Vec::new()));
    props
        .entry("implementation".into())
        .or_insert_with(|| handler.implementation().into());
    ModuleModel {
        name: m.name,
        props,
        accessibles,
        handler,
        poll: ModulePoll {
            fast,
            slow: m.slow_poll.unwrap_or(DEFAULT_SLOW_POLL),
            driven,
        },
    }
}

/// The base class, the last entry of `interface_classes`.
fn base_class(props: &Properties) -> Option<&str> {
    props
        .get("interface_classes")
        .and_then(Json::as_array)
        .and_then(|c| c.last())
        .and_then(Json::as_str)
}

/// Whether the module has a `pollinterval` parameter that can drive its polling: a
/// `double` without a negative `min`.
fn drives_polling(m: &Module) -> bool {
    match m.accessibles.get(POLLINTERVAL) {
        Some(Accessible::Parameter(p)) => match &p.datainfo {
            DataInfo::Double(d) => d.min().is_none_or(|min| min >= 0.0),
            _ => false,
        },
        _ => false,
    }
}

#[derive(Default)]
struct Validator {
    errors: Vec<ModelError>,
    warnings: Vec<ModelWarning>,
}

impl Validator {
    fn error(&mut self, e: ModelError) {
        self.errors.push(e);
    }

    fn check_node(&mut self, node: &NodeBuilder) {
        let p = &node.props;
        self.require_text(p, "", "equipment_id");
        self.require_text(p, "", "description");
        for key in ["firmware", "implementor"] {
            self.check_string(p, "", key);
        }
        if let Some(t) = p.get("timeout")
            && !t.as_f64().is_some_and(|t| t.is_finite() && t > 0.0)
        {
            self.error(ModelError::property(
                "",
                "timeout",
                "must be a positive number of seconds",
            ));
        }
        if let Some(s) = p.get("systems")
            && !s.is_object()
        {
            self.error(ModelError::property("", "systems", "must be an object"));
        }
        self.check_string_list(p, "", "schemata");
        self.warn_non_standard(p, "");

        // names for group checks: module names, and module plus accessible names
        let module_names: Vec<&str> = node.modules.keys().map(String::as_str).collect();
        let mut all_names = module_names.clone();
        for m in node.modules.values() {
            all_names.extend(m.accessibles.keys().map(String::as_str));
        }

        for module in node.modules.values() {
            self.check_module(module, &module_names, &all_names);
        }
    }

    fn check_module(&mut self, m: &Module, module_names: &[&str], all_names: &[&str]) {
        let path = m.name.as_str();
        let p = &m.props;
        self.require_text(p, path, "description");
        match p.get("interface_classes") {
            None => self.error(ModelError::new(
                path,
                ModelErrorKind::MissingProperty("interface_classes"),
            )),
            Some(classes) => {
                self.check_string_list(p, path, "interface_classes");
                let names = classes
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Json::as_str);
                for name in names {
                    if !INTERFACE_CLASSES.contains(&name) && !name.starts_with('_') {
                        self.error(ModelError::new(
                            format!("{path}.interface_classes"),
                            ModelErrorKind::UnknownInterfaceClass(name.to_owned()),
                        ));
                    }
                }
            }
        }
        self.check_string_list(p, path, "features");
        for key in ["implementor", "implementation"] {
            self.check_string(p, path, key);
        }
        self.check_common(p, path, module_names);
        if let Some(c) = p.get("acquisition_channels")
            && !c
                .as_object()
                .is_some_and(|o| o.values().all(Json::is_string))
        {
            self.error(ModelError::property(
                path,
                "acquisition_channels",
                "must be an object of strings",
            ));
        }
        self.warn_non_standard(p, path);

        for (name, accessible) in &m.accessibles {
            let path = format!("{}:{name}", m.name);
            match accessible {
                Accessible::Parameter(param) => self.check_parameter(param, &path, all_names),
                Accessible::Command(cmd) => self.check_command(cmd, &path, all_names),
            }
        }
        self.check_interface_classes(m);
        self.check_status(m);
        self.check_postfixes(m);
        self.check_polling(m);
    }

    fn check_parameter(&mut self, param: &Parameter, path: &str, names: &[&str]) {
        let p = &param.props;
        self.require_text(p, path, "description");
        self.check_common(p, path, names);
        self.check_bool(p, path, "readonly");
        self.check_bool(p, path, "checkable");
        if let Some(constant) = p.get("constant")
            && let Err(e) = param.datainfo.decode(constant, Mode::Request)
        {
            self.error(ModelError::property(
                path,
                "constant",
                format!("does not fit the datainfo: {e}"),
            ));
        }
        self.warn_non_standard(p, path);
    }

    fn check_command(&mut self, cmd: &Command, path: &str, names: &[&str]) {
        if !matches!(cmd.datainfo, DataInfo::Command(_)) {
            self.error(ModelError::new(
                path,
                ModelErrorKind::WrongDatainfoKind("a command needs a `command` datainfo"),
            ));
        }
        let p = &cmd.props;
        self.require_text(p, path, "description");
        self.check_common(p, path, names);
        self.check_bool(p, path, "checkable");
        self.warn_non_standard(p, path);
    }

    /// `visibility`, `group` and `meaning`, shared by modules and accessibles.
    fn check_common(&mut self, p: &Properties, path: &str, group_names: &[&str]) {
        if let Some(v) = p.get("visibility")
            && !v.as_str().is_some_and(|v| VISIBILITIES.contains(&v))
        {
            self.error(ModelError::property(
                path,
                "visibility",
                format!("{v} is not a SECoP visibility like \"www\", \"wr-\" or \"expert\""),
            ));
        }
        if let Some(meaning) = p.get("meaning")
            && !meaning.is_object()
        {
            self.error(ModelError::property(path, "meaning", "must be an object"));
        }
        self.check_string(p, path, "group");
        if let Some(group) = p.str("group") {
            for component in group.split(':') {
                let lower = component.to_lowercase();
                if let Some(existing) = group_names.iter().find(|n| n.to_lowercase() == lower) {
                    self.error(ModelError::new(
                        format!("{path}.group"),
                        ModelErrorKind::GroupClash {
                            component: component.to_owned(),
                            existing: (*existing).to_owned(),
                        },
                    ));
                }
            }
        }
    }

    fn check_polling(&mut self, m: &Module) {
        let path = m.name.as_str();
        let modes = [
            m.tiered_poll.is_some(),
            m.uniform_poll.is_some(),
            m.polling_off,
        ];
        if modes.into_iter().filter(|set| *set).count() > 1 {
            self.error(ModelError::new(
                path,
                ModelErrorKind::Polling(
                    "only one of `poll_interval` (tiered), `poll_all` (uniform) and \
                     `no_polling` can be set",
                ),
            ));
        }
        let module_intervals = [m.tiered_poll, m.uniform_poll, m.slow_poll];
        if module_intervals.iter().flatten().any(Duration::is_zero) {
            self.error(ModelError::new(
                path,
                ModelErrorKind::Polling("poll intervals must be positive"),
            ));
        }
        if m.slow_poll.is_some() && (m.uniform_poll.is_some() || m.polling_off) {
            self.warnings.push(ModelWarning {
                path: path.to_owned(),
                message: "`slow_poll_interval` has no effect with `poll_all` or `no_polling`"
                    .into(),
            });
        }
        let driven = drives_polling(m);
        if driven && m.polling_off {
            self.warnings.push(ModelWarning {
                path: format!("{path}:{POLLINTERVAL}"),
                message: "`pollinterval` has no effect: the module's polling is off".into(),
            });
        }
        for (name, accessible) in &m.accessibles {
            let Accessible::Parameter(p) = accessible else {
                continue;
            };
            let path = format!("{path}:{name}");
            if name == POLLINTERVAL && !driven {
                self.warnings.push(ModelWarning {
                    path: path.clone(),
                    message: "`pollinterval` does not drive polling: it needs a `double` \
                              datainfo without a negative `min`"
                        .into(),
                });
            }
            let Some(own) = p.poll_interval else {
                continue;
            };
            if own.is_zero() {
                self.error(ModelError::new(
                    path,
                    ModelErrorKind::Polling("poll intervals must be positive"),
                ));
            } else if p.cached || p.props.get("constant").is_some() || name == POLLINTERVAL {
                self.warnings.push(ModelWarning {
                    path,
                    message: "poll interval ignored: constant and cached parameters and \
                              `pollinterval` are never polled"
                        .into(),
                });
            }
        }
    }

    fn check_interface_classes(&mut self, m: &Module) {
        let Some(base) = base_class(&m.props) else {
            return;
        };
        let param = |name: &str| match m.accessibles.get(name) {
            Some(Accessible::Parameter(p)) => Some(p),
            _ => None,
        };
        let mut missing = |what: &str| {
            self.errors.push(ModelError::new(
                m.name.clone(),
                ModelErrorKind::InterfaceClass {
                    class: base.to_owned(),
                    what: what.to_owned(),
                },
            ));
        };
        if matches!(base, "Readable" | "Writable" | "Drivable") {
            for name in ["value", "status"] {
                if param(name).is_none() {
                    missing(&format!("a parameter `{name}`"));
                }
            }
        }
        if matches!(base, "Writable" | "Drivable") {
            match param("target") {
                None => missing("a parameter `target`"),
                Some(t) if t.props.get("readonly") != Some(&Json::Bool(false)) => {
                    missing("`target` to be writable");
                }
                Some(_) => {}
            }
        }
        if base == "Drivable" && !matches!(m.accessibles.get("stop"), Some(Accessible::Command(_)))
        {
            missing("a command `stop`");
        }
    }

    fn check_status(&mut self, m: &Module) {
        let Some(Accessible::Parameter(status)) = m.accessibles.get("status") else {
            return;
        };
        let problem = match &status.datainfo {
            DataInfo::Tuple(t) => match t.members() {
                [DataInfo::Enum(e), DataInfo::String(_)] => e
                    .members()
                    .iter()
                    .find(|(_, v)| !(0..=499).contains(*v))
                    .map(|(name, v)| format!("code {v} of `{name}` is outside 0..499")),
                _ => Some("the members are not an enum and a string".to_owned()),
            },
            other => Some(format!("it is a `{}`", other.type_name())),
        };
        if let Some(problem) = problem {
            self.error(ModelError::new(
                format!("{}:status", m.name),
                ModelErrorKind::StatusShape(problem),
            ));
        }
    }

    fn check_postfixes(&mut self, m: &Module) {
        for (name, accessible) in &m.accessibles {
            let Accessible::Parameter(param) = accessible else {
                continue;
            };
            let Some((parent_name, postfix)) = split_postfix(name) else {
                continue;
            };
            // without a parent parameter, the name is just an ordinary parameter
            let Some(Accessible::Parameter(parent)) = m.accessibles.get(parent_name) else {
                continue;
            };
            if let Err(e) = postfix.check_declared(&parent.datainfo, &param.datainfo) {
                self.error(ModelError::new(
                    format!("{}:{name}", m.name),
                    ModelErrorKind::Postfix(e),
                ));
            }
            if postfix == Postfix::Limits {
                for other in [Postfix::Min, Postfix::Max] {
                    let other_name = format!("{parent_name}{other}");
                    if m.accessibles.contains_key(&other_name) {
                        self.error(ModelError::new(
                            m.name.clone(),
                            ModelErrorKind::ConflictingLimits {
                                limits: name.clone(),
                                other: other_name,
                            },
                        ));
                    }
                }
            }
        }
    }

    fn require_text(&mut self, p: &Properties, path: &str, key: &'static str) {
        if p.str(key).is_none_or(|s| s.trim().is_empty()) {
            self.error(ModelError::new(
                format!("{path}.{key}").trim_start_matches('.'),
                ModelErrorKind::MissingProperty(key),
            ));
        }
    }

    fn check_string(&mut self, p: &Properties, path: &str, key: &str) {
        if p.get(key).is_some_and(|v| !v.is_string()) {
            self.error(ModelError::property(path, key, "must be a string"));
        }
    }

    fn check_bool(&mut self, p: &Properties, path: &str, key: &str) {
        if p.get(key).is_some_and(|v| !v.is_boolean()) {
            self.error(ModelError::property(path, key, "must be a boolean"));
        }
    }

    fn check_string_list(&mut self, p: &Properties, path: &str, key: &str) {
        if let Some(v) = p.get(key)
            && !v
                .as_array()
                .is_some_and(|items| items.iter().all(Json::is_string))
        {
            self.error(ModelError::property(path, key, "must be a list of strings"));
        }
    }

    fn warn_non_standard(&mut self, p: &Properties, path: &str) {
        for key in &p.non_standard {
            self.warnings.push(ModelWarning {
                path: format!("{path}.{key}").trim_start_matches('.').to_owned(),
                message: format!("`{key}` is not a SECoP property and not marked custom with `_`"),
            });
        }
    }
}

#[cfg(test)]
mod tests;
