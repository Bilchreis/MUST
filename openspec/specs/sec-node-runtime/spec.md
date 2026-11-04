# sec-node-runtime Specification

## Purpose

Defines how a running must SEC node connects SECoP requests to application hardware code, keeps clients up to date, and manages connections and its own lifecycle.

## Requirements

### Requirement: Module handlers
The library SHALL route `read`, `change`, `do` and `check` requests for a module to an application-provided handler for that module, which returns a value with optional timestamp and uncertainty (for `check`: acceptance), or a SECoP error class with a message. A handler that does not implement `check` SHALL answer it with `NotImplemented`.

#### Scenario: Handler error reaches the client
- **WHEN** the handler fails a read of `t1:value` with `CommunicationFailed` "no answer from controller"
- **THEN** the client receives `error_read t1:value` with that error class and text

#### Scenario: Check not implemented by the handler
- **WHEN** `t1:target` is declared `checkable: true` but the module's handler does not implement `check`
- **THEN** a client sending `check t1:target 5` receives `error_check t1:target` with a `NotImplemented` error report

#### Scenario: Check counts as a request of the module
- **WHEN** a slow `change` of `t1:target` is in progress and the same module receives a `check`
- **THEN** the check handler is called only after the change handler has finished, as for any other request of the module

### Requirement: Value cache
The library SHALL keep the last value, timestamp and uncertainty (or last error) of every parameter, use it for initial updates on `activate`, and answer reads of parameters marked as cached without calling the handler.

#### Scenario: Activation does not hit the hardware
- **WHEN** a client sends `activate`
- **THEN** the initial updates come from the cache and no handler read is made

#### Scenario: Constant parameter
- **WHEN** a client reads a parameter that has a `constant` property
- **THEN** it receives the constant value without a handler call, and the parameter is not part of initial updates

### Requirement: Updates to activated clients
The library SHALL send an `update` (or `error_update`) to every client that has the parameter's module activated whenever a parameter value is obtained by a read, a change, a poll or an application push, and SHALL let the application push new values at any time. Every published value SHALL be sent, even when it equals the previous value of the parameter.

#### Scenario: Application push
- **WHEN** the application pushes `t1:value` 295.2 while two clients are activated and one is not
- **THEN** both activated clients receive `update t1:value [295.2, {...}]` and the third receives nothing

#### Scenario: Unchanged value
- **WHEN** a poll of `t1:value` returns 295.2 twice in a row while a client is activated
- **THEN** the client receives two `update t1:value` messages, each with its own timestamp

### Requirement: Ordering of side effects
The library SHALL deliver all updates caused while handling a `change` or `do` (including the changed parameter itself) to activated clients before the `changed` or `done` reply is sent, and all initial updates before `active`.

#### Scenario: Status update before changed
- **WHEN** a change handler sets `t1:status` to BUSY and returns the new `t1:target`
- **THEN** an activated client receives `update t1:status` and `update t1:target` before `changed t1:target`

### Requirement: Polling
The library SHALL poll every module unless its module-level polling is turned off, and SHALL publish each poll result (value or error) as an update. By default a module SHALL poll in two tiers:
- fast tier: `value` and `status` of a module whose base class is `Readable`, `Writable` or `Drivable`, every 5 s unless the application sets another fast interval
- slow tier: every other parameter that is neither constant nor cached, every 60 s unless the application sets another slow interval

A module configured with a uniform interval SHALL instead poll all non-constant, non-cached parameters at that one interval. A module whose module-level polling is turned off SHALL poll only parameters with an interval of their own. For each parameter the effective interval SHALL be, in order of precedence: the parameter's own interval, the module's `pollinterval` parameter (for the fast tier or the uniform interval), the module's configured interval, and the defaults. Constant and cached parameters and the `pollinterval` parameter itself SHALL never be polled.

#### Scenario: Periodic read
- **WHEN** `t1:value` is configured to poll every second and a client is activated
- **THEN** the client receives about one `update t1:value` per second

#### Scenario: Default tiers
- **WHEN** a `Drivable` module with `value`, `status`, `target` and `ramp` and no poll settings runs for 130 s
- **THEN** `value` and `status` are each polled about 26 times, and `target` and `ramp` about twice

#### Scenario: Fast and slow tier
- **WHEN** that module is configured with a fast interval of 1 s and runs for 130 s
- **THEN** `value` and `status` are each polled about 130 times, and `target` and `ramp` about twice

#### Scenario: Module without a fast tier
- **WHEN** a module with `interface_classes` `[]` and parameters `value` and `x` and no poll settings runs for 130 s
- **THEN** both are polled about twice

