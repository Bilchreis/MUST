// SPDX-License-Identifier: LGPL-3.0-or-later

//! Checks a node's structure report with the SECoP checker (`secop-check`, from
//! [SampleEnvironment/secop-checker](https://github.com/SampleEnvironment/secop-checker)).
//!
//! The checker is a Python program. It is run as a child process that reads the
//! structure report on stdin and writes one JSON object per diagnostic on stdout
//! (`secop-check --json -`). By default it is run with `uvx` from the checker's git
//! repository ([`DEFAULT_COMMAND`]), so nothing but [uv](https://docs.astral.sh/uv/) has to
//! be installed; `MUST_SECOP_CHECK` names another command, e.g. an installed
//! `secop-check`. The checker only sees the structure report: it does not connect to the
//! node.
//!
//! A check that runs into [`TIMEOUT`] kills the command it started. A process that
//! command started in turn (the Python interpreter that `uvx` runs) may outlive it.

use std::io::ErrorKind;
use std::process::Stdio;
use std::time::Duration;

use serde_json::Value as Json;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

/// The environment variable naming the checker command, split at whitespace, e.g.
/// `secop-check` or `uv run --project /src/secop-checker secop-check`.
pub const COMMAND_VAR: &str = "MUST_SECOP_CHECK";

/// The checker command when [`COMMAND_VAR`] is not set: the checker's `work` branch, run
/// with `uvx`.
// TODO: switch to the checker's main branch (or a release) once it has the JSON output
pub const DEFAULT_COMMAND: &[&str] = &[
    "uvx",
    "--from",
    "git+https://github.com/SampleEnvironment/secop-checker@work",
    "secop-check",
];

/// The arguments added to the command: JSON output, the structure report on stdin,
/// checked against SECoP 2.0 (the version MUST speaks).
pub const ARGUMENTS: &[&str] = &["--json", "--version", "2.0", "-"];

/// How long a check may take; the first `uvx` run downloads the checker.
pub const TIMEOUT: Duration = Duration::from_secs(120);

/// How bad a finding is, as the checker rates it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    /// Not a violation, but might be interesting.
    Hint,
    /// Violates the semantics of the specification.
    Warning,
    /// Violates the specification.
    Error,
    /// Stopped the checker.
    Catastrophic,
}

impl Severity {
    fn parse(text: &str) -> Self {
        match text {
            "HINT" => Self::Hint,
            "WARNING" => Self::Warning,
            "CATASTROPHIC" => Self::Catastrophic,
            _ => Self::Error,
        }
    }

    /// `hint`, `warning`, `error`, `fatal`.
    pub fn label(self) -> &'static str {
        match self {
            Self::Hint => "hint",
            Self::Warning => "warning",
            Self::Error => "error",
            Self::Catastrophic => "fatal",
        }
    }
}

/// One finding of the checker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    /// How bad it is.
    pub severity: Severity,
    /// The checker step that found it, e.g. `properties-basic`.
    pub step: String,
    /// Where in the structure report, outermost first, e.g. `["SECNode", "Module t"]`.
    pub context: Vec<String>,
    /// What is wrong.
    pub message: String,
}

impl Diagnostic {
    /// Where, for display: the context without the leading `SECNode`, joined with ` / `.
    pub fn location(&self) -> String {
        let skip = usize::from(self.context.first().is_some_and(|c| c == "SECNode"));
        self.context[skip..].join(" / ")
    }
}

/// What a check found: its findings, most severe first, or why it could not run.
pub type Outcome = Result<Vec<Diagnostic>, String>;

/// The checker command: [`COMMAND_VAR`] if set and not empty, else [`DEFAULT_COMMAND`].
pub fn command() -> Vec<String> {
    let configured: Vec<String> = std::env::var(COMMAND_VAR)
        .unwrap_or_default()
        .split_whitespace()
        .map(str::to_owned)
        .collect();
    if configured.is_empty() {
        DEFAULT_COMMAND.iter().map(|s| (*s).to_owned()).collect()
    } else {
        configured
    }
}

