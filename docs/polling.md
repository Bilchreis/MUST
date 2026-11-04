# Polling

A node polls its modules and sends every result to activated clients as an update, even
when the value did not change.

## Defaults

`must-server` modules are polled in two tiers, inferred from the interface class:

- `value` and `status` of a `Readable`, `Writable` or `Drivable` module every 5 s,
- every other parameter that is neither constant nor cached every 60 s.

## Changing it

- `Module::poll_interval(d)` changes the 5 s, `Module::slow_poll_interval(d)` the 60 s.
- `Module::poll_all(d)` polls all those parameters every `d`, as SHALL did.
- `Module::no_polling()` turns the module's polling off.
- `Parameter::poll_interval(d)` gives a parameter an interval of its own, which wins over
  the module's (also with polling off).

A module with a `double` parameter named `pollinterval` lets clients change the rate at
runtime: a `change` goes to the handler, and the value it returns (or any value pushed
later) replaces the module's fast or uniform interval; 0 stops that polling. The node
never polls `pollinterval` itself.

At each tick the node calls `ModuleHandler::poll` once with all parameters of the module
that are due; override it to read them in one hardware transaction. The crate
documentation of [`must-server`](api/must_server/index.html#polling) has the details,
including how polling interacts with concurrent requests and shared reads.

**API change:** before 0.1.0, `Module::poll_interval` polled all parameters at one
interval (use `Module::poll_all` for that), and modules without poll settings were not
polled.

## C and LabVIEW programs

Programs that use SHALL's `pollinterval` properties through the C interface keep SHALL's
behavior; see [COMPATIBILITY.md](https://github.com/Bilchreis/MUST/blob/main/COMPATIBILITY.md).

## Trying it

The example node polls the noisy readback of its simulated cryostat:

```bash
cargo run --example simple_node
# in another terminal: nc localhost 2055, then `activate` and `change cryo:pollinterval 0.2`
```
