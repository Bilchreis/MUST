// SPDX-License-Identifier: LGPL-3.0-or-later

//! Runs the C tests of the `SECoP_S_*` functions (c/server.c) and checks the nodes they
//! build over TCP, as a SECoP client would.

use std::ffi::{CStr, c_int};
use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use must_ffi_ctests::server as c;
use serde_json::Value as Json;

/// The library state is global, so the tests run one at a time.
static SERIAL: Mutex<()> = Mutex::new(());

fn serial() -> MutexGuard<'static, ()> {
    SERIAL.lock().unwrap_or_else(PoisonError::into_inner)
}

fn check(test: &str, line: c_int) {
    assert_eq!(line, 0, "C test {test} failed at c/server.c line {line}");
}

fn done(context: &CStr) {
    // SAFETY: a C test taking a NUL-terminated string
    check("done", unsafe { c::ctest_server_done(context.as_ptr()) });
}

/// A port that was free a moment ago.
fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .and_then(|l| l.local_addr())
        .map(|a| a.port())
        .expect("a free port")
}

fn addr(port: u16) -> SocketAddr {
    SocketAddr::from(([127, 0, 0, 1], port))
}

/// A line-based SECoP client.
struct Client {
    reader: BufReader<TcpStream>,
    writer: TcpStream,
}

impl Client {
    fn connect(port: u16) -> Self {
        let stream = TcpStream::connect_timeout(&addr(port), Duration::from_secs(5))
            .expect("the node accepts connections");
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        Self {
            reader: BufReader::new(stream.try_clone().unwrap()),
            writer: stream,
        }
    }

    fn send(&mut self, line: &str) {
        self.writer.write_all(line.as_bytes()).unwrap();
        self.writer.write_all(b"\n").unwrap();
    }

    fn recv(&mut self) -> String {
        let mut line = String::new();
        self.reader.read_line(&mut line).expect("a message");
        assert!(!line.is_empty(), "the node closed the connection");
        line.trim_end().to_owned()
    }

    /// Receives messages until one starts with `prefix`.
    fn recv_until(&mut self, prefix: &str) -> String {
        loop {
            let line = self.recv();
            if line.starts_with(prefix) {
                return line;
            }
        }
    }

    fn request(&mut self, line: &str) -> String {
        self.send(line);
        self.recv()
    }

    fn describe(&mut self) -> Json {
        let reply = self.request("describe");
        let json = reply
            .strip_prefix("describing . ")
            .unwrap_or_else(|| panic!("unexpected reply {reply}"));
        serde_json::from_str(json).unwrap()
    }
}

/// The JSON part of a message `<action> <specifier> <json>`.
fn data(message: &str) -> Json {
    let json = message.splitn(3, ' ').nth(2).expect("a data part");
    serde_json::from_str(json).unwrap()
}

fn closed(port: u16) -> bool {
    TcpStream::connect_timeout(&addr(port), Duration::from_secs(2)).is_err()
}

#[test]
fn basics() {
    let _serial = serial();
    // SAFETY: a C test without arguments
    check("basics", unsafe { c::ctest_server_basics() });
}

#[test]
fn shall_example() {
    let _serial = serial();
    let port = free_port();
    // SAFETY: a C test with a port argument
    check("example", unsafe { c::ctest_server_example(port) });
    let description = Client::connect(port).describe();
    assert_eq!(description["equipment_id"], "my_node");
    let value = &description["modules"]["mod1"]["accessibles"]["value"];
    assert_eq!(value["description"], "actual value");
    assert_eq!(value["datainfo"]["type"], "double");
    assert_eq!(value["datainfo"]["unit"], "mm");
    done(c"example");
    assert!(closed(port));
}

