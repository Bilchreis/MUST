// SPDX-License-Identifier: LGPL-3.0-or-later

//! End-to-end tests of the scenarios in specs/sec-node-protocol, over loopback TCP.

mod common;

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use common::{Client, Sim, error_class, node, parse};
use must_server::RunningNode;
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
    assert_eq!(error_class(&c.request("read t1").await), "ProtocolError");
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
}

#[tokio::test]
async fn optional_and_custom_messages() {
    let (n, _) = start();
    let mut c = Client::connect(n.local_addr()).await;
    let line = c.request("check t1:target 5").await;
    assert!(line.starts_with("error_check t1:target "), "{line}");
    assert_eq!(error_class(&line), "ProtocolError");
    let line = c.request("logging t1 \"debug\"").await;
    assert!(line.starts_with("error_logging t1 "), "{line}");
    // empty lines are ignored
    c.send("").await;
    c.send("").await;
    assert!(c.request("ping x").await.starts_with("pong x "));
}
