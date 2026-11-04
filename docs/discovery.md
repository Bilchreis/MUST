# Discovery

Every node takes part in SECoP's
[UDP discovery](https://github.com/SampleEnvironment/SECoP/blob/main/protocol/specification/discovery.rst):
it answers `{"SECoP":"discover"}` broadcasts on UDP port 10767 with its TCP port,
`equipment_id`, `firmware` and `description`, and announces itself once at startup.

## Behavior

- Several MUST and Frappy nodes can run on one host: the port is shared, as Frappy does.
- Clients should broadcast: on Linux a request sent to one host directly reaches only one
  of its nodes.
- A node bound to a loopback address only answers local requests and announces nothing.
- A node without a `firmware` property sends `must-<version>`.
- A reply never exceeds 508 bytes; the description is shortened to fit.
- Problems are logged; discovery never stops a node.

## Turning it off

Set `MUST_DISCOVERY=0`, or call `NodeBuilder::discovery(Discovery::off())` in a Rust
program.

## Firewalls

On a host with a firewall, open 10767/udp for discovery to work.

## Trying it

Frappy's `frappy-scan` finds the example node (it needs only Python, no installation):

```bash
cargo run --example simple_node
# in another terminal, with a checkout of https://github.com/SampleEnvironment/frappy:
python3 frappy/bin/frappy-scan
```