#[test]
fn two_contexts_and_done_for_one() {
    let _serial = serial();
    let (port1, port2) = (free_port(), free_port());
    // SAFETY: a C test with port arguments
    check("two_contexts", unsafe {
        c::ctest_server_two_contexts(port1, port2)
    });
    let node1 = Client::connect(port1).describe();
    let node2 = Client::connect(port2).describe();
    assert_eq!(node1["modules"]["m1"]["description"], "module of id1");
    assert_eq!(node1["modules"]["m1"]["_tag"], "one");
    assert_eq!(
        node1["modules"]["m1"]["accessibles"]["value"]["description"],
        "value of id1"
    );
    assert_eq!(node2["modules"]["m2"]["description"], "module of id2");
    assert_eq!(
        node2["modules"]["m2"]["accessibles"]["value"]["datainfo"]["type"],
        "int"
    );

    let mut client1 = Client::connect(port1);
    // SAFETY: a C test taking a NUL-terminated string
    check("done_nodes", unsafe {
        c::ctest_server_done_nodes(c"id2".as_ptr())
    });
    assert!(closed(port2), "the node of id2 stopped");
    assert!(client1.request("ping p").starts_with("pong p"));
    assert_eq!(Client::connect(port1).describe()["equipment_id"], "node1");
    done(c"id2");
    done(c"id1");
    assert!(closed(port1));
}

#[test]
fn two_threads_build_at_the_same_time() {
    let _serial = serial();
    let (port1, port2) = (free_port(), free_port());
    let build = |context: &'static CStr, node: &'static CStr, port| {
        std::thread::spawn(move || {
            // SAFETY: a C test taking NUL-terminated strings and a port
            unsafe { c::ctest_server_build_many(context.as_ptr(), node.as_ptr(), port) }
        })
    };
    let a = build(c"ctxa", c"nodea", port1);
    let b = build(c"ctxb", c"nodeb", port2);
    check("build_many a", a.join().unwrap());
    check("build_many b", b.join().unwrap());
    for (port, context) in [(port1, "ctxa"), (port2, "ctxb")] {
        let description = Client::connect(port).describe();
        let modules = description["modules"].as_object().unwrap();
        assert_eq!(modules.len(), 20);
        for module in modules.values() {
            assert_eq!(module["description"], context);
            assert_eq!(module["accessibles"]["value"]["description"], context);
        }
    }
    done(c"ctxa");
    done(c"ctxb");
}

#[test]
fn validation_failure_and_warnings() {
    let _serial = serial();
    let port = free_port();
    // SAFETY: a C test with a port argument
    check("validation_fails", unsafe {
        c::ctest_server_validation_fails(port)
    });
    assert!(closed(port), "an invalid node does not listen");
    // SAFETY: a C test without arguments
    check("validation_fixed", unsafe {
        c::ctest_server_validation_fixed()
    });
    let description = Client::connect(port).describe();
    let module = &description["modules"]["m"];
    assert_eq!(module["description"], "now with description");
    assert_eq!(module["myprop"], "x", "custom property published as given");
    assert_eq!(module["visibility"], "advanced");
    assert_eq!(module["accessibles"]["value"]["_flag"], true);
    done(c"val");
}

#[test]
fn callbacks() {
    let _serial = serial();
    let port = free_port();
    // SAFETY: a C test with a port argument
    check("callbacks", unsafe { c::ctest_server_callbacks(port) });
    let mut client = Client::connect(port);

    // getter with uncertainty
    let reply = client.request("read mod1:value");
    assert!(reply.starts_with("reply mod1:value "), "{reply}");
    let report = data(&reply);
    assert_eq!(report[0], 4.2);
    assert_eq!(report[1]["e"], 0.1);
    assert!(report[1]["t"].as_f64().unwrap() > 1.0e9);

    // setter and getter with timestamp
    let reply = client.request("change mod1:target 5");
    assert!(reply.starts_with("changed mod1:target "), "{reply}");
    assert_eq!(data(&reply)[0], 5.0);
    let reply = client.request("read mod1:target");
    assert_eq!(data(&reply)[0], 5.0);
    assert_eq!(data(&reply)[1]["t"], 1000.5);

    // an error code of the setter
    let reply = client.request("change mod1:target 500");
    assert!(reply.starts_with("error_change mod1:target "), "{reply}");
    assert_eq!(data(&reply)[0], "IsBusy");

    // commands
    let reply = client.request("do mod1:reset");
    assert!(reply.starts_with("done mod1:reset "), "{reply}");
    let reply = client.request("do mod1:plus1 2");
    assert_eq!(data(&reply)[0], 3.0);

    // a slow callback does not hold up other modules
    let mut slow = Client::connect(port);
    slow.send("read slow:value");
    std::thread::sleep(Duration::from_millis(100));
    let start = Instant::now();
    let reply = client.request("read mod1:value");
    assert!(reply.starts_with("reply mod1:value "));
    assert!(start.elapsed() < Duration::from_millis(500));
    assert!(slow.recv().starts_with("reply slow:value "));

    // pollinterval: the getter is called and the result sent as update
    let mut active = Client::connect(port);
    active.send("activate counter");
    active.recv_until("active");
    let first = data(&active.recv_until("update counter:value "))[0]
        .as_i64()
        .unwrap();
    let second = data(&active.recv_until("update counter:value "))[0]
        .as_i64()
        .unwrap();
    assert!(second > first);

    done(c"cb");
    assert!(closed(port));
}

