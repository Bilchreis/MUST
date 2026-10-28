// SPDX-License-Identifier: LGPL-3.0-or-later

use super::*;
use serde_json::json;

#[allow(clippy::needless_pass_by_value)]
fn di(json: Json) -> DataInfo {
    DataInfo::from_json(&json).unwrap()
}

fn status_info() -> DataInfo {
    di(json!({"type": "tuple", "members": [
        {"type": "enum", "members": {"IDLE": 100, "BUSY": 300, "ERROR": 400}},
        {"type": "string"}]}))
}

fn kelvin() -> DataInfo {
    di(json!({"type": "double", "min": 0, "max": 300, "unit": "K"}))
}

/// A valid Readable module.
fn sensor(name: &str) -> Module {
    Module::new(name, "a temperature sensor")
        .interface_classes(["Readable"])
        .parameter(Parameter::new("value", kelvin(), "temperature"))
        .unwrap()
        .parameter(Parameter::new("status", status_info(), "status"))
        .unwrap()
}

/// A valid Drivable module.
fn controller(name: &str) -> Module {
    Module::new(name, "a temperature controller")
        .interface_classes(["Drivable"])
        .parameter(Parameter::new("value", kelvin(), "temperature"))
        .unwrap()
        .parameter(Parameter::new("status", status_info(), "status"))
        .unwrap()
        .parameter(Parameter::new("target", kelvin(), "setpoint").writable())
        .unwrap()
        .command(Command::new(
            "stop",
            DataInfo::command().build().unwrap(),
            "stop ramping",
        ))
        .unwrap()
}

fn node() -> NodeBuilder {
    NodeBuilder::new("HZB-vm4", "test node")
}

fn errors(builder: NodeBuilder) -> Vec<ModelError> {
    match builder.build() {
        Ok(_) => panic!("build should fail"),
        Err(BuildError(errors)) => errors,
    }
}

fn kinds(builder: NodeBuilder) -> Vec<ModelErrorKind> {
    errors(builder).into_iter().map(|e| e.kind).collect()
}

// --- 2.1 declaration

#[test]
fn minimal_node() {
    let node = node().module(sensor("t1")).unwrap().build().unwrap();
    assert_eq!(node.equipment_id(), "HZB-vm4");
    assert_eq!(node.warnings(), []);
}

#[test]
fn missing_node_description() {
    let e = errors(
        NodeBuilder::new("HZB-vm4", "")
            .module(sensor("t1"))
            .unwrap(),
    );
    assert_eq!(e[0].kind, ModelErrorKind::MissingProperty("description"));
    assert_eq!(
        e[0].to_string(),
        "description: missing mandatory property `description`"
    );
}

#[test]
fn module_without_description() {
    let e = errors(
        node()
            .module(Module::new("t1", "").interface_classes([]))
            .unwrap(),
    );
    assert_eq!(e[0].path, "t1.description");
    assert_eq!(e[0].kind, ModelErrorKind::MissingProperty("description"));
}

#[test]
fn module_without_interface_classes() {
    let k = kinds(node().module(Module::new("t1", "x")).unwrap());
    assert_eq!(k, [ModelErrorKind::MissingProperty("interface_classes")]);
}

#[test]
fn command_datainfo_for_parameter() {
    let e = Module::new("t1", "x")
        .parameter(Parameter::new(
            "p",
            DataInfo::command().build().unwrap(),
            "p",
        ))
        .err()
        .unwrap();
    assert!(matches!(e.kind, ModelErrorKind::WrongDatainfoKind(_)));
    let k = kinds(
        node()
            .module(
                Module::new("t1", "x")
                    .interface_classes([])
                    .command(Command::new("c", DataInfo::bool().build().unwrap(), "c"))
                    .unwrap(),
            )
            .unwrap(),
    );
    assert!(matches!(k[0], ModelErrorKind::WrongDatainfoKind(_)));
}

