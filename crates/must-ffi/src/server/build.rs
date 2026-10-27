// SPDX-License-Identifier: LGPL-3.0-or-later

//! Nodes under construction: SHALL's focus-based building, with SHALL's property rules,
//! turned into a `must-server` node by [`DraftNode::build`].

use std::collections::HashMap;
use std::ffi::{CString, c_char, c_int};
use std::net::SocketAddr;
use std::time::Duration;

use must_server::model::{
    COMMAND_PROPERTIES, MODULE_PROPERTIES, NODE_PROPERTIES, PARAMETER_PROPERTIES,
};
use must_server::{Command, ModelError, ModelErrorKind, Module, Node, NodeBuilder, Parameter};
use must_types::{DataInfo, Mode};
use serde_json::{Map, Value as Json};

use super::codes::err;
use super::handler::{Accessible, ShallModule};
use crate::variant::{self, CSECoPbaseType, Kind};

/// `SECoP_S_getsetFunction`: getter or setter of a parameter.
pub(crate) type GetSet = unsafe extern "C" fn(
    *const c_char,
    *mut c_int,
    *mut *mut CSECoPbaseType,
    *mut *mut CSECoPbaseType,
    *mut f64,
);

/// `SECoP_S_callFunction`: the function of a command.
pub(crate) type CallFunction = unsafe extern "C" fn(
    *const c_char,
    *const CSECoPbaseType,
    *mut c_int,
    *mut *mut CSECoPbaseType,
    *mut f64,
);

/// The item that `SECoP_S_add*` calls apply to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Focus {
    Node,
    Module(usize),
    Accessible(usize, usize),
}

/// A node being built.
#[derive(Clone)]
pub(crate) struct DraftNode {
    pub name: String,
    pub bind: SocketAddr,
    pub props: Map<String, Json>,
    pub modules: Vec<DraftModule>,
    pub focus: Focus,
}

#[derive(Clone)]
pub(crate) struct DraftModule {
    pub name: String,
    pub props: Map<String, Json>,
    pub poll: Option<Duration>,
    pub accessibles: Vec<DraftAccessible>,
}

#[derive(Clone)]
pub(crate) struct DraftAccessible {
    pub name: String,
    pub props: Map<String, Json>,
    /// The datainfo, as SHALL's variant type (which accepts SECoP 1.0 descriptions).
    pub datainfo: Option<variant::Node>,
    pub poll: Option<Duration>,
    pub kind: AccessibleKind,
}

#[derive(Clone)]
pub(crate) enum AccessibleKind {
    Parameter {
        writable: bool,
        getter: Option<GetSet>,
        setter: Option<GetSet>,
        /// A value pushed before the node was complete.
        initial: Option<Json>,
    },
    Command {
        function: Option<CallFunction>,
    },
}

/// A problem found by [`DraftNode::build`], with the SHALL code to return for it.
#[derive(Debug)]
pub(crate) struct Problem {
    pub code: c_int,
    pub message: String,
}

/// SHALL's name rule (`SECoP_S_Main::isValidName`).
pub(crate) fn valid_name(name: &str) -> bool {
    let rest = name.strip_prefix('_').unwrap_or(name);
    name.len() <= 63
        && rest.starts_with(|c: char| c.is_ascii_alphabetic())
        && rest.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Where a property goes; each has SECoP 2.0's and SHALL's standard properties.
#[derive(Clone, Copy)]
enum Level {
    Node,
    Module,
    Parameter,
    Command,
}

impl Level {
    /// Properties that SHALL knows but SECoP 2.0 does not define; published as
    /// non-standard properties without a warning.
    fn shall_only(self) -> &'static [&'static str] {
        match self {
            Level::Node | Level::Parameter | Level::Command => &["order"],
            Level::Module => &["order", "importance"],
        }
    }

    fn secop(self) -> &'static [&'static str] {
        match self {
            Level::Node => NODE_PROPERTIES,
            Level::Module => MODULE_PROPERTIES,
            Level::Parameter => PARAMETER_PROPERTIES,
            Level::Command => COMMAND_PROPERTIES,
        }
    }

    /// The standard spelling of `key` if it is a standard property here.
    fn standard(self, key: &str) -> Option<&'static str> {
        self.secop()
            .iter()
            .chain(self.shall_only())
            .find(|k| k.eq_ignore_ascii_case(key))
            .copied()
    }
}

