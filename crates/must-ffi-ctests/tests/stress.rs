// SPDX-License-Identifier: LGPL-3.0-or-later

//! Stress test of the `SECoP_S_*` functions: eight threads push updates, poll and answer
//! requests, and send requests over TCP at the same time for 10 s (or
//! `MUST_STRESS_SECS`). Every request must be answered, and no call may fail or hang.

use std::ffi::{c_char, c_int, c_ulonglong};
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use must_ffi_ctests::server as c;

const SUCCESS: c_int = 0;
const NO_DATA: c_int = -10;
const ACTION_READ: c_int = 1;

unsafe extern "C" {
    fn SECoP_S_updateParameter2(
        name: *const c_char,
        value: *const c_char,
        value_size: c_int,
        sigma: *const c_char,
        sigma_size: c_int,
        timestamp: f64,
    ) -> c_int;
    fn SECoP_S_getStoredCommand2(
        id: *mut c_ulonglong,
        action: *mut c_int,
        name: *mut c_char,
        name_size: *mut c_int,
        value: *mut c_char,
        value_size: *mut c_int,
        context: *const c_char,
    ) -> c_int;
    fn SECoP_S_putCommandAnswer2(
        id: c_ulonglong,
        error: c_int,
        value: *const c_char,
        value_size: c_int,
        sigma: *const c_char,
        sigma_size: c_int,
        timestamp: f64,
    ) -> c_int;
    fn SECoP_S_doneLibrary(node_only: c_int, context: *const c_char);
}

fn duration() -> Duration {
    let secs = std::env::var("MUST_STRESS_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(10);
    Duration::from_secs(secs)
}

/// Pushes new values, about every 0.1 ms.
fn pusher(stop: &AtomicBool, count: &AtomicU64) {
    let mut i = 0u32;
    while !stop.load(Ordering::Relaxed) {
        let value = format!("{i}.5");
        // SAFETY: NUL-terminated name, value of the given length
        let rc = unsafe {
            SECoP_S_updateParameter2(
                c"snode:m:value".as_ptr(),
                value.as_ptr().cast(),
                c_int::try_from(value.len()).unwrap(),
                std::ptr::null(),
                0,
                f64::NAN,
            )
        };
        assert_eq!(rc, SUCCESS, "SECoP_S_updateParameter2");
        count.fetch_add(1, Ordering::Relaxed);
        i = i.wrapping_add(1);
        // paced, so that the activated client keeps up and is not disconnected as a
        // slow client (a different test)
        std::thread::sleep(Duration::from_micros(100));
    }
}

/// Takes requests with `SECoP_S_getStoredCommand2` and answers them; keeps going until
/// the clients have stopped and the queue is empty.
fn poller(clients_done: &AtomicBool, count: &AtomicU64) {
    let mut name = [0 as c_char; 64];
    let mut value = [0 as c_char; 64];
    loop {
        let (mut id, mut action) = (0, 0);
        let (mut name_size, mut value_size) = (64, 64);
        // SAFETY: buffers of the given sizes, NUL-terminated context
        let rc = unsafe {
            SECoP_S_getStoredCommand2(
                &raw mut id,
                &raw mut action,
                name.as_mut_ptr(),
                &raw mut name_size,
                value.as_mut_ptr(),
                &raw mut value_size,
                c"stress".as_ptr(),
            )
        };
        match rc {
            SUCCESS => {
                // a read is answered with 1.0, a change with the requested value
                let answer = if action == ACTION_READ {
                    c"1.0".as_ptr()
                } else {
                    value.as_ptr()
                };
                // SAFETY: NUL-terminated answer
                let rc = unsafe {
                    SECoP_S_putCommandAnswer2(id, SUCCESS, answer, -1, std::ptr::null(), 0, 0.0)
                };
                assert_eq!(rc, SUCCESS, "SECoP_S_putCommandAnswer2");
                count.fetch_add(1, Ordering::Relaxed);
            }
            NO_DATA if clients_done.load(Ordering::Relaxed) => return,
            NO_DATA => std::thread::yield_now(),
            other => panic!("SECoP_S_getStoredCommand2 returned {other}"),
        }
    }
}

/// Sends `read` and `change` requests and checks every reply; an activated client also
/// receives the pushed updates.
fn client(port: u16, activate: bool, stop: &AtomicBool, count: &AtomicU64) {
    let stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(30)))
        .unwrap();
    let mut writer = stream.try_clone().unwrap();
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    let mut next = |reader: &mut BufReader<TcpStream>| {
        loop {
            line.clear();
            reader.read_line(&mut line).expect("a reply within 30 s");
            assert!(!line.is_empty(), "the node closed the connection");
            if !line.starts_with("update ") {
                return line.trim_end().to_owned();
            }
        }
    };
    if activate {
        writer.write_all(b"activate\n").unwrap();
        while next(&mut reader) != "active" {}
    }
    let mut i = 0u32;
    while !stop.load(Ordering::Relaxed) {
        let (request, expected) = if i.is_multiple_of(2) {
            ("read m:value\n".to_owned(), "reply m:value ")
        } else {
            (format!("change m:target {i}\n"), "changed m:target ")
        };
        writer.write_all(request.as_bytes()).unwrap();
        let reply = next(&mut reader);
        assert!(reply.starts_with(expected), "{request} -> {reply}");
        count.fetch_add(1, Ordering::Relaxed);
        i += 1;
    }
}

