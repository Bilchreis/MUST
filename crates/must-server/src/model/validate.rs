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
