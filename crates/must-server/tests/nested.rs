// SPDX-License-Identifier: LGPL-3.0-or-later

//! End-to-end tests of parameters and commands with deeply nested datainfos (ragged
//! arrays, arrays of structs, structs of tuples, ...) over loopback TCP: they are
//! described, read, changed, pushed and called like simple ones, and a bad value deep
//! inside a request is rejected with the path to the offending element.

mod common;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Mutex;

use async_trait::async_trait;
use common::{Client, di, error_class, parse, status, status_info};
use must_server::logfiles::LogFiles;
use must_server::{
    CallResult, Command, Ctx, HandlerResult, Module, ModuleHandler, NodeBuilder, Parameter,
    RunningNode,
};
use must_types::{DataInfo, ErrorClass, Mode, SecopError, Value};
use serde_json::{Value as Json, json};
use tokio::runtime::Handle;

fn metres() -> Json {
    json!({"type": "double", "min": 0.0, "max": 100.0, "unit": "m"})
}

/// `array<array<double, 1..3>, 0..4>`
fn ragged() -> Json {
    json!({"type": "array", "minlen": 0, "maxlen": 4,
           "members": {"type": "array", "minlen": 1, "maxlen": 3, "members": {"type": "double"}}})
}

/// `struct{name, points: array<struct{x, y, tags: array<string>}, 1..5>}`
fn path_info() -> Json {
    json!({"type": "struct", "members": {
        "name": {"type": "string", "maxchars": 20},
        "points": {"type": "array", "minlen": 1, "maxlen": 5, "members": {"type": "struct", "members": {
            "x": metres(), "y": metres(),
            "tags": {"type": "array", "maxlen": 2, "members": {"type": "string", "maxchars": 8}}}}}}})
}

/// `tuple<array<double>, struct{limits: tuple<double, double>, mode: enum}>`
fn tuple_info() -> Json {
    json!({"type": "tuple", "members": [
        {"type": "array", "maxlen": 50, "members": {"type": "double"}},
        {"type": "struct", "members": {
            "limits": {"type": "tuple", "members": [metres(), metres()]},
            "mode": {"type": "enum", "members": {"off": 0, "on": 1}}}}]})
}

/// `array<struct{x, t?}>` with an optional member.
fn optional_info() -> Json {
    json!({"type": "array", "maxlen": 3, "members": {"type": "struct",
           "members": {"x": metres(), "t": {"type": "double"}}, "optional": ["t"]}})
}

#[allow(clippy::needless_pass_by_value)]
fn initial(info: &Json, json: Json) -> Value {
    DataInfo::from_json(info)
        .unwrap()
        .decode(&json, Mode::Report)
        .unwrap()
}

/// Keeps the values set, echoes command arguments.
#[derive(Default)]
struct Store(Mutex<HashMap<String, Value>>);

#[async_trait]
impl ModuleHandler for Store {
    async fn read(&self, _ctx: &Ctx, parameter: &str) -> HandlerResult {
        self.0
            .lock()
            .unwrap()
            .get(parameter)
            .cloned()
            .map(Into::into)
            .ok_or_else(|| SecopError::new(ErrorClass::ReadFailed, "never set"))
    }

    async fn change(&self, _ctx: &Ctx, parameter: &str, value: Value) -> HandlerResult {
        self.0
            .lock()
            .unwrap()
            .insert(parameter.to_owned(), value.clone());
        Ok(value.into())
    }

    async fn call(&self, _ctx: &Ctx, _command: &str, argument: Option<Value>) -> CallResult {
        Ok(argument.map(Into::into))
    }
}

