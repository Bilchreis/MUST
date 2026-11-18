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
    for bad in ["", "ww", "wwww", "rwr", "-rw", "User"] {
        let k = kinds(node().module(sensor("t1").visibility(bad)).unwrap());
        assert_eq!(k.len(), 1, "{bad:?}");
    }
}

#[test]
fn every_visibility_of_the_specification() {
    for visibility in [
        "www", "wwr", "ww-", "wrr", "wr-", "w--", "rrr", "rr-", "r--", "---", "user", "advanced",
        "expert",
    ] {
        let module = sensor("t1")
            .parameter(Parameter::new("p", kelvin(), "x").visibility(visibility))
            .unwrap()
            .visibility(visibility);
        let node = node().module(module).unwrap().build().unwrap();
        let t1 = &node.structure_report()["modules"]["t1"];
        assert_eq!(t1["visibility"], json!(visibility));
        assert_eq!(t1["accessibles"]["p"]["visibility"], json!(visibility));
    }
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

/// A module and its `value`, both with the same `meaning`.
fn with_meaning(meaning: &Json) -> NodeBuilder {
    let module = Module::new("t1", "a temperature sensor")
        .interface_classes(["Readable"])
        .property("meaning", meaning.clone())
        .unwrap()
        .parameter(
            Parameter::new("value", kelvin(), "temperature")
                .property("meaning", meaning.clone())
                .unwrap(),
        )
        .unwrap()
        .parameter(Parameter::new("status", status_info(), "status"))
        .unwrap();
    node().module(module).unwrap()
}

#[test]
fn meaning_combinations_of_the_specification() {
    let allowed = [
        "function importance belongs_to",
        "function importance",
        "key link",
        "link",
        "function importance link",
        "function importance key link",
        "function importance belongs_to link",
        "function importance belongs_to key link",
    ];
    for keys in allowed {
        let meaning: serde_json::Map<String, Json> = keys
            .split(' ')
            .map(|key| {
                let value = if key == "importance" {
                    json!(20)
                } else {
                    json!("x")
                };
                (key.to_owned(), value)
            })
            .collect();
        let node = with_meaning(&Json::Object(meaning)).build().unwrap();
        assert_eq!(node.warnings(), [], "{keys:?}");
    }
    // an unknown function and belongs_to are fine: both lists are open
    let open = json!({"function": "_mine_regulation", "importance": 0, "belongs_to": "x"});
    assert!(with_meaning(&open).build().is_ok());
}

#[test]
fn invalid_meanings() {
    let invalid = [
        json!({}),
        json!({"importance": 20}),
        json!({"key": "k"}),
        json!({"function": "temperature", "importance": 99}),
        json!({"function": "temperature", "importance": -1}),
        json!({"function": "temperature"}),
        json!({"link": "https://w3id.org/x", "belongs_to": "sample"}),
        json!({"function": 3, "importance": "x"}),
        json!({"link": 1}),
        json!(["temperature", 10]),
    ];
    for meaning in invalid {
        let e = errors(with_meaning(&meaning));
        let paths: Vec<&str> = e.iter().map(|e| e.path.as_str()).collect();
        assert!(paths.contains(&"t1.meaning"), "{meaning}: {paths:?}");
        assert!(paths.contains(&"t1:value.meaning"), "{meaning}: {paths:?}");
        assert!(
            e.iter().all(|e| matches!(&e.kind,
                ModelErrorKind::InvalidProperty { property, .. } if property == "meaning")),
            "{meaning}: {e:?}"
        );
    }
}

#[test]
fn unknown_meaning_key_warns() {
    let meaning = json!({"link": "https://w3id.org/x", "colour": "red", "_mine": 1});
    let node = with_meaning(&meaning).build().unwrap();
    let paths: Vec<&str> = node.warnings().iter().map(|w| w.path.as_str()).collect();
    assert_eq!(paths, ["t1.meaning", "t1:value.meaning"]);
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

fn go() -> Command {
    Command::new("go", DataInfo::command().build().unwrap(), "start")
}

/// A module with `value` and `status` and the given classes.
fn acquisition(name: &str, classes: &[&str]) -> Module {
    Module::new(name, "an acquisition module")
        .interface_classes(classes.iter().copied())
        .parameter(Parameter::new("value", kelvin(), "counts"))
        .unwrap()
        .parameter(Parameter::new("status", status_info(), "status"))
        .unwrap()
}

/// What each `InterfaceClass` error of a failed build requires.
fn class_requirements(builder: NodeBuilder) -> Vec<(String, String)> {
    kinds(builder)
        .into_iter()
        .filter_map(|k| match k {
            ModelErrorKind::InterfaceClass { class, what } => Some((class, what)),
            _ => None,
        })
        .collect()
}

#[test]
fn acquisition_classes() {
    let controller = Module::new("ctrl", "a controller")
        .interface_classes(["AcquisitionController"])
        .property("acquisition_channels", json!({"t": "timer"}))
        .unwrap()
        .parameter(Parameter::new("status", status_info(), "status"))
        .unwrap()
        .command(go())
        .unwrap();
    let channel = acquisition("timer", &["AcquisitionChannel", "Readable"]);
    let single = acquisition("single", &["Acquisition", "Readable"])
        .command(go())
        .unwrap();
    let built = node()
        .module(controller)
        .unwrap()
        .module(channel)
        .unwrap()
        .module(single)
        .unwrap()
        .build()
        .unwrap();
    assert_eq!(built.warnings(), []);

    // a controller without status, go and acquisition_channels
    let bare = Module::new("ctrl", "a controller").interface_classes(["AcquisitionController"]);
    let reqs = class_requirements(node().module(bare).unwrap());
    let reqs: Vec<&str> = reqs.iter().map(|(_, what)| what.as_str()).collect();
    assert_eq!(
        reqs,
        [
            "a parameter `status`",
            "a command `go`",
            "the module property `acquisition_channels`"
        ]
    );

    // a channel without value or status (checked even though `Readable` would be too)
    let channel = Module::new("ch", "a channel").interface_classes(["AcquisitionChannel"]);
    let reqs = class_requirements(node().module(channel).unwrap());
    assert_eq!(reqs.len(), 2, "{reqs:?}");
    assert!(reqs.iter().all(|(class, _)| class == "AcquisitionChannel"));

    // an Acquisition without go, and with acquisition_channels
    let single = acquisition("single", &["Acquisition", "Readable"])
        .property("acquisition_channels", json!({"t": "single"}))
        .unwrap();
    let reqs = class_requirements(node().module(single).unwrap());
    let reqs: Vec<&str> = reqs.iter().map(|(_, what)| what.as_str()).collect();
    assert_eq!(
        reqs,
        [
            "a command `go`",
            "no module property `acquisition_channels`"
        ]
    );
}

#[test]
fn acquisition_channels_name_modules_of_the_node() {
    let controller = Module::new("ctrl", "a controller")
        .interface_classes(["AcquisitionController"])
        .property("acquisition_channels", json!({"t": "timer", "m": "nope"}))
        .unwrap()
        .parameter(Parameter::new("status", status_info(), "status"))
        .unwrap()
        .command(go())
        .unwrap();
    let channel = acquisition("timer", &["AcquisitionChannel", "Readable"]);
    let e = errors(node().module(controller).unwrap().module(channel).unwrap());
    assert_eq!(e.len(), 1, "{e:?}");
    assert_eq!(e[0].path, "ctrl.acquisition_channels");
    assert!(e[0].to_string().contains("`nope`"), "{}", e[0]);
}

#[test]
fn communicator_without_communicate_warns() {
    let bare = Module::new("io", "serial line").interface_classes(["Communicator"]);
    let built = node().module(bare).unwrap().build().unwrap();
    assert_eq!(built.warnings().len(), 1);
    assert_eq!(built.warnings()[0].path, "io");

    let string = DataInfo::string().build().unwrap();
    let communicate = DataInfo::command()
        .argument(&string)
        .result(&string)
        .build()
        .unwrap();
    let io = Module::new("io", "serial line")
        .interface_classes(["Communicator"])
        .command(Command::new("communicate", communicate, "send and receive"))
        .unwrap();
    let built = node().module(io).unwrap().build().unwrap();
    assert_eq!(built.warnings(), []);
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
    // every SECoP 2.0 class name is known (the acquisition classes need more accessibles)
    for class in INTERFACE_CLASSES {
        let module = sensor("t1").interface_classes([*class, "Readable"]);
        if let Err(BuildError(e)) = node().module(module).unwrap().build() {
            assert!(
                e.iter()
                    .all(|e| !matches!(e.kind, ModelErrorKind::UnknownInterfaceClass(_))),
                "{class}: {e:?}"
            );
        }
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

/// A Readable `t1` whose status enum has the given members.
fn with_status(members: &Json) -> NodeBuilder {
    let info = di(json!({"type": "tuple", "members": [
        {"type": "enum", "members": members}, {"type": "string"}]}));
    let module = Module::new("t1", "x")
        .interface_classes(["Readable"])
        .parameter(Parameter::new("value", kelvin(), "v"))
        .unwrap()
        .parameter(Parameter::new("status", info, "s"))
        .unwrap();
    node().module(module).unwrap()
}

#[test]
fn status_names_of_the_table() {
    // the table's names, and codes it does not name, are fine
    let fine = json!({"DISABLED": 0, "IDLE": 100, "STANDBY": 130, "WARN": 200,
        "BUSY": 300, "RAMPING": 370, "ERROR": 400, "_MINE": 376, "HOLDING": 160});
    assert_eq!(with_status(&fine).build().unwrap().warnings(), []);

    // a group code or name used otherwise is an error
    let group = [
        json!({"idle": 100}),
        json!({"READY": 100}),
        json!({"BUSY": 370}),
        json!({"IDLE": 130}),
        json!({"ERROR": 450}),
    ];
    for members in group {
        let k = kinds(with_status(&members));
        assert_eq!(k.len(), 1, "{members}: {k:?}");
        assert!(
            matches!(&k[0], ModelErrorKind::StatusShape(_)),
            "{members}: {k:?}"
        );
    }

    // another code or name of the table used otherwise is a warning
    let others = [
        json!({"IDLE": 100, "READY": 150}),
        json!({"IDLE": 100, "PREPARED": 160}),
        json!({"IDLE": 100, "RAMPING": 360}),
    ];
    for members in others {
        let built = with_status(&members).build().unwrap();
        let paths: Vec<&str> = built.warnings().iter().map(|w| w.path.as_str()).collect();
        assert_eq!(paths, ["t1:status"], "{members}");
    }
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
    assert!(node().module(module).unwrap().build().is_ok());
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

/// No UDP test can use the default port, so the default is checked in the model.
#[test]
fn discovery_is_on_by_default() {
    let node = NodeBuilder::new("HZB-vm4", "test node").build().unwrap();
    assert!(node.model.options.discovery.is_enabled());
}

// --- predefined accessibles with a fixed data type

fn enum_info(members: &Json) -> DataInfo {
    di(json!({"type": "enum", "members": members}))
}

/// A Drivable `name` with the extra parameter.
fn controller_with(name: &str, extra: Parameter) -> Module {
    controller(name).parameter(extra).unwrap()
}

#[test]
fn controlled_by() {
    let good = Parameter::new(
        "controlled_by",
        enum_info(&json!({"self": 0, "t2": 1})),
        "who controls t1",
    );
    let built = node()
        .module(controller_with("t1", good))
        .unwrap()
        .module(controller("t2"))
        .unwrap()
        .build()
        .unwrap();
    assert_eq!(built.warnings(), []);

    let bad = [
        Parameter::new("controlled_by", DataInfo::string().build().unwrap(), "x"),
        Parameter::new("controlled_by", enum_info(&json!({"t2": 0})), "x"),
        Parameter::new(
            "controlled_by",
            enum_info(&json!({"self": 1, "t2": 0})),
            "x",
        ),
        Parameter::new("controlled_by", enum_info(&json!({"self": 0})), "x").writable(),
    ];
    for param in bad {
        let e = errors(node().module(controller_with("t1", param)).unwrap());
        assert_eq!(e.len(), 1, "{e:?}");
        assert_eq!(e[0].path, "t1:controlled_by");
        assert!(matches!(
            e[0].kind,
            ModelErrorKind::PredefinedAccessible {
                name: "controlled_by",
                ..
            }
        ));
    }

    // a member that is no module of this node may be on another node: a warning
    let elsewhere = Parameter::new(
        "controlled_by",
        enum_info(&json!({"self": 0, "remote": 1})),
        "x",
    );
    let built = node()
        .module(controller_with("t1", elsewhere))
        .unwrap()
        .build()
        .unwrap();
    assert_eq!(built.warnings().len(), 1);
    assert_eq!(built.warnings()[0].path, "t1:controlled_by");
}

#[test]
fn control_active_and_mode() {
    let bool_info = DataInfo::bool().build().unwrap();
    let int_info = di(json!({"type": "int", "min": 0, "max": 1}));
    let mode = enum_info(&json!({"off": 0, "on": 1}));
    let fitting = [
        Parameter::new("control_active", bool_info.clone(), "x"),
        Parameter::new("mode", mode.clone(), "x"),
        Parameter::new("mode", mode, "x").writable(),
    ];
    for param in fitting {
        let built = node()
            .module(controller_with("t1", param))
            .unwrap()
            .build()
            .unwrap();
        assert_eq!(built.warnings(), []);
    }
    let unfitting = [
        (
            "control_active",
            Parameter::new("control_active", int_info, "x"),
        ),
        (
            "control_active",
            Parameter::new("control_active", bool_info, "x").writable(),
        ),
        (
            "mode",
            Parameter::new("mode", DataInfo::string().build().unwrap(), "x"),
        ),
    ];
    for (name, param) in unfitting {
        let e = errors(node().module(controller_with("t1", param)).unwrap());
        assert_eq!(e.len(), 1, "{name}: {e:?}");
        assert_eq!(e[0].path, format!("t1:{name}"));
        assert!(
            matches!(e[0].kind, ModelErrorKind::PredefinedAccessible { name: n, .. } if n == name),
            "{name}: {e:?}"
        );
    }
}
