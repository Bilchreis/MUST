// SPDX-License-Identifier: LGPL-3.0-or-later

//! Module templates: a module table whose key contains a placeholder expands into one
//! module per instance (`count = N` or `instances = [...]`), with placeholders replaced
//! in the module name and in the template's string settings.

use crate::config::Setting;
use crate::source::{Entry, Node, Span, Table, Val};

/// The keys of a template that are not settings.
const TEMPLATE_KEYS: &[&str] = &["count", "instances", "first"];

/// One module declared by a module table: directly, or as an instance of a template.
#[derive(Debug, Clone)]
pub(crate) struct Instance {
    pub name: String,
    pub name_span: Span,
    pub table_span: Span,
    pub settings: Vec<Setting>,
    /// Instance keys used as placeholders.
    pub vars: Vec<String>,
    /// The template key and the 1-based instance number.
    pub origin: Option<(String, usize)>,
}

/// A problem with a template, at a span.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct TemplateError {
    pub span: Span,
    pub message: String,
}

fn error(span: &Span, message: impl Into<String>) -> TemplateError {
    TemplateError {
        span: span.clone(),
        message: message.into(),
    }
}

/// A piece of a string with placeholders.
#[derive(Debug, Clone, PartialEq)]
enum Piece {
    Text(String),
    Var(String),
}

/// Splits `s` into text and placeholders; `{{` and `}}` stand for braces.
fn pieces(s: &str) -> Result<Vec<Piece>, String> {
    let mut out = Vec::new();
    let mut text = String::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '{' if chars.peek() == Some(&'{') => {
                chars.next();
                text.push('{');
            }
            '}' if chars.peek() == Some(&'}') => {
                chars.next();
                text.push('}');
            }
            '{' => {
                let mut name = String::new();
                loop {
                    match chars.next() {
                        Some('}') => break,
                        Some(c) => name.push(c),
                        None => return Err(format!("unclosed placeholder in `{s}`")),
                    }
                }
                let valid =
                    !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
                if !valid {
                    return Err(format!(
                        "`{{{name}}}` in `{s}` is not a placeholder; write `{{{{` and `}}}}` for braces"
                    ));
                }
                if !text.is_empty() {
                    out.push(Piece::Text(std::mem::take(&mut text)));
                }
                out.push(Piece::Var(name));
            }
            '}' => return Err(format!("unmatched `}}` in `{s}`; write `}}}}` for a brace")),
            c => text.push(c),
        }
    }
    if !text.is_empty() {
        out.push(Piece::Text(text));
    }
    Ok(out)
}

/// Whether a module key is a template: it has a placeholder.
pub(crate) fn is_template(key: &str) -> bool {
    pieces(key).is_ok_and(|p| p.iter().any(|p| matches!(p, Piece::Var(_))))
        || pieces(key).is_err() && key.contains('{')
}

/// The variables of one instance.
struct Vars<'a> {
    values: Vec<(&'a str, Node)>,
}

