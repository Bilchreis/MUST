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
fn unknown_class_before_base_class() {
    let module = sensor("t1").interface_classes(["MyHeater", "Readable"]);
    assert!(node().module(module).unwrap().build().is_ok());
    let module = Module::new("t1", "x").interface_classes(["Readable"]);
    assert_eq!(
        kinds(node().module(module).unwrap()).len(),
        2,
        "value and status missing"
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
    let node = node().module(module).unwrap().build().unwrap();
    let t1 = &node.model.modules["t1"];
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
    let node = node()
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
    let report = node.structure_report();
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
