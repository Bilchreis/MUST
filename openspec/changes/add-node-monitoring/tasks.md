# Tasks

## 1. Registry and discovery

- [ ] 1.1 Add `crates/must-monitor` with protocol types (hello, welcome, snapshot, events, counters, lagged, error) and serde round-trip tests; verify `cargo test -p must-monitor protocol` passes
- [ ] 1.2 Implement the per-user registry directory with Linux permission checks and Windows path; verify the fallback and unsafe-directory scenarios on Linux and the path on Windows in CI
- [ ] 1.3 Implement atomic registration write/delete with process start time and name, and listing with stale detection and cleanup; verify the registration-lifecycle, listing and host-crashed scenarios (a killed child process leaves a stale file) on all three targets

## 2. Endpoint

- [ ] 2.1 Implement the loopback listener, token-checked hello with version negotiation and multiple read-only monitors; verify the wrong-token, version-mismatch and two-monitors scenarios
- [ ] 2.2 Implement the per-process `Monitor` singleton started and stopped with the first/last node in `must-server`, honoring `MUST_MONITOR=0`; verify the one-endpoint, last-node-stops and disabled scenarios
- [ ] 2.3 Implement snapshot assembly from node models and module caches (no hardware reads); verify the snapshot-content scenario and that no handler read happens during a snapshot
- [ ] 2.4 Implement event fan-out from `NodeEvent` with bounded queues, `lagged`, per-node traffic subscription, 4 KiB truncation and per-second counters; verify the traffic, disconnect, not-subscribed, long-describe and stalled-monitor scenarios (latency check: node reply time unchanged within tolerance while a monitor is stalled)
- [ ] 2.5 Implement the `tracing` layer with a 2000-record ring buffer and node/module attribution, and the default subscriber installation; verify the handler-error-logged scenario and that an existing global subscriber is not replaced

## 3. Client library, docs and integration

- [ ] 3.1 Implement the client API (`list_endpoints()`, `connect(registration)` returning snapshot plus event stream, `subscribe_traffic`); verify an integration test with a real `must-server` node in-process and a client reading snapshot and events
- [ ] 3.2 Add a C ABI integration test: a C host built against `SHALLserver` with two contexts and two nodes is discovered and read by the client library; verify on all three targets, including a 32-bit host read by a 64-bit client on Windows
- [ ] 3.3 Document the protocol (`docs/monitoring-protocol.md`), the registry locations, `MUST_MONITOR` and the tracing layer for Rust hosts; verify a manual session with `nc 127.0.0.1 <port>` following the document receives a snapshot

## Workflow follow-up

- Archive after review; `add-status-tui` consumes the client library.