fn start() -> RunningNode {
    let echo = DataInfo::command()
        .argument(&di(path_info()))
        .result(&di(path_info()))
        .build()
        .unwrap();
    let module = Module::new("geo", "nested parameters")
        .interface_classes(["Readable"])
        .parameter(
            Parameter::new("value", di(ragged()), "a ragged 2d array")
                .cached()
                .initial_value(initial(&ragged(), json!([[1.0], [2.0, 3.0]]))),
        )
        .unwrap()
        .parameter(
            Parameter::new("status", status_info(), "status")
                .cached()
                .initial_value(status(100, "idle")),
        )
        .unwrap()
        .parameter(
            Parameter::new("path", di(path_info()), "struct with an array of structs").writable(),
        )
        .unwrap()
        .parameter(
            Parameter::new("pair", di(tuple_info()), "tuple of an array and a struct").writable(),
        )
        .unwrap()
        .parameter(Parameter::new("ragged", di(ragged()), "writable ragged array").writable())
        .unwrap()
        .parameter(Parameter::new("partial", di(optional_info()), "optional members").writable())
        .unwrap()
        .command(Command::new("echo", echo, "returns its argument"))
        .unwrap()
        .no_polling()
        .handler(Store::default());
    NodeBuilder::new("must-nested", "nested datainfos")
        .bind(SocketAddr::from(([127, 0, 0, 1], 0)))
        .initial_read(false)
        .log_files(LogFiles::off())
        .module(module)
        .unwrap()
        .build()
        .unwrap()
        .spawn(&Handle::current())
        .unwrap()
}

/// Sends `change <specifier> <value>` and returns the value of the `changed` reply.
async fn change(c: &mut Client, specifier: &str, value: &Json) -> Json {
    let line = c.request(&format!("change {specifier} {value}")).await;
    let (action, spec, data) = parse(&line);
    assert_eq!(
        (action.as_str(), spec.as_str()),
        ("changed", specifier),
        "{line}"
    );
    data[0].clone()
}

#[tokio::test]
async fn nested_datainfos_are_described_unchanged() {
    let n = start();
    let mut c = Client::connect(n.local_addr()).await;
    let (_, _, report) = parse(&c.request("describe").await);
    let accessibles = &report["modules"]["geo"]["accessibles"];
    assert_eq!(accessibles["value"]["datainfo"], ragged());
    assert_eq!(accessibles["path"]["datainfo"], path_info());
    assert_eq!(accessibles["pair"]["datainfo"], tuple_info());
    assert_eq!(accessibles["partial"]["datainfo"], optional_info());
    assert_eq!(accessibles["echo"]["datainfo"]["argument"], path_info());
}

#[tokio::test]
async fn nested_values_are_changed_and_read_back() {
    let n = start();
    let mut c = Client::connect(n.local_addr()).await;

    let (_, _, data) = parse(&c.request("read geo:value").await);
    assert_eq!(data[0], json!([[1.0], [2.0, 3.0]]));

    let path = json!({"name": "loop", "points": [
        {"x": 0.0, "y": 0.0, "tags": []},
        {"x": 1.5, "y": 2.5, "tags": ["start", "fast"]}]});
    assert_eq!(change(&mut c, "geo:path", &path).await, path);
    let (_, _, data) = parse(&c.request("read geo:path").await);
    assert_eq!(data[0], path);

    let pair = json!([[1.0, 2.0, 3.0], {"limits": [0.0, 50.0], "mode": 1}]);
    assert_eq!(change(&mut c, "geo:pair", &pair).await, pair);

    // inner arrays of different lengths, and an empty outer one
    for ragged in [json!([[1.0], [1.0, 2.0, 3.0], [4.0, 5.0]]), json!([])] {
        assert_eq!(change(&mut c, "geo:ragged", &ragged).await, ragged);
    }

    let line = c.request(&format!("do geo:echo {path}")).await;
    let (action, _, data) = parse(&line);
    assert_eq!((action.as_str(), &data[0]), ("done", &path), "{line}");
}

