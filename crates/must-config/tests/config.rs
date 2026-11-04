// SPDX-License-Identifier: LGPL-3.0-or-later

//! Loading node configs: structure, node table, modules, templates, errors, and the
//! built-in simulation types served from a config.

use std::time::Duration;

use must_config::{ConfigErrors, Loaded, ModuleConfig, Registry, load_str};
use must_server::typed::prelude::*;
use must_server::{Module, Parameter, RunningNode};
use must_types::DataInfo;
use serde_json::{Value as Json, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::runtime::Handle;

/// A module type declared with the macro, for property scenarios.
#[module(description = "a sensor with a channel")]
struct Channel {
    value: param!("value", double()),
    channel: property!("sensor channel", string(maxchars = 1)),
}

#[methods]
impl Channel {
    async fn read_value(&mut self) -> Result<f64> {
        Ok(if self.channel == "A" { 1.0 } else { 2.0 })
    }
}

fn registry() -> Registry {
    let mut r = Registry::default();
    r.register::<Channel>("test::Channel").unwrap();
    // a factory reading its settings by hand
    r.register_fn("test::Factory", |c: &mut ModuleConfig<'_>| {
        let range: f64 = c.get("range")?;
        if range <= 0.0 {
            return Err(c.error("range", "must be positive"));
        }
        let warn: bool = c.get_or("warn", false)?;
        let info = DataInfo::double().max(range).build().unwrap();
        let mut p = Parameter::new("value", info, "value").cached();
        if warn {
            // a poll interval on a cached parameter is a completion warning
            p = p.poll_interval(Duration::from_secs(1));
        }
        c.new_module("a factory module")
            .interface_classes(std::iter::empty())
            .parameter(p)
            .map_err(|e| c.error("range", e))
    })
    .unwrap();
    r
}

const NODE: &str = "[node]\nequipment_id = \"cryo-1\"\ndescription = \"cryostat\"\ninterface = \"tcp://127.0.0.1:0\"\ndiscovery = false\nlog_files = false\n";

fn load(text: &str) -> Result<Loaded, ConfigErrors> {
    load_str(text, "node.toml", None, &registry())
}

fn errors(text: &str) -> String {
    load(text).err().expect("loading should fail").to_string()
}

fn with_node(modules: &str) -> String {
    format!("{NODE}\n{modules}")
}

// --- structure and node table

#[test]
fn minimal_config() {
    let loaded = load(&with_node(
        "[modules.T]\ntype = \"must::sim::Temperature\"\ndescription = \"sample temperature\"\n",
    ))
    .ok()
    .unwrap();
    assert_eq!(
        loaded.modules,
        [("T".to_owned(), "must::sim::Temperature".to_owned())]
    );
    let report = loaded.node.structure_report();
    assert!(report["modules"].get("T").is_some());
}

#[test]
fn unknown_top_level_table() {
    let e = errors(&format!("{NODE}\n[module.T]\ntype = \"x\"\n"));
    assert!(e.contains("node.toml:8:") && e.contains("`module`"), "{e}");
}

#[test]
fn not_toml() {
    let text = format!("{NODE}x = = 1\n");
    let e = errors(&text);
    assert!(
        e.starts_with("node.toml:7:") && e.contains("not valid TOML"),
        "{e}"
    );
}

#[test]
fn custom_node_property() {
    let loaded = load(&format!(
        "{NODE}_facility = \"HZB\"\n[modules.T]\ntype = \"must::sim::Temperature\"\n"
    ))
    .ok()
    .unwrap();
    assert_eq!(loaded.node.structure_report()["_facility"], "HZB");
}

#[test]
fn missing_equipment_id() {
    let e = errors("[node]\ndescription = \"x\"\n[modules.T]\ntype = \"must::sim::Temperature\"\n");
    assert!(e.contains("equipment_id"), "{e}");
}

#[test]
fn port_only() {
    let text = format!(
        "{}interface = \"tcp://10767\"\n[modules.T]\ntype = \"must::sim::Temperature\"\n",
        NODE.replace("interface = \"tcp://127.0.0.1:0\"\n", "")
    );
    let loaded = load(&text).ok().unwrap();
    assert_eq!(loaded.node.bind_address().to_string(), "0.0.0.0:10767");
}