/// Runs `command` (with [`ARGUMENTS`]) on `structure`, for at most [`TIMEOUT`].
pub async fn check(command: &[String], structure: &Json) -> Outcome {
    check_within(command, structure, TIMEOUT).await
}

/// Runs `command` (with [`ARGUMENTS`]) on `structure`, for at most `timeout`.
pub async fn check_within(command: &[String], structure: &Json, timeout: Duration) -> Outcome {
    let Some((program, args)) = command.split_first() else {
        return Err("no checker command".to_owned());
    };
    let mut child = Command::new(program)
        .args(args)
        .args(ARGUMENTS)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| match e.kind() {
            ErrorKind::NotFound => format!(
                "{program} not found: install uv (https://docs.astral.sh/uv/) or set \
                 {COMMAND_VAR} to the secop-check command"
            ),
            _ => format!("cannot run {program}: {e}"),
        })?;
    let stdin = child.stdin.take();
    let text = structure.to_string();
    // stdin is written while stdout and stderr are read, so a checker that writes before
    // it has read everything cannot block both sides; the timeout covers all of it
    let feed = async move {
        if let Some(mut stdin) = stdin {
            // a checker that exits early closes stdin; its output tells why
            let _ = stdin.write_all(text.as_bytes()).await;
            // dropping stdin closes it: the checker reads to the end
        }
    };
    let exchange = async {
        let ((), output) = tokio::join!(feed, child.wait_with_output());
        output
    };
    let output = tokio::time::timeout(timeout, exchange)
        .await
        .map_err(|_| format!("the checker took longer than {}s", timeout.as_secs()))?
        .map_err(|e| format!("cannot run {program}: {e}"))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let diagnostics = parse(&stdout);
    // output that is not diagnostics must not pass for a description without findings,
    // e.g. after a change of the checker's output format
    if diagnostics.is_empty()
        && let Some(line) = stdout.lines().find(|l| !l.trim().is_empty())
    {
        return Err(format!(
            "unexpected output of {program}: {}",
            line.chars().take(200).collect::<String>()
        ));
    }
    if !output.status.success() && diagnostics.is_empty() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let reason = stderr
            .lines()
            .rev()
            .find(|l| !l.trim().is_empty())
            .map_or_else(
                || format!("{program} failed: {}", output.status),
                str::to_owned,
            );
        return Err(reason);
    }
    Ok(diagnostics)
}