#[tokio::test]
async fn bad_values_deep_inside_are_rejected_with_their_path() {
    let n = start();
    let mut c = Client::connect(n.local_addr()).await;
    let point = |x: f64, tags: Json| json!({"x": x, "y": 0.0, "tags": tags});
    let cases = [
        // an inner array of a ragged array below its minlen, above its maxlen
        ("geo:ragged", json!([[1.0], []]), "RangeError", "[1]"),
        (
            "geo:ragged",
            json!([[1.0, 2.0, 3.0, 4.0]]),
            "RangeError",
            "[0]",
        ),
        (
            "geo:ragged",
            json!([[1.0], [1.0], [1.0], [1.0], [1.0]]),
            "RangeError",
            "",
        ),
        ("geo:ragged", json!([[1.0], ["x"]]), "WrongType", "[1][0]"),
        // a struct in an array in a struct
        (
            "geo:path",
            json!({"name": "p", "points": [point(1.0, json!([])), point(101.0, json!([]))]}),
            "RangeError",
            "points[1].x",
        ),
        (
            "geo:path",
            json!({"name": "p", "points": [point(1.0, json!(["a", "b", "c"]))]}),
            "RangeError",
            "points[0].tags",
        ),
        (
            "geo:path",
            json!({"name": "p", "points": [point(1.0, json!(["too long a tag"]))]}),
            "RangeError",
            "points[0].tags[0]",
        ),
        (
            "geo:path",
            json!({"name": "p", "points": [{"x": 1.0, "tags": []}]}),
            "WrongType",
            "points[0]",
        ),
        (
            "geo:path",
            json!({"name": "p", "points": []}),
            "RangeError",
            "points",
        ),
        // a struct of tuples in a tuple
        (
            "geo:pair",
            json!([[], {"limits": [0.0, 200.0], "mode": 0}]),
            "RangeError",
            "[1].limits[1]",
        ),
        (
            "geo:pair",
            json!([[], {"limits": [0.0, 1.0], "mode": 2}]),
            "RangeError",
            "[1].mode",
        ),
        (
            "geo:pair",
            json!([[], {"limits": [0.0], "mode": 0}]),
            "WrongType",
            "[1].limits",
        ),
    ];
    for (specifier, value, class, path) in cases {
        let line = c.request(&format!("change {specifier} {value}")).await;
        assert!(
            line.starts_with(&format!("error_change {specifier} ")),
            "{line}"
        );
        assert_eq!(error_class(&line), class, "{value}: {line}");
        let (_, _, data) = parse(&line);
        let message = data[1].as_str().unwrap();
        if path.is_empty() {
            assert!(
                message.ends_with("5 elements, at most 4 allowed"),
                "{value}: {message}"
            );
        } else {
            assert!(
                message.contains(&format!("{path}: ")),
                "{value}: expected path {path} in {message}"
            );
        }
    }
    // nothing was changed
    let line = c.request("read geo:ragged").await;
    assert_eq!(error_class(&line), "ReadFailed", "{line}");

    // a command argument is checked the same way
    let line = c
        .request(&format!(
            "do geo:echo {}",
            json!({"name": "p", "points": [point(-1.0, json!([]))]})
        ))
        .await;
    assert_eq!(error_class(&line), "RangeError", "{line}");
    assert!(line.contains("argument.points[0].x: "), "{line}");
}

#[tokio::test]
async fn nested_updates_reach_activated_clients() {
    let n = start();
    let mut c = Client::connect(n.local_addr()).await;
    c.send("activate geo").await;
    let mut updates = HashMap::new();
    loop {
        let line = c.line().await;
        if line.starts_with("active") {
            break;
        }
        let (action, specifier, data) = parse(&line);
        assert_eq!(action, "update", "{line}");
        updates.insert(specifier, data[0].clone());
    }
    assert_eq!(updates["geo:value"], json!([[1.0], [2.0, 3.0]]));

    // a change is pushed to activated clients with the whole nested value
    let other = Client::connect(n.local_addr()).await;
    let mut other = other;
    let ragged = json!([[9.0, 8.0], [7.0]]);
    change(&mut other, "geo:ragged", &ragged).await;
    let (action, specifier, data) = parse(&c.line().await);
    assert_eq!(
        (action.as_str(), specifier.as_str(), &data[0]),
        ("update", "geo:ragged", &ragged)
    );
}

/// A change may leave out optional struct members, also inside an array. The handler
/// gets the value as sent; the node only reports a value with all members, so a handler
/// that returns the value unchanged gets an `InternalError` and has to fill in the
/// members it left out.
#[tokio::test]
async fn optional_members_in_a_change() {
    let n = start();
    let mut c = Client::connect(n.local_addr()).await;
    let full = json!([{"x": 1.0, "t": 2.0}, {"x": 3.0, "t": 4.0}]);
    assert_eq!(change(&mut c, "geo:partial", &full).await, full);

    let partial = json!([{"x": 1.0}, {"x": 3.0, "t": 4.0}]);
    let line = c.request(&format!("change geo:partial {partial}")).await;
    assert_eq!(error_class(&line), "InternalError", "{line}");
    assert!(line.contains("[0]: missing struct member `t`"), "{line}");
    // a mandatory member is still required
    let line = c.request("change geo:partial [{\"t\": 1.0}]").await;
    assert_eq!(error_class(&line), "WrongType", "{line}");
}
