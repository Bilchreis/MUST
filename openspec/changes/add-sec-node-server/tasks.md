# Tasks

## 1. Crate setup and shared types

- [x] 1.1 Add `crates/must-server` (tokio, tracing, async-trait, must-types) to the workspace; verify `cargo build --workspace` passes on all three CI targets
- [x] 1.2 Add `ErrorClass` with all SECoP 2.0 error classes and `SecopError` to `must-types`, plus data-report and error-report encoding; verify unit tests for name round trips and report JSON shapes pass

## 2. Node model

- [x] 2.1 Implement node, module, parameter and command builders with typed SECoP properties, custom `_` properties and explicitly non-standard properties (warning at completion); verify unit tests for the declaration and unknown-property scenarios in specs/sec-node-model pass
- [x] 2.2 Implement completion checks: mandatory properties, identifiers and case-insensitive scopes, group path clashes, property value checks (visibility tables, timeout, lists), constant values; verify each scenario has a passing test and that several errors are reported together
- [x] 2.3 Implement interface-class rules (Readable/Writable/Drivable) and the status tuple shape check; verify the interface-class and status scenarios pass
- [x] 2.4 Implement module-level postfix rules using `must-types` postfix helpers (parent present, datainfo match, `_limits` vs `_min`/`_max` exclusivity); verify the postfix scenarios pass
- [x] 2.5 Implement structure report generation keeping declaration order; verify a golden-file test against a hand-written expected report

## 3. Protocol

- [x] 3.1 Implement line framing (LF, optional CR, maximum length) and request parsing into action/specifier/data; verify framing and malformed-message tests pass
- [x] 3.2 Implement reply and event formatting (`*IDN?`, `describing`, `active`/`inactive`, `pong`, `reply`, `changed`, `done`, `update`, `error_*`); verify string-level tests for every message in specs/sec-node-protocol pass

## 4. Runtime

- [x] 4.1 Implement the module actor with cache, handler calls, `Ctx` side-effect updates and timeout handling; verify unit tests with a fake handler cover cache use, handler errors, timeouts and panics
- [x] 4.2 Implement the update bus and per-connection writer with activation state (global and module-wise); verify ordering tests (side-effect updates before `changed`/`done`, initial updates before `active`)
- [x] 4.3 Implement per-connection byte accounting with the configurable `max_outgoing_bytes` limit (default 16 MiB): replies always enqueued with request-reading backpressure while over the limit, unsolicited updates beyond the limit closing the connection with a warn log; verify every scenario of the slow-clients requirement in specs/sec-node-runtime with test clients that stop reading, flood requests, and request a 50 MiB command result
- [x] 4.4 Implement the TCP listener, connection tasks and request dispatch including `change` validation with dynamic limits and constant parameters; verify end-to-end loopback tests for every scenario in specs/sec-node-protocol
- [x] 4.5 Implement polling and application pushes; verify tests with a paused tokio clock for poll intervals and pushed updates reaching only activated clients
- [x] 4.6 Implement node start/stop, multiple nodes per process and the library-owned runtime; verify the lifecycle and port-in-use scenarios pass
- [x] 4.7 Emit `tracing` events and the `NodeEvent` broadcast for start/stop, connections, messages and updates; verify a test subscriber sees the expected events for a scripted session

## 5. Examples, docs and integration

- [x] 5.1 Add `examples/simple_node.rs` (a simulated temperature Drivable) and document the crate API with rustdoc; verify `cargo run --example simple_node` serves a node that a SECoP client (e.g. frappy-gui or netcat) can `describe`, `activate` and `change`
- [ ] 5.2 Add an interoperability test that runs the example node and drives it with the Frappy Python client in CI (Linux job); verify it passes
- [ ] 5.3 Run the full CI matrix (Linux x86_64, Windows x86_64, Windows i686); verify all jobs are green

## Workflow follow-up

- Archive this change after review; `add-shall-c-abi` and `add-node-monitoring` build on it.
