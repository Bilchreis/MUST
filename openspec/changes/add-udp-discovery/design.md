# Design

## Context

- `Node::spawn` (`must-server/src/runtime/node.rs`) binds the TCP listener synchronously, so the real port is known before any task runs, even after binding to port 0. Every task of a node (accept loop, connections, actors, pollers) runs on the node's `TaskTracker` and ends on its `CancellationToken`. `RunningNode::stop` cancels the token and waits for the tracker.
- Nodes run on the library's shared runtime (`Node::start`) or on the application's own runtime (`Node::spawn`). The C ABI starts its nodes with `Node::start`.
- Process-wide features follow one pattern: an environment variable for C hosts plus an API switch. Monitoring (`MUST_MONITOR`, `monitor::set_enabled`), log files (`MUST_LOG_*`, `NodeBuilder::log_files`) and the TUI (`MUST_TUI`) all work this way.
- Things a node logs about itself go through `Hub::log`, so they reach `tracing`, the node's events, the monitor and the node's log file.
- Frappy's reference implementation (`frappy/protocol/discovery.py`, class `UDPListener`) does the following:
  - one socket per server, `SO_REUSEPORT` on POSIX and `SO_REUSEADDR` on Windows, bound to `0.0.0.0:10767`
  - one broadcast to every interface's broadcast address at startup (`psutil`), falling back to `255.255.255.255`
  - replies from the same socket
  - truncates the description to fit 508 bytes, and turns discovery off if `equipment_id` + `firmware` alone are too long
  - no rate limit and no loopback rule
- `frappy-scan` is Frappy's client. It sends `{"SECoP":"discover"}` to `255.255.255.255:10767` from an ephemeral port and collects replies for one second. In `--listen` mode it binds 10767 with the same reuse options and prints announcements.

## Goals / Non-Goals

**Goals:**
- Interoperate with Frappy on both sides: must nodes and Frappy nodes share port 10767 on one host, and `frappy-scan` finds must nodes.
- Discovery follows the node's lifecycle exactly and can never fail a node or slow its TCP service.
- Same behavior on Linux x86_64, Windows x86_64 and Windows i686.

**Non-Goals:**
- One shared discovery socket per process. See the decision below.
- Answering for nodes in other processes, or acting as a discovery relay.
- Unicast-reliable discovery of several nodes on one Linux host (see Risks).

## Decisions

### One discovery socket per node
Each running node opens its own UDP socket and runs one listener task on its tracker, stopped by its token. The listener is fully tied to the node, so stopping a node needs no extra code. Each socket answers only for its own node, and a node's discovery problems (size limit, bind failure) stay with that node.
- *Alternative considered:* one socket per process that answers for all its nodes, like the monitoring endpoint.
  - It would save a socket per node and answer unicast requests for every node of the process.
  - But it needs its own thread or runtime, because nodes may live on different runtimes. It also needs a registry of nodes, plus start and stop reference counting.
  - Unicast across processes would still be unreliable, so the gain is small. Frappy also runs one socket per node.

### Socket setup with `socket2`
`std::net::UdpSocket` cannot set reuse options before `bind`. So the socket is built with `socket2`:
- `Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))`
- on Unix, `set_reuse_port(true)`, which needs `socket2`'s `all` feature; on Windows, `set_reuse_address(true)`
- `set_broadcast(true)`, `set_nonblocking(true)`, then `bind(0.0.0.0:<port>)`
- `tokio::net::UdpSocket::from_std` inside `runtime.enter()`

These are the same options Frappy sets, which matters. On Linux, a socket joins a port group only if every member set `SO_REUSEPORT` and runs under the same uid. Setting `SO_REUSEADDR` as well would be harmless but is not needed. `socket2` is already in the lock file through tokio. It is added as a direct workspace dependency on the same 0.6 line.

The socket is set up synchronously in `spawn`, after the TCP bind. Any error at this point (socket creation, options or bind) is logged as a warning through `Hub::log`, and the node continues without discovery. The listener task is spawned only when setup succeeded.

### Interface broadcast addresses with `if-addrs`
The startup announcement needs every IPv4 interface's broadcast address on Linux and Windows. The `if-addrs` crate (MIT/Apache-2.0) provides `Ifv4Addr::broadcast` on both. Loopback interfaces are skipped. If the list is empty or fails, the fallback is `255.255.255.255`, as in Frappy.
- *Alternative considered:* compute `ip | !netmask` ourselves from `getifaddrs` and `GetAdaptersAddresses`. That adds `unsafe` FFI to a crate that forbids `unsafe_code`.
- *Alternative considered:* send only to `255.255.255.255`. Linux sends it out of the default-route interface only, so multi-homed lab PCs would announce on just one network.

The list is read once, when the listener starts. The announcement is sent from the listener socket.

### Packet building and truncation
A pure function builds the packet bytes from `(port, equipment_id, firmware, description)` with `serde_json` (compact, UTF-8, keys in spec order via `preserve_order`). Truncation works on the encoded size, not the raw description length, because JSON escapes (`\"`, `\n`, `\u0001`) make the two differ:
1. Encode with the full description. If the packet fits in 508 bytes, use it.
2. Encode with an empty description. If that does not fit either, discovery is off for this node (warning).
3. Otherwise, binary-search the longest prefix of the description, cut at `char` boundaries, whose packet fits.