impl DraftNode {
    pub(crate) fn new(name: &str, description: &str, bind: SocketAddr) -> Self {
        let mut props = Map::new();
        if !description.is_empty() {
            props.insert("description".into(), description.into());
        }
        Self {
            name: name.to_owned(),
            bind,
            props,
            modules: Vec::new(),
            focus: Focus::Node,
        }
    }

    /// The focused item, as `node`, `node:module` or `node:module:accessible`.
    pub(crate) fn focus_path(&self) -> String {
        match self.focus {
            Focus::Node => self.name.clone(),
            Focus::Module(m) => format!("{}:{}", self.name, self.modules[m].name),
            Focus::Accessible(m, a) => format!(
                "{}:{}:{}",
                self.name, self.modules[m].name, self.modules[m].accessibles[a].name
            ),
        }
    }

    pub(crate) fn module_index(&self, name: &str) -> Option<usize> {
        self.modules
            .iter()
            .position(|m| m.name.eq_ignore_ascii_case(name))
    }

    /// `SECoP_S_addModule`.
    pub(crate) fn add_module(&mut self, name: &str) -> c_int {
        if !valid_name(name) {
            return err::INVALID_NAME;
        }
        if self.module_index(name).is_some() {
            return err::NAME_ALREADY_USED;
        }
        self.modules.push(DraftModule {
            name: name.to_owned(),
            props: Map::new(),
            poll: None,
            accessibles: Vec::new(),
        });
        self.focus = Focus::Module(self.modules.len() - 1);
        err::SUCCESS
    }

    /// `SECoP_S_addCommand` and `SECoP_S_add*Parameter`: adds to the focused module.
    pub(crate) fn add_accessible(&mut self, name: &str, kind: AccessibleKind) -> c_int {
        let (Focus::Module(m) | Focus::Accessible(m, _)) = self.focus else {
            return err::INVALID_MODULE;
        };
        if !valid_name(name) {
            return err::INVALID_NAME;
        }
        let module = &mut self.modules[m];
        if module.accessible_index(name).is_some() {
            return err::NAME_ALREADY_USED;
        }
        module.accessibles.push(DraftAccessible {
            name: name.to_owned(),
            props: Map::new(),
            datainfo: None,
            poll: None,
            kind,
        });
        self.focus = Focus::Accessible(m, module.accessibles.len() - 1);
        err::SUCCESS
    }

    /// `SECoP_S_setAddFocus` below the node: `""`, `module` or `module:accessible`.
    pub(crate) fn set_focus(&mut self, path: &str) -> c_int {
        if path.is_empty() {
            self.focus = Focus::Node;
            return err::SUCCESS;
        }
        let (module, accessible) = match path.split_once(':') {
            Some((m, a)) => (m, Some(a)),
            None => (path, None),
        };
        let Some(m) = self.module_index(module) else {
            return err::INVALID_MODULE;
        };
        self.focus = match accessible.filter(|a| !a.is_empty()) {
            None => Focus::Module(m),
            Some(a) => match self.modules[m].accessible_index(a) {
                Some(a) => Focus::Accessible(m, a),
                None => return err::INVALID_PARAMETER,
            },
        };
        err::SUCCESS
    }

