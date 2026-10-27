// SPDX-License-Identifier: LGPL-3.0-or-later

//! Helpers of the C ABI tests: a line-based SECoP client and test serialization.

#![allow(dead_code, reason = "each test binary uses a different part")]

use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use serde_json::Value as Json;

/// The library state is global, so the tests run one at a time.
pub static SERIAL: Mutex<()> = Mutex::new(());

pub fn serial() -> MutexGuard<'static, ()> {
    SERIAL.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A port that was free a moment ago.
pub fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .and_then(|l| l.local_addr())
        .map(|a| a.port())
        .expect("a free port")
}

pub fn addr(port: u16) -> SocketAddr {
    SocketAddr::from(([127, 0, 0, 1], port))
}

/// A line-based SECoP client.
pub struct Client {
    reader: BufReader<TcpStream>,
    writer: TcpStream,
}

impl Client {
    pub fn connect(port: u16) -> Self {
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

    pub fn send(&mut self, line: &str) {
        self.writer.write_all(line.as_bytes()).unwrap();
        self.writer.write_all(b"\n").unwrap();
    }

    pub fn recv(&mut self) -> String {
        let mut line = String::new();
        self.reader.read_line(&mut line).expect("a message");
        assert!(!line.is_empty(), "the node closed the connection");
        line.trim_end().to_owned()
    }

    /// Receives messages until one starts with `prefix`.
    pub fn recv_until(&mut self, prefix: &str) -> String {
        loop {
            let line = self.recv();
            if line.starts_with(prefix) {
                return line;
            }
        }
    }

    pub fn request(&mut self, line: &str) -> String {
        self.send(line);
        self.recv()
    }

    pub fn describe(&mut self) -> Json {
        let reply = self.request("describe");
        let json = reply
            .strip_prefix("describing . ")
            .unwrap_or_else(|| panic!("unexpected reply {reply}"));
        serde_json::from_str(json).unwrap()
    }
}

/// The JSON part of a message `<action> <specifier> <json>`.
pub fn data(message: &str) -> Json {
    let json = message.splitn(3, ' ').nth(2).expect("a data part");
    serde_json::from_str(json).unwrap()
}

pub fn closed(port: u16) -> bool {
    TcpStream::connect_timeout(&addr(port), Duration::from_secs(2)).is_err()
}