#### Scenario: Polling turned off
- **WHEN** a module's polling is turned off and only its `ramp` has an interval of its own (5 s)
- **THEN** only `ramp` is polled, every 5 s

#### Scenario: Parameter interval wins
- **WHEN** a module polls in tiers at 1 s and its `ramp` parameter has its own interval of 5 s
- **THEN** `ramp` is polled every 5 s, not every 60 s

#### Scenario: Uniform interval
- **WHEN** a module with `value`, `status` and `target` is configured with a uniform interval of 2 s
- **THEN** all three are polled about every 2 s

### Requirement: Handler timeout
The library SHALL fail a request with `TimeoutError` when its handler does not finish within the node's `timeout` property (default 10 s), and SHALL keep serving other requests meanwhile.

#### Scenario: Hanging handler
- **WHEN** a read handler for `t1:value` never returns and the node timeout is 2 s
- **THEN** the client receives `error_read t1:value` with `TimeoutError` after about 2 s, and a concurrent `ping` is answered immediately

### Requirement: Request concurrency
The library SHALL process requests of one connection in the order received, except that repeated reads are taken together as described in "Repeated reads on one connection", and SHALL call a module's handler for one request at a time, while requests for different modules and from different connections may run concurrently. A module configured for concurrent requests SHALL instead have its handler called for each request (`read`, `change`, `do`, poll tick) as it arrives, while other requests of that module are still running, up to a limit of 64 running requests per module; further requests wait until one finishes.

#### Scenario: Two modules in parallel
- **WHEN** a slow read on module `a` is in progress and another client reads module `b`
- **THEN** the read on `b` completes without waiting for `a`

#### Scenario: One call at a time by default
- **WHEN** a read of `m:x` on a module without concurrent requests waits on the hardware and another client reads `m:y`
- **THEN** the handler is called for `m:y` only after the read of `m:x` has finished

#### Scenario: Concurrent requests
- **WHEN** a read of `m:x` on a module configured for concurrent requests waits on the hardware and another client reads `m:y`
- **THEN** the handler is called for `m:y` at once, and `m:y` is answered without waiting for `m:x`

#### Scenario: Order within a connection
- **WHEN** one client sends `change m:target 5` and then `read m:target` to a module configured for concurrent requests
- **THEN** the read is handled only after the `changed` reply, as on any other module

### Requirement: Connections
The library SHALL listen on the configured interface and TCP port, accept many simultaneous connections, and clean up a connection's activation state when it closes.

#### Scenario: Port in use
- **WHEN** a node is started on a port that is already in use
- **THEN** starting fails with an error naming the address, and no node runs

### Requirement: Slow clients
The library SHALL buffer outgoing messages per connection so a slow client never delays other clients or handlers. Replies to a client's own requests never cause a disconnect: while its buffer exceeds the per-node limit (configurable, default 16 MiB), the node stops reading that client's next request. Only an unsolicited update that pushes the buffer beyond the limit closes the connection, logged with the client address.

#### Scenario: Stuck client does not delay others
- **WHEN** one activated client stops reading while `t1:value` is pushed 100 times per second, and a second activated client sends `change t1:target 5`
- **THEN** the second client keeps receiving every update without added delay and gets its `changed t1:target` reply

#### Scenario: Stuck client is disconnected
- **WHEN** updates for the stuck client push its buffered outgoing data beyond the node's limit
- **THEN** the node closes that connection, logs it with the client address, and frees its buffered data

#### Scenario: Large reply while updates are queued
- **WHEN** an activated client with updates already buffered calls a command whose result is 50 MiB, and keeps reading normally
- **THEN** it receives the queued updates and the complete `done` reply, and the connection stays open

#### Scenario: Requests without reading replies
- **WHEN** a client that has not activated updates sends `read` requests in a loop without reading any replies
- **THEN** once its buffer exceeds the limit the node stops reading its requests, the node's memory for that client stays near the limit, the connection stays open, and other clients are unaffected

#### Scenario: Backpressure is lifted
- **WHEN** that client starts reading again
- **THEN** it receives all buffered replies in order and the node resumes processing its remaining requests

#### Scenario: Reconnect after disconnect
- **WHEN** the disconnected client reconnects and sends `activate`
- **THEN** it receives the current value of every parameter as initial updates, followed by `active`

#### Scenario: Configured limit
- **WHEN** a node is configured with an outgoing limit of 1 MiB and an activated client stops reading
- **THEN** the connection is closed once updates push about 1 MiB of buffered data for it, not at the default 16 MiB

#### Scenario: Large describe reply is not a slow client
- **WHEN** a client requests `describe` for a node whose structure report is 2 MiB and reads it normally
- **THEN** the reply is delivered completely and the connection stays open

