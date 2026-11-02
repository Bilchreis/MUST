# Tasks

## 1. Dependencies and configuration

- [x] 1.1 Add `socket2` (0.6, feature `all`) and `if-addrs` to `[workspace.dependencies]` and as dependencies of `must-server`; verify `cargo build --workspace`, `cargo deny check` and `cargo build -p must-server --target i686-pc-windows-msvc` / `x86_64-pc-windows-msvc` (as CI does) succeed
- [x] 1.2 Add `Discovery` (`default()` on, port `DISCOVERY_PORT` = 10767, announcing; `off()`; `.port()`, `.announce()`) and `DISCOVERY_VARIABLE` (`MUST_DISCOVERY`) in a new `must-server/src/discovery.rs`, store it in `NodeOptions` via `NodeBuilder::discovery`, re-export from `lib.rs`, with rustdoc on every item; verify `cargo doc -p must-server` has no missing-docs warnings and a model test shows the default is on

## 2. Packet

- [x] 2.1 Implement the packet builder (compact UTF-8 JSON, keys `SECoP`, `port`, `equipment_id`, `firmware`, `description`; firmware fallback `must-<version>`) and the encoded-size truncation (full → empty-description check → binary search on char boundaries); verify unit tests: exact bytes for the "Node bound to port 0" scenario, the no-firmware fallback, Frappy's empty-overhead (78 bytes for port 65535 and empty strings), ASCII and 4-byte-UTF-8 truncation, a description full of `"` and `\n` stays ≤ 508 bytes, and the too-long `equipment_id` returns "no discovery"

## 3. Listener

- [x] 3.1 Implement socket setup (reuse port on Unix, reuse address on Windows, broadcast, non-blocking, bind `0.0.0.0:<port>`, tokio `from_std`) and start it in `Node::spawn` after the TCP bind when discovery is enabled and `MUST_DISCOVERY` is not `0`, logging setup failures through `Hub::log` as warnings and continuing without discovery; verify an integration test where the discovery port is held exclusively by a plain socket without reuse options: the node starts, logs the warning, and serves `*IDN?`
- [x] 3.2 Implement the listener task on the node's tracker and stop token: 1025-byte receive buffer, `discover` check, loopback rule, reply to source, debug logs, `WSAECONNRESET` ignored on Windows; add `crates/must-server/tests/discovery.rs` (each test on its own ephemeral discovery port, announcements off) verifying: reply to a unicast request on `127.0.0.1`, extra keys accepted, ignored datagrams (`hello`, `[1,2]`, another node's `node` packet, 1100 bytes) followed by a still-answered request, bound to `127.0.0.1` answers loopback, bound to `127.0.0.1` ignores a request sent from the host's non-loopback address (skipped if none), stop closes the socket (no reply afterwards, port can be bound exclusively again), `Discovery::off()` opens no socket, and `MUST_DISCOVERY=0` in a child process opens no socket
- [x] 3.3 Add the 20-in-any-second rate limiter; verify a unit test with paused tokio time (21st reply in one second refused, allowed again after the window slides) and an integration test sending 200 requests in a burst gets at most 20 replies
- [x] 3.4 Implement the startup announcement (interface broadcast addresses from `if-addrs` excluding loopback, fallback `255.255.255.255`, sent once from the listener socket, failures at debug level, skipped for loopback-bound nodes and with `announce(false)`); verify integration tests that skip with a message when the host has no broadcast interface: a sharing listener socket on the test port receives the node's packet at startup, and a broadcast discover reaches two nodes of one process (replies with both TCP ports)

## 4. Documentation

- [x] 4.1 Document discovery in the `must-server` crate docs (`lib.rs`) and add a "Discovery" section to the README (on by default, `MUST_DISCOVERY=0`, `Discovery::off()`, clients must broadcast because unicast reaches one node per host on Linux, firewall note for 10767/udp, `frappy-scan` example against the example node); verify `cargo test --doc -p must-server` passes and the README command runs as written
- [x] 4.2 Add a "(new)" Discovery entry to "Library behavior" in `COMPATIBILITY.md` (C nodes now listen on UDP 10767 and announce themselves; `MUST_DISCOVERY=0` turns it off); verify the entry names the variable and the port

## 5. Integration checks

- [ ] 5.1 Run `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings` and `cargo test --workspace` on Linux, and confirm the discovery tests ran (not skipped) on all three CI targets (Linux x86_64, Windows x86_64, Windows i686) from the CI test logs
- [ ] 5.2 Manual interop on one Linux host: start a Frappy node (`frappy-server` with `cfg/demo_cfg.py`) and two must example nodes on different TCP ports, then run `frappy-scan`; verify it lists all three with their ports and the must nodes' firmware, and that `frappy-scan --listen` prints a must node's announcement when it starts
- [ ] 5.3 Manual check on Windows: run the example node, then run `frappy-scan` from another machine on the same network; verify it lists the node