impl Vars<'_> {
    fn get(&self, name: &str) -> Option<&Node> {
        self.values.iter().find(|(n, _)| *n == name).map(|(_, v)| v)
    }

    fn names(&self) -> String {
        self.values
            .iter()
            .map(|(n, _)| format!("`{n}`"))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// The text a variable inserts into a longer string.
fn text_of(name: &str, node: &Node) -> Result<String, String> {
    Ok(match &node.value {
        Val::String(s) | Val::Datetime(s) => s.clone(),
        Val::Integer(i) => i.to_string(),
        Val::Float(f) => format!("{f:?}"),
        Val::Bool(b) => b.to_string(),
        Val::Array(_) | Val::Table(_) => {
            return Err(format!(
                "the placeholder `{{{name}}}` is {} and cannot be part of a longer string",
                node.value.type_name()
            ));
        }
    })
}

/// Replaces the placeholders of `s`; a string that is exactly one placeholder becomes the
/// variable's value with its own type.
fn substitute(
    s: &str,
    span: &Span,
    vars: &Vars,
    used: &mut Vec<String>,
) -> Result<Val, TemplateError> {
    let pieces = pieces(s).map_err(|e| error(span, e))?;
    let lookup = |name: &str| {
        vars.get(name).ok_or_else(|| {
            error(
                span,
                format!(
                    "unknown placeholder `{{{name}}}`; this template has {}",
                    vars.names()
                ),
            )
        })
    };
    if let [Piece::Var(name)] = pieces.as_slice() {
        used.push(name.clone());
        return Ok(lookup(name)?.value.clone());
    }
    let mut out = String::new();
    for piece in pieces {
        match piece {
            Piece::Text(t) => out.push_str(&t),
            Piece::Var(name) => {
                used.push(name.clone());
                out.push_str(&text_of(&name, lookup(&name)?).map_err(|e| error(span, e))?);
            }
        }
    }
    Ok(Val::String(out))
}

/// Substitutes every string in `node`, recursively.
fn substitute_node(
    node: &Node,
    vars: &Vars,
    used: &mut Vec<String>,
) -> Result<Node, TemplateError> {
    let value = match &node.value {
        Val::String(s) => substitute(s, &node.span, vars, used)?,
        Val::Array(items) => Val::Array(
            items
                .iter()
                .map(|n| substitute_node(n, vars, used))
                .collect::<Result<_, _>>()?,
        ),
        Val::Table(t) => Val::Table(Table {
            entries: t
                .entries
                .iter()
                .map(|e| {
                    Ok(Entry {
                        key: e.key.clone(),
                        key_span: e.key_span.clone(),
                        node: substitute_node(&e.node, vars, used)?,
                    })
                })
                .collect::<Result<_, TemplateError>>()?,
        }),
        other => other.clone(),
    };
    Ok(Node {
        value,
        span: node.span.clone(),
    })
}

fn setting(e: &Entry) -> Setting {
    Setting {
        key: e.key.clone(),
        key_span: e.key_span.clone(),
        node: e.node.clone(),
    }
}

#[allow(clippy::too_many_lines)]
/// Expands the module table `key = table` into its modules: one for a plain table, one
/// per instance for a template.
pub(crate) fn expand(
    key: &str,
    key_span: &Span,
    table: &Table,
    table_span: &Span,
) -> Result<Vec<Instance>, Vec<TemplateError>> {
    if !is_template(key) {
        let errors: Vec<_> = TEMPLATE_KEYS
            .iter()
            .filter_map(|k| table.get(k))
            .map(|e| {
                error(
                    &e.key_span,
                    format!(
                        "`{}` is only for templates: a module key with a placeholder such as `\"{key}{{i}}\"`",
                        e.key
                    ),
                )
            })
            .collect();
        if !errors.is_empty() {
            return Err(errors);
        }
        return Ok(vec![Instance {
            name: key.to_owned(),
            name_span: key_span.clone(),
            table_span: table_span.clone(),
            settings: table.entries.iter().map(setting).collect(),
            vars: Vec::new(),
            origin: None,
        }]);
    }
    if let Err(e) = pieces(key) {
        return Err(vec![error(key_span, e)]);
    }

    let mut errors = Vec::new();
    let first = match table.get("first").map(|e| &e.node) {
        None => 1,
        Some(Node {
            value: Val::Integer(i),
            ..
        }) => *i,
        Some(n) => {
            errors.push(error(&n.span, "`first` must be an integer"));
            1
        }
    };
    // the instance tables, each with the span the instance is declared at
    let instances: Vec<(Table, Span)> = match (table.get("count"), table.get("instances")) {
        (Some(_), Some(_)) | (None, None) => {
            errors.push(error(
                key_span,
                format!(
                    "the template `{key}` needs exactly one of `count = N` or `instances = [...]`"
                ),
            ));
            Vec::new()
        }
        (Some(count), None) => match &count.node.value {
            Val::Integer(n) if *n >= 1 => (0..*n)
                .map(|_| (Table::default(), count.node.span.clone()))
                .collect(),
            _ => {
                errors.push(error(
                    &count.node.span,
                    "`count` must be an integer of at least 1",
                ));
                Vec::new()
            }
        },
        (None, Some(list)) => match &list.node.value {
            Val::Array(items) if !items.is_empty() => items
                .iter()
                .filter_map(|n| {
                    if let Val::Table(t) = &n.value {
                        Some((t.clone(), n.span.clone()))
                    } else {
                        errors.push(error(
                            &n.span,
                            "each instance must be a table, e.g. `{ gas = \"N2\" }`",
                        ));
                        None
                    }
                })
                .collect(),
            _ => {
                errors.push(error(
                    &list.node.span,
                    "`instances` must be a non-empty array of tables",
                ));
                Vec::new()
            }
        },
    };
    if !errors.is_empty() {
        return Err(errors);
    }

    let base: Vec<&Entry> = table
        .entries
        .iter()
        .filter(|e| !TEMPLATE_KEYS.contains(&e.key.as_str()))
        .collect();
    let mut out = Vec::new();
    for (k, (inst, inst_span)) in instances.iter().enumerate() {
        let number = k + 1;
        let origin = Some((key.to_owned(), number));
        for e in &inst.entries {
            if e.key == "type" || TEMPLATE_KEYS.contains(&e.key.as_str()) {
                errors.push(error(
                    &e.key_span,
                    format!("instance {number} of `{key}` cannot set `{}`", e.key),
                ));
            }
        }
        let i = first + i64::try_from(k).unwrap_or(i64::MAX);
        let mut values = vec![(
            "i",
            Node {
                value: Val::Integer(i),
                span: inst_span.clone(),
            },
        )];
        values.extend(
            inst.entries
                .iter()
                .map(|e| (e.key.as_str(), e.node.clone())),
        );
        let vars = Vars { values };
        let mut used = Vec::new();
        let name = match substitute(key, key_span, &vars, &mut used) {
            Ok(Val::String(s)) => s,
            Ok(v) => match text_of(
                key,
                &Node {
                    value: v,
                    span: key_span.clone(),
                },
            ) {
                Ok(s) => s,
                Err(e) => {
                    errors.push(error(key_span, e));
                    continue;
                }
            },
            Err(e) => {
                errors.push(e);
                continue;
            }
        };
        let mut settings = Vec::new();
        for e in &base {
            if inst.get(&e.key).is_some() {
                continue;
            }
            match substitute_node(&e.node, &vars, &mut used) {
                Ok(node) => settings.push(Setting {
                    key: e.key.clone(),
                    key_span: e.key_span.clone(),
                    node,
                }),
                Err(err) => errors.push(err),
            }
        }
        settings.extend(inst.entries.iter().map(setting));
        let vars_used: Vec<String> = inst
            .entries
            .iter()
            .filter(|e| used.contains(&e.key))
            .map(|e| e.key.clone())
            .collect();
        out.push(Instance {
            name,
            name_span: key_span.clone(),
            table_span: inst_span.clone(),
            settings,
            vars: vars_used,
            origin,
        });
    }
    if errors.is_empty() {
        Ok(out)
    } else {
        Err(errors)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source;

    /// Expands the single module of `text` (a `[modules]` table).
    fn expand_text(text: &str) -> Result<Vec<Instance>, Vec<TemplateError>> {
        let doc = source::parse(text).unwrap();
        let Val::Table(modules) = &doc.get("modules").unwrap().node.value else {
            panic!("no modules")
        };
        let e = &modules.entries[0];
        let Val::Table(t) = &e.node.value else {
            panic!("not a table")
        };
        expand(&e.key, &e.key_span, t, &e.node.span)
    }

    fn get<'a>(i: &'a Instance, key: &str) -> &'a Val {
        &i.settings.iter().find(|s| s.key == key).unwrap().node.value
    }

    fn names(instances: &[Instance]) -> Vec<&str> {
        instances.iter().map(|i| i.name.as_str()).collect()
    }

    fn message(r: Result<Vec<Instance>, Vec<TemplateError>>) -> String {
        r.unwrap_err()
            .into_iter()
            .map(|e| e.message)
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn five_identical_flow_controllers() {
        let i = expand_text(
            "[modules.\"mfc{i}\"]\ntype = \"must::sim::FlowController\"\ncount = 5\nrange = 100.0\n",
        )
        .unwrap();
        assert_eq!(names(&i), ["mfc1", "mfc2", "mfc3", "mfc4", "mfc5"]);
        assert!(i.iter().all(|m| *get(m, "range") == Val::Float(100.0)));
        assert!(
            i.iter()
                .all(|m| m.settings.iter().all(|s| s.key != "count"))
        );
        assert_eq!(i[2].origin, Some(("mfc{i}".to_owned(), 3)));
    }

    #[test]
    fn numbering_from_another_start() {
        let i =
            expand_text("[modules.\"valve{i}\"]\ntype = \"t\"\ncount = 3\nfirst = 0\n").unwrap();
        assert_eq!(names(&i), ["valve0", "valve1", "valve2"]);
    }

    #[test]
    fn template_without_count_or_instances() {
        let m = message(expand_text("[modules.\"mfc{i}\"]\ntype = \"t\"\n"));
        assert!(m.contains("mfc{i}") && m.contains("exactly one"), "{m}");
    }

    #[test]
    fn both_count_and_instances() {
        let m = message(expand_text(
            "[modules.\"mfc{i}\"]\ntype = \"t\"\ncount = 2\ninstances = [{}, {}]\n",
        ));
        assert!(m.contains("mfc{i}"), "{m}");
    }

    #[test]
    fn count_on_a_plain_module() {
        let m = message(expand_text("[modules.mfc]\ntype = \"t\"\ncount = 2\n"));
        assert!(m.contains("`count` is only for templates"), "{m}");
    }

    #[test]
    fn per_instance_settings() {
        let i = expand_text(
            "[modules.\"mfc_{gas}\"]\ntype = \"t\"\nrange = 100.0\ninstances = [{ gas = \"N2\", range = 500.0 }, { gas = \"Ar\" }]\n",
        )
        .unwrap();
        assert_eq!(names(&i), ["mfc_N2", "mfc_Ar"]);
        assert_eq!(*get(&i[0], "range"), Val::Float(500.0));
        assert_eq!(*get(&i[1], "range"), Val::Float(100.0));
        // `gas` is only a placeholder here: a template variable
        assert_eq!(i[0].vars, ["gas"]);
    }

    #[test]
    fn instance_changes_the_type() {
        let m = message(expand_text(
            "[modules.\"m{i}\"]\ntype = \"t\"\ninstances = [{ type = \"must::sim::Temperature\" }]\n",
        ));
        assert!(m.contains("instance 1") && m.contains("`type`"), "{m}");
    }

    #[test]
    fn description_from_instance_values() {
        let i = expand_text(
            "[modules.\"mfc{i}\"]\ntype = \"t\"\ndescription = \"{gas} flow, controller {i}\"\ninstances = [{ gas = \"N2\" }, { gas = \"Ar\" }]\n",
        )
        .unwrap();
        assert_eq!(
            *get(&i[1], "description"),
            Val::String("Ar flow, controller 2".into())
        );
    }

    #[test]
    fn unknown_placeholder() {
        let m = message(expand_text(
            "[modules.\"mfc{n}\"]\ntype = \"t\"\ncount = 2\n",
        ));
        assert!(m.contains("`{n}`"), "{m}");
    }

    #[test]
    fn literal_braces() {
        let i = expand_text("[modules.\"m{i}\"]\ntype = \"t\"\ncount = 1\n_fmt = \"{{value}}\"\n")
            .unwrap();
        assert_eq!(*get(&i[0], "_fmt"), Val::String("{value}".into()));
    }

    #[test]
    fn instance_values_are_not_substituted() {
        let i = expand_text("[modules.\"m{i}\"]\ntype = \"t\"\ninstances = [{ note = \"{i}\" }]\n")
            .unwrap();
        assert_eq!(*get(&i[0], "note"), Val::String("{i}".into()));
    }

    #[test]
    fn bus_address_from_the_instance_number() {
        let i = expand_text(
            "[modules.\"m{i}\"]\ntype = \"t\"\ncount = 3\nfirst = 4\naddress = \"{i}\"\n",
        )
        .unwrap();
        let addresses: Vec<_> = i.iter().map(|m| get(m, "address").clone()).collect();
        assert_eq!(
            addresses,
            [Val::Integer(4), Val::Integer(5), Val::Integer(6)]
        );
    }

    #[test]
    fn range_from_an_instance_value() {
        let i = expand_text(
            "[modules.\"m{i}\"]\ntype = \"t\"\nrange = \"{max}\"\ninstances = [{ max = 50.0 }]\n",
        )
        .unwrap();
        assert_eq!(*get(&i[0], "range"), Val::Float(50.0));
    }

    #[test]
    fn values_inserted_as_text() {
        let i = expand_text(
            "[modules.\"m{i}\"]\ntype = \"t\"\nlabel = \"{i}: {x} {on} {g}\"\ninstances = [{ x = 2.5, on = true, g = \"N2\" }]\n",
        )
        .unwrap();
        assert_eq!(*get(&i[0], "label"), Val::String("1: 2.5 true N2".into()));
    }

    #[test]
    fn array_in_a_longer_string() {
        let m = message(expand_text(
            "[modules.\"m{i}\"]\ntype = \"t\"\nlabel = \"x {a}\"\ninstances = [{ a = [1, 2] }]\n",
        ));
        assert!(m.contains("`{a}`") && m.contains("longer string"), "{m}");
    }

    #[test]
    fn plain_modules_never_substitute() {
        let i = expand_text("[modules.m]\ntype = \"t\"\n_fmt = \"{{x}}\"\n").unwrap();
        assert_eq!(*get(&i[0], "_fmt"), Val::String("{{x}}".into()));
    }
}
