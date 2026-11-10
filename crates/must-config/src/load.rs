// SPDX-License-Identifier: LGPL-3.0-or-later

//! Loading a node config: checks the whole file and builds the node.

use std::collections::HashMap;
use std::net::{SocketAddr, ToSocketAddrs};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};

use must_server::logfiles::LogFiles;
use must_server::{Discovery, Module, Node, NodeBuilder};
use must_types::validate_identifier;
use serde_json::Value as Json;

use crate::config::{ConfigError, ConfigErrors, Locator, ModuleConfig, Setting};
use crate::registry::{Registry, TypeEntry};
use crate::source::{self, Node as TomlNode, Span, Table, Val};
use crate::template::{self, Instance};

/// A loaded config: the node, ready to start, and what was built.
pub struct Loaded {
    /// The node.
    pub node: Node,
    /// The modules in file order, with their type names.
    pub modules: Vec<(String, String)>,
    /// Completion warnings, located at their modules.
    pub warnings: Vec<ConfigError>,
}

/// Loads the config file at `path`.
pub fn load_file(path: &Path, registry: &Registry) -> Result<Loaded, ConfigErrors> {
    let file = path.display().to_string();
    let text = std::fs::read_to_string(path).map_err(|e| {
        ConfigErrors(vec![ConfigError {
            file: file.clone(),
            line: 1,
            column: 1,
            module: None,
            origin: None,
            message: format!("cannot read the config file: {e}"),
        }])
    })?;
    load_str(&text, &file, path.parent(), registry)
}

/// Loads a config from `text`; `file` names it in errors, `base_dir` is where relative
/// paths in it are resolved (the config file's directory).
pub fn load_str(
    text: &str,
    file: &str,
    base_dir: Option<&Path>,
    registry: &Registry,
) -> Result<Loaded, ConfigErrors> {
    let locator = Locator::new(file, text);
    let mut loader = Loader {
        locator: &locator,
        errors: Vec::new(),
    };
    let doc = match source::parse(text) {
        Ok(doc) => doc,
        Err((message, span)) => {
            let span = span.unwrap_or(0..0);
            return Err(ConfigErrors(vec![locator.error(
                &span,
                None,
                None,
                format!("not valid TOML: {message}"),
            )]));
        }
    };
    match loader.load(&doc, base_dir, registry) {
        Some(config) if loader.errors.is_empty() => Ok(config),
        _ => {
            let mut errors = loader.errors;
            errors.sort_by_key(|e| (e.line, e.column));
            Err(ConfigErrors(errors))
        }
    }
}

/// The template and instance a module came from.
type Origin = Option<(String, usize)>;

struct Loader<'a> {
    locator: &'a Locator,
    errors: Vec<ConfigError>,
}

/// What a module table declares besides its type's settings.
struct Built {
    name: String,
    type_name: String,
    module: Module,
    table_span: Span,
    origin: Option<(String, usize)>,
}

fn as_str(node: &TomlNode) -> Option<&str> {
    match &node.value {
        Val::String(s) => Some(s),
        _ => None,
    }
}