#[test]
fn default_interface() {
    let text = format!(
        "{}[modules.T]\ntype = \"must::sim::Temperature\"\n",
        NODE.replace("interface = \"tcp://127.0.0.1:0\"\n", "")
    );
    let loaded = load(&text).ok().unwrap();
    assert_eq!(loaded.node.bind_address().to_string(), "0.0.0.0:2055");
}

#[test]
fn unsupported_scheme() {
    let text = NODE.replace("tcp://127.0.0.1:0", "serial:///dev/ttyS0")
        + "[modules.T]\ntype = \"must::sim::Temperature\"\n";
    let e = errors(&text);
    assert!(e.contains("`interface`"), "{e}");
}

#[test]
fn discovery_and_log_files() {
    let text = NODE
        .replace(
            "discovery = false",
            "discovery = { port = 10768, announce = false }",
        )
        .replace("log_files = false", "log_files = { days = 7 }")
        + "[modules.T]\ntype = \"must::sim::Temperature\"\n";
    assert!(load(&text).is_ok());
    let e = errors(
        &(NODE.replace("discovery = false", "discovery = { prot = 1 }")
            + "[modules.T]\ntype = \"must::sim::Temperature\"\n"),
    );
    assert!(e.contains("discovery.prot"), "{e}");
}

// --- modules

#[test]
fn custom_module_property() {
    let loaded = load(&with_node(
        "[modules.mfc1]\ntype = \"must::sim::FlowController\"\nrange = 100.0\n_serial_number = \"M1234\"\n",
    ))
    .ok()
    .unwrap();
    assert_eq!(
        loaded.node.structure_report()["modules"]["mfc1"]["_serial_number"],
        "M1234"
    );
}

#[test]
fn unknown_type() {
    let e = errors(&with_node(
        "[modules.mfc]\ntype = \"must::sim::Flowcontroller\"\n",
    ));
    assert!(
        e.contains("node.toml:9:")
            && e.contains("module `mfc`")
            && e.contains("must::sim::Flowcontroller")
            && e.contains("must::sim::FlowController, must::sim::Temperature"),
        "{e}"
    );
}

#[test]
fn file_order_kept() {
    let loaded = load(&with_node(
        "[modules.p]\ntype = \"must::sim::Temperature\"\n\n[modules.\"mfc{i}\"]\ntype = \"must::sim::FlowController\"\ncount = 3\nrange = 10.0\n\n[modules.a]\ntype = \"must::sim::Temperature\"\n",
    ))
    .ok()
    .unwrap();
    let names: Vec<_> = loaded.modules.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names, ["p", "mfc1", "mfc2", "mfc3", "a"]);
    let report = loaded.node.structure_report();
    let order: Vec<_> = report["modules"]
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect();
    assert_eq!(order, ["p", "mfc1", "mfc2", "mfc3", "a"]);
}

#[test]
fn property_from_the_config_and_too_long() {
    assert!(
        load(&with_node(
            "[modules.c]\ntype = \"test::Channel\"\nchannel = \"A\"\n"
        ))
        .is_ok()
    );
    let e = errors(&with_node(
        "[modules.c]\ntype = \"test::Channel\"\nchannel = \"AB\"\n",
    ));
    assert!(
        e.contains("node.toml:10:") && e.contains("module `c`") && e.contains("channel"),
        "{e}"
    );
}

#[test]
fn initial_value_out_of_range() {
    let e = errors(&with_node(
        "[modules.T]\ntype = \"must::sim::Temperature\"\npollinterval = -1.0\n",
    ));
    assert!(
        e.contains("node.toml:10:") && e.contains("module `T`") && e.contains("pollinterval"),
        "{e}"
    );
}

#[test]
fn value_of_a_hardware_parameter() {
    let e = errors(&with_node(
        "[modules.T]\ntype = \"must::sim::Temperature\"\nvalue = 4.2\n",
    ));
    assert!(
        e.contains("`value`") && e.contains("cannot be configured"),
        "{e}"
    );
}