    /// `SECoP_S_addProperty*`: sets a property of the focused item.
    pub(crate) fn add_property(&mut self, key: &str, value: Json) -> c_int {
        if !valid_name(key) {
            return err::INVALID_NAME;
        }
        let lower = key.to_ascii_lowercase();
        if lower == "description" && value.as_str().is_none_or(str::is_empty) {
            return err::INVALID_VALUE;
        }
        match self.focus {
            Focus::Node => match lower.as_str() {
                "modules" => err::INVALID_PROPERTY,
                _ => set_property(&mut self.props, Level::Node, key, value),
            },
            Focus::Module(m) => {
                let module = &mut self.modules[m];
                match lower.as_str() {
                    "accessibles" => err::INVALID_PROPERTY,
                    "pollinterval" => set_poll(&mut module.poll, &value),
                    "interface_classes" => match interface_classes(value) {
                        Some(classes) => {
                            set_property(&mut module.props, Level::Module, key, classes)
                        }
                        None => err::INVALID_VALUE,
                    },
                    _ => set_property(&mut module.props, Level::Module, key, value),
                }
            }
            Focus::Accessible(m, a) => {
                let acc = &mut self.modules[m].accessibles[a];
                let is_command = matches!(acc.kind, AccessibleKind::Command { .. });
                match (lower.as_str(), &mut acc.kind) {
                    ("datainfo", _) => {
                        if acc.datainfo.is_some() {
                            return err::NAME_ALREADY_USED;
                        }
                        match json_text(value)
                            .and_then(|d| variant::Node::from_description(&d, is_command))
                        {
                            Some(d) if is_command == matches!(d.kind, Kind::Command { .. }) => {
                                acc.datainfo = Some(d);
                                err::SUCCESS
                            }
                            _ => err::INVALID_VALUE,
                        }
                    }
                    ("pollinterval", AccessibleKind::Parameter { .. }) => {
                        set_poll(&mut acc.poll, &value)
                    }
                    ("readonly", AccessibleKind::Parameter { writable, .. }) => {
                        match value.as_bool().or_else(|| value.as_i64().map(|i| i == 0)) {
                            Some(readonly) => {
                                *writable = !readonly;
                                err::SUCCESS
                            }
                            None => err::INVALID_VALUE,
                        }
                    }
                    _ => {
                        let level = if is_command {
                            Level::Command
                        } else {
                            Level::Parameter
                        };
                        set_property(&mut acc.props, level, key, value)
                    }
                }
            }
        }
    }

    /// Validates the node and turns it into a `must-server` node, with remarks on what
    /// was filled in; on failure all problems are listed.
    pub(crate) fn build(
        &self,
        context: &str,
        callbacks: bool,
    ) -> Result<(Node, Vec<String>), Vec<Problem>> {
        let (draft, notes) = self.with_defaults();
        draft
            .build_complete(context, callbacks)
            .map(|node| (node, notes))
    }

    /// A copy with what SHALL fills in automatically: descriptions (`"module without
    /// description"`, ...), a command's datainfo `{"type":"command"}`, and
    /// `interface_classes`, which SECoP 2.0 needs. Each is noted.
    fn with_defaults(&self) -> (Self, Vec<String>) {
        let mut draft = self.clone();
        let mut notes = Vec::new();
        describe(&mut notes, &mut draft.props, "node", &draft.name);
        for module in &mut draft.modules {
            let path = format!("{}:{}", draft.name, module.name);
            describe(&mut notes, &mut module.props, "module", &path);
            if !module.props.contains_key("interface_classes") {
                let classes = module.default_interface_classes();
                notes.push(format!(
                    "{path}: no interface_classes given (needed by SECoP 2.0), using {classes}"
                ));
                module.props.insert("interface_classes".into(), classes);
            }
            for acc in &mut module.accessibles {
                let path = format!("{path}:{}", acc.name);
                match acc.kind {
                    AccessibleKind::Parameter { .. } => {
                        describe(&mut notes, &mut acc.props, "parameter", &path);
                    }
                    AccessibleKind::Command { .. } => {
                        describe(&mut notes, &mut acc.props, "command", &path);
                        if acc.datainfo.is_none() {
                            acc.datainfo = variant::Node::from_description(
                                &serde_json::json!({"type": "command"}),
                                true,
                            );
                        }
                    }
                }
            }
        }
        (draft, notes)
    }

    fn build_complete(&self, context: &str, callbacks: bool) -> Result<Node, Vec<Problem>> {
        let mut problems = Vec::new();
        let mut props = self.props.clone();
        let description = take_string(&mut props, "description");
        // the C ABI registers its nodes with the monitor itself, from creation on
        let mut node = NodeBuilder::new(&self.name, &description)
            .bind(self.bind)
            .initial_read(callbacks)
            .monitored(false);
        for (key, value) in props {
            node = if is_settable(Level::Node, &key) {
                match node.property(&key, value) {
                    Ok(n) => n,
                    Err(e) => return Err(vec![problem(&self.name, &e)]),
                }
            } else {
                node.non_standard_property(&key, value)
            };
        }
        for module in &self.modules {
            let Some(built) = self.build_module(module, context, callbacks, &mut problems) else {
                continue;
            };
            node = match node.module(built) {
                Ok(n) => n,
                Err(e) => {
                    problems.push(problem(&self.name, &e));
                    return Err(problems);
                }
            };
        }
        if !problems.is_empty() {
            return Err(problems);
        }
        node.build()
            .map_err(|e| e.0.iter().map(|e| problem(&self.name, e)).collect())
    }