#[test]
fn constant_must_fit_datainfo() {
    let int = di(json!({"type": "int", "min": 0, "max": 10}));
    let module = sensor("t1")
        .parameter(Parameter::new("n", int.clone(), "count").constant(20))
        .unwrap();
    let e = errors(node().module(module).unwrap());
    assert_eq!(e[0].path, "t1:n.constant");
    assert!(e[0].to_string().contains("RangeError"), "{}", e[0]);

    let module = sensor("t1")
        .parameter(Parameter::new("n", int, "count").constant(7))
        .unwrap();
    assert!(node().module(module).unwrap().build().is_ok());
}

#[test]
fn unknown_property_rejected() {
    let e = Module::new("t1", "x").property("myprop", 1).err().unwrap();
    assert_eq!(
        e.kind,
        ModelErrorKind::UnknownProperty {
            name: "myprop".into()
        }
    );
    assert!(e.to_string().contains("`_myprop`"), "{e}");
    assert!(Module::new("t1", "x").property("_myprop", 1).is_ok());
    assert!(Module::new("t1", "x").property("visibility", "www").is_ok());
    assert!(matches!(
        Module::new("t1", "x")
            .property("accessibles", json!({}))
            .err()
            .unwrap()
            .kind,
        ModelErrorKind::StructuralProperty(_)
    ));
}

#[test]
fn non_standard_property_allowed_explicitly() {
    let node = node()
        .module(sensor("t1").non_standard_property("myprop", 1))
        .unwrap()
        .build()
        .unwrap();
    assert_eq!(node.warnings().len(), 1);
    assert_eq!(node.warnings()[0].path, "t1.myprop");
    assert_eq!(node.structure_report()["modules"]["t1"]["myprop"], json!(1));
}

// --- 2.2 completion checks

#[test]
fn module_names_clash() {
    let e = node()
        .module(sensor("T1"))
        .unwrap()
        .module(sensor("t1"))
        .err()
        .unwrap();
    assert!(matches!(e.kind, ModelErrorKind::NameClash(_)));
    assert!(matches!(
        node().module(sensor("1t")).err().unwrap().kind,
        ModelErrorKind::InvalidName(_)
    ));
}

#[test]
fn accessible_names_clash() {
    let e = sensor("t1")
        .parameter(Parameter::new("Value", kelvin(), "x"))
        .err()
        .unwrap();
    assert!(matches!(e.kind, ModelErrorKind::NameClash(_)));
}

#[test]
fn group_clashes_with_module() {
    let e = errors(
        node()
            .module(sensor("cryo"))
            .unwrap()
            .module(sensor("t1").group("Cryo:sensors"))
            .unwrap(),
    );
    assert_eq!(
        e[0].kind,
        ModelErrorKind::GroupClash {
            component: "Cryo".into(),
            existing: "cryo".into()
        }
    );
    assert!(e[0].to_string().contains("`Cryo`"));
}

#[test]
fn accessible_group_clashes_with_accessible() {
    let module = sensor("t1")
        .parameter(Parameter::new("p", kelvin(), "x").group("Value"))
        .unwrap();
    let k = kinds(node().module(module).unwrap());
    assert!(matches!(&k[0], ModelErrorKind::GroupClash { existing, .. } if existing == "value"));
}

#[test]
fn invalid_visibility() {
    let module = sensor("t1")
        .parameter(Parameter::new("p", kelvin(), "x").visibility("wxw"))
        .unwrap();
    let e = errors(node().module(module).unwrap());
    assert_eq!(e[0].path, "t1:p.visibility");
    assert!(e[0].to_string().contains("\"wxw\""), "{}", e[0]);
}

#[test]
fn old_style_visibility_kept() {
    let node = node()
        .module(sensor("t1").visibility("expert"))
        .unwrap()
        .build()
        .unwrap();
    assert_eq!(
        node.structure_report()["modules"]["t1"]["visibility"],
        json!("expert")
    );
}

