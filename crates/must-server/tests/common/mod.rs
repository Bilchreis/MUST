// SPDX-License-Identifier: LGPL-3.0-or-later

//! Helpers for end-to-end tests: a simulated module and a line-based SECoP client.

#![allow(dead_code)]

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use must_server::{
    CallResult, Command, Ctx, HandlerResult, Module, ModuleHandler, NodeBuilder, Parameter, Reading,
};
use must_types::{DataInfo, ErrorClass, SecopError, Value};
use serde_json::{Value as Json, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};

#[allow(clippy::needless_pass_by_value)]
pub fn di(json: Json) -> DataInfo {
    DataInfo::from_json(&json).unwrap()
}

pub fn kelvin() -> DataInfo {
    di(json!({"type": "double", "min": 0, "max": 300, "unit": "K"}))
}

pub fn status_info() -> DataInfo {
    di(json!({"type": "tuple", "members": [
        {"type": "enum", "members": {"IDLE": 100, "BUSY": 300, "ERROR": 400}},
        {"type": "string"}]}))
}

pub fn status(code: i64, text: &str) -> Value {
    Value::Tuple(vec![Value::Enum(code), Value::String(text.into())])
}

/// A simulated temperature controller that counts handler calls.
#[derive(Default)]
pub struct Sim {
    pub reads: AtomicUsize,
    pub changes: AtomicUsize,
    pub target: Mutex<f64>,
    /// Size of the string returned by the `big` command.
    pub big_len: usize,
}

#[async_trait]
impl ModuleHandler for Sim {
    async fn read(&self, _ctx: &Ctx, parameter: &str) -> HandlerResult {
        self.reads.fetch_add(1, Ordering::SeqCst);
        match parameter {
            "value" => Ok(Reading::new(Value::Double(295.0)).with_uncertainty(0.01)),
            "status" => Ok(status(100, "idle").into()),
            "target" => Ok(Value::Double(*self.target.lock().unwrap()).into()),
            "broken" => Err(SecopError::new(
                ErrorClass::HardwareError,
                "sensor disconnected",
            )),
            "nocomm" => Err(SecopError::new(
                ErrorClass::CommunicationFailed,
                "no answer from controller",
            )),
            "hang" => std::future::pending().await,
            "slow" => {
                tokio::time::sleep(Duration::from_millis(500)).await;
                Ok(Value::Double(1.0).into())
            }
            "panic" => panic!("simulated bug"),
            "chunk" => Ok(Value::String("y".repeat(2048)).into()),
            other => Err(SecopError::new(
                ErrorClass::ReadFailed,
                format!("no {other}"),
            )),
        }
    }

    async fn change(&self, ctx: &Ctx, parameter: &str, value: Value) -> HandlerResult {
        self.changes.fetch_add(1, Ordering::SeqCst);
        if parameter == "target" {
            ctx.update("status", status(300, "ramping")).unwrap();
            *self.target.lock().unwrap() = value.as_f64().unwrap();
        }
        Ok(value.into())
    }

    async fn call(&self, ctx: &Ctx, command: &str, argument: Option<Value>) -> CallResult {
        match command {
            "stop" => {
                ctx.update("status", status(100, "stopped")).unwrap();
                Ok(None)
            }
            "echo" => Ok(argument.map(Reading::new)),
            "big" => Ok(Some(Value::String("x".repeat(self.big_len)).into())),
            _ => Err(SecopError::new(ErrorClass::NotImplemented, "no such thing")),
        }
    }
}

/// A Drivable module `t1` with value, status, target, `target_max` (cached), extra
/// parameters for error cases, and commands stop, echo and big.
pub fn controller(sim: Arc<Sim>) -> Module {
    let s = DataInfo::string().prop("isUTF8", true).build().unwrap();
    Module::new("t1", "a simulated temperature controller")
        .interface_classes(["Drivable"])
        .parameter(Parameter::new("value", kelvin(), "temperature").initial_value(Value::Double(295.0)))
        .unwrap()
        .parameter(Parameter::new("status", status_info(), "status").initial_value(status(100, "idle")))
        .unwrap()
        .parameter(Parameter::new("target", kelvin(), "setpoint").writable().initial_value(Value::Double(0.0)))
        .unwrap()
        .parameter(
            Parameter::new("target_max", kelvin(), "dynamic upper limit")
                .writable()
                .cached()
                .initial_value(Value::Double(200.0)),
        )
        .unwrap()
        .parameter(
            Parameter::new("mode", di(json!({"type": "struct", "members": {
                "a": {"type": "int", "min": 0, "max": 9}, "b": {"type": "int", "min": 0, "max": 9}}})), "mode")
                .writable(),
        )
        .unwrap()
        .parameter(Parameter::new("serial", s, "serial number").constant("SN-1"))
        .unwrap()
        .parameter(Parameter::new("broken", kelvin(), "always fails"))
        .unwrap()
        .parameter(Parameter::new("nocomm", kelvin(), "fails with CommunicationFailed"))
        .unwrap()
        .parameter(Parameter::new("hang", kelvin(), "never answers"))
        .unwrap()
        .parameter(Parameter::new("slow", kelvin(), "answers after 0.5 s"))
        .unwrap()
        .parameter(Parameter::new("panic", kelvin(), "handler panics"))
        .unwrap()
        .parameter(Parameter::new("chunk", DataInfo::string().build().unwrap(), "2 KiB of text"))
        .unwrap()
        .command(Command::new("stop", DataInfo::command().build().unwrap(), "stop"))
        .unwrap()
        .command(Command::new(
            "echo",
            DataInfo::command().argument(&kelvin()).result(&kelvin()).build().unwrap(),
            "returns its argument",
        ))
        .unwrap()
        .command(Command::new(
            "big",
            DataInfo::command().result(&DataInfo::string().build().unwrap()).build().unwrap(),
            "returns a large string",
        ))
        .unwrap()
        .handler_arc(sim)
}