    #[allow(clippy::too_many_lines, reason = "one arm per accessible kind")]
    fn build_module(
        &self,
        draft: &DraftModule,
        context: &str,
        callbacks: bool,
        problems: &mut Vec<Problem>,
    ) -> Option<Module> {
        let mut props = draft.props.clone();
        let description = take_string(&mut props, "description");
        let mut module = Module::new(&draft.name, &description);
        if let Some(interval) = draft.poll {
            module = module.poll_interval(interval);
        }
        for (key, value) in props {
            module = if is_settable(Level::Module, &key) {
                match module.property(&key, value) {
                    Ok(m) => m,
                    Err(e) => {
                        problems.push(problem(&self.name, &e));
                        return None;
                    }
                }
            } else {
                module.non_standard_property(&key, value)
            };
        }

        let mut accessibles = HashMap::new();
        for acc in &draft.accessibles {
            let path = format!("{}:{}:{}", self.name, draft.name, acc.name);
            let full_name = CString::new(path.clone()).ok()?;
            let Some(proto) = &acc.datainfo else {
                problems.push(Problem {
                    code: err::MISSING_MANDATORY,
                    message: format!("{path}: missing mandatory property `datainfo`"),
                });
                continue;
            };
            let Some(info) = proto.to_datainfo() else {
                problems.push(Problem {
                    code: err::INVALID_VALUE,
                    message: format!(
                        "{path}: datainfo {} is not valid in SECoP 2.0",
                        proto.export_type().unwrap_or_default()
                    ),
                });
                continue;
            };
            let mut props = acc.props.clone();
            let description = take_string(&mut props, "description");
            match &acc.kind {
                AccessibleKind::Parameter {
                    writable,
                    getter,
                    setter,
                    initial,
                } => {
                    let constant = props.contains_key("constant");
                    if callbacks && !constant {
                        let code = match (getter.is_some(), !*writable || setter.is_some()) {
                            (true, true) => err::SUCCESS,
                            (false, true) => err::NO_GETTER,
                            (true, false) => err::NO_SETTER,
                            (false, false) => err::NO_SETTER_GETTER,
                        };
                        if code != err::SUCCESS {
                            problems.push(Problem {
                                code,
                                message: format!(
                                    "{path}: {}",
                                    super::codes::description(code).to_string_lossy()
                                ),
                            });
                        }
                    }
                    let mut parameter = Parameter::new(&acc.name, info.clone(), &description);
                    if *writable {
                        parameter = parameter.writable();
                    }
                    if let Some(interval) = acc.poll {
                        parameter = parameter.poll_interval(interval);
                    }
                    if let Some(value) = initial {
                        match info.decode(value, Mode::Report) {
                            Ok(v) => parameter = parameter.initial_value(v),
                            Err(e) => problems.push(Problem {
                                code: err::INVALID_VALUE,
                                message: format!("{path}: pushed value {value}: {e}"),
                            }),
                        }
                    }
                    for (key, value) in props {
                        parameter = if is_settable(Level::Parameter, &key) {
                            match parameter.property(&key, value) {
                                Ok(p) => p,
                                Err(e) => {
                                    problems.push(problem(&path, &e));
                                    return None;
                                }
                            }
                        } else {
                            parameter.non_standard_property(&key, value)
                        };
                    }
                    module = match module.parameter(parameter) {
                        Ok(m) => m,
                        Err(e) => {
                            problems.push(problem(&self.name, &e));
                            return None;
                        }
                    };
                    accessibles.insert(
                        acc.name.clone(),
                        Accessible::Parameter {
                            full_name,
                            info,
                            proto: proto.clone(),
                            getter: *getter,
                            setter: *setter,
                        },
                    );
                }
                AccessibleKind::Command { function } => {
                    let DataInfo::Command(command_info) = &info else {
                        continue; // datainfo was checked to be a command when it was set
                    };
                    let (argument, result) = match &proto.kind {
                        Kind::Command { argument, result } => {
                            (argument.as_deref().cloned(), result.as_deref().cloned())
                        }
                        _ => (None, None),
                    };
                    let mut command = Command::new(&acc.name, info.clone(), &description);
                    for (key, value) in props {
                        command = if is_settable(Level::Command, &key) {
                            match command.property(&key, value) {
                                Ok(c) => c,
                                Err(e) => {
                                    problems.push(problem(&path, &e));
                                    return None;
                                }
                            }
                        } else {
                            command.non_standard_property(&key, value)
                        };
                    }
                    module = match module.command(command) {
                        Ok(m) => m,
                        Err(e) => {
                            problems.push(problem(&self.name, &e));
                            return None;
                        }
                    };
                    accessibles.insert(
                        acc.name.clone(),
                        Accessible::Command {
                            full_name,
                            info: command_info.clone(),
                            argument,
                            result,
                            function: *function,
                        },
                    );
                }
            }
        }
        Some(module.handler(ShallModule {
            node: self.name.clone(),
            module: draft.name.clone(),
            context: context.to_owned(),
            callbacks,
            accessibles,
        }))
    }
}