/// The diagnostics in the checker's JSON output (one object per line), most severe
/// first; other lines are skipped.
pub fn parse(output: &str) -> Vec<Diagnostic> {
    let mut diagnostics: Vec<Diagnostic> = output
        .lines()
        .filter_map(|line| serde_json::from_str::<Json>(line).ok())
        .filter_map(|json| {
            let text = |key: &str| json.get(key).and_then(Json::as_str).map(str::to_owned);
            Some(Diagnostic {
                severity: Severity::parse(&text("severity")?),
                step: text("step").unwrap_or_default(),
                context: json
                    .get("ctx")
                    .and_then(Json::as_array)
                    .map(|items| {
                        items
                            .iter()
                            .filter_map(Json::as_str)
                            .map(str::to_owned)
                            .collect()
                    })
                    .unwrap_or_default(),
                message: text("msg").unwrap_or_default(),
            })
        })
        .collect();
    // stable: the checker's order within one severity
    diagnostics.sort_by_key(|d| std::cmp::Reverse(d.severity));
    diagnostics
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_json_lines_most_severe_first() {
        let output = concat!(
            "{\"severity\": \"HINT\", \"step\": \"names\", \"msg\": \"short\", \"ctx\": [\"SECNode\"]}\n",
            "not json\n",
            "{\"severity\": \"ERROR\", \"step\": \"properties-basic\", \"msg\": \"missing required properties: 'implementation'\", \"ctx\": [\"SECNode\", \"Module t\"]}\n",
        );
        let diagnostics = parse(output);
        assert_eq!(diagnostics.len(), 2);
        assert_eq!(diagnostics[0].severity, Severity::Error);
        assert_eq!(diagnostics[0].location(), "Module t");
        assert_eq!(diagnostics[0].step, "properties-basic");
        assert_eq!(diagnostics[1].severity, Severity::Hint);
        assert_eq!(diagnostics[1].location(), "");
    }

    /// A checker that reads stdin and prints `output` (to stdout) and `error` (to stderr).
    #[cfg(unix)]
    fn fake(output: &str, error: &str, status: u8) -> Vec<String> {
        let script = format!(
            "cat > /dev/null; printf '%s' '{output}'; printf '%s\\n' '{error}' >&2; exit {status}"
        );
        // sh -c <script> <$0>: the appended arguments become $0, $1, ...
        vec!["sh".into(), "-c".into(), script]
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn runs_the_command_on_the_structure() {
        let line = r#"{"severity": "WARNING", "step": "s", "msg": "m", "ctx": ["SECNode"]}"#;
        let found = check(&fake(line, "", 0), &serde_json::json!({"modules": {}}))
            .await
            .unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].severity, Severity::Warning);

        // a fatal finding comes with a failure status: still a result
        let line = r#"{"severity": "CATASTROPHIC", "step": "s", "msg": "bad json", "ctx": []}"#;
        let found = check(&fake(line, "", 1), &serde_json::json!({}))
            .await
            .unwrap();
        assert_eq!(found[0].severity, Severity::Catastrophic);

        // a failure without findings reports the last line on stderr
        let error = check(&fake("", "no such version", 2), &serde_json::json!({}))
            .await
            .unwrap_err();
        assert_eq!(error, "no such version");

        // output that is not diagnostics is not a description without findings
        let error = check(
            &fake("usage: secop-check [-h]", "", 0),
            &serde_json::json!({}),
        )
        .await
        .unwrap_err();
        assert!(
            error.contains("unexpected output") && error.contains("usage"),
            "{error}"
        );
    }

    /// A report larger than the pipe buffers: a checker that writes before it reads
    /// works, and one that never reads runs into the timeout instead of hanging.
    #[cfg(unix)]
    #[tokio::test]
    async fn large_reports_do_not_block() {
        let report = serde_json::json!({"description": "x".repeat(1 << 20)});
        let writes_first = vec![
            "sh".into(),
            "-c".into(),
            "head -c 1048576 /dev/zero | tr '\\0' x >&2; cat > /dev/null".into(),
        ];
        let found = check_within(&writes_first, &report, Duration::from_secs(20))
            .await
            .unwrap();
        assert_eq!(found, Vec::new());

        let never_reads = vec!["sh".into(), "-c".into(), "sleep 30".into()];
        let started = std::time::Instant::now();
        let error = check_within(&never_reads, &report, Duration::from_millis(500))
            .await
            .unwrap_err();
        assert!(error.contains("took longer"), "{error}");
        assert!(started.elapsed() < Duration::from_secs(10));
    }

    /// The real checker through `uvx` (needs uv and the network): a minimal node that
    /// lacks required properties.
    #[tokio::test]
    #[ignore = "needs uv and network access"]
    async fn real_checker() {
        let structure = serde_json::json!({
            "equipment_id": "test", "description": "test node", "modules": {
                "t": {"interface_classes": ["Readable"], "description": "t",
                      "accessibles": {"value": {"datainfo": {"type": "double"},
                                                "readonly": true, "description": "v"}}}}});
        let command: Vec<String> = DEFAULT_COMMAND.iter().map(|s| (*s).to_owned()).collect();
        let found = check(&command, &structure).await.unwrap();
        assert!(!found.is_empty(), "{found:?}");
        assert!(
            found.iter().any(|d| d.location() == "Module t"),
            "{found:?}"
        );
    }

    #[tokio::test]
    async fn missing_program_is_reported() {
        let command = vec!["must-no-such-secop-check".to_owned()];
        let error = check(&command, &serde_json::json!({})).await.unwrap_err();
        assert!(error.contains("not found"), "{error}");
        assert!(error.contains(COMMAND_VAR), "{error}");
    }
}
