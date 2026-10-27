// SPDX-License-Identifier: LGPL-3.0-or-later

//! SHALL's own test scenarios (`gtest-serverlib/tst_contextid.cpp` and `tst_protocol.cpp`),
//! run against must: the C side (`c/shall_ported.c`) builds SHALL's "hpd" test node, this
//! side checks it over TCP, with SECoP 2.0 wire output.

mod common;

use std::ffi::{CStr, c_int};

use common::{Client, closed, data, free_port, serial};
use must_ffi_ctests::shall as c;

fn check(test: &str, line: c_int) {
    assert_eq!(
        line, 0,
        "C test {test} failed at c/shall_ported.c line {line}"
    );
}

fn node(context: &CStr, node: &CStr, port: u16, function_pointers: bool) {
    // SAFETY: a C test taking NUL-terminated strings
    let line = unsafe {
        c::ctest_shall_node(
            context.as_ptr(),
            node.as_ptr(),
            port,
            c_int::from(function_pointers),
        )
    };
    check("Node_no_wait", line);
}

fn done(node_only: bool, context: &CStr) {
    // SAFETY: a C test taking a NUL-terminated string
    check("done", unsafe {
        c::ctest_shall_done(c_int::from(node_only), context.as_ptr())
    });
}

fn delete_node(node: &CStr) {
    // SAFETY: a C test taking a NUL-terminated string
    check("deleteNode", unsafe {
        c::ctest_shall_delete_node(node.as_ptr())
    });
}

fn equipment_id(port: u16) -> String {
    Client::connect(port).describe()["equipment_id"]
        .as_str()
        .unwrap()
        .to_owned()
}

/// `tst_contextid.cpp` `concurrent`: two contexts build their nodes at the same time.
#[test]
fn concurrent() {
    let _serial = serial();
    let (port1, port2) = (free_port(), free_port());
    let a = std::thread::spawn(move || node(c"default", c"Node1", port1, true));
    let b = std::thread::spawn(move || node(c"default_n2", c"Node2", port2, true));
    a.join().unwrap();
    b.join().unwrap();
    assert_eq!(equipment_id(port1), "Node1");
    assert_eq!(equipment_id(port2), "Node2");
    let description = Client::connect(port1).describe();
    let hpd = &description["modules"]["hpd"];
    assert_eq!(hpd["interface_classes"][0], "Drivable");
    assert_eq!(
        hpd["accessibles"]["stop"]["datainfo"]["type"], "command",
        "SHALL's default datainfo of a command"
    );
    done(true, c"default");
    done(true, c"default_n2");
    assert!(closed(port1) && closed(port2));
    done(false, c"default");
}

/// `tst_contextid.cpp` `GetCommad`: polled requests reach only their own context.
#[test]
fn get_command() {
    let _serial = serial();
    let (port1, port2) = (free_port(), free_port());
    node(c"id1", c"node1", port1, false);
    node(c"id2", c"node2", port2, false);
    let mut client1 = Client::connect(port1);
    let mut client2 = Client::connect(port2);
    // the command to id2 is sent first
    client2.send("change hpd:target 500");
    client1.send("change hpd:target 500");

    for (context, expected, client) in [
        (c"id1", c"node1:hpd:target", &mut client1),
        (c"id2", c"node2:hpd:target", &mut client2),
    ] {
        // SAFETY: a C test taking NUL-terminated strings
        let line = unsafe { c::ctest_shall_get_command(context.as_ptr(), expected.as_ptr()) };
        check("GetCommad", line);
        let reply = client.recv();
        assert!(reply.starts_with("changed hpd:target "), "{reply}");
        assert_eq!(data(&reply)[0], 500.0);
    }
    done(true, c"id1");
    done(true, c"id2");
    done(false, c"id1");
}

/// `tst_contextid.cpp` `delete_node`: deleting one node leaves the other running.
#[test]
fn delete_one_node() {
    let _serial = serial();
    let (port1, port2) = (free_port(), free_port());
    node(c"delete_one_node1", c"Node1", port1, true);
    node(c"delete_one_node2", c"Node2", port2, true);
    delete_node(c"Node1");
    assert!(closed(port1));
    assert_eq!(equipment_id(port2), "Node2");
    done(false, c"delete_one_node1");
    done(false, c"delete_one_node2");
    assert!(closed(port2));
}

/// `tst_contextid.cpp` `gui_state`, as node lifecycle: a context's node is stopped and
/// built again on the same port while another context's node is deleted.
#[test]
fn node_lifecycle() {
    let _serial = serial();
    let (port1, port2) = (free_port(), free_port());
    node(c"delete_one_node1", c"Node1", port1, true);
    node(c"delete_one_node2", c"Node2", port2, true);
    done(true, c"delete_one_node2");
    assert!(closed(port2));
    node(c"delete_one_node2", c"Node2", port2, true);
    assert_eq!(equipment_id(port2), "Node2");
    delete_node(c"Node1");
    assert!(closed(port1));
    done(true, c"delete_one_node2");
    done(true, c"delete_one_node1");
    assert!(closed(port2));
    done(false, c"delete_one_node1");
}

/// `tst_protocol.cpp`: `change hpd:target 20` through the setter callback.
#[test]
fn protocol_change() {
    let _serial = serial();
    let port = free_port();
    node(c"id1", c"node1", port, true);
    let mut client = Client::connect(port);
    let reply = client.request("change hpd:target 20");
    // SHALL: changed hpd:target [20.0,{"t":1722351020.974}]
    assert!(
        reply.starts_with("changed hpd:target [20.0,{\"t\":"),
        "{reply}"
    );
    let qualifiers = data(&reply)[1].clone();
    assert_eq!(qualifiers.as_object().unwrap().len(), 1, "only t");
    // and the setter's limit
    let reply = client.request("change hpd:target 5000");
    assert_eq!(data(&reply)[0], 1000.0);
    done(false, c"id1");
}
