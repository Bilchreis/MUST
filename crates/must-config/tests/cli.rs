// SPDX-License-Identifier: LGPL-3.0-or-later

//! The `serve`, `check` and `types` commands, through the `acme_node` example: a driver
//! collection's binary with the built-in types and `acme::Valve`.

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::Duration;

/// The `acme_node` example, which cargo builds next to the test binaries.
fn acme_node() -> PathBuf {
    let exe = std::env::current_exe().unwrap();
    let dir = exe.parent().unwrap().parent().unwrap().join("examples");
    let path = dir.join(format!("acme_node{}", std::env::consts::EXE_SUFFIX));
    assert!(path.is_file(), "{} is not built", path.display());
    path
}

fn examples() -> &'static Path {
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/examples"))
}

fn run(args: &[&str]) -> Output {
    Command::new(acme_node()).args(args).output().unwrap()
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// A config file in a temporary directory.
fn config(name: &str, content: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("must-config-cli-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    std::fs::write(&path, content).unwrap();
    path
}

const NODE: &str =
    "[node]\nequipment_id = \"t\"\ndescription = \"test\"\ndiscovery = false\nlog_files = false\n";

#[test]
fn valid_config() {
    let out = run(&["check", examples().join("gas-rack.toml").to_str().unwrap()]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    let lines: Vec<String> = text(&out.stdout).lines().map(str::to_owned).collect();
    assert_eq!(lines.len(), 5);
    assert_eq!(lines[0], "mfc1 must::sim::FlowController");
    assert_eq!(lines[4], "mfc5 must::sim::FlowController");
}

#[test]
fn errors_are_reported() {
    let path = config(
        "broken.toml",
        &format!(
            "{NODE}\n[modules.a]\ntype = \"nope::Nothing\"\n\n[modules.b]\ntype = \"must::sim::FlowController\"\nrange = 1.0\nrnage = 1.0\n"
        ),
    );
    let out = run(&["check", path.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(1));
    let errors = text(&out.stderr);
    assert_eq!(
        errors.lines().filter(|l| l.starts_with("error: ")).count(),
        2,
        "{errors}"
    );
    assert!(
        errors.contains("nope::Nothing") && errors.contains("rnage"),
        "{errors}"
    );
}

#[test]
fn built_in_and_custom_types() {
    let out = run(&["types"]);
    assert!(out.status.success());
    assert_eq!(
        text(&out.stdout).lines().collect::<Vec<_>>(),
        [
            "acme::Valve",
            "must::sim::FlowController",
            "must::sim::Temperature"
        ]
    );
}

#[test]
fn custom_binary() {
    let path = config(
        "valve.toml",
        &format!("{NODE}\n[modules.v1]\ntype = \"acme::Valve\"\nport = 3\n"),
    );
    let out = run(&["check", path.to_str().unwrap()]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert_eq!(text(&out.stdout).trim(), "v1 acme::Valve");
}

#[test]
fn usage() {
    let out = run(&["frobnicate"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(text(&out.stderr).contains("serve <config.toml>"));
}

#[test]
fn invalid_config_is_not_served() {
    let path = config(
        "invalid.toml",
        &format!(
            "{NODE}interface = \"tcp://127.0.0.1:0\"\n[modules.x]\ntype = \"nope::Nothing\"\n"
        ),
    );
    let out = run(&["serve", path.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(1));
    let stderr = text(&out.stderr);
    assert!(
        stderr.contains("invalid.toml:8:") && stderr.contains("nope::Nothing"),
        "{stderr}"
    );
    assert!(!text(&out.stdout).contains("listening"));
}

#[test]
fn port_in_use() {
    let taken = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = taken.local_addr().unwrap().port();
    let path = config(
        "taken.toml",
        &format!(
            "{NODE}interface = \"tcp://127.0.0.1:{port}\"\n[modules.T]\ntype = \"must::sim::Temperature\"\n"
        ),
    );
    let out = run(&["serve", path.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(1));
    let stderr = text(&out.stderr);
    assert!(stderr.contains(&format!("127.0.0.1:{port}")), "{stderr}");
}

#[test]
fn serving_the_gas_rack() {
    let path = config(
        "rack.toml",
        &std::fs::read_to_string(examples().join("gas-rack.toml"))
            .unwrap()
            .replace("tcp://2056", "tcp://127.0.0.1:0")
            .replace(
                "_facility = \"example lab\"",
                "_facility = \"example lab\"\ndiscovery = false\nlog_files = false",
            ),
    );
    let mut child = Command::new(acme_node())
        .args(["serve", path.to_str().unwrap()])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    stdout.read_line(&mut line).unwrap();
    let addr = line
        .trim()
        .strip_prefix("listening on ")
        .unwrap()
        .to_owned();
    let stream = TcpStream::connect(&addr).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let mut writer = stream.try_clone().unwrap();
    writer.write_all(b"describe\n").unwrap();
    let mut reply = String::new();
    BufReader::new(stream).read_line(&mut reply).unwrap();
    child.kill().unwrap();
    let _ = child.wait();
    let json: serde_json::Value =
        serde_json::from_str(reply.splitn(3, ' ').nth(2).unwrap()).unwrap();
    let modules: Vec<_> = json["modules"]
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect();
    assert_eq!(modules, ["mfc1", "mfc2", "mfc3", "mfc4", "mfc5"]);
}
