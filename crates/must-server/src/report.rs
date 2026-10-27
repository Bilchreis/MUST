// SPDX-License-Identifier: LGPL-3.0-or-later

//! The structure report sent in reply to `describe`.

use serde_json::{Map, Value as Json};

use crate::model::{AccessibleModel, NodeModel};

/// Builds the structure report: node properties, then `modules`, each with its
/// properties and `accessibles`, all in declaration order.
pub(crate) fn structure_report(model: &NodeModel) -> Json {
    let mut node: Map<String, Json> = model
        .props
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    let modules: Map<String, Json> = model
        .modules
        .iter()
        .map(|(name, module)| {
            let mut m: Map<String, Json> = module
                .props
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            let accessibles: Map<String, Json> = module
                .accessibles
                .iter()
                .map(|(name, accessible)| {
                    let (datainfo, props) = match accessible {
                        AccessibleModel::Parameter(p) => (&p.datainfo, &p.props),
                        AccessibleModel::Command(c) => (&c.datainfo, &c.props),
                    };
                    // description and datainfo first, then the other properties
                    let mut a = Map::new();
                    if let Some(d) = props.get("description") {
                        a.insert("description".into(), d.clone());
                    }
                    a.insert("datainfo".into(), datainfo.to_json());
                    for (k, v) in props {
                        if k != "description" {
                            a.insert(k.clone(), v.clone());
                        }
                    }
                    (name.clone(), Json::Object(a))
                })
                .collect();
            m.insert("accessibles".into(), Json::Object(accessibles));
            (name.clone(), Json::Object(m))
        })
        .collect();
    node.insert("modules".into(), Json::Object(modules));
    Json::Object(node)
}