impl DraftModule {
    /// `interface_classes` for a module that sets none (SHALL did not require it): the
    /// highest base class whose accessibles the module has, else none.
    pub(crate) fn default_interface_classes(&self) -> Json {
        let find = |name: &str| self.accessibles.iter().find(|a| a.name == name);
        let parameter = |name: &str| match find(name).map(|a| &a.kind) {
            Some(AccessibleKind::Parameter { writable, .. }) => Some(*writable),
            _ => None,
        };
        let readable = parameter("value").is_some() && parameter("status").is_some();
        let writable = readable && parameter("target") == Some(true);
        let drivable = writable
            && matches!(
                find("stop").map(|a| &a.kind),
                Some(AccessibleKind::Command { .. })
            );
        let class = match (drivable, writable, readable) {
            (true, _, _) => Some("Drivable"),
            (_, true, _) => Some("Writable"),
            (_, _, true) => Some("Readable"),
            _ => None,
        };
        Json::Array(class.into_iter().map(Json::from).collect())
    }

    pub(crate) fn accessible_index(&self, name: &str) -> Option<usize> {
        self.accessibles
            .iter()
            .position(|a| a.name.eq_ignore_ascii_case(name))
    }
}

/// Stores a property under its standard spelling, or as given for custom ones. Returns
/// `WARNING_CUSTOM_PROPERTY` for a non-standard name without underscore, which is kept
/// and published as SHALL did.
fn set_property(props: &mut Map<String, Json>, level: Level, key: &str, value: Json) -> c_int {
    if props.keys().any(|k| k.eq_ignore_ascii_case(key)) {
        return err::NAME_ALREADY_USED;
    }
    let standard = level.standard(key);
    let value = match standard {
        Some("visibility") => visibility(value),
        _ => value,
    };
    props.insert(standard.unwrap_or(key).to_owned(), value);
    if standard.is_some() || key.starts_with('_') {
        err::SUCCESS
    } else {
        err::WARNING_CUSTOM_PROPERTY
    }
}

/// Sets SHALL's default description if there is none, noting it.
fn describe(notes: &mut Vec<String>, props: &mut Map<String, Json>, what: &str, path: &str) {
    if !props.contains_key("description") {
        props.insert(
            "description".into(),
            format!("{what} without description").into(),
        );
        notes.push(format!("{path}: no description given"));
    }
}

/// Whether a stored property can be passed to `must-server`'s `property` (SECoP 2.0 or
/// custom); others are published as non-standard properties.
fn is_settable(level: Level, key: &str) -> bool {
    level.secop().contains(&key) || key.starts_with('_')
}

/// SHALL's `pollinterval` in seconds; 0 means no polling.
fn set_poll(poll: &mut Option<Duration>, value: &Json) -> c_int {
    match value.as_f64().filter(|s| s.is_finite() && *s >= 0.0) {
        Some(s) => {
            *poll = (s > 0.0).then(|| Duration::from_secs_f64(s.min(3600.0)));
            err::SUCCESS
        }
        None => err::INVALID_VALUE,
    }
}

/// JSON given as a JSON value or, as SHALL allowed, as a string holding JSON.
fn json_text(value: Json) -> Option<Json> {
    match value {
        Json::String(s) => serde_json::from_str(&s).ok(),
        other => Some(other),
    }
}