Frappy's simpler byte-slice approach can exceed the limit when the description contains escaped characters. The packet is built once at listener start, for the node's real port, because none of these properties change while the node runs.

Firmware falls back to `concat!("must-", env!("CARGO_PKG_VERSION"))`. This is the same form the example node uses for its own `firmware` property.

### Listener loop
- `recv_from` into a 1025-byte buffer, so a datagram longer than 1024 bytes shows up as exactly 1025 bytes received and is dropped.
- Parse with `serde_json::from_slice::<Json>`. Accept only an object whose `"SECoP"` is the string `"discover"`.
- Loopback rule: if the node's TCP address `is_loopback()`, drop requests whose source address is not loopback, and skip the announcement.
- Rate limit: a `VecDeque<Instant>` of the last 20 reply times. Reply only if fewer than 20 replies lie within the last second. This enforces the spec's "any one-second window" strictly; a token bucket of capacity 20 would allow up to 40.
- `send_to` errors are logged at debug level and the loop continues. A `recv_from` error other than the Windows `WSAECONNRESET` noise (ICMP port unreachable after a reply) is logged once at warning level, and the listener ends; the node keeps running.
- Each answered request is logged at debug level with the sender address.
- `tokio::select!` on the stop token ends the loop. The socket drops with the task, so `RunningNode::stop` closes it before it returns.

### Configuration API
- `pub struct Discovery { enabled: bool, port: u16, announce: bool }` in `must-server`, re-exported from the crate root.
  - `Discovery::default()` is on, port `DISCOVERY_PORT` (10767), announcing.
  - `Discovery::off()` turns it off.
  - The builder methods `.port(u16)` and `.announce(bool)` serve tests and special setups.
- `NodeBuilder::discovery(Discovery)` stores it in `NodeOptions`.
- `DISCOVERY_VARIABLE = "MUST_DISCOVERY"` is read in `spawn`: a value of `0` (after trimming) turns discovery off, as `monitor::is_enabled` does for `MUST_MONITOR`.
- `must-ffi` needs no change. It inherits the default, and C hosts use the variable.
- *Alternative considered:* off by default. Rejected because must should behave like Frappy, which is on by default, and the feature costs one idle socket. The environment variable gives C hosts an opt-out without a new C function, which SHALL's ABI could not take anyway.

### Testing approach
- Unit tests in the `discovery` module:
  - packet layout and key order
  - Frappy's four cases: empty overhead, basic, ASCII truncation, 4-byte UTF-8 truncation
  - truncation with escaped characters
  - the equipment-id-too-long case
  - the rate limiter, with paused tokio time
- Integration tests (`crates/must-server/tests/discovery.rs`):
  - Each test uses its own discovery port, taken from an ephemeral UDP bind, so parallel tests and other processes on the CI host never collide.
  - Unicast to `127.0.0.1:<port>` covers the reply, ignored datagrams, the size limit, the loopback rule, stopping, `Discovery::off()`, `MUST_DISCOVERY=0` (in a child process, as the monitor tests do for their variables) and the rate limit.
  - The loopback-rule negative case sends from the host's first non-loopback IPv4 address.
  - Broadcast cases (two nodes in one process, the startup announcement) send to or listen on the first interface broadcast address from `if-addrs`. They print a skip message and return when the host has none.
- A manual interop check: run `frappy-scan` against the example node, and run two must nodes next to a Frappy node on one Linux host.

## Risks / Trade-offs

- [On Linux, unicast datagrams to a port group go to only one member socket] → A client that sends a *unicast* discover to a host running several nodes gets one reply. This is inherent to SO_REUSEPORT and matches Frappy. Clients are expected to broadcast. Documented in the README section.
- [Another user's process holds 10767 with `SO_REUSEPORT` on Linux, and the bind fails] → Warning and no discovery for that node, per spec. Typical on shared lab PCs where Frappy runs as a service user. Documented.
- [Windows delivers unicast to only one of several `SO_REUSEADDR` sockets; another program could bind 10767 exclusively] → The same "clients broadcast" expectation applies. An exclusive bind by another program shows up as a bind failure, which gives a warning.
- [Amplification: a 21-byte spoofed request yields a reply of up to 508 bytes] → The strict 20-per-second limit per node bounds what any one node can send.
- [Broadcast tests depend on the CI host's network] → They skip with a message when no broadcast interface exists. The CI test log check would still catch a test binary that does not run at all.
- [32-bit Windows] → Nothing in the design depends on pointer width. `socket2` and `if-addrs` support `i686-pc-windows-msvc`, and CI already builds and tests that target.
- [Firewall: a new UDP listener on all interfaces] → On Windows, the per-program rule created when the user allows the host covers TCP and UDP. On Linux hosts with a firewall, port 10767/udp must be opened for discovery to work, which the README notes.

## Migration Plan

This is additive and needs no migration. Existing Rust and C hosts gain discovery on upgrade. `MUST_DISCOVERY=0` or `Discovery::off()` restores the old behavior. Rollback means reverting the change; no persisted state is involved.