### Requirement: Node lifecycle
The library SHALL run several independent nodes in one process and SHALL stop a node on request by closing its listener and connections and stopping its polling, without affecting other nodes.

#### Scenario: Stop one of two nodes
- **WHEN** two nodes run on ports 2055 and 2056 and the first is stopped
- **THEN** port 2055 is closed, its clients are disconnected, and clients of port 2056 keep working

### Requirement: Poll interval parameter
The library SHALL drive a module's polling by its `pollinterval` parameter when the module declares one. A `change` of it SHALL go to the module's handler first, and the value the handler returns SHALL be published as usual. Every published value of it (from a change, the initial value or an application push) SHALL re-time the module's polling: its fast tier, or its uniform interval. A value of 0 SHALL stop that polling until a positive value is published. Parameters with an interval of their own and the slow tier SHALL keep their intervals. Without an initial value, the parameter SHALL start with the module's fast or uniform interval (the configured one, else the default 5 s). On a module whose module-level polling is turned off, the parameter SHALL have no effect on polling.

#### Scenario: Client tunes the rate
- **WHEN** a client changes `cryo:pollinterval` from 1 to 0.2 on a module whose fast tier polls at 1 s
- **THEN** the handler's change is called with 0.2, the client receives `changed cryo:pollinterval [0.2, ...]`, and `cryo:value` is polled about every 0.2 s from then on

#### Scenario: Handler adjusts the value
- **WHEN** a client changes `cryo:pollinterval` to 0.01 and the handler returns 0.1, its hardware minimum
- **THEN** the client receives `changed cryo:pollinterval [0.1, ...]` and polling runs every 0.1 s

#### Scenario: Handler refuses
- **WHEN** the handler fails the change of `cryo:pollinterval`
- **THEN** the client receives the error and the polling interval is unchanged

#### Scenario: Zero stops polling
- **WHEN** `cryo:pollinterval` is set to 0
- **THEN** `cryo:value` and `cryo:status` are no longer polled, while a parameter with its own interval keeps being polled

#### Scenario: Application push re-times
- **WHEN** the application pushes `cryo:pollinterval` 5.0
- **THEN** activated clients receive the update and the fast tier polls every 5 s

#### Scenario: Never polled
- **WHEN** a tiered module with a `pollinterval` parameter runs for several minutes
- **THEN** the handler is never asked to read `pollinterval` by a poll

### Requirement: Poll hook
The library SHALL, at each poll tick of a module, call the module handler's poll hook once with the names of all parameters of that module due at that tick, in declaration order. The default hook SHALL read each parameter through the handler's read and publish the value or error. A handler that overrides the hook is responsible for publishing the values it obtains, and parameters it does not publish SHALL produce no update. A tick SHALL be skipped while the module's request backlog is full, and the hook SHALL be subject to the node's handler timeout.

#### Scenario: Parameters due together
- **WHEN** a tiered module's fast and slow tiers fall due at the same tick
- **THEN** the hook is called once with `value`, `status` and the slow-tier parameters

#### Scenario: Default hook
- **WHEN** a handler implements only `read` and its module is polled
- **THEN** each due parameter is read and its value sent as an update

#### Scenario: One hardware transaction
- **WHEN** a handler overrides the hook and publishes `value` and `status` through its context from one controller query
- **THEN** activated clients receive both updates and the handler's `read` is not called by the poll

#### Scenario: Hook times out
- **WHEN** the hook does not return within the node timeout
- **THEN** the hook is abandoned, the error is logged, and the next tick polls again

### Requirement: Shared waiting reads
When the library starts a read of a parameter, for a client's `read` or for a poll, every request still waiting for the module that would read the same parameter SHALL share that read instead of calling the handler again: a client's `read`, and that parameter in a waiting poll tick. Each joined `read` SHALL receive the result (value or error) as its reply, and the result SHALL be published once.

#### Scenario: Reads waiting behind a change
- **WHEN** the handler of `m` is busy with a `change` and two clients read `m:value` meanwhile
- **THEN** after the change the handler's read of `m:value` is called once, and both clients receive a reply with the same value and timestamp

#### Scenario: Two clients share a read, a third is updated
- **WHEN** client C has activated `m`, the handler of `m` is busy, and clients A and B read `m:p` meanwhile
- **THEN** the handler reads `m:p` once; A and B each receive `reply m:p` with the same value and timestamp, and C receives one `update m:p` with that value and no reply

#### Scenario: Reads around a waiting change
- **WHEN** the handler of `m` is busy, and a `read m:target` from client A, a `change m:target 5` from client B and a `read m:target` from client C wait in that order
- **THEN** the handler reads `m:target` once, before B's change, and A and C receive that value; B then receives `changed m:target [5, ...]` and activated clients an `update m:target` with 5

