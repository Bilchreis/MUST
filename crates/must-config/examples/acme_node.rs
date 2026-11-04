// SPDX-License-Identifier: LGPL-3.0-or-later

//! A driver collection's own binary: the built-in module types plus `acme::Valve`, with
//! the `serve`, `check` and `types` commands of `must`.
//!
//! ```text
//! cargo run --example acme_node -- check my-node.toml
//! ```

use std::process::ExitCode;

use must_config::Registry;
use must_server::typed::prelude::*;

/// A valve that is open or closed.
#[module(Writable, description = "a valve")]
struct Valve {
    value: param!("whether the valve is open", bool()),
    status: param!("current status", status(IDLE, ERROR)),
    target: param!("open or close", bool(), writable),
    port: property!("controller port", int(min = 1, max = 8) as u8, default = 1),
}

#[methods]
impl Valve {
    async fn read_value(&mut self) -> Result<bool> {
        Ok(self.target.get())
    }

    async fn read_status(&mut self) -> Result<Status> {
        Ok(Status::idle(format!("port {}", self.port)))
    }
}

fn main() -> ExitCode {
    let mut registry = Registry::default();
    registry
        .register::<Valve>("acme::Valve")
        .expect("a valid, new type name");
    must_config::cli::main(registry)
}
