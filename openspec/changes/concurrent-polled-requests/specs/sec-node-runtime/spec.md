# Spec Delta

## MODIFIED Requirements

### Requirement: Request concurrency
The library SHALL process requests of one connection in the order received and SHALL call a module's handler for one request at a time, while requests for different modules and from different connections may run concurrently. A module configured for concurrent requests SHALL instead have its handler called for each request (`read`, `change`, `do`, poll tick) as it arrives, while other requests of that module are still running, up to a limit of 64 running requests per module; further requests wait until one finishes.

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

## ADDED Requirements

### Requirement: Shared waiting reads
When the library starts a read of a parameter, for a client's `read` or for a poll, every request still waiting for the module that would read the same parameter SHALL share that read instead of calling the handler again: a client's `read`, and that parameter in a waiting poll tick. Each joined `read` SHALL receive the result (value or error) as its reply, and the result SHALL be published once.

#### Scenario: Reads waiting behind a change
- **WHEN** the handler of `m` is busy with a `change` and two clients read `m:value` meanwhile
- **THEN** after the change the handler's read of `m:value` is called once, and both clients receive a reply with the same value and timestamp

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
On a module configured for concurrent requests, a request to read a parameter that is being read SHALL share that running read instead of calling the handler again: always for a poll, and for a client's `read` only if the running read started after the last `change` or `do` of the module that had finished before the `read` arrived. A shared read SHALL continue until it finishes or the node timeout passes, even when the request that started it is gone.

#### Scenario: Client joins a client's read
- **WHEN** client A's read of `m:value` waits on the hardware and client B reads `m:value`
- **THEN** the handler's read of `m:value` is called once and both clients receive its result

#### Scenario: Client joins a poll
- **WHEN** a poll's read of `m:value` is running, no `change` or `do` of `m` has finished since it started, and a client reads `m:value`
- **THEN** the handler is not called again; the client receives the poll's result as its reply, and activated clients receive one `update m:value`

#### Scenario: Poll joins a client's read
- **WHEN** a client's read of `m:value` is running and a poll tick falls due for `m:value`
- **THEN** the handler is not called again for the poll, and activated clients receive one `update m:value` from the client's read

#### Scenario: Read after changed sees the change
- **WHEN** a read of `m:status` started while `m` was idle is still running, client B's `change m:target 5` finishes with the handler publishing `m:status` BUSY, and B then reads `m:status`
- **THEN** B's read does not share the earlier read; the handler is called again and B receives the BUSY status

#### Scenario: First reader leaves
- **WHEN** client A starts a read of `m:value`, client B joins it, and A disconnects before the result
- **THEN** the read continues and B receives the reply

### Requirement: Concurrent polls
On a module configured for concurrent requests, the default poll hook SHALL start the reads of all parameters due at a tick at once and publish each result when it arrives. A tick SHALL not wait for an earlier tick's reads that are still running.

#### Scenario: Reads started together
- **WHEN** 11 parameters of a module configured for concurrent requests fall due at one tick and each handler read takes 100 ms
- **THEN** all 11 updates are sent about 100 ms after the tick, not over 1.1 s

#### Scenario: Hanging read
- **WHEN** the read of `m:p0` hangs and the next tick falls due for `m:p0` and `m:p1`
- **THEN** `m:p1` is read and published, and the handler is not called again for `m:p0` while its read is still running