#[test]
fn misspelled_setting() {
    let e = errors(&with_node(
        "[modules.mfc]\ntype = \"must::sim::FlowController\"\nrange = 200.0\nrnage = 200.0\n",
    ));
    assert!(
        e.contains("node.toml:11:")
            && e.contains("rnage")
            && e.contains("module `mfc`")
            && e.contains("must::sim::FlowController"),
        "{e}"
    );
}

#[test]
fn several_errors_at_once() {
    let e = errors(&with_node(
        "[modules.a]\ntype = \"nope::Nothing\"\n\n[modules.b]\ntype = \"must::sim::FlowController\"\nrange = 1.0\nrnage = 1.0\n",
    ));
    let lines: Vec<_> = e.lines().collect();
    assert_eq!(lines.len(), 2, "{e}");
    assert!(
        lines[0].contains("node.toml:9:") && lines[1].contains("node.toml:14:"),
        "{e}"
    );
}

#[test]
fn template_collides_with_a_module() {
    let e = errors(&with_node(
        "[modules.mfc2]\ntype = \"must::sim::FlowController\"\nrange = 1.0\n\n[modules.\"mfc{i}\"]\ntype = \"must::sim::FlowController\"\ncount = 3\nrange = 1.0\n",
    ));
    assert!(
        e.contains("`mfc2`")
            && e.contains("module table `mfc2` on line 8")
            && e.contains("instance 2"),
        "{e}"
    );
}

#[test]
fn invalid_expanded_name() {
    let e = errors(&with_node(
        "[modules.\"mfc_{gas}\"]\ntype = \"must::sim::FlowController\"\nrange = 1.0\ninstances = [{ gas = \"N2 low\" }]\n",
    ));
    assert!(
        e.contains("`mfc_N2 low` is not a valid SECoP identifier"),
        "{e}"
    );
}

#[test]
fn error_in_an_expanded_module() {
    let e = errors(&with_node(
        "[modules.\"mfc{i}\"]\ntype = \"must::sim::FlowController\"\ninstances = [\n  { range = 1.0 },\n  { range = 2.0 },\n  { range = -3.0 },\n]\n",
    ));
    assert!(
        e.contains("node.toml:13:")
            && e.contains("module `mfc3` (template `mfc{i}`, instance 3)")
            && e.contains("range"),
        "{e}"
    );
}

#[test]
fn warnings_do_not_fail_loading() {
    let loaded = load(&with_node(
        "[modules.f]\ntype = \"test::Factory\"\nrange = 1.0\nwarn = true\n",
    ))
    .ok()
    .unwrap();
    assert_eq!(loaded.warnings.len(), 1);
    let w = loaded.warnings[0].to_string();
    assert!(
        w.contains("node.toml:8:") && w.contains("module `f`"),
        "{w}"
    );
}

// --- factories

#[test]
fn factory_settings() {
    // Type rejects a setting
    let e = errors(&with_node(
        "[modules.f]\ntype = \"test::Factory\"\nrange = -1\n",
    ));
    assert!(
        e.contains("node.toml:10:") && e.contains("`range`: must be positive"),
        "{e}"
    );
    // Missing required setting
    let e = errors(&with_node("[modules.f]\ntype = \"test::Factory\"\n"));
    assert!(
        e.contains("module `f`") && e.contains("`range` is missing"),
        "{e}"
    );
    // Wrong value type
    let e = errors(&with_node(
        "[modules.f]\ntype = \"test::Factory\"\nrange = \"high\"\n",
    ));
    assert!(e.contains("node.toml:10:") && e.contains("a number"), "{e}");
    // Integer for a float
    let r = load(&with_node(
        "[modules.f]\ntype = \"test::Factory\"\nrange = 500\n",
    ));
    assert!(r.is_ok(), "{}", r.err().unwrap());
    // unread settings of a factory are unknown
    let e = errors(&with_node(
        "[modules.f]\ntype = \"test::Factory\"\nrange = 1.0\nextra = 2\n",
    ));
    assert!(e.contains("extra") && e.contains("test::Factory"), "{e}");
}

// --- served configs

struct Client {
    reader: BufReader<OwnedReadHalf>,
    writer: OwnedWriteHalf,
}

