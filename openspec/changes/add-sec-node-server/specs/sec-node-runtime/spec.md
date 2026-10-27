# Spec Delta

## Purpose

Defines how a running must SEC node connects SECoP requests to application hardware code, keeps clients up to date, and manages connections and its own lifecycle.

## ADDED Requirements

### Requirement: Module handlers
The library SHALL route `read`, `change` and `do` requests for a module to an application-provided handler for that module, which returns a value with optional timestamp and uncertainty, or a SECoP error class with a message.

#### Scenario: Handler error reaches the client
- **WHEN** the handler fails a read of `t1:value` with `CommunicationFailed` "no answer from controller"
- **THEN** the client receives `error_read t1:value` with that error class and text

### Requirement: Value cache
The library SHALL keep the last value, timestamp and uncertainty (or last error) of every parameter, use it for initial updates on `activate`, and answer reads of parameters marked as cached without calling the handler.

#### Scenario: Activation does not hit the hardware
- **WHEN** a client sends `activate`
- **THEN** the initial updates come from the cache and no handler read is made

#### Scenario: Constant parameter
- **WHEN** a client reads a parameter that has a `constant` property
- **THEN** it receives the constant value without a handler call, and the parameter is not part of initial updates

### Requirement: Updates to activated clients
The library SHALL send an `update` (or `error_update`) to every client that has the parameter's module activated whenever a parameter value is obtained by a read, a change, a poll or an application push, and SHALL let the application push new values at any time.

#### Scenario: Application push
- **WHEN** the application pushes `t1:value` 295.2 while two clients are activated and one is not
- **THEN** both activated clients receive `update t1:value [295.2, {...}]` and the third receives nothing

### Requirement: Ordering of side effects
The library SHALL deliver all updates caused while handling a `change` or `do` (including the changed parameter itself) to activated clients before the `changed` or `done` reply is sent, and all initial updates before `active`.

#### Scenario: Status update before changed
- **WHEN** a change handler sets `t1:status` to BUSY and returns the new `t1:target`
- **THEN** an activated client receives `update t1:status` and `update t1:target` before `changed t1:target`

### Requirement: Polling
The library SHALL optionally poll parameters by calling the handler's read at an application-configured interval per module or parameter, and SHALL send the result as an update.

#### Scenario: Periodic read
- **WHEN** `t1:value` is configured to poll every second and a client is activated
- **THEN** the client receives about one `update t1:value` per second

### Requirement: Handler timeout
The library SHALL fail a request with `TimeoutError` when its handler does not finish within the node's `timeout` property (default 10 s), and SHALL keep serving other requests meanwhile.

#### Scenario: Hanging handler
- **WHEN** a read handler for `t1:value` never returns and the node timeout is 2 s
- **THEN** the client receives `error_read t1:value` with `TimeoutError` after about 2 s, and a concurrent `ping` is answered immediately

### Requirement: Request concurrency
The library SHALL process requests of one connection in the order received and SHALL call a module's handler for one request at a time, while requests for different modules and from different connections may run concurrently.

#### Scenario: Two modules in parallel
- **WHEN** a slow read on module `a` is in progress and another client reads module `b`
- **THEN** the read on `b` completes without waiting for `a`

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
