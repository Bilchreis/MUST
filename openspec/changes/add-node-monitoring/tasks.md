# Tasks

## 1. Registry and discovery

- [x] 1.1 Add `crates/must-monitor` with protocol types (hello, welcome, snapshot, events, counters, lagged, error) and serde round-trip tests; verify `cargo test -p must-monitor protocol` passes
- [x] 1.2 Implement the per-user registry directory with Linux permission checks and Windows path; verify the fallback and unsafe-directory scenarios on Linux and the path on Windows in CI
- [x] 1.3 Implement atomic registration write/delete with process start time, name and endpoint description (`kind` with its fields, unknown kinds reported as unsupported), and listing with stale detection and cleanup (including the socket file); verify the registration-lifecycle, listing, Linux-registration, unknown-kind and host-crashed scenarios (a killed child process leaves a stale file) on all three targets

## 2. Endpoint

- [x] 2.1 Add `NodeEvent::ClientActivated`/`ClientDeactivated` (with the module or `None` for the whole node) to `must-server`; verify a runtime test sees both for `activate t1`/`deactivate t1` and for a global `activate`

- [x] 2.2 Implement the transport seam (`Listener` yielding byte streams, `connect(&EndpointDescription)`) with the Unix domain socket (0600, in the 0700 registry directory) and the Windows named pipe (user-only security descriptor, first-instance flag, server pid check); verify the other-user scenarios (Linux: a test that changes to another uid where CI allows it, otherwise checks the socket's and directory's permission bits; Windows: a test that reads the pipe's DACL) and the not-on-the-network scenario on all three targets
- [x] 2.3 Implement the hello with version negotiation and multiple read-only monitors over the seam; verify the no-hello, version-mismatch and two-monitors scenarios
- [x] 2.4 Implement the per-process `Monitor` singleton with its node registry (open with the first registered node, close with the last), Rust nodes registering when they start, honoring `MUST_MONITOR=0`; verify the one-endpoint, last-node-stops and disabled scenarios
- [x] 2.5 Register C ABI nodes from `SECoP_S_createNode` with their context ID, report `nodeComplete` results (failed with messages, or running) and remove them on `deleteNode`/`doneLibrary`; verify the C-ABI-node-being-built, nodeComplete-fails, fixed-and-completed and two-LabVIEW-contexts scenarios with C tests
- [x] 2.6 Implement snapshot assembly from node models and module caches (no hardware reads); verify the snapshot-content scenario and that no handler read happens during a snapshot
- [x] 2.7 Implement event fan-out from `NodeEvent` with bounded queues, `lagged`, per-node traffic subscription, 4 KiB truncation and per-second counters; verify the traffic, disconnect, not-subscribed, long-describe and stalled-monitor scenarios (latency check: node reply time unchanged within tolerance while a monitor is stalled)
- [x] 2.8 Implement coalesced parameter updates (per-monitor latest-value map flushed every interval, interval from `hello`) and the per-node `subscribe_updates` "all" mode; verify the fast-polled-parameter, last-value-not-lost and every-update scenarios
- [x] 2.9 Implement the `tracing` layer with a 2000-record ring buffer and node/module attribution, and the default subscriber installation; verify the handler-error-logged scenario and that an existing global subscriber is not replaced

## 3. Client library, docs and integration

- [x] 3.1 Implement the client API (`list_endpoints()`, `connect(registration)` returning snapshot plus event stream, `subscribe_traffic`); verify an integration test with a real `must-server` node in-process and a client reading snapshot and events
- [x] 3.2 Add a C ABI integration test: a C host built against `SHALLserver` with two contexts and two nodes is discovered and read by the client library; verify on all three targets, including a 32-bit host read by a 64-bit client on Windows
- [x] 3.3 Document the protocol (`docs/monitoring-protocol.md`), the registry locations, `MUST_MONITOR` and the tracing layer for Rust hosts; verify a manual session following the document (`socat` on Linux, the PowerShell snippet on Windows) receives a snapshot

## Workflow follow-up

- Archive after review; `add-status-tui` consumes the client library.
