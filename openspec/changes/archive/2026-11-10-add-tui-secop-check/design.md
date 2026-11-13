# Design

## Context

- `must-tui` already holds every node's structure report (`NodeInfo::structure`, the `describing` data), sent by the monitoring endpoint in the snapshot and in `node_started`.
- The TUI is an Elm-style loop: `App::update(Msg) -> Vec<Effect>`, effects run by the main loop or discovery, results come back as messages. Rendering (`ui.rs`) is a pure function of the state.
- `secop-check` is a Python program (Python 3.12+, `pyyaml`, `requests`, `rich`). Its `work` branch prints one JSON object per finding with `--json` (`{severity, step, msg, ctx}`) and reads descriptive data from stdin with `-`. It exits 1 only after a fatal finding.

## Goals / Non-Goals

**Goals:**
- Show the checker's findings for the node in the detail view, without blocking the UI and without the TUI talking to the node.
- Need nothing but uv on the machine, on Linux and Windows.
- Never show a wrong result: no stale findings for a changed report, no "ok" when nothing was checked, no findings cut away unnoticed.

**Non-Goals:**
- Checking every node up front, or showing results in the overview.
- Porting the checker or bundling Python.
- A scrollable list of findings; the section shows what fits and counts the rest.

## Decisions

### Child process, default through `uvx` at a pinned commit
The checker runs as `uvx --from git+https://github.com/SampleEnvironment/secop-checker@<commit> secop-check --json --version 2.0 -`, with the structure report on stdin. `uvx` fetches and caches the checker, so only uv must be installed. The commit is pinned rather than the branch name: results are reproducible, and a cached commit runs offline, while a branch is looked up on every run. `MUST_SECOP_CHECK` replaces the command (split at whitespace) for an installed `secop-check` or a checkout. Alternatives: a Python dependency (pyo3) or a Rust port, both far heavier than a child process.

### Lazy, per structure report, identified by an id
`App::check_effect` starts a check only while the detail view shows a node whose report has no check or a check of another report (compared by value). Each check gets an id from a counter; `Msg::Checked` is applied only if the id matches the node's current check, so answers of superseded checks are dropped. `c` drops a finished check so the next update starts a new one. A reconnect keeps the check of each node; a changed report is checked again. The main loop runs `Effect::Check` itself (`tokio::spawn`), discovery ignores it.

### Process handling
stdin is written concurrently with reading stdout and stderr (`tokio::join!`), and one timeout (120 s, long enough for a first download) covers the whole exchange, so neither a checker that writes before it reads nor one that never reads can hang the check. The child is killed when the check is dropped (`kill_on_drop`). Results:
- findings parsed from stdout, most severe first: the result, also with a failure status (fatal findings);
- no findings and a failure status: the last stderr line as the error;
- no findings but other output: an error ("unexpected output"), so a change of the output format cannot pass for a conforming description;
- program not found: an error naming uv and `MUST_SECOP_CHECK`.

### The section
A bordered block between the modules and the clients, only for nodes with a structure report. Findings wrap; their rows are counted with ratatui's own wrapping (`Paragraph::line_count`, feature `unstable-rendered-line-info`). The block gets at most 8 rows, fewer when the modules (at least 5 rows) and clients leave less, and none when no row is left. Findings that do not fit are counted in a `… N more` row; a single finding too long for the block is shown cut rather than left out. The title counts findings by severity.

## Risks / Trade-offs

- **The checker's JSON output is not yet on its main branch.** The pin has to move once it is (follow-up in the proposal). The "unexpected output" rule makes a format change visible instead of silent.
- **A timed-out check may leave the Python process behind**: `kill_on_drop` kills `uvx` but not its child. Killing the process tree would need `unsafe` (forbidden in the workspace) or platform-specific helpers, and it matters only after a 120 s timeout. Documented in `must_tui::checker`.
- **No automatic retry**: a failed check stays failed until `c` or a new report. Retrying a missing uv or a network failure in a loop would add noise.
- **Number of checks**: one child per node viewed with a new report, not capped. Each node has at most one check running.
- **The real checker is tested only in an ignored test** (needs uv and network); CI tests use fake checkers.