#[test]
fn missing_functions() {
    let _serial = serial();
    // SAFETY: a C test without arguments
    check("missing_functions", unsafe {
        c::ctest_server_missing_functions()
    });
}

/// Sends a request on another thread, runs a step of the C polling test and returns the
/// reply.
fn poll_step(port: u16, request: &'static str, step: c_int) -> String {
    let client = std::thread::spawn(move || Client::connect(port).request(request));
    // SAFETY: a C test with a step number
    check(&format!("poll step {step}"), unsafe {
        c::ctest_server_poll_step(step)
    });
    client.join().unwrap()
}

#[test]
fn polling() {
    let _serial = serial();
    let port = free_port();
    // SAFETY: a C test with a port argument
    check("polling", unsafe { c::ctest_server_polling(port) });

    let mut active = Client::connect(port);
    active.send("activate mod1");
    let initial = active.recv_until("update mod1:value ");
    assert_eq!(
        data(&initial)[0],
        1.5,
        "the value pushed before nodeComplete"
    );
    active.recv_until("active");

    let reply = poll_step(port, "change mod1:target 500", 1);
    assert!(reply.starts_with("changed mod1:target "), "{reply}");
    assert_eq!(data(&reply)[0], 500.0);

    let reply = poll_step(port, "read mod1:value", 2);
    assert!(reply.starts_with("reply mod1:value "), "{reply}");
    assert_eq!(data(&reply)[0], 2.5);
    assert_eq!(data(&reply)[1]["e"], 0.25);
    assert_eq!(data(&reply)[1]["t"], 1000.5);

    let reply = poll_step(port, "do mod1:go 3", 3);
    assert!(reply.starts_with("done mod1:go "), "{reply}");

    let reply = poll_step(port, "change mod1:target 7", 4);
    assert!(reply.starts_with("error_change mod1:target "), "{reply}");
    assert_eq!(data(&reply)[0], "IsBusy");

    // unanswered: the client gets a timeout, a late answer is refused
    must_ffi::testing::set_poll_timeout(Duration::from_millis(300));
    let reply = poll_step(port, "read mod1:value", 5);
    must_ffi::testing::set_poll_timeout(Duration::from_secs(60));
    assert!(reply.starts_with("error_read mod1:value "), "{reply}");
    assert_eq!(data(&reply)[0], "TimeoutError");
    // SAFETY: a C test with a step number
    check("poll step 6", unsafe { c::ctest_server_poll_step(6) });

    // pushed updates reach activated clients
    // SAFETY: a C test with a step number
    check("poll step 7", unsafe { c::ctest_server_poll_step(7) });
    let update = data(&active.recv_until("update mod1:value [3.5"));
    assert!(update[1]["t"].as_f64().unwrap() > 1.0e9, "NaN means now");
    let update = data(&active.recv_until("update mod1:value "));
    assert_eq!(update[0], 4.5);
    assert_eq!(update[1]["t"], 1234.5);
    assert_eq!(update[1]["e"], 0.5);

    done(c"poll");
    assert!(closed(port));
}