impl Client {
    async fn connect(node: &RunningNode) -> Self {
        let stream = TcpStream::connect(node.local_addr()).await.unwrap();
        let (r, w) = stream.into_split();
        Self {
            reader: BufReader::new(r),
            writer: w,
        }
    }

    async fn request(&mut self, line: &str) -> String {
        self.writer
            .write_all(format!("{line}\n").as_bytes())
            .await
            .unwrap();
        let mut reply = String::new();
        tokio::time::timeout(Duration::from_secs(5), self.reader.read_line(&mut reply))
            .await
            .unwrap()
            .unwrap();
        reply.trim_end().to_owned()
    }

    async fn data(&mut self, line: &str) -> Json {
        let reply = self.request(line).await;
        let json = reply.splitn(3, ' ').nth(2).unwrap_or("null");
        serde_json::from_str(json).unwrap()
    }
}

fn serve(text: &str) -> RunningNode {
    load(text)
        .ok()
        .unwrap()
        .node
        .spawn(&Handle::current())
        .unwrap()
}

#[tokio::test]
async fn poll_interval_and_target_from_the_config() {
    let n = serve(&with_node(
        "[modules.T]\ntype = \"must::sim::Temperature\"\npollinterval = 2.0\ntarget = 10.0\n",
    ));
    let mut c = Client::connect(&n).await;
    assert_eq!(c.data("read T:pollinterval").await[0], 2.0);
    assert_eq!(c.data("read T:target").await[0], 10.0);
    assert_eq!(
        c.data("read T:status").await[0][0],
        300,
        "ramping to the written target"
    );
    n.stop().await;
}

#[tokio::test]
async fn five_identical_flow_controllers_and_two_instances() {
    let n = serve(&with_node(
        "[modules.\"mfc{i}\"]\ntype = \"must::sim::FlowController\"\ncount = 5\nrange = 100.0\nsettling_time = 0.0\n",
    ));
    let mut c = Client::connect(&n).await;
    let d = c.data("describe").await;
    for i in 1..=5 {
        assert_eq!(
            d["modules"][format!("mfc{i}")]["accessibles"]["target"]["datainfo"]["max"],
            100.0
        );
    }
    assert!(
        c.request("change mfc1:target 40")
            .await
            .starts_with("changed")
    );
    assert_eq!(c.data("read mfc1:value").await[0], 40.0);
    assert_eq!(
        c.data("read mfc2:value").await[0],
        0.0,
        "mfc2 is a module of its own"
    );
    n.stop().await;
}

#[tokio::test]
async fn flow_controller_range_and_settling() {
    let n = serve(&with_node(
        "[modules.mfc1]\ntype = \"must::sim::FlowController\"\nrange = 200.0\nsettling_time = 0.3\n",
    ));
    let mut c = Client::connect(&n).await;
    let reply = c.request("change mfc1:target 250").await;
    assert!(reply.contains("RangeError"), "{reply}");
    assert!(
        c.request("change mfc1:target 100")
            .await
            .starts_with("changed")
    );
    assert_eq!(c.data("read mfc1:status").await[0][0], 300);
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(c.data("read mfc1:value").await[0], 100.0);
    assert_eq!(c.data("read mfc1:status").await[0][0], 100);
    n.stop().await;
}

#[test]
fn example_configs_load() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/examples");
    for file in ["cryostat.toml", "gas-rack.toml"] {
        let path = std::path::Path::new(dir).join(file);
        let loaded = must_config::load_file(&path, &Registry::default());
        assert!(loaded.is_ok(), "{file}: {}", loaded.err().unwrap());
    }
    let rack = must_config::load_file(
        &std::path::Path::new(dir).join("gas-rack.toml"),
        &Registry::default(),
    )
    .ok()
    .unwrap();
    let report = rack.node.structure_report();
    assert_eq!(
        report["modules"]["mfc2"]["description"],
        "Ar mass flow controller"
    );
    assert_eq!(report["modules"]["mfc5"]["_serial_number"], "M1205");
    assert_eq!(
        report["modules"]["mfc5"]["accessibles"]["target"]["datainfo"],
        json!({"type": "double", "min": 0.0, "max": 50.0, "unit": "ml/min"})
    );
}

#[allow(dead_code)]
fn unused(_: Module) {}
