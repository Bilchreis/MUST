# Proposal

## Why

A must node can only be reached by a client that already knows its host and TCP port. SECoP defines a UDP discovery protocol for this: a node listens on port 10767, answers a `{"SECoP":"discover"}` broadcast with its port and identity, and can announce itself at startup ([discovery.rst](https://github.com/SampleEnvironment/SECoP/blob/a6e90ebba846160b815295d4def15e0bcce17c7c/protocol/specification/discovery.rst)). Frappy nodes already do this, and tools like `frappy-scan` and the Frappy GUI use it. A host such as LabVIEW may run several must nodes, on ports picked per setup, so discovery saves users from writing those ports down and looking them up.

## What Changes

- **Every running node takes part in UDP discovery**, Rust API and C ABI alike. It is on by default, like Frappy, and stops with the node.
  - The node opens a UDP socket on `0.0.0.0:10767` with the port shared, so many nodes, and Frappy nodes, can run on one host: `SO_REUSEPORT` on Linux, `SO_REUSEADDR` on Windows, the same options Frappy sets.
  - It answers each valid `{"SECoP":"discover"}` packet by sending `{"SECoP":"node","port":…,"equipment_id":…,"firmware":…,"description":…}` to the sender. The packet is compact JSON with `port` set to the node's actual TCP port.
  - It ignores everything else on the port: malformed JSON, other `SECoP` values (including other nodes' announcements), and oversized packets.
  - At startup it broadcasts the same packet once to the broadcast address of every IPv4 interface. With no such interface it falls back to `255.255.255.255`.
- **Packet size:** a reply never exceeds SECoP's 508-byte safe UDP size.
  - `description` is shortened on a UTF-8 character boundary until the encoded packet fits.
  - If `equipment_id` and `firmware` alone are too long, the node stays out of discovery and logs a warning. It keeps serving TCP.
- **`firmware`:** the node's `firmware` property, or `must-<version>` when the node has none. The spec requires the key, and many C hosts never set the property.
- **Loopback-bound nodes** (e.g. `127.0.0.1:0`) answer only requests from loopback senders and send no announcement, so they never advertise a port that other hosts cannot reach.
- **Replies are rate-limited** per node: at most 20 per second, and further requests in that second are dropped. A 20-byte spoofed request cannot turn a node into a flood source.
- **Failure never stops a node.** If the UDP port cannot be bound (another user's process holds it with `SO_REUSEPORT` on Linux, or a firewall or policy blocks it), the node logs a warning and runs without discovery.
- **Configuration:**
  - `NodeBuilder::discovery(Discovery)`: `Discovery::default()` (on, port 10767, announce at startup), `Discovery::off()`, and `.port(p)` / `.announce(bool)` for tests and special setups.
  - The environment variable `MUST_DISCOVERY=0` turns discovery off for every node in the process. This is the switch for C hosts, as `MUST_MONITOR=0` is for monitoring.
- **Docs:**
  - README: a "Discovery" section that shows `frappy-scan` finding the example node.
  - `COMPATIBILITY.md`: a "(new)" entry under "Library behavior", since SHALL nodes opened no UDP port.

## Capabilities

### New Capabilities
- `sec-node-udp-discovery`: the SECoP UDP discovery protocol on the node side: listener, reply format and size limit, startup announcement, sharing the port, loopback rule, rate limit, failure handling and configuration. It is separate from `node-monitoring-discovery`, which finds local monitoring endpoints through files and has nothing to do with SECoP.

### Modified Capabilities
None. `sec-node-runtime`'s "Node lifecycle" already requires that stopping a node closes what it opened; the new spec states that stopping the node closes its discovery socket.

## Impact

- `must-server`:
  - new `discovery` module: socket setup, the listener task, packet building and truncation, interface broadcast addresses
  - `model.rs`: `Discovery` options in `NodeOptions` and `NodeBuilder::discovery`
  - `runtime/node.rs`: start the listener on the node's tracker with its stop token, once the TCP port is known
  - `lib.rs`: re-export and crate docs
- Dependencies:
  - `socket2` (already in the lock file through tokio): sets the reuse options before `bind`
  - `if-addrs`: lists interface broadcast addresses on Linux and Windows
  - both MIT/Apache-2.0, checked by `cargo deny`
- `must-ffi`: no code change. C nodes are built with `NodeBuilder` and get discovery by default. `MUST_DISCOVERY` is read in `must-server`.
- Tests:
  - unit tests for packet building and truncation, ported from Frappy's `test_discovery.py`
  - integration tests on a non-standard UDP port, on all three CI targets
  - a manual interop check with `frappy-scan`
- Networking: every node now listens on UDP 10767 on all interfaces. Windows Firewall already asks about the host program for its TCP port, and that rule covers UDP too.
- Out of scope:
  - a discovery client (scanning) in must or must-tui
  - periodic announcements (the spec allows them, Frappy does not send them)
  - IPv6 discovery (the spec is IPv4 broadcast)
  - showing discovery state in the monitoring protocol or the TUI
