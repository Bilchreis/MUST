// SPDX-License-Identifier: LGPL-3.0-or-later

//! End-to-end tests of the scenarios in specs/sec-node-protocol, over loopback TCP.

mod common;

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use common::{Client, Sim, error_class, kelvin, node, parse};
use must_server::logfiles::LogFiles;
use must_server::{AppLogLevel, Discovery, Module, NodeBuilder, Parameter, RunningNode};
use must_types::Value;
use serde_json::json;
use tokio::runtime::Handle;

fn start() -> (RunningNode, Arc<Sim>) {
    let sim = Arc::new(Sim::default());
    let running = node(Arc::clone(&sim))
        .build()
        .unwrap()
        .spawn(&Handle::current())
        .unwrap();
    (running, sim)
}

#[tokio::test]
async fn crlf_line_ending() {
    let (n, _) = start();
    let mut c = Client::connect(n.local_addr()).await;
    c.send_raw(b"read t1:value\r\n").await;
    assert!(c.line().await.starts_with("reply t1:value [295.0,"));
}

#[tokio::test]
async fn data_containing_spaces() {
    let (n, _) = start();
    let mut c = Client::connect(n.local_addr()).await;
    let line = c.request(r#"change t1:mode {"a": 1, "b": 2}"#).await;
    let (action, spec, data) = parse(&line);
    assert_eq!((action.as_str(), spec.as_str()), ("changed", "t1:mode"));
    assert_eq!(data[0], json!({"a": 1, "b": 2}));
}

#[tokio::test]
async fn unknown_action() {
    let (n, _) = start();
    let mut c = Client::connect(n.local_addr()).await;
    let line = c.request("meas:volt?").await;
    assert!(
        line.starts_with("error_meas:volt?  [\"ProtocolError\""),
        "{line}"
    );
    // a custom message (leading underscore) this node does not implement
    let line = c.request("_help me").await;
    assert!(line.starts_with("error__help me "), "{line}");
    assert_eq!(error_class(&line), "ProtocolError");
}

/// The parts the specification's "must accept" grammar calls `ignored-value`.
#[tokio::test]
async fn ignored_values() {
    let (n, _) = start();
    let mut c = Client::connect(n.local_addr()).await;
    assert!(
        c.request("describe . extra")
            .await
            .starts_with("describing . {")
    );
    assert!(
        c.request("ping tok extra")
            .await
            .starts_with("pong tok [null,")
    );
    assert!(
        c.request("read t1:value 5")
            .await
            .starts_with("reply t1:value [295.0,")
    );
    c.send("activate t2 extra").await;
    while c.line().await != "active t2" {}
    assert_eq!(c.request("deactivate t2 extra").await, "inactive t2");
}

/// Compatibility rule 5: a specifier with more `:` parts than the action takes is treated
/// as its leading part, and the reply names that part.
#[tokio::test]
async fn longer_specifiers() {
    let (n, _) = start();
    let mut c = Client::connect(n.local_addr()).await;
    c.send("activate t2:value").await;
    let mut updates = Vec::new();
    loop {
        let line = c.line().await;
        if line == "active t2" {
            break;
        }
        updates.push(parse(&line).1);
    }
    assert!(updates.contains(&"t2:value".to_owned()), "{updates:?}");
    assert!(updates.iter().all(|s| s.starts_with("t2:")), "{updates:?}");
    assert_eq!(c.request("deactivate t2:value").await, "inactive t2");

    let line = c.request("read t1:value:x").await;
    assert!(line.starts_with("reply t1:value [295.0,"), "{line}");
    let line = c.request("change t1:target:x 5").await;
    assert!(line.starts_with("changed t1:target [5.0,"), "{line}");
    let line = c.request("do t1:stop:x").await;
    assert!(line.starts_with("done t1:stop [null,"), "{line}");
    let plain = c.request("check t1:target 1").await;
    let longer = c.request("check t1:target:x 1").await;
    assert_eq!(longer.split(' ').next(), plain.split(' ').next());
    assert_eq!(parse(&longer).1, "t1:target");
    assert_eq!(error_class(&longer), error_class(&plain));
    // the part understood still has to name something
    let line = c.request("read t1:nope:x").await;
    assert!(line.starts_with("error_read t1:nope "), "{line}");
    assert_eq!(error_class(&line), "NoSuchParameter");
}

/// A message is ASCII: other bytes get an error reply and the connection keeps working.
#[tokio::test]
async fn non_ascii_bytes() {
    let (n, _) = start();
    let mut c = Client::connect(n.local_addr()).await;
    c.send_raw(b"read t1:val\xff\xfe\n").await;
    assert!(c.line().await.starts_with("error_read t1:val"));
    assert!(c.request("ping 1").await.starts_with("pong 1 "));
}

#[tokio::test]
async fn invalid_json() {
    let (n, sim) = start();
    let mut c = Client::connect(n.local_addr()).await;
    let line = c.request("change t1:target {oops").await;
    assert!(line.starts_with("error_change t1:target "), "{line}");
    assert_eq!(error_class(&line), "BadJSON");
    assert_eq!(sim.changes.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn oversized_message() {
    let sim = Arc::new(Sim::default());
    let n = node(sim)
        .max_line_length(100)
        .build()
        .unwrap()
        .spawn(&Handle::current())
        .unwrap();
    let mut c = Client::connect(n.local_addr()).await;
    c.send(&format!("change t1:target {}", "1".repeat(200)))
        .await;
    assert_eq!(error_class(&c.line().await), "ProtocolError");
    // the connection keeps working
    assert!(c.request("ping 1").await.starts_with("pong 1 "));
}

#[tokio::test]
async fn identification_resets_activation() {
    let (n, _) = start();
    let mut c = Client::connect(n.local_addr()).await;
    c.send("activate").await;
    while c.line().await != "active" {}
    assert_eq!(c.request("*IDN?").await, "ISSE,SECoP,2026-07-07,v2.0");
    n.update("t1", "value", must_types::Value::Double(1.0))
        .unwrap();
    assert_eq!(c.try_line(Duration::from_millis(300)).await, None);
}

#[tokio::test]
async fn describe() {
    let (n, _) = start();
    let mut c = Client::connect(n.local_addr()).await;
    let line = c.request("describe").await;
    assert!(line.starts_with("describing . {"), "{line}");
    let report: serde_json::Value = serde_json::from_str(&line["describing . ".len()..]).unwrap();
    assert_eq!(report["equipment_id"], "must-test");
    assert_eq!(
        report["modules"]["t1"]["accessibles"]["value"]["readonly"],
        true
    );
    // any specifier is accepted
    assert!(c.request("describe .").await.starts_with("describing . {"));
}

#[tokio::test]
async fn global_activation() {
    let (n, _) = start();
    let mut c = Client::connect(n.local_addr()).await;
    c.send("activate").await;
    let mut lines = Vec::new();
    loop {
        let line = c.line().await;
        if line == "active" {
            break;
        }
        lines.push(line);
    }
    assert!(
        lines
            .iter()
            .any(|l| l.starts_with("update t1:value [295.0,")),
        "{lines:?}"
    );
    assert!(
        lines
            .iter()
            .any(|l| l.starts_with("update t1:status [[100,\"idle\"],")),
        "{lines:?}"
    );
    assert!(
        lines.iter().any(|l| l.starts_with("update t2:value ")),
        "{lines:?}"
    );
    // constant parameters are not part of the initial updates
    assert!(!lines.iter().any(|l| l.contains("t1:serial")), "{lines:?}");
}

#[tokio::test]
async fn module_wise_activation() {
    let (n, _) = start();
    let mut c = Client::connect(n.local_addr()).await;
    let line = c.request("activate nope").await;
    assert!(line.starts_with("error_activate nope "), "{line}");
    assert_eq!(error_class(&line), "NoSuchModule");

    c.send("activate t2").await;
    assert!(c.line().await.starts_with("update t2:"));
    assert!(c.line().await.starts_with("update t2:"));
    assert_eq!(c.line().await, "active t2");
    n.update("t1", "value", must_types::Value::Double(1.0))
        .unwrap();
    n.update("t2", "value", must_types::Value::Double(2.0))
        .unwrap();
    assert!(c.line().await.starts_with("update t2:value [2.0,"));
}

#[tokio::test]
async fn module_wise_deactivation() {
    let (n, _) = start();
    let mut c = Client::connect(n.local_addr()).await;
    c.send("activate").await;
    while c.line().await != "active" {}
    assert_eq!(c.request("deactivate t1").await, "inactive t1");
    n.update("t1", "value", must_types::Value::Double(1.0))
        .unwrap();
    n.update("t2", "value", must_types::Value::Double(2.0))
        .unwrap();
    assert!(c.line().await.starts_with("update t2:value [2.0,"));
    let line = c.request("deactivate nope").await;
    assert!(line.starts_with("error_deactivate nope "), "{line}");
    assert_eq!(error_class(&line), "NoSuchModule");
    assert_eq!(c.request("deactivate").await, "inactive");
    n.update("t2", "value", must_types::Value::Double(3.0))
        .unwrap();
    assert_eq!(c.try_line(Duration::from_millis(300)).await, None);
}

#[tokio::test]
async fn heartbeat() {
    let (n, _) = start();
    let mut c = Client::connect(n.local_addr()).await;
    let (action, token, data) = parse(&c.request("ping 42").await);
    assert_eq!((action.as_str(), token.as_str()), ("pong", "42"));
    assert!(data[0].is_null() && data[1]["t"].is_f64());
    assert_eq!(data[1].as_object().unwrap().len(), 1, "only the timestamp");
    let line = c.request("ping").await;
    assert!(line.starts_with("pong  [null,{\"t\":"), "{line}");
}

#[tokio::test]
async fn read_errors() {
    let (n, _) = start();
    let mut c = Client::connect(n.local_addr()).await;
    let line = c.request("read t1:nope").await;
    assert!(line.starts_with("error_read t1:nope "));
    assert_eq!(error_class(&line), "NoSuchParameter");
    let line = c.request("read t1:stop").await;
    assert_eq!(error_class(&line), "NoSuchParameter");
    assert_eq!(
        error_class(&c.request("read tx:value").await),
        "NoSuchModule"
    );
    for malformed in ["read t1", "read :value", "read t1:", "read"] {
        assert_eq!(
            error_class(&c.request(malformed).await),
            "ProtocolError",
            "{malformed}"
        );
    }
}

/// A data report carries the timestamp `t` and, if the handler gives one, the
/// uncertainty `e`.
#[tokio::test]
async fn qualifiers() {
    let (n, _) = start();
    let mut c = Client::connect(n.local_addr()).await;
    let (_, _, data) = parse(&c.request("read t1:value").await);
    assert!(data[1]["t"].is_f64());
    assert_eq!(data[1]["e"], json!(0.01));
    let (_, _, data) = parse(&c.request("read t1:target").await);
    assert_eq!(data[1].as_object().unwrap().len(), 1, "no uncertainty");
}

#[tokio::test]
async fn change_readonly_and_dynamic_limit() {
    let (n, sim) = start();
    let mut c = Client::connect(n.local_addr()).await;
    let line = c.request("change t1:value 5").await;
    assert!(line.starts_with("error_change t1:value "));
    assert_eq!(error_class(&line), "ReadOnly");
    assert_eq!(
        error_class(&c.request("change t1:serial \"x\"").await),
        "ReadOnly"
    );

    assert_eq!(
        error_class(&c.request("change t1:stop 1").await),
        "NoSuchParameter"
    );

    // t1:target_max is 200
    let line = c.request("change t1:target 250").await;
    assert!(line.starts_with("error_change t1:target "));
    assert_eq!(error_class(&line), "RangeError");
    assert!(line.contains("dynamic upper limit 200"), "{line}");
    assert_eq!(
        sim.changes.load(Ordering::SeqCst),
        0,
        "the hardware is not asked"
    );

    // static limits and types are checked too
    assert_eq!(
        error_class(&c.request("change t1:target 400").await),
        "RangeError"
    );
    assert_eq!(
        error_class(&c.request("change t1:target \"hot\"").await),
        "WrongType"
    );
    assert_eq!(
        error_class(&c.request("change t1:target").await),
        "ProtocolError"
    );

    // raising the limit allows the change
    assert!(
        c.request("change t1:target_max 280")
            .await
            .starts_with("changed t1:target_max [280.0,")
    );
    assert!(
        c.request("change t1:target 250")
            .await
            .starts_with("changed t1:target [250.0,")
    );
}

/// The dynamic limits of the postfixed parameters `_min` and `_limits` (`_max` is part of
/// the test node), which a client can change.
#[tokio::test]
async fn dynamic_min_and_limits() {
    let limits = common::di(json!({"type": "tuple", "members": [
        {"type": "double", "min": 0, "max": 300, "unit": "K"},
        {"type": "double", "min": 0, "max": 300, "unit": "K"}]}));
    let cached = |name: &str, info, value| {
        Parameter::new(name, info, name)
            .writable()
            .cached()
            .initial_value(value)
    };
    let pair = |low, high| Value::Tuple(vec![Value::Double(low), Value::Double(high)]);
    let module = Module::new("m", "dynamic limits")
        .interface_classes([])
        .parameter(Parameter::new("setpoint", kelvin(), "setpoint").writable())
        .unwrap()
        .parameter(cached("setpoint_min", kelvin(), Value::Double(10.0)))
        .unwrap()
        .parameter(Parameter::new("ramp", kelvin(), "ramp").writable())
        .unwrap()
        .parameter(cached("ramp_limits", limits, pair(1.0, 5.0)))
        .unwrap()
        .no_polling()
        .handler(Sim::default());
    let n = NodeBuilder::new("must-limits", "test node")
        .bind(std::net::SocketAddr::from(([127, 0, 0, 1], 0)))
        .initial_read(false)
        .log_files(LogFiles::off())
        .discovery(Discovery::off())
        .module(module)
        .unwrap()
        .build()
        .unwrap()
        .spawn(&Handle::current())
        .unwrap();
    let mut c = Client::connect(n.local_addr()).await;

    let line = c.request("change m:setpoint 5").await;
    assert_eq!(error_class(&line), "RangeError");
    assert!(line.contains("dynamic lower limit 10"), "{line}");
    assert!(
        c.request("change m:setpoint 10")
            .await
            .starts_with("changed m:setpoint [10.0,")
    );

    for outside in ["0.5", "6"] {
        let line = c.request(&format!("change m:ramp {outside}")).await;
        assert_eq!(error_class(&line), "RangeError", "{line}");
        assert!(line.contains("dynamic"), "{line}");
    }
    assert!(
        c.request("change m:ramp 5")
            .await
            .starts_with("changed m:ramp [5.0,")
    );
    // the node's own value of the limits moves them
    n.update("m", "ramp_limits", pair(1.0, 8.0)).unwrap();
    assert!(
        c.request("change m:ramp 6")
            .await
            .starts_with("changed m:ramp [6.0,")
    );
}

#[tokio::test]
async fn do_commands() {
    let (n, _) = start();
    let mut c = Client::connect(n.local_addr()).await;
    for request in ["do t1:stop", "do t1:stop null"] {
        let (action, spec, data) = parse(&c.request(request).await);
        assert_eq!((action.as_str(), spec.as_str()), ("done", "t1:stop"));
        assert!(data[0].is_null() && data[1]["t"].is_f64());
    }
    let (_, _, data) = parse(&c.request("do t1:echo 12.5").await);
    assert_eq!(data[0], json!(12.5));
    assert_eq!(
        error_class(&c.request("do t1:echo 400").await),
        "RangeError"
    );
    assert_eq!(error_class(&c.request("do t1:stop 5").await), "WrongType");
    // a command with an argument needs one
    assert_eq!(error_class(&c.request("do t1:echo").await), "WrongType");

    let line = c.request("do t1:value").await;
    assert!(line.starts_with("error_do t1:value "));
    assert_eq!(error_class(&line), "NoSuchCommand");
}

#[tokio::test]
async fn error_report_format() {
    let (n, _) = start();
    let mut c = Client::connect(n.local_addr()).await;
    let (action, spec, data) = parse(&c.request("read t1:broken").await);
    assert_eq!(
        (action.as_str(), spec.as_str()),
        ("error_read", "t1:broken")
    );
    assert_eq!(data[0], "HardwareError");
    assert_eq!(data[1], "sensor disconnected");
    assert!(data[2]["t"].is_f64());

    let (_, _, data) = parse(&c.request("read t1:overheated").await);
    assert_eq!(data[0], "_Overheated");
    assert_eq!(data[1], "coil too hot");
    assert!(data[2]["t"].is_f64());
}

#[tokio::test]
async fn empty_line_between_requests() {
    let (n, _) = start();
    let mut c = Client::connect(n.local_addr()).await;
    c.send("").await;
    c.send("").await;
    assert!(c.request("ping 1").await.starts_with("pong 1 "));
    assert_eq!(c.try_line(Duration::from_millis(300)).await, None);
}

#[tokio::test]
async fn check_of_an_accessible_without_checkable() {
    let (n, _) = start();
    let mut c = Client::connect(n.local_addr()).await;
    let line = c.request("check t1:target 5").await;
    assert!(line.starts_with("error_check t1:target "), "{line}");
    assert_eq!(error_class(&line), "NotCheckable");
}

// --- remote logging: the scenarios of "Logging request" and "Logging levels" in
// specs/sec-node-remote-logging

/// Logs one message per level for `module`, then a marker at error level that every
/// level but off admits, so that a test knows when it has seen everything.
fn log_all(n: &RunningNode, module: &str) {
    for (level, text) in [
        (AppLogLevel::Debug, "d"),
        (AppLogLevel::Info, "i"),
        (AppLogLevel::Warning, "w"),
        (AppLogLevel::Error, "e"),
    ] {
        n.log(module, level, text).unwrap();
    }
}

/// Lines received until none arrives for a moment.
async fn drain(c: &mut Client) -> Vec<String> {
    let mut lines = Vec::new();
    while let Some(line) = c.try_line(Duration::from_millis(300)).await {
        lines.push(line);
    }
    lines
}

#[tokio::test]
async fn logging_whole_node() {
    let (n, _) = start();
    let mut c = Client::connect(n.local_addr()).await;
    assert_eq!(
        c.request(r#"logging  "debug""#).await,
        r#"logging  "debug""#
    );
    log_all(&n, "t1");
    log_all(&n, "t2");
    assert_eq!(
        drain(&mut c).await,
        [
            r#"log t1:debug "d""#,
            r#"log t1:info "i""#,
            r#"log t1:error "w""#,
            r#"log t1:error "e""#,
            r#"log t2:debug "d""#,
            r#"log t2:info "i""#,
            r#"log t2:error "w""#,
            r#"log t2:error "e""#,
        ]
    );
}

#[tokio::test]
async fn logging_one_module() {
    let (n, _) = start();
    let mut c = Client::connect(n.local_addr()).await;
    assert_eq!(
        c.request(r#"logging t1 "error""#).await,
        r#"logging t1 "error""#
    );
    log_all(&n, "t1");
    log_all(&n, "t2");
    assert_eq!(
        drain(&mut c).await,
        [r#"log t1:error "w""#, r#"log t1:error "e""#]
    );
}

#[tokio::test]
async fn logging_parameter_specifier() {
    let (n, _) = start();
    let mut c = Client::connect(n.local_addr()).await;
    assert_eq!(
        c.request(r#"logging t1:target "info""#).await,
        r#"logging t1 "info""#
    );
    log_all(&n, "t1");
    assert_eq!(
        drain(&mut c).await,
        [
            r#"log t1:info "i""#,
            r#"log t1:error "w""#,
            r#"log t1:error "e""#
        ]
    );
}

#[tokio::test]
async fn logging_single_space() {
    let (n, _) = start();
    let mut c = Client::connect(n.local_addr()).await;
    assert_eq!(c.request(r#"logging "info""#).await, r#"logging  "info""#);
    log_all(&n, "t2");
    assert_eq!(
        drain(&mut c).await,
        [
            r#"log t2:info "i""#,
            r#"log t2:error "w""#,
            r#"log t2:error "e""#
        ]
    );
}

#[tokio::test]
async fn logging_default_off() {
    let (n, _) = start();
    let mut c = Client::connect(n.local_addr()).await;
    c.send("activate").await;
    while c.line().await != "active" {}
    log_all(&n, "t1");
    assert_eq!(drain(&mut c).await, Vec::<String>::new());
}

#[tokio::test]
async fn logging_false_is_off() {
    let (n, _) = start();
    let mut c = Client::connect(n.local_addr()).await;
    assert_eq!(
        c.request(r#"logging t1 "debug""#).await,
        r#"logging t1 "debug""#
    );
    assert_eq!(c.request("logging t1 false").await, "logging t1 false");
    log_all(&n, "t1");
    assert_eq!(
        c.request(r#"logging t1 "off""#).await,
        r#"logging t1 "off""#
    );
    log_all(&n, "t1");
    assert_eq!(drain(&mut c).await, Vec::<String>::new());
}

#[tokio::test]
async fn logging_module_overrides_node() {
    let (n, _) = start();
    let mut c = Client::connect(n.local_addr()).await;
    c.request(r#"logging  "error""#).await;
    c.request(r#"logging t1 "debug""#).await;
    log_all(&n, "t1");
    log_all(&n, "t2");
    assert_eq!(
        drain(&mut c).await,
        [
            r#"log t1:debug "d""#,
            r#"log t1:info "i""#,
            r#"log t1:error "w""#,
            r#"log t1:error "e""#,
            r#"log t2:error "w""#,
            r#"log t2:error "e""#,
        ]
    );
}

#[tokio::test]
async fn logging_node_replaces_modules() {
    let (n, _) = start();
    let mut c = Client::connect(n.local_addr()).await;
    c.request(r#"logging t1 "debug""#).await;
    assert_eq!(c.request(r#"logging  "off""#).await, r#"logging  "off""#);
    log_all(&n, "t1");
    log_all(&n, "t2");
    assert_eq!(drain(&mut c).await, Vec::<String>::new());
}

#[tokio::test]
async fn logging_invalid_level() {
    let (n, _) = start();
    let mut c = Client::connect(n.local_addr()).await;
    c.request(r#"logging t1 "error""#).await;
    for request in [r#"logging t1 "verbose""#, "logging t1 3"] {
        let line = c.request(request).await;
        assert!(line.starts_with("error_logging t1 "), "{line}");
        assert_eq!(error_class(&line), "WrongType");
    }
    let line = c.request(r#"logging  "loud""#).await;
    assert!(line.starts_with("error_logging  "), "{line}");
    assert_eq!(error_class(&line), "WrongType");
    assert_eq!(error_class(&c.request("logging t1 {oops").await), "BadJSON");
    assert_eq!(error_class(&c.request("logging t1").await), "ProtocolError");
    assert_eq!(error_class(&c.request("logging").await), "ProtocolError");
    // the levels are unchanged
    log_all(&n, "t1");
    assert_eq!(
        drain(&mut c).await,
        [r#"log t1:error "w""#, r#"log t1:error "e""#]
    );
}

#[tokio::test]
async fn logging_unknown_module() {
    let (n, _) = start();
    let mut c = Client::connect(n.local_addr()).await;
    let line = c.request(r#"logging nope "info""#).await;
    assert!(line.starts_with("error_logging nope "), "{line}");
    assert_eq!(error_class(&line), "NoSuchModule");
    let line = c.request(r#"logging nope:value "info""#).await;
    assert!(line.starts_with("error_logging nope:value "), "{line}");
    assert_eq!(error_class(&line), "NoSuchModule");
}