#### Scenario: Poll tick across a change
- **WHEN** the handler of `m` is busy, and a `read m:target` from client A, a `change m:target 5` from client B and a poll tick due for `m:target` wait in that order
- **THEN** the poll makes no handler read of `m:target`; activated clients receive one `update m:target` from A's read and one from B's change

#### Scenario: Shared error
- **WHEN** a read shared by two clients fails with `CommunicationFailed`
- **THEN** both clients receive `error_read` with `CommunicationFailed`

### Requirement: Shared running reads
On every module, a request to read a parameter that is being read SHALL share that running read instead of calling the handler again: always for a poll, and for a client's `read` only if the running read started after the last `change` or `do` of the module that had finished before the `read` arrived. A shared read SHALL continue until it finishes or the node timeout passes, even when the request that started it is gone.

#### Scenario: Client joins a client's read
- **WHEN** client A's read of `m:value` waits on the hardware and client B reads `m:value`
- **THEN** the handler's read of `m:value` is called once and both clients receive its result

#### Scenario: Idle module, reads in close succession
- **WHEN** a module without concurrent requests is idle, client A reads `m:p`, and client B reads `m:p` while A's read is running
- **THEN** B's read shares A's: the handler reads `m:p` once and both clients receive its result

#### Scenario: Two clients share a running read, a third is updated
- **WHEN** client C has activated `m`, client A's read of `m:p` is running, and client B reads `m:p`
- **THEN** the handler reads `m:p` once; A and B each receive `reply m:p` with the same value and timestamp, and C receives one `update m:p` with that value and no reply

#### Scenario: Client joins a poll
- **WHEN** a poll's read of `m:value` is running, no `change` or `do` of `m` has finished since it started, and a client reads `m:value`
- **THEN** the handler is not called again; the client receives the poll's result as its reply, and activated clients receive one `update m:value`

#### Scenario: Poll joins a client's read
- **WHEN** a client's read of `m:value` is running and a poll tick falls due for `m:value`
- **THEN** the handler is not called again for the poll, and activated clients receive one `update m:value` from the client's read

#### Scenario: Read after changed sees the change
- **WHEN** on a module configured for concurrent requests a read of `m:status` started while `m` was idle is still running, client B's `change m:target 5` finishes with the handler publishing `m:status` BUSY, and B then reads `m:status`
- **THEN** B's read does not share the earlier read; the handler is called again and B receives the BUSY status

#### Scenario: First reader leaves
- **WHEN** client A starts a read of `m:value`, client B joins it, and A disconnects before the result
- **THEN** the read continues and B receives the reply

### Requirement: Repeated reads on one connection
While a client's `read` of a parameter is being handled, the library SHALL also take that client's following requests as long as each is a `read` of the same parameter, and pass each on at once, so that it can share the running read under the rules for shared reads. It SHALL send the replies in the order of the requests. The first other request SHALL wait until all earlier requests of that client are answered.

#### Scenario: Repeated read shares the running read
- **WHEN** client A sends `read m:p` twice in a row, and the first is still being read when the second arrives
- **THEN** the handler reads `m:p` once, and A receives two `reply m:p` with the same value and timestamp

#### Scenario: Another request in between
- **WHEN** client A sends `read m:p`, `read m:q` and `read m:p` while the first is still being read
- **THEN** `read m:q` is handled after the first reply and the second `read m:p` after the reply to `m:q`; A receives the replies in the order p, q, p, and the handler reads `m:p` twice

#### Scenario: Read after another client's change
- **WHEN** on a module configured for concurrent requests client A's read of `m:status` is running, client B's `change m:target 5` finishes, and A then sends `read m:status` again
- **THEN** A's second read does not share the first, and its reply comes from a read after B's change

#### Scenario: Slow client
- **WHEN** client A's outgoing messages exceed the node's limit while its `read m:p` is being handled
- **THEN** the node takes none of A's following requests, not even a `read m:p`, until A has received enough

### Requirement: Concurrent polls
On a module configured for concurrent requests, the default poll hook SHALL start the reads of all parameters due at a tick at once and publish each result when it arrives. A tick SHALL not wait for an earlier tick's reads that are still running.

#### Scenario: Reads started together
- **WHEN** 11 parameters of a module configured for concurrent requests fall due at one tick and each handler read takes 100 ms
- **THEN** all 11 updates are sent about 100 ms after the tick, not over 1.1 s

#### Scenario: Hanging read
- **WHEN** the read of `m:p0` hangs and the next tick falls due for `m:p0` and `m:p1`
- **THEN** `m:p1` is read and published, and the handler is not called again for `m:p0` while its read is still running