/// `interface_classes` as a JSON array of strings; a single name is accepted too.
fn interface_classes(value: Json) -> Option<Json> {
    let value = match value {
        Json::String(s) => serde_json::from_str(&s).unwrap_or(Json::String(s)),
        other => other,
    };
    match value {
        Json::String(s) => Some(Json::Array(vec![Json::String(s)])),
        Json::Array(items) if items.iter().all(Json::is_string) => Some(Json::Array(items)),
        _ => None,
    }
}

/// SECoP 1.0's numeric visibility (1 user, 2 advanced, 3 expert) as a 2.0 string.
fn visibility(value: Json) -> Json {
    match value.as_f64() {
        Some(v) if v <= 1.0 => "user".into(),
        Some(v) if v <= 2.0 => "advanced".into(),
        Some(_) => "expert".into(),
        None => value,
    }
}

fn take_string(props: &mut Map<String, Json>, key: &str) -> String {
    match props.shift_remove(key) {
        Some(Json::String(s)) => s,
        _ => String::new(),
    }
}

fn problem(node: &str, e: &ModelError) -> Problem {
    let code = match e.kind {
        ModelErrorKind::InvalidName(_) => err::INVALID_NAME,
        ModelErrorKind::NameClash(_) => err::NAME_ALREADY_USED,
        ModelErrorKind::MissingProperty(_) => err::MISSING_MANDATORY,
        ModelErrorKind::WrongDatainfoKind(_) => err::INVALID_VALUE,
        _ => err::INVALID_PROPERTY,
    };
    Problem {
        code,
        message: format!("{node}: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert!(valid_name("mod1") && valid_name("_custom") && valid_name("a_1"));
        assert!(!valid_name("") && !valid_name("1a") && !valid_name("__a") && !valid_name("a-b"));
        assert!(!valid_name(&"a".repeat(64)));
    }

    #[test]
    fn properties_follow_shall() {
        let mut node = DraftNode::new("n", "a node", "127.0.0.1:0".parse().unwrap());
        assert_eq!(node.add_property("firmware", "x".into()), err::SUCCESS);
        assert_eq!(
            node.add_property("Firmware", "y".into()),
            err::NAME_ALREADY_USED
        );
        assert_eq!(
            node.add_property("myprop", "x".into()),
            err::WARNING_CUSTOM_PROPERTY
        );
        assert_eq!(
            node.add_property("description", "".into()),
            err::INVALID_VALUE
        );
        assert_eq!(node.add_property("bad-name", 1.into()), err::INVALID_NAME);
        assert_eq!(node.add_module("m"), err::SUCCESS);
        assert_eq!(
            node.add_property("interface_classes", "Readable".into()),
            err::SUCCESS
        );
        assert_eq!(
            node.modules[0].props["interface_classes"],
            serde_json::json!(["Readable"])
        );
        assert_eq!(node.add_property("pollinterval", 2.0.into()), err::SUCCESS);
        assert_eq!(node.modules[0].poll, Some(Duration::from_secs(2)));
        assert_eq!(node.add_property("visibility", 3.into()), err::SUCCESS);
        assert_eq!(node.modules[0].props["visibility"], "expert");
    }

    #[test]
    fn focus() {
        let mut node = DraftNode::new("n", "a node", "127.0.0.1:0".parse().unwrap());
        let readable = || AccessibleKind::Parameter {
            writable: false,
            getter: None,
            setter: None,
            initial: None,
        };
        assert_eq!(node.add_accessible("p", readable()), err::INVALID_MODULE);
        node.add_module("m1");
        node.add_accessible("p", readable());
        assert_eq!(node.add_accessible("P", readable()), err::NAME_ALREADY_USED);
        node.add_module("m2");
        assert_eq!(node.set_focus("M1:p"), err::SUCCESS);
        assert_eq!(node.focus_path(), "n:m1:p");
        assert_eq!(node.set_focus("m1:q"), err::INVALID_PARAMETER);
        assert_eq!(node.set_focus("m3"), err::INVALID_MODULE);
        assert_eq!(node.set_focus(""), err::SUCCESS);
        assert_eq!(node.focus, Focus::Node);
    }
}