impl Loader<'_> {
    fn error(&mut self, span: &Span, message: impl Into<String>) {
        self.errors
            .push(self.locator.error(span, None, None, message));
    }

    fn module_error(
        &mut self,
        span: &Span,
        module: &str,
        origin: Option<&(String, usize)>,
        message: impl Into<String>,
    ) {
        self.errors
            .push(self.locator.error(span, Some(module), origin, message));
    }

    fn load(
        &mut self,
        doc: &Table,
        base_dir: Option<&Path>,
        registry: &Registry,
    ) -> Option<Loaded> {
        let mut node_table = None;
        let mut modules_table = None;
        for e in &doc.entries {
            match (e.key.as_str(), &e.node.value) {
                ("node", Val::Table(t)) => node_table = Some((t, e.node.span.clone())),
                ("modules", Val::Table(t)) => modules_table = Some((t, e.node.span.clone())),
                ("node" | "modules", _) => self.error(&e.node.span, format!("`{}` must be a table", e.key)),
                (key, _) => self.error(
                    &e.key_span,
                    format!("unknown top-level key `{key}`; a node config has a `[node]` table and `[modules.<name>]` tables"),
                ),
            }
        }
        let builder = if let Some((t, span)) = node_table {
            self.node(t, &span, base_dir)
        } else {
            self.error(&(0..0), "the `[node]` table is missing");
            None
        };
        let built = match modules_table {
            Some((t, span)) if !t.entries.is_empty() => self.modules(t, &span, registry),
            Some((_, span)) => {
                self.error(&span, "the config declares no modules");
                Vec::new()
            }
            None => {
                self.error(
                    &(0..0),
                    "the config declares no modules: add `[modules.<name>]` tables",
                );
                Vec::new()
            }
        };
        let mut builder = builder?;
        let spans: HashMap<String, (Span, Option<(String, usize)>)> = built
            .iter()
            .map(|b| (b.name.clone(), (b.table_span.clone(), b.origin.clone())))
            .collect();
        let mut modules = Vec::new();
        for b in built {
            modules.push((b.name.clone(), b.type_name));
            match builder.module(b.module) {
                Ok(next) => builder = next,
                Err(e) => {
                    self.module_error(&b.table_span, &b.name, b.origin.as_ref(), e.to_string());
                    return None;
                }
            }
        }
        if !self.errors.is_empty() {
            return None;
        }
        let locate = |path: &str| {
            let module = path.split([':', '.']).next().unwrap_or_default();
            spans
                .get(module)
                .map_or((0..0, None, None), |(span, origin)| {
                    (span.clone(), Some(module.to_owned()), origin.clone())
                })
        };
        match builder.build() {
            Ok(node) => {
                let warnings = node
                    .warnings()
                    .iter()
                    .map(|w| {
                        let (span, module, origin) = locate(&w.path);
                        self.locator
                            .error(&span, module.as_deref(), origin.as_ref(), w.to_string())
                    })
                    .collect();
                Some(Loaded {
                    node,
                    modules,
                    warnings,
                })
            }
            Err(e) => {
                for error in e.0 {
                    let (span, module, origin) = locate(&error.path);
                    self.errors.push(self.locator.error(
                        &span,
                        module.as_deref(),
                        origin.as_ref(),
                        error.to_string(),
                    ));
                }
                None
            }
        }
    }

    /// The `[node]` table.
    #[allow(clippy::too_many_lines)]
    fn node(&mut self, t: &Table, span: &Span, base_dir: Option<&Path>) -> Option<NodeBuilder> {
        let string = |this: &mut Self, key: &str| -> Option<String> {
            let e = t.get(key)?;
            let s = as_str(&e.node).map(str::to_owned);
            if s.is_none() {
                this.error(&e.node.span, format!("`{key}` must be a string"));
            }
            s
        };
        let id = string(self, "equipment_id");
        let description = string(self, "description");
        if t.get("equipment_id").is_none() {
            self.error(span, "the node property `equipment_id` is missing");
        }
        if t.get("description").is_none() {
            self.error(span, "the node property `description` is missing");
        }
        let (Some(id), Some(description)) = (id, description) else {
            return None;
        };
        let mut b = NodeBuilder::new(&id, &description)
            .firmware(concat!("must-", env!("CARGO_PKG_VERSION")));
        for e in &t.entries {
            let key = e.key.as_str();
            match key {
                "equipment_id" | "description" => {}
                "firmware" => {
                    if let Some(s) = string(self, key) {
                        b = b.firmware(&s);
                    }
                }
                "implementor" => {
                    if let Some(s) = string(self, key) {
                        b = b.implementor(&s);
                    }
                }
                "timeout" => match e.node.value {
                    #[allow(clippy::cast_precision_loss)]
                    Val::Integer(i) if i > 0 => b = b.timeout(i as f64),
                    Val::Float(f) if f > 0.0 => b = b.timeout(f),
                    _ => self.error(
                        &e.node.span,
                        "`timeout` must be a positive number of seconds",
                    ),
                },
                "interface" => match as_str(&e.node).map(interface) {
                    Some(Ok(addr)) => b = b.bind(addr),
                    Some(Err(message)) => {
                        self.error(&e.node.span, format!("`interface`: {message}"));
                    }
                    None => self.error(
                        &e.node.span,
                        "`interface` must be a string such as \"tcp://2055\"",
                    ),
                },
                "discovery" => match &e.node.value {
                    Val::Bool(false) => b = b.discovery(Discovery::off()),
                    Val::Bool(true) => b = b.discovery(Discovery::default()),
                    Val::Table(d) => {
                        let mut discovery = Discovery::default();
                        for de in &d.entries {
                            match (de.key.as_str(), &de.node.value) {
                                ("port", Val::Integer(p)) if u16::try_from(*p).is_ok() => {
                                    discovery = discovery.port(u16::try_from(*p).unwrap_or_default());
                                }
                                ("announce", Val::Bool(a)) => discovery = discovery.announce(*a),
                                _ => self.error(&de.key_span, format!("`discovery.{}`: expected `port` (an integer) or `announce` (a boolean)", de.key)),
                            }
                        }
                        b = b.discovery(discovery);
                    }
                    _ => self.error(
                        &e.node.span,
                        "`discovery` must be `false` or a table with `port` and `announce`",
                    ),
                },
                "log_files" => match &e.node.value {
                    Val::Bool(false) => b = b.log_files(LogFiles::off()),
                    Val::Bool(true) => b = b.log_files(LogFiles::from_env()),
                    Val::Table(l) => {
                        let mut files = LogFiles::from_env();
                        files.enabled = true;
                        for le in &l.entries {
                            match (le.key.as_str(), &le.node.value) {
                                ("dir", Val::String(dir)) => {
                                    let dir = PathBuf::from(dir);
                                    let dir = match base_dir {
                                        Some(base) if dir.is_relative() => base.join(dir),
                                        _ => dir,
                                    };
                                    files = files.dir(dir);
                                }
                                ("days", Val::Integer(d)) if *d >= 1 => {
                                    files = files.days(u32::try_from(*d).unwrap_or(u32::MAX));
                                }
                                _ => self.error(&le.key_span, format!("`log_files.{}`: expected `dir` (a string) or `days` (a positive integer)", le.key)),
                            }
                        }
                        b = b.log_files(files);
                    }
                    _ => self.error(
                        &e.node.span,
                        "`log_files` must be `false` or a table with `dir` and `days`",
                    ),
                },
                _ if key.starts_with('_') => match b.property(key, e.node.value.to_json()) {
                    Ok(next) => b = next,
                    Err(err) => {
                        self.error(&e.key_span, err.to_string());
                        return None;
                    }
                },
                _ => self.error(
                    &e.key_span,
                    format!("unknown node property `{key}`; custom properties start with `_`"),
                ),
            }
        }
        Some(b)
    }

    /// The `[modules]` tables: expands templates, checks names and builds the modules.
    fn modules(&mut self, t: &Table, span: &Span, registry: &Registry) -> Vec<Built> {
        let _ = span;
        let mut instances: Vec<Instance> = Vec::new();
        for e in &t.entries {
            let Val::Table(table) = &e.node.value else {
                self.error(
                    &e.node.span,
                    format!("the module `{}` must be a table", e.key),
                );
                continue;
            };
            match template::expand(&e.key, &e.key_span, table, &e.node.span) {
                Ok(list) => instances.extend(list),
                Err(errors) => {
                    for err in errors {
                        self.error(&err.span, err.message);
                    }
                }
            }
        }
        // names: valid identifiers, unique ignoring case
        let mut seen: HashMap<String, (String, Span, Origin)> = HashMap::new();
        let mut valid = Vec::new();
        for inst in instances {
            if let Err(e) = validate_identifier(&inst.name) {
                self.module_error(
                    &inst.name_span,
                    &inst.name,
                    inst.origin.as_ref(),
                    format!("`{}` is not a valid SECoP identifier: {e}", inst.name),
                );
                continue;
            }
            let lower = inst.name.to_lowercase();
            if let Some((other, other_span, other_origin)) = seen.get(&lower) {
                let (line, _) = self.locator.lines.locate(other_span.start);
                let declared = match other_origin {
                    Some((template, n)) => {
                        format!("instance {n} of the template `{template}` on line {line}")
                    }
                    None => format!("the module table `{other}` on line {line}"),
                };
                self.module_error(
                    &inst.table_span,
                    &inst.name,
                    inst.origin.as_ref(),
                    format!(
                        "the module name `{}` is also declared by {declared}",
                        inst.name
                    ),
                );
                continue;
            }
            seen.insert(
                lower,
                (
                    inst.name.clone(),
                    inst.table_span.clone(),
                    inst.origin.clone(),
                ),
            );
            valid.push(inst);
        }
        valid
            .into_iter()
            .filter_map(|inst| self.build(inst, registry))
            .collect()
    }

    /// Builds one module.
    #[allow(clippy::too_many_lines)]
    fn build(&mut self, inst: Instance, registry: &Registry) -> Option<Built> {
        let Instance {
            name,
            table_span,
            settings,
            vars,
            origin,
            ..
        } = inst;
        let mut type_setting = None;
        let mut description = None;
        let mut group = None;
        let mut visibility = None;
        let mut custom: Vec<(String, Json, Span)> = Vec::new();
        let mut rest = Vec::new();
        for s in settings {
            match s.key.as_str() {
                "type" => type_setting = Some(s),
                "description" | "group" | "visibility" => match as_str(&s.node) {
                    Some(v) => {
                        let v = v.to_owned();
                        match s.key.as_str() {
                            "description" => description = Some(v),
                            "group" => group = Some(v),
                            _ => visibility = Some(v),
                        }
                    }
                    None => self.module_error(
                        &s.node.span,
                        &name,
                        origin.as_ref(),
                        format!("`{}` must be a string", s.key),
                    ),
                },
                k if k.starts_with('_') => {
                    custom.push((s.key.clone(), s.node.value.to_json(), s.key_span.clone()));
                }
                _ => rest.push(s),
            }
        }
        let Some(type_setting) = type_setting else {
            self.module_error(
                &table_span,
                &name,
                origin.as_ref(),
                "the module has no `type`",
            );
            return None;
        };
        let Some(type_name) = as_str(&type_setting.node).map(str::to_owned) else {
            self.module_error(
                &type_setting.node.span,
                &name,
                origin.as_ref(),
                "`type` must be a string",
            );
            return None;
        };
        let Some(entry) = registry.get(&type_name) else {
            self.module_error(
                &type_setting.node.span,
                &name,
                origin.as_ref(),
                format!(
                    "unknown module type `{type_name}`; registered types: {}",
                    registry.names().join(", ")
                ),
            );
            return None;
        };
        let mut config = ModuleConfig {
            name: name.clone(),
            description: description.clone(),
            settings: rest.into_iter().map(|s: Setting| (s, false)).collect(),
            vars,
            table_span: table_span.clone(),
            origin: origin.clone(),
            locator: self.locator,
        };
        let outcome = catch_unwind(AssertUnwindSafe(|| match entry {
            TypeEntry::Typed {
                build,
                default_description,
            } => {
                let description = description.as_deref().or(*default_description);
                build(&name, description, &mut config).map_err(|errors| {
                    errors
                        .0
                        .iter()
                        .map(|e| {
                            let mut located = config.locate(e);
                            if located.message.contains("unknown setting") {
                                located.message =
                                    format!("{} of the module type `{type_name}`", located.message);
                            }
                            located
                        })
                        .collect::<Vec<_>>()
                })
            }
            TypeEntry::Factory(factory) => {
                let module = factory(&mut config).map_err(|e| vec![e])?;
                let unread: Vec<_> = config
                    .unread()
                    .into_iter()
                    .map(|s| {
                        config.error(
                            &s.key,
                            format!("unknown setting for the module type `{type_name}`"),
                        )
                    })
                    .collect();
                if unread.is_empty() {
                    Ok(module)
                } else {
                    Err(unread)
                }
            }
        }));
        let mut module = match outcome {
            Ok(Ok(module)) => module,
            Ok(Err(errors)) => {
                self.errors.extend(errors);
                return None;
            }
            Err(panic) => {
                let message = panic
                    .downcast_ref::<String>()
                    .map(String::as_str)
                    .or_else(|| panic.downcast_ref::<&str>().copied())
                    .unwrap_or("a panic");
                self.module_error(
                    &table_span,
                    &name,
                    origin.as_ref(),
                    format!("the module type `{type_name}` panicked: {message}"),
                );
                return None;
            }
        };
        if let Some(g) = &group {
            module = module.group(g);
        }
        if let Some(v) = &visibility {
            module = module.visibility(v);
        }
        for (key, json, span) in custom {
            match module.property(&key, json) {
                Ok(m) => module = m,
                Err(e) => {
                    self.module_error(&span, &name, origin.as_ref(), e.to_string());
                    return None;
                }
            }
        }
        Some(Built {
            name,
            type_name,
            module,
            table_span,
            origin,
        })
    }
}

/// The address of an `interface`: `tcp://<port>` (all interfaces) or
/// `tcp://<host>:<port>`.
fn interface(text: &str) -> Result<SocketAddr, String> {
    let Some(rest) = text.strip_prefix("tcp://") else {
        return Err(format!(
            "`{text}` is not supported; use `tcp://<port>` or `tcp://<host>:<port>`"
        ));
    };
    if let Ok(port) = rest.parse::<u16>() {
        return Ok(SocketAddr::from(([0, 0, 0, 0], port)));
    }
    rest.to_socket_addrs()
        .map_err(|e| format!("`{text}`: {e}"))?
        .next()
        .ok_or_else(|| format!("`{text}` resolves to no address"))
}

#[cfg(test)]
mod tests {
    use super::interface;

    #[test]
    fn interfaces() {
        assert_eq!(
            interface("tcp://10767").unwrap().to_string(),
            "0.0.0.0:10767"
        );
        assert_eq!(
            interface("tcp://127.0.0.1:2056").unwrap().to_string(),
            "127.0.0.1:2056"
        );
        assert!(interface("serial:///dev/ttyS0").is_err());
    }
}
