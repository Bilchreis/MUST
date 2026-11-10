# Tasks

## 1. Checker

- [x] 1.1 Add `must_tui::checker`: the command (`MUST_SECOP_CHECK`, default `uvx` from the checker's `work` branch), running it on a structure report with a timeout, and parsing its JSON lines into diagnostics, most severe first; verify with unit tests: parsing, a fake checker (findings, fatal with failure status, failure without findings), a missing program, and an ignored test against the real checker

## 2. App and view

- [x] 2.1 Add `Effect::Check`, `Msg::Checked` and `Node::check`: start a check when the detail view shows a node whose report is unchecked or changed, ignore stale answers, keep the check across reconnects, and re-check with `c`; run checks from the main loop
- [x] 2.2 Draw the SECoP check section (wrapped, at most 8 rows, counts in the title, `… N more`), the `c` binding and the footer hint; verify with view tests and updated snapshots

## 3. Docs

- [x] 3.1 Document the section, the `c` key and `MUST_SECOP_CHECK` in `docs/status-tui.md`

## 4. Later

- [ ] 4.1 Point `DEFAULT_COMMAND` at the checker's main branch or a release once it has the JSON output
