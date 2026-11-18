// SPDX-License-Identifier: LGPL-3.0-or-later

//! Checking a node's model against SECoP before it is built.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use must_types::{DataInfo, Mode, Postfix, Value, split_postfix};
use serde_json::Value as Json;

use super::{
    Accessible, AccessibleModel, BuildError, Command, CommandModel, DEFAULT_FAST_POLL,
    DEFAULT_SLOW_POLL, DEFAULT_TIMEOUT, FAST_TIER_CLASSES, INTERFACE_CLASSES, ModelError,
    ModelErrorKind, ModelWarning, Module, ModuleModel, ModulePoll, Node, NodeBuilder, NodeModel,
    POLLINTERVAL, Parameter, ParameterModel, PollSource, Properties, VISIBILITIES,
};
use crate::handler::NoHandler;

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
    let concurrent = m.concurrent;
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
        concurrent,
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
        self.check_string_list(p, "", "schemata");
        self.warn_non_standard(p, "");

        // names for group checks: module names, and module plus accessible names
        let module_names: Vec<&str> = node.modules.keys().map(String::as_str).collect();
        if let Some(systems) = p.get("systems") {
            self.check_systems(systems, &module_names);
        }
        let mut all_names = module_names.clone();
        for m in node.modules.values() {
            all_names.extend(m.accessibles.keys().map(String::as_str));
        }

        for module in node.modules.values() {
            self.check_module(module, &module_names, &all_names);
        }
    }

    /// `systems`: what can be checked without the systems' schemata. Each entry has a
    /// string `description` and `system` and an object `modules` of strings, each naming a
    /// module or another system of the node; no system is named like a module, and none
    /// contains itself through subsystems.
    fn check_systems(&mut self, systems: &Json, module_names: &[&str]) {
        let Some(systems) = systems.as_object() else {
            return self.error(ModelError::property("", "systems", "must be an object"));
        };
        let invalid = |reason: String| ModelError::property("", "systems", reason);
        for (name, system) in systems {
            if module_names.contains(&name.as_str()) {
                self.error(invalid(format!("`{name}` is also the name of a module")));
            }
            let Some(system) = system.as_object() else {
                self.error(invalid(format!("`{name}` must be an object")));
                continue;
            };
            for key in ["description", "system"] {
                if !system.get(key).is_some_and(Json::is_string) {
                    self.error(invalid(format!("`{name}` needs a string `{key}`")));
                }
            }
            let Some(modules) = system.get("modules").and_then(Json::as_object) else {
                self.error(invalid(format!("`{name}` needs an object `modules`")));
                continue;
            };
            for (role, member) in modules {
                match member.as_str() {
                    Some(m) if module_names.contains(&m) || systems.contains_key(m) => {}
                    Some(m) => self.error(invalid(format!(
                        "`{name}.modules.{role}` names `{m}`, which is neither a module nor a \
                         system of this node"
                    ))),
                    None => {
                        self.error(invalid(format!("`{name}.modules.{role}` must be a string")));
                    }
                }
            }
        }
        // subsystems: follow the members that name systems; a name that is also a module's
        // is ambiguous and already reported, so it is not followed
        let subsystems = |name: &str| -> Vec<&str> {
            systems
                .get(name)
                .and_then(|s| s.get("modules"))
                .and_then(Json::as_object)
                .into_iter()
                .flat_map(|m| m.values().filter_map(Json::as_str))
                .filter(|m| systems.contains_key(*m) && !module_names.contains(m))
                .collect()
        };
        for name in systems.keys() {
            let mut seen = vec![name.as_str()];
            let mut todo = subsystems(name);
            while let Some(next) = todo.pop() {
                if next == name {
                    self.error(invalid(format!("`{name}` contains itself")));
                    break;
                }
                if !seen.contains(&next) {
                    seen.push(next);
                    todo.extend(subsystems(next));
                }
            }
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
        self.check_acquisition_classes(m, module_names);
        self.check_status(m);
        self.check_postfixes(m);
        self.check_predefined(m, module_names);
        self.check_polling(m);
    }

    fn check_parameter(&mut self, param: &Parameter, path: &str, names: &[&str]) {
        let p = &param.props;
        self.require_text(p, path, "description");
        self.check_common(p, path, names);
        self.check_bool(p, path, "readonly");
        self.check_bool(p, path, "checkable");
        if p.get("checkable") == Some(&Json::Bool(true))
            && (p.get("readonly") != Some(&Json::Bool(false)) || p.get("constant").is_some())
        {
            self.warnings.push(ModelWarning {
                path: path.to_owned(),
                message: "`checkable` has no effect on a readonly parameter: a `check` of it \
                          is answered with `ReadOnly`"
                    .into(),
            });
        }
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
        if let Some(meaning) = p.get("meaning") {
            self.check_meaning(meaning, path);
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

    /// `meaning` of a module or an accessible: the key types and the key combinations of
    /// the specification. Any `function` and `belongs_to` string is accepted, since both
    /// lists are open.
    fn check_meaning(&mut self, meaning: &Json, path: &str) {
        let Some(m) = meaning.as_object() else {
            return self.error(ModelError::property(path, "meaning", "must be an object"));
        };
        for (key, value) in m {
            let valid = match key.as_str() {
                "function" | "belongs_to" | "link" | "key" => value.is_string(),
                "importance" => value.as_u64().is_some_and(|i| i <= 50),
                _ => {
                    if !key.starts_with('_') {
                        self.warnings.push(ModelWarning {
                            path: format!("{path}.meaning").trim_start_matches('.').to_owned(),
                            message: format!("`{key}` is not a key of SECoP's `meaning`"),
                        });
                    }
                    continue;
                }
            };
            if !valid {
                let expected = if key == "importance" {
                    "an integer from 0 to 50"
                } else {
                    "a string"
                };
                self.error(ModelError::property(
                    path,
                    "meaning",
                    format!("`{key}` must be {expected}"),
                ));
            }
        }
        let has = |key| m.contains_key(key);
        let rules = [
            (
                !has("function") && !has("link"),
                "must contain `function` or `link`",
            ),
            (
                has("function") && !has("importance"),
                "`function` needs `importance`",
            ),
            (
                has("importance") && !has("function"),
                "`importance` needs `function`",
            ),
            (
                has("belongs_to") && !has("function"),
                "`belongs_to` needs `function`",
            ),
            (has("key") && !has("link"), "`key` needs `link`"),
        ];
        for (broken, reason) in rules {
            if broken {
                self.error(ModelError::property(path, "meaning", reason));
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

    /// The acquisition classes and `Communicator`, wherever they are in
    /// `interface_classes`: `AcquisitionChannel` and `Acquisition` come before `Readable`,
    /// which is the base class then.
    fn check_acquisition_classes(&mut self, m: &Module, module_names: &[&str]) {
        let classes: Vec<&str> = m
            .props
            .get("interface_classes")
            .and_then(Json::as_array)
            .into_iter()
            .flatten()
            .filter_map(Json::as_str)
            .collect();
        let has = |name: &str| matches!(m.accessibles.get(name), Some(Accessible::Parameter(_)));
        let command = |name: &str| matches!(m.accessibles.get(name), Some(Accessible::Command(_)));
        let channels = m.props.get("acquisition_channels");
        for class in classes {
            let mut what = Vec::new();
            match class {
                "AcquisitionController" => {
                    if !has("status") {
                        what.push("a parameter `status`");
                    }
                    if !command("go") {
                        what.push("a command `go`");
                    }
                    if channels.is_none() {
                        what.push("the module property `acquisition_channels`");
                    }
                }
                "AcquisitionChannel" | "Acquisition" => {
                    for (name, needed) in [
                        ("value", "a parameter `value`"),
                        ("status", "a parameter `status`"),
                    ] {
                        if !has(name) {
                            what.push(needed);
                        }
                    }
                    if class == "Acquisition" {
                        if !command("go") {
                            what.push("a command `go`");
                        }
                        if channels.is_some() {
                            what.push("no module property `acquisition_channels`");
                        }
                    }
                }
                // "should", not "must": `communicate` is meant for Communicators
                "Communicator" if !command("communicate") => {
                    self.warnings.push(ModelWarning {
                        path: m.name.clone(),
                        message: "a `Communicator` should have a command `communicate`".into(),
                    });
                }
                _ => {}
            }
            for what in what {
                self.errors.push(ModelError::new(
                    m.name.clone(),
                    ModelErrorKind::InterfaceClass {
                        class: class.to_owned(),
                        what: what.to_owned(),
                    },
                ));
            }
        }
        // the channels are modules of this node
        let names = channels.and_then(Json::as_object).into_iter().flatten();
        for (role, channel) in names {
            if let Some(channel) = channel.as_str()
                && !module_names.contains(&channel)
            {
                self.error(ModelError::property(
                    &m.name,
                    "acquisition_channels",
                    format!("`{role}` names `{channel}`, which is no module of this node"),
                ));
            }
        }
    }

    fn check_status(&mut self, m: &Module) {
        let Some(Accessible::Parameter(status)) = m.accessibles.get("status") else {
            return;
        };
        let path = format!("{}:status", m.name);
        let members = match &status.datainfo {
            DataInfo::Tuple(t) => match t.members() {
                [DataInfo::Enum(e), DataInfo::String(_)] => Ok(e.members()),
                _ => Err("the members are not an enum and a string".to_owned()),
            },
            other => Err(format!("it is a `{}`", other.type_name())),
        };
        let members = match members {
            Ok(members) => members,
            Err(problem) => {
                self.error(ModelError::new(path, ModelErrorKind::StatusShape(problem)));
                return;
            }
        };
        if let Some((name, v)) = members.iter().find(|(_, v)| !(0..=499).contains(*v)) {
            self.error(ModelError::new(
                path.clone(),
                ModelErrorKind::StatusShape(format!("code {v} of `{name}` is outside 0..499")),
            ));
        }
        // The spec leaves status names free; MUST is stricter: a group code (0, 100, …,
        // 400) must carry its group name, and the other codes of the table "Useful
        // statuscodes" should carry theirs.
        for (name, &code) in members {
            let by_code = STATUS_CODES.iter().find(|(c, ..)| *c == code);
            let by_name = STATUS_CODES.iter().find(|(_, n, _)| *n == name.as_str());
            // a group code or name used wrongly is an error, so it goes first
            let wrong = [by_code, by_name]
                .into_iter()
                .flatten()
                .filter(|&&(c, n, _)| c != code || n != name.as_str())
                .max_by_key(|&&(.., group)| group);
            let Some(&(c, n, group)) = wrong else {
                continue;
            };
            let problem = format!("`{name}` = {code}: SECoP names code {c} `{n}`");
            if group {
                self.error(ModelError::new(
                    path.clone(),
                    ModelErrorKind::StatusShape(problem),
                ));
            } else {
                self.warnings.push(ModelWarning {
                    path: path.clone(),
                    message: format!("{problem}, which this overrides"),
                });
            }
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

    /// `controlled_by`, `control_active` and `mode`, whose names SECoP reserves with a
    /// fixed data type. A name without a leading underscore claims the standard meaning,
    /// so one of another type is an error.
    fn check_predefined(&mut self, m: &Module, module_names: &[&str]) {
        let param = |name: &str| m.accessibles.get(name);
        let path = |name: &str| format!("{}:{name}", m.name);
        if let Some(accessible) = param("controlled_by") {
            let mut wrong = |what| {
                self.errors.push(ModelError::new(
                    path("controlled_by"),
                    ModelErrorKind::PredefinedAccessible {
                        name: "controlled_by",
                        what,
                    },
                ));
            };
            match accessible {
                Accessible::Command(_) => wrong("a readonly enum parameter"),
                Accessible::Parameter(p) => {
                    if p.props.get("readonly") == Some(&Json::Bool(false)) {
                        wrong("readonly");
                    }
                    match &p.datainfo {
                        DataInfo::Enum(e) if e.value_of("self") == Some(0) => {
                            let others = e.members().keys().filter(|n| *n != "self");
                            for other in others {
                                if !module_names.contains(&other.as_str()) {
                                    self.warnings.push(ModelWarning {
                                        path: path("controlled_by"),
                                        message: format!(
                                            "member `{other}` is no module of this node"
                                        ),
                                    });
                                }
                            }
                        }
                        DataInfo::Enum(_) => wrong("an enum with the member `self` = 0"),
                        _ => wrong("an enum of module names and `self` = 0"),
                    }
                }
            }
        }
        let parameter = |name| match param(name) {
            Some(Accessible::Parameter(p)) => Some(p),
            _ => None,
        };
        let checks = [
            (
                "control_active",
                "a readonly bool parameter",
                parameter("control_active").is_some_and(|p| {
                    matches!(p.datainfo, DataInfo::Bool(_))
                        && p.props.get("readonly") != Some(&Json::Bool(false))
                }),
            ),
            (
                "mode",
                "an enum parameter",
                parameter("mode").is_some_and(|p| matches!(p.datainfo, DataInfo::Enum(_))),
            ),
        ];
        for (name, what, fits) in checks {
            if param(name).is_some() && !fits {
                self.errors.push(ModelError::new(
                    path(name),
                    ModelErrorKind::PredefinedAccessible { name, what },
                ));
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

/// The table "Useful statuscodes" of the SECoP specification: code, name and whether it
/// is a group code.
const STATUS_CODES: [(i64, &str, bool); 18] = [
    (0, "DISABLED", true),
    (100, "IDLE", true),
    (130, "STANDBY", false),
    (150, "PREPARED", false),
    (200, "WARN", true),
    (230, "WARN_STANDBY", false),
    (250, "WARN_PREPARED", false),
    (300, "BUSY", true),
    (310, "DISABLING", false),
    (320, "INITIALIZING", false),
    (340, "PREPARING", false),
    (360, "STARTING", false),
    (370, "RAMPING", false),
    (380, "STABILIZING", false),
    (390, "FINALIZING", false),
    (400, "ERROR", true),
    (430, "ERROR_STANDBY", false),
    (450, "ERROR_PREPARED", false),
];