#[test]
fn property_value_checks() {
    let k = kinds(
        node()
            .timeout(0.0)
            .property("schemata", json!("not a list"))
            .unwrap()
            .module(
                sensor("t1")
                    .property("features", json!([1]))
                    .unwrap()
                    .property("meaning", json!("x"))
                    .unwrap(),
            )
            .unwrap(),
    );
    let props: Vec<&str> = k
        .iter()
        .filter_map(|k| match k {
            ModelErrorKind::InvalidProperty { property, .. } => Some(property.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(props, ["timeout", "schemata", "features", "meaning"]);
}

#[test]
fn several_problems_at_once() {
    let e = errors(
        node()
            .module(Module::new("a", "").interface_classes(["Readable"]))
            .unwrap()
            .module(controller("b").visibility("nope"))
            .unwrap(),
    );
    let paths: Vec<&str> = e.iter().map(|e| e.path.as_str()).collect();
    assert!(paths.contains(&"a.description"), "{paths:?}");
    assert!(paths.contains(&"a"), "{paths:?}");
    assert!(paths.contains(&"b.visibility"), "{paths:?}");
    let message = BuildError(e).to_string();
    assert!(message.lines().count() >= 4, "{message}");
}

// --- 2.3 interface classes and status

#[test]
fn drivable_without_stop() {
    let module = Module::new("t1", "x")
        .interface_classes(["Drivable"])
        .parameter(Parameter::new("value", kelvin(), "v"))
        .unwrap()
        .parameter(Parameter::new("status", status_info(), "s"))
        .unwrap()
        .parameter(Parameter::new("target", kelvin(), "t").writable())
        .unwrap();
    let k = kinds(node().module(module).unwrap());
    assert_eq!(
        k,
        [ModelErrorKind::InterfaceClass {
            class: "Drivable".into(),
            what: "a command `stop`".into()
        }]
    );
    assert!(node().module(controller("t1")).unwrap().build().is_ok());
}

#[test]
fn writable_needs_writable_target() {
    let module = sensor("t1")
        .interface_classes(["Writable"])
        .parameter(Parameter::new("target", kelvin(), "t"))
        .unwrap();
    let k = kinds(node().module(module).unwrap());
    assert!(
        matches!(&k[0], ModelErrorKind::InterfaceClass { what, .. } if what.contains("writable"))
    );
}

#[test]
fn custom_class_before_base_class() {
    let module = sensor("t1").interface_classes(["_MyHeater", "Readable"]);
    assert!(node().module(module).unwrap().build().is_ok());
    let module = Module::new("t1", "x").interface_classes(["Readable"]);
    assert_eq!(
        kinds(node().module(module).unwrap()).len(),
        2,
        "value and status missing"
    );
}

#[test]
fn non_standard_class_without_underscore() {
    let module = sensor("t1").interface_classes(["MyHeater", "Readable"]);
    let e = errors(node().module(module).unwrap());
    assert_eq!(e.len(), 1, "{e:?}");
    assert_eq!(e[0].path, "t1.interface_classes");
    assert_eq!(
        e[0].kind,
        ModelErrorKind::UnknownInterfaceClass("MyHeater".into())
    );
    assert!(e[0].to_string().contains("`_MyHeater`"), "{}", e[0]);
    // every SECoP 2.0 class is accepted
    for class in INTERFACE_CLASSES {
        let module = sensor("t1").interface_classes([*class, "Readable"]);
        assert!(node().module(module).unwrap().build().is_ok(), "{class}");
    }
}

// --- default module properties

struct MyDriver;

#[async_trait::async_trait]
impl ModuleHandler for MyDriver {}

#[test]
fn default_module_properties() {
    let built = node()
        .module(sensor("t1").handler(MyDriver))
        .unwrap()
        .build()
        .unwrap();
    let t1 = &built.structure_report()["modules"]["t1"];
    assert_eq!(t1["features"], json!([]));
    assert_eq!(
        t1["implementation"],
        json!("must_server::model::tests::MyDriver")
    );

    // also through an `Arc<dyn ModuleHandler>`
    let shared: Arc<dyn ModuleHandler> = Arc::new(MyDriver);
    let built = node()
        .module(sensor("t1").handler_arc(shared))
        .unwrap()
        .build()
        .unwrap();
    assert_eq!(
        built.structure_report()["modules"]["t1"]["implementation"],
        json!("must_server::model::tests::MyDriver")
    );
}

#[test]
fn application_values_win() {
    let module = sensor("t1")
        .features(["HasOffset"])
        .implementation("lakeshore.Model336")
        .handler(MyDriver);
    let built = node().module(module).unwrap().build().unwrap();
    let t1 = &built.structure_report()["modules"]["t1"];
    assert_eq!(t1["features"], json!(["HasOffset"]));
    assert_eq!(t1["implementation"], json!("lakeshore.Model336"));
    // the same through the generic property setter
    let module = sensor("t1").property("implementation", "x").unwrap();
    let built = node().module(module).unwrap().build().unwrap();
    assert_eq!(
        built.structure_report()["modules"]["t1"]["implementation"],
        json!("x")
    );
}

#[test]
fn status_shape() {
    assert!(node().module(sensor("t1")).unwrap().build().is_ok());

    let bad = di(json!({"type": "tuple", "members": [
        {"type": "enum", "members": {"IDLE": 100, "WEIRD": 500}}, {"type": "string"}]}));
    let module = Module::new("t1", "x")
        .interface_classes(["Readable"])
        .parameter(Parameter::new("value", kelvin(), "v"))
        .unwrap()
        .parameter(Parameter::new("status", bad, "s"))
        .unwrap();
    let k = kinds(node().module(module).unwrap());
    assert!(matches!(&k[0], ModelErrorKind::StatusShape(m) if m.contains("500")));

    let module = Module::new("t1", "x")
        .interface_classes([])
        .parameter(Parameter::new("status", kelvin(), "s"))
        .unwrap();
    assert!(matches!(
        kinds(node().module(module).unwrap())[0],
        ModelErrorKind::StatusShape(_)
    ));
}

// --- 2.4 postfixes

#[test]
fn limits_and_max_together() {
    let limits = DataInfo::tuple([&kelvin(), &kelvin()]).build().unwrap();
    let module = controller("t1")
        .parameter(Parameter::new("target_limits", limits, "l").writable())
        .unwrap()
        .parameter(Parameter::new("target_max", kelvin(), "m").writable())
        .unwrap();
    let e = errors(node().module(module).unwrap());
    assert_eq!(
        e[0].kind,
        ModelErrorKind::ConflictingLimits {
            limits: "target_limits".into(),
            other: "target_max".into()
        }
    );
    let text = e[0].to_string();
    assert!(
        text.contains("target_limits") && text.contains("target_max"),
        "{text}"
    );
}

#[test]
fn postfix_without_parent_is_ordinary() {
    let module = sensor("t1")
        .parameter(Parameter::new("ramp_max", kelvin(), "r"))
        .unwrap();
    assert!(node().module(module).unwrap().build().is_ok());
}

#[test]
fn postfix_datainfo_must_match_parent() {
    let module = controller("t1")
        .parameter(Parameter::new(
            "target_max",
            di(json!({"type": "double", "unit": "mK"})),
            "m",
        ))
        .unwrap();
    let k = kinds(node().module(module).unwrap());
    assert!(matches!(k[0], ModelErrorKind::Postfix(_)));

    let module = controller("t1")
        .parameter(Parameter::new("target_max", kelvin(), "m").writable())
        .unwrap()
        .parameter(Parameter::new("target_min", kelvin(), "m").writable())
        .unwrap()
        .parameter(Parameter::new(
            "target_enable",
            DataInfo::bool().build().unwrap(),
            "e",
        ))
        .unwrap();
    let built = node().module(module).unwrap().build().unwrap();
    let t1 = &built.model.modules["t1"];
    let names: Vec<&str> = t1
        .parameter("target")
        .unwrap()
        .limit_params
        .iter()
        .map(|(_, n)| n.as_str())
        .collect();
    assert_eq!(names, ["target_min", "target_max"]);
}

#[test]
fn string_parent_cannot_have_limits() {
    let s = DataInfo::string().build().unwrap();
    let module = sensor("t1")
        .parameter(Parameter::new("label", s.clone(), "l"))
        .unwrap()
        .parameter(Parameter::new("label_max", s, "m"))
        .unwrap();
    let k = kinds(node().module(module).unwrap());
    assert!(matches!(
        k[0],
        ModelErrorKind::Postfix(PostfixError::NonNumericParent { .. })
    ));
}

// --- 2.5 structure report

#[test]
fn structure_report_golden() {
    let built = node()
        .firmware("must-0.1.0")
        .property("_site", "lab 3")
        .unwrap()
        .module(controller("t1").group("cryostat"))
        .unwrap()
        .build()
        .unwrap();
    let expected = json!({
        "equipment_id": "HZB-vm4",
        "description": "test node",
        "firmware": "must-0.1.0",
        "_site": "lab 3",
        "modules": {
            "t1": {
                "description": "a temperature controller",
                "interface_classes": ["Drivable"],
                "group": "cryostat",
                "features": [],
                "implementation": "must_server::handler::NoHandler",
                "accessibles": {
                    "value": {
                        "description": "temperature",
                        "datainfo": {"type": "double", "min": 0.0, "max": 300.0, "unit": "K"},
                        "readonly": true
                    },
                    "status": {
                        "description": "status",
                        "datainfo": {"type": "tuple", "members": [
                            {"type": "enum", "members": {"IDLE": 100, "BUSY": 300, "ERROR": 400}},
                            {"type": "string"}]},
                        "readonly": true
                    },
                    "target": {
                        "description": "setpoint",
                        "datainfo": {"type": "double", "min": 0.0, "max": 300.0, "unit": "K"},
                        "readonly": false
                    },
                    "stop": {
                        "description": "stop ramping",
                        "datainfo": {"type": "command"}
                    }
                }
            }
        }
    });
    let report = built.structure_report();
    assert_eq!(report, expected);
    // declaration order is kept, also in the serialized text
    let text = report.to_string();
    let order: Vec<usize> = ["\"value\"", "\"status\"", "\"target\"", "\"stop\""]
        .iter()
        .map(|k| text.find(k).unwrap())
        .collect();
    assert!(order.windows(2).all(|w| w[0] < w[1]), "{text}");
    assert!(text.find("\"_site\"").unwrap() < text.find("\"modules\"").unwrap());
}

// --- polling

fn poll_of(node: &Node, module: &str, parameter: &str) -> PollSource {
    node.model.modules[module]
        .parameter(parameter)
        .unwrap()
        .poll
}

fn seconds() -> DataInfo {
    di(json!({"type": "double", "min": 0, "max": 3600, "unit": "s"}))
}

#[test]
fn tiers_follow_the_base_class() {
    let ramp = di(json!({"type": "double", "unit": "K/min"}));
    let drivable = controller("cryo")
        .parameter(Parameter::new("ramp", ramp, "ramp rate").writable())
        .unwrap()
        .parameter(
            Parameter::new("target_max", kelvin(), "limit")
                .cached()
                .initial_value(Value::Double(300.0)),
        )
        .unwrap()
        .poll_interval(Duration::from_secs(1));
    let readable = sensor("t1").poll_interval(Duration::from_secs(2));
    let writable = Module::new("w", "a writable")
        .interface_classes(["Writable"])
        .parameter(Parameter::new("value", kelvin(), "v"))
        .unwrap()
        .parameter(Parameter::new("status", status_info(), "s"))
        .unwrap()
        .parameter(Parameter::new("target", kelvin(), "t").writable())
        .unwrap()
        .poll_interval(Duration::from_secs(1))
        .slow_poll_interval(Duration::from_secs(30));
    let node = node()
        .module(drivable)
        .unwrap()
        .module(readable)
        .unwrap()
        .module(writable)
        .unwrap()
        .build()
        .unwrap();
    assert_eq!(node.warnings(), []);
    for (module, p) in [
        ("cryo", "value"),
        ("cryo", "status"),
        ("t1", "value"),
        ("w", "status"),
    ] {
        assert_eq!(poll_of(&node, module, p), PollSource::Fast, "{module}:{p}");
    }
    for (module, p) in [("cryo", "target"), ("cryo", "ramp"), ("w", "target")] {
        assert_eq!(poll_of(&node, module, p), PollSource::Slow, "{module}:{p}");
    }
    assert_eq!(poll_of(&node, "cryo", "target_max"), PollSource::None);
    let cryo = &node.model.modules["cryo"].poll;
    assert_eq!(
        (cryo.fast, cryo.slow),
        (Some(Duration::from_secs(1)), Duration::from_secs(60))
    );
    assert_eq!(node.model.modules["w"].poll.slow, Duration::from_secs(30));
}

#[test]
fn no_base_class_puts_everything_in_the_slow_tier() {
    let m = Module::new("x", "no base class")
        .interface_classes([])
        .parameter(Parameter::new("value", kelvin(), "v"))
        .unwrap()
        .parameter(Parameter::new("x", kelvin(), "x"))
        .unwrap()
        .poll_interval(Duration::from_secs(1));
    let node = node().module(m).unwrap().build().unwrap();
    assert_eq!(poll_of(&node, "x", "value"), PollSource::Slow);
    assert_eq!(poll_of(&node, "x", "x"), PollSource::Slow);
}

#[test]
fn uniform_and_own_intervals() {
    let m = controller("cryo")
        .parameter(Parameter::new("ramp", kelvin(), "ramp").poll_interval(Duration::from_secs(5)))
        .unwrap()
        .poll_all(Duration::from_secs(2));
    let node = node().module(m).unwrap().build().unwrap();
    for p in ["value", "status", "target"] {
        assert_eq!(poll_of(&node, "cryo", p), PollSource::Fast);
    }
    assert_eq!(
        poll_of(&node, "cryo", "ramp"),
        PollSource::Own(Duration::from_secs(5))
    );
}

#[test]
fn default_tiers() {
    let node = node().module(controller("cryo")).unwrap().build().unwrap();
    assert_eq!(poll_of(&node, "cryo", "value"), PollSource::Fast);
    assert_eq!(poll_of(&node, "cryo", "status"), PollSource::Fast);
    assert_eq!(poll_of(&node, "cryo", "target"), PollSource::Slow);
    let poll = &node.model.modules["cryo"].poll;
    assert_eq!(
        (poll.fast, poll.slow),
        (Some(Duration::from_secs(5)), Duration::from_secs(60))
    );
}

#[test]
fn no_base_class_without_configuration() {
    let m = Module::new("x", "no base class")
        .interface_classes([])
        .parameter(Parameter::new("value", kelvin(), "v"))
        .unwrap()
        .parameter(Parameter::new("x", kelvin(), "x"))
        .unwrap();
    let node = node().module(m).unwrap().build().unwrap();
    assert_eq!(poll_of(&node, "x", "value"), PollSource::Slow);
    assert_eq!(poll_of(&node, "x", "x"), PollSource::Slow);
}

#[test]
fn polling_off_keeps_own_intervals() {
    let m = controller("cryo")
        .parameter(Parameter::new("ramp", kelvin(), "ramp").poll_interval(Duration::from_secs(5)))
        .unwrap()
        .no_polling();
    let node = node().module(m).unwrap().build().unwrap();
    assert_eq!(node.warnings(), []);
    for p in ["value", "status", "target"] {
        assert_eq!(poll_of(&node, "cryo", p), PollSource::None);
    }
    assert_eq!(
        poll_of(&node, "cryo", "ramp"),
        PollSource::Own(Duration::from_secs(5))
    );
    assert_eq!(node.model.modules["cryo"].poll.fast, None);
}

#[test]
fn polling_off_with_uniform() {
    let m = sensor("t1").no_polling().poll_all(Duration::from_secs(2));
    let e = errors(node().module(m).unwrap());
    assert_eq!(e.len(), 1);
    assert_eq!(e[0].path, "t1");
    assert!(matches!(e[0].kind, ModelErrorKind::Polling(_)));
}

#[test]
fn pollinterval_with_polling_off_warns() {
    let m = sensor("t1")
        .parameter(Parameter::new("pollinterval", seconds(), "p").writable())
        .unwrap()
        .no_polling();
    let node = node().module(m).unwrap().build().unwrap();
    assert_eq!(node.warnings().len(), 1);
    assert_eq!(node.warnings()[0].path, "t1:pollinterval");
    assert!(!node.model.modules["t1"].poll.driven);
}

#[test]
fn tiered_and_uniform_together() {
    let m = sensor("t1")
        .poll_interval(Duration::from_secs(1))
        .poll_all(Duration::from_secs(2));
    let e = errors(node().module(m).unwrap());
    assert_eq!(e.len(), 1);
    assert_eq!(e[0].path, "t1");
    assert!(matches!(e[0].kind, ModelErrorKind::Polling(_)));
}

#[test]
fn zero_intervals_rejected() {
    let m = sensor("t1")
        .parameter(Parameter::new("x", kelvin(), "x").poll_interval(Duration::ZERO))
        .unwrap()
        .poll_interval(Duration::ZERO);
    let paths: Vec<String> = errors(node().module(m).unwrap())
        .into_iter()
        .map(|e| e.path)
        .collect();
    assert_eq!(paths, ["t1", "t1:x"]);
}

#[test]
fn interval_on_cached_parameter_ignored() {
    let m = sensor("t1")
        .parameter(
            Parameter::new("target_max", kelvin(), "limit")
                .cached()
                .poll_interval(Duration::from_secs(1)),
        )
        .unwrap();
    let node = node().module(m).unwrap().build().unwrap();
    assert_eq!(node.warnings().len(), 1);
    assert_eq!(node.warnings()[0].path, "t1:target_max");
    assert_eq!(poll_of(&node, "t1", "target_max"), PollSource::None);
}

#[test]
fn slow_interval_without_tiers_warns() {
    let m = sensor("t1")
        .slow_poll_interval(Duration::from_secs(10))
        .poll_all(Duration::from_secs(1));
    let node = node().module(m).unwrap().build().unwrap();
    assert_eq!(node.warnings().len(), 1);
    assert_eq!(node.warnings()[0].path, "t1");
}

#[test]
fn valid_pollinterval_drives_polling() {
    let m = controller("cryo")
        .parameter(Parameter::new("pollinterval", seconds(), "poll interval").writable())
        .unwrap()
        .poll_interval(Duration::from_millis(1500));
    let node = node().module(m).unwrap().build().unwrap();
    assert_eq!(node.warnings(), []);
    let module = &node.model.modules["cryo"];
    assert!(module.poll.driven);
    let param = module.parameter("pollinterval").unwrap();
    assert_eq!(param.poll, PollSource::None, "never polled");
    assert_eq!(
        param.initial,
        Some(Value::Double(1.5)),
        "seeded from the module"
    );
    assert_eq!(
        node.structure_report()["modules"]["cryo"]["accessibles"]["pollinterval"]["datainfo"],
        json!({"type": "double", "min": 0.0, "max": 3600.0, "unit": "s"})
    );
}

#[test]
fn pollinterval_alone_uses_the_default_tiers() {
    let m = controller("cryo")
        .parameter(Parameter::new("pollinterval", seconds(), "poll interval").writable())
        .unwrap();
    let node = node().module(m).unwrap().build().unwrap();
    let module = &node.model.modules["cryo"];
    assert!(module.poll.driven);
    assert_eq!(poll_of(&node, "cryo", "value"), PollSource::Fast);
    assert_eq!(poll_of(&node, "cryo", "target"), PollSource::Slow);
    assert_eq!(
        module.parameter("pollinterval").unwrap().initial,
        Some(Value::Double(5.0))
    );
}

#[test]
fn integer_pollinterval_is_ordinary() {
    let m = sensor("t1")
        .parameter(Parameter::new(
            "pollinterval",
            di(json!({"type": "int", "min": 0, "max": 100})),
            "p",
        ))
        .unwrap()
        .poll_all(Duration::from_secs(1));
    let node = node().module(m).unwrap().build().unwrap();
    assert_eq!(node.warnings().len(), 1);
    assert_eq!(node.warnings()[0].path, "t1:pollinterval");
    let module = &node.model.modules["t1"];
    assert!(!module.poll.driven);
    assert_eq!(module.parameter("pollinterval").unwrap().initial, None);
}
