// SPDX-License-Identifier: LGPL-3.0-or-later

//! The parsed config file: an ordered tree of values with their byte spans.

use std::ops::Range;

use serde_json::{Map, Value as Json};
use toml_edit::{Document, Item, Value};

/// A byte range in the config file.
pub type Span = Range<usize>;

/// A value with where it is written.
#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    /// The value.
    pub value: Val,
    /// Where the value is written.
    pub span: Span,
}

/// A TOML value.
#[derive(Debug, Clone, PartialEq)]
pub enum Val {
    /// A string.
    String(String),
    /// An integer.
    Integer(i64),
    /// A float.
    Float(f64),
    /// A boolean.
    Bool(bool),
    /// A date or time, as written.
    Datetime(String),
    /// An array (also an array of tables).
    Array(Vec<Node>),
    /// A table (also an inline table).
    Table(Table),
}

/// A table: its entries in file order.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Table {
    /// The entries.
    pub entries: Vec<Entry>,
}

/// An entry of a table.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    /// The key.
    pub key: String,
    /// Where the key is written.
    pub key_span: Span,
    /// The value.
    pub node: Node,
}

impl Table {
    /// The entry for `key`.
    pub fn get(&self, key: &str) -> Option<&Entry> {
        self.entries.iter().find(|e| e.key == key)
    }
}

impl Val {
    /// The TOML type name, for error messages.
    pub fn type_name(&self) -> &'static str {
        match self {
            Self::String(_) => "a string",
            Self::Integer(_) => "an integer",
            Self::Float(_) => "a float",
            Self::Bool(_) => "a boolean",
            Self::Datetime(_) => "a date-time",
            Self::Array(_) => "an array",
            Self::Table(_) => "a table",
        }
    }

    /// The value as JSON, as settings are passed to module types.
    pub fn to_json(&self) -> Json {
        match self {
            Self::String(s) | Self::Datetime(s) => Json::String(s.clone()),
            Self::Integer(i) => Json::from(*i),
            Self::Float(f) => serde_json::Number::from_f64(*f).map_or(Json::Null, Json::Number),
            Self::Bool(b) => Json::Bool(*b),
            Self::Array(items) => Json::Array(items.iter().map(|n| n.value.to_json()).collect()),
            Self::Table(t) => Json::Object(
                t.entries
                    .iter()
                    .map(|e| (e.key.clone(), e.node.value.to_json()))
                    .collect::<Map<_, _>>(),
            ),
        }
    }
}

/// Parses `text` into a tree; the error has the parser's message and span.
pub fn parse(text: &str) -> Result<Table, (String, Option<Span>)> {
    let doc = Document::parse(text).map_err(|e| (e.message().trim().to_owned(), e.span()))?;
    Ok(table(doc.as_table(), 0..0))
}

fn table(t: &toml_edit::Table, fallback: Span) -> Table {
    let mut entries = Vec::new();
    for (key, item) in t {
        let key_span = t
            .key(key)
            .and_then(toml_edit::Key::span)
            .unwrap_or_else(|| fallback.clone());
        if let Some(node) = item_node(item, &key_span) {
            entries.push(Entry {
                key: key.to_owned(),
                key_span,
                node,
            });
        }
    }
    Table { entries }
}

fn item_node(item: &Item, key_span: &Span) -> Option<Node> {
    match item {
        Item::None => None,
        Item::Value(v) => Some(value_node(v, key_span)),
        Item::Table(t) => {
            let span = t.span().unwrap_or_else(|| key_span.clone());
            Some(Node {
                value: Val::Table(table(t, span.clone())),
                span,
            })
        }
        Item::ArrayOfTables(a) => {
            let span = a.span().unwrap_or_else(|| key_span.clone());
            let items = a
                .iter()
                .map(|t| {
                    let s = t.span().unwrap_or_else(|| span.clone());
                    Node {
                        value: Val::Table(table(t, s.clone())),
                        span: s,
                    }
                })
                .collect();
            Some(Node {
                value: Val::Array(items),
                span,
            })
        }
    }
}

fn value_node(v: &Value, key_span: &Span) -> Node {
    let span = v.span().unwrap_or_else(|| key_span.clone());
    let value = match v {
        Value::String(s) => Val::String(s.value().clone()),
        Value::Integer(i) => Val::Integer(*i.value()),
        Value::Float(f) => Val::Float(*f.value()),
        Value::Boolean(b) => Val::Bool(*b.value()),
        Value::Datetime(d) => Val::Datetime(d.value().to_string()),
        Value::Array(a) => Val::Array(a.iter().map(|v| value_node(v, &span)).collect()),
        Value::InlineTable(t) => {
            let mut entries = Vec::new();
            for (key, v) in t {
                let key_span = t
                    .key(key)
                    .and_then(toml_edit::Key::span)
                    .unwrap_or_else(|| span.clone());
                entries.push(Entry {
                    key: key.to_owned(),
                    node: value_node(v, &key_span),
                    key_span,
                });
            }
            Val::Table(Table { entries })
        }
    };
    Node { value, span }
}

/// Maps byte offsets to 1-based lines and columns.
#[derive(Debug, Clone)]
pub struct Lines {
    starts: Vec<usize>,
    text: String,
}

impl Lines {
    /// The line starts of `text`.
    pub fn new(text: &str) -> Self {
        let mut starts = vec![0];
        starts.extend(text.match_indices('\n').map(|(i, _)| i + 1));
        Self {
            starts,
            text: text.to_owned(),
        }
    }

    /// Line and column of `offset`, both 1-based; columns count characters.
    pub fn locate(&self, offset: usize) -> (usize, usize) {
        let line = self.starts.partition_point(|s| *s <= offset).max(1);
        let start = self.starts[line - 1];
        let end = offset.min(self.text.len());
        let column = self
            .text
            .get(start..end)
            .map_or(1, |s| s.chars().count() + 1);
        (line, column)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn order_and_spans() {
        let text = "[node]\nequipment_id = \"x\"\n\n[modules.b]\ntype = \"t\"\n\n[modules.a]\ntype = \"t\"\n";
        let t = parse(text).unwrap();
        let Val::Table(modules) = &t.get("modules").unwrap().node.value else {
            panic!()
        };
        let keys: Vec<_> = modules.entries.iter().map(|e| e.key.as_str()).collect();
        assert_eq!(keys, ["b", "a"]);
        let lines = Lines::new(text);
        let id = t.get("node").unwrap();
        let Val::Table(node) = &id.node.value else {
            panic!()
        };
        let e = node.get("equipment_id").unwrap();
        assert_eq!(lines.locate(e.key_span.start), (2, 1));
        assert_eq!(lines.locate(e.node.span.start), (2, 16));
    }

    #[test]
    fn syntax_error() {
        let (message, span) = parse("[node]\nx = \n").unwrap_err();
        assert!(message.len() > 3, "{message}");
        assert_eq!(
            Lines::new("[node]\nx = \n").locate(span.unwrap().start).0,
            2
        );
    }
}
