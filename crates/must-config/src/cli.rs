// SPDX-License-Identifier: LGPL-3.0-or-later

//! The `serve`, `check` and `types` commands: in `must`, and in the binary of a driver
//! collection with its own module types:
//!
//! ```no_run
//! fn main() -> std::process::ExitCode {
//!     let mut registry = must_config::Registry::default();
//!     // registry.register::<acme::Valve>("acme::Valve").unwrap();
//!     must_config::cli::main(registry)
//! }
//! ```

use std::io::Write;
use std::path::Path;
use std::process::ExitCode;

use tracing_subscriber::EnvFilter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

use crate::{Registry, load_file};

/// The usage text of the commands.
pub const USAGE: &str = "\
  serve <config.toml>          serve the SEC node declared in a config file
  check <config.toml>          check a config file and list its modules
  types                        list the module types a config can use";

/// Runs the command in the process's arguments (after the program name).
#[allow(clippy::needless_pass_by_value)]
pub fn main(registry: Registry) -> ExitCode {
    run(&registry, std::env::args().skip(1))
}

/// Runs a command: `serve <file>`, `check <file>` or `types`. Exits with 0 on success, 1
/// when the config has errors or the node cannot start, and 2 for wrong arguments.
pub fn run(registry: &Registry, args: impl IntoIterator<Item = String>) -> ExitCode {
    let args: Vec<String> = args.into_iter().collect();
    match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["serve", file] => serve(registry, Path::new(file)),
        ["check", file] => check(registry, Path::new(file)),
        ["types"] => {
            let mut out = std::io::stdout().lock();
            for name in registry.names() {
                let _ = writeln!(out, "{name}");
            }
            ExitCode::SUCCESS
        }
        ["help" | "--help" | "-h"] => {
            println!("commands:\n{USAGE}");
            ExitCode::SUCCESS
        }
        _ => {
            eprintln!("usage:\n{USAGE}");
            ExitCode::from(2)
        }
    }
}

fn check(registry: &Registry, file: &Path) -> ExitCode {
    match load_file(file, registry) {
        Ok(loaded) => {
            for warning in &loaded.warnings {
                eprintln!("warning: {warning}");
            }
            let mut out = std::io::stdout().lock();
            for (name, type_name) in &loaded.modules {
                let _ = writeln!(out, "{name} {type_name}");
            }
            ExitCode::SUCCESS
        }
        Err(errors) => {
            for error in &errors.0 {
                eprintln!("error: {error}");
            }
            ExitCode::FAILURE
        }
    }
}

fn serve(registry: &Registry, file: &Path) -> ExitCode {
    let loaded = match load_file(file, registry) {
        Ok(loaded) => loaded,
        Err(errors) => {
            for error in &errors.0 {
                eprintln!("error: {error}");
            }
            return ExitCode::FAILURE;
        }
    };
    for warning in &loaded.warnings {
        eprintln!("warning: {warning}");
    }
    // log to the terminal (RUST_LOG, else info), and to the monitoring endpoint for the
    // status TUI's log view
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = tracing_subscriber::registry()
        .with(filter)
        .with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr))
        .with(must_server::monitor::log_layer())
        .try_init();
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(e) => {
            eprintln!("error: cannot start the runtime: {e}");
            return ExitCode::FAILURE;
        }
    };
    let addr = loaded.node.bind_address();
    let node = match loaded.node.spawn(runtime.handle()) {
        Ok(node) => node,
        Err(e) => {
            eprintln!("error: cannot serve on {addr}: {e}");
            return ExitCode::FAILURE;
        }
    };
    println!("listening on {}", node.local_addr());
    let _ = std::io::stdout().flush();
    runtime.block_on(async {
        let _ = tokio::signal::ctrl_c().await;
        node.stop().await;
    });
    ExitCode::SUCCESS
}