#[test]
fn eight_threads_for_ten_seconds() {
    let port = TcpListener::bind("127.0.0.1:0")
        .and_then(|l| l.local_addr())
        .unwrap()
        .port();
    // SAFETY: a C test with a port argument
    let line = unsafe { c::ctest_server_stress_build(port) };
    assert_eq!(
        line, 0,
        "C test stress_build failed at c/server.c line {line}"
    );

    let stop = Arc::new(AtomicBool::new(false));
    let clients_done = Arc::new(AtomicBool::new(false));
    let pushes = Arc::new(AtomicU64::new(0));
    let answers = Arc::new(AtomicU64::new(0));
    let replies = Arc::new(AtomicU64::new(0));

    let mut workers = Vec::new();
    for _ in 0..2 {
        let (stop, pushes) = (Arc::clone(&stop), Arc::clone(&pushes));
        workers.push(std::thread::spawn(move || pusher(&stop, &pushes)));
    }
    let mut pollers = Vec::new();
    for _ in 0..2 {
        let (done, answers) = (Arc::clone(&clients_done), Arc::clone(&answers));
        pollers.push(std::thread::spawn(move || poller(&done, &answers)));
    }
    let mut clients = Vec::new();
    for k in 0..4 {
        let (stop, replies) = (Arc::clone(&stop), Arc::clone(&replies));
        clients.push(std::thread::spawn(move || {
            client(port, k == 0, &stop, &replies);
        }));
    }

    let start = Instant::now();
    while start.elapsed() < duration() {
        assert!(
            workers
                .iter()
                .chain(&pollers)
                .chain(&clients)
                .all(|t| !t.is_finished()),
            "a thread ended early"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    stop.store(true, Ordering::Relaxed);
    for t in clients {
        t.join().expect("client thread");
    }
    clients_done.store(true, Ordering::Relaxed);
    for t in workers.into_iter().chain(pollers) {
        t.join().expect("worker thread");
    }
    // SAFETY: NUL-terminated context
    unsafe { SECoP_S_doneLibrary(0, c"stress".as_ptr()) };

    let (pushes, answers, replies) = (
        pushes.load(Ordering::Relaxed),
        answers.load(Ordering::Relaxed),
        replies.load(Ordering::Relaxed),
    );
    println!("{pushes} pushes, {answers} answers, {replies} replies");
    assert!(pushes > 0 && replies > 0);
    // every request a client sent was answered by a poller
    assert_eq!(answers, replies);
}
