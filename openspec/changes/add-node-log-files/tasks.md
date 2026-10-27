# Tasks

## 1. Node log records as events

- [x] 1.1 Add `NodeEvent::Log { level, module, message }` and an internal helper in `must-server` that emits both the `tracing` record and the event; switch the node's `info!`/`warn!`/`error!` calls in `runtime/` to it; verify a runtime test sees a `Log` event with module `t1` when a handler of `t1` fails, and that existing tests still pass
- [x] 1.2 Add the published message to `NodeEvent::Updated` (`line`, built once by the hub for all clients); verify a runtime test with two activated clients sees one `Updated` event per update carrying the same text both clients receive

## 2. Configuration and layout

- [x] 2.1 Add `LogFiles` (enabled, dir, days) with defaults from `MUST_LOG_FILES`, `MUST_LOG_DIR`, `MUST_LOG_DAYS` (invalid values warn and fall back), the per-user default directories, and `NodeBuilder::log_files`; verify unit tests for parsing, precedence of the Rust API and the default paths on Linux and Windows (CI)
- [x] 2.2 Implement node folder names (`equipment_id` sanitizing), the day file name, and the line formatter for every event kind; verify unit tests for the unusual-equipment-id scenario and the readable-lines and update-line scenarios, including a `Lagged` gap line

## 3. Writer

- [x] 3.1 Implement the per-process writer thread: per-node open file in append mode, day change at local midnight (with an injectable clock for tests), exclusive write access (`try_lock` on Unix, share mode on Windows) with fallback to `<date>-<pid>.log`, flush every second and on node stop; verify the midnight, stopped-node and node-name-used-twice scenarios (the latter with two processes, as a test that spawns a child process) on all three targets
- [x] 3.2 Implement the per-node logger task fed from the broadcast channel and the bounded hand-off with drop counting; start it with the node when enabled and stop it with the node; verify the two-nodes, request-and-reply, handler-error, three-activated-clients, no-activated-client and activation scenarios with a real node and clients, and the turned-off scenario (no directory created)
- [x] 3.3 Implement retention (at node start and hourly, only matching file names, only this process's node folders); verify the default-retention, foreign-files-kept and longer-retention scenarios with an injected date
- [x] 3.4 Implement error handling: one warning and logging stopped for the node when the directory or file cannot be written, retry at the next day change; verify the read-only-directory scenario (Linux: directory mode 0500; Windows: read-only attribute or a path under a file) and that the node serves clients

## 4. Guarantees, C ABI and docs

- [x] 4.1 Add a latency test: a node with file logging to a stalled writer (test hook that blocks the writer thread) answers requests within the same tolerance as without logging, and the file later contains a dropped-lines line; verify on all three targets
- [x] 4.2 Add a C ABI test: a C host using `SHALLserver` with `MUST_LOG_DIR` and `MUST_LOG_DAYS` set writes its node's traffic to the configured directory; verify on all three targets
- [x] 4.3 Document file logging (location, line format, retention, environment variables, Rust API, disk usage estimate and privacy note) in the README and `COMPATIBILITY.md` (new behavior compared with SHALL); verify the documented `MUST_LOG_DIR` example produces the documented file path

## Workflow follow-up

- Archive after review.
