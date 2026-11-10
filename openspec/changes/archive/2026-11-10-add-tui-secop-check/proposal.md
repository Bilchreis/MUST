# Proposal

## Why

A node's description (its `describe` reply) is built from application code, C calls or LabVIEW VIs, so it can drift from the SECoP specification without anyone noticing until a client trips over it. The SECoP checker (`secop-check`, [SampleEnvironment/secop-checker](https://github.com/SampleEnvironment/secop-checker)) checks descriptive data against the standard. The TUI already holds every running node's structure report, so the detail view can show what the checker finds, next to the modules it concerns.

## What Changes

- The detail view gets a **SECoP check** section between the modules and the clients. It shows the checker's findings for the selected node, most severe first (`fatal`, `error`, `warning`, `hint`), each with the place in the description it concerns, and a title that counts them or says `ok`. While the check runs it says so; when the checker cannot run, it says why.
- A node is checked when the detail view first shows it, and again when its structure report changes. A new key, `c` (detail view), checks it again.
- The checker runs as a child process on the structure report the TUI already has (`secop-check --json --version 2.0 -`, report on stdin): it does not connect to the node, so the TUI stays read-only. By default it is run with `uvx` from a pinned commit of the checker's `work` branch, which has the JSON output, so only uv needs to be installed, and once cached the check works offline. `MUST_SECOP_CHECK` names another command. The default moves to the checker's main branch or a release once that has the changes.
- Documented in `docs/status-tui.md` (views, keys, a SECoP check section, environment variables).

## Capabilities

### New Capabilities
None.

### Modified Capabilities
- `status-tui`: the detail view checks the node's description with the SECoP checker.

## Follow-up

Point `must_tui::checker::DEFAULT_COMMAND` (and the default in `docs/status-tui.md`) at the checker's main branch or a release once it has the JSON output.

## Out of scope

- Checking nodes outside the detail view (e.g. a column in the overview).
- Bundling the checker or porting it to Rust.
- Mapping findings onto the module and parameter lines of the detail view.