/// A second, plain Readable module `t2`.
pub fn sensor(name: &str) -> Module {
    Module::new(name, "a sensor")
        .interface_classes(["Readable"])
        .parameter(
            Parameter::new("value", kelvin(), "temperature")
                .cached()
                .initial_value(Value::Double(4.2)),
        )
        .unwrap()
        .parameter(
            Parameter::new("status", status_info(), "status")
                .cached()
                .initial_value(status(100, "ok")),
        )
        .unwrap()
}

pub fn node(sim: Arc<Sim>) -> NodeBuilder {
    NodeBuilder::new("must-test", "test node")
        .bind(SocketAddr::from(([127, 0, 0, 1], 0)))
        .initial_read(false)
        .module(controller(sim))
        .unwrap()
        .module(sensor("t2"))
        .unwrap()
}

/// A line-based SECoP test client.
pub struct Client {
    reader: BufReader<OwnedReadHalf>,
    writer: OwnedWriteHalf,
}

impl Client {
    pub async fn connect(addr: SocketAddr) -> Self {
        let stream = TcpStream::connect(addr).await.unwrap();
        let (r, w) = stream.into_split();
        Self {
            reader: BufReader::new(r),
            writer: w,
        }
    }

    pub async fn send(&mut self, line: &str) {
        self.writer
            .write_all(format!("{line}\n").as_bytes())
            .await
            .unwrap();
    }

    pub async fn send_raw(&mut self, bytes: &[u8]) {
        self.writer.write_all(bytes).await.unwrap();
    }

    /// The next line, without line ending; panics after 5 s.
    pub async fn line(&mut self) -> String {
        self.try_line(Duration::from_secs(5))
            .await
            .expect("no line within 5 s")
    }

    /// The next line, or `None` if none arrives within `timeout` or the connection closed.
    pub async fn try_line(&mut self, timeout: Duration) -> Option<String> {
        let mut line = String::new();
        match tokio::time::timeout(timeout, self.reader.read_line(&mut line)).await {
            Ok(Ok(0)) | Err(_) => None,
            Ok(Ok(_)) => Some(line.trim_end_matches('\n').to_owned()),
            Ok(Err(e)) => panic!("read error: {e}"),
        }
    }

    /// Whether the node closed the connection (EOF or reset) within `timeout`, skipping
    /// any data still in flight.
    pub async fn is_closed(&mut self, timeout: Duration) -> bool {
        let deadline = tokio::time::Instant::now() + timeout;
        let mut line = String::new();
        loop {
            line.clear();
            match tokio::time::timeout_at(deadline, self.reader.read_line(&mut line)).await {
                Err(_) => return false,
                Ok(Ok(0) | Err(_)) => return true,
                Ok(Ok(_)) => {}
            }
        }
    }

    /// Sends a request and returns the next line.
    pub async fn request(&mut self, line: &str) -> String {
        self.send(line).await;
        self.line().await
    }
}

/// Splits `<action> <specifier> <json>` and parses the JSON part.
pub fn parse(line: &str) -> (String, String, Json) {
    let mut parts = line.splitn(3, ' ');
    let action = parts.next().unwrap().to_owned();
    let specifier = parts.next().unwrap_or_default().to_owned();
    let data = parts
        .next()
        .map_or(Json::Null, |d| serde_json::from_str(d).unwrap());
    (action, specifier, data)
}

/// The error class of an `error_*` line.
pub fn error_class(line: &str) -> String {
    let (action, _, data) = parse(line);
    assert!(action.starts_with("error_"), "not an error: {line}");
    data[0].as_str().unwrap().to_owned()
}
