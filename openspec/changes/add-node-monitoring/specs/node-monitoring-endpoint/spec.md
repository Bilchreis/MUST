# Spec Delta

## Purpose

Lets other local processes observe the must SEC nodes of a process (their status, clients, traffic and logs) through one read-only monitoring endpoint per process, without affecting the nodes.

## ADDED Requirements

### Requirement: One endpoint per process
The library SHALL open one monitoring endpoint per process when the first node of the process starts, covering all nodes of that process, and close it when the last node stops or the library shuts down.

#### Scenario: Two nodes, one endpoint
- **WHEN** a process starts nodes on ports 2055 and 2056
- **THEN** exactly one monitoring endpoint exists for the process and its snapshot lists both nodes

#### Scenario: Last node stops
- **WHEN** the last node of a process is stopped
- **THEN** the endpoint closes and its registration is removed

### Requirement: Loopback only with token
The endpoint SHALL listen only on the loopback interface and SHALL require a client to send, as its first message, a hello containing the endpoint's random access token from the registration; otherwise it closes the connection.

#### Scenario: Wrong token
- **WHEN** a client connects and sends a hello with a wrong token
- **THEN** the endpoint closes the connection without sending any data

### Requirement: Versioned line protocol
The endpoint SHALL exchange UTF-8 JSON objects, one per line, each with a `type` field; the hello and its answer carry a protocol version, and the endpoint SHALL reject a hello whose major version it does not support.

#### Scenario: Version mismatch
- **WHEN** a client sends a hello with protocol major version 99
- **THEN** the endpoint answers with an error message naming its supported version and closes the connection

### Requirement: Snapshot on connect
After a valid hello the endpoint SHALL send a snapshot with: process id, process name, library version, start time; per node its name, equipment_id, listen address, state, structure report and the current value, timestamp or error of every parameter; the connected SECoP clients; and the buffered log records.

#### Scenario: Snapshot content
- **WHEN** a monitor connects to a process with node `cryo` (module `t1`) and one SECoP client connected
- **THEN** the snapshot contains node `cryo` with its structure report, the current `t1:value`, and one client entry with its peer address and connect time

### Requirement: Event stream
After the snapshot the endpoint SHALL send events for: node started, stopped or failed; SECoP client connected, disconnected, activated or deactivated; message received or sent (direction, node, client, text, time); parameter value or error updated; log record.

#### Scenario: Traffic event
- **WHEN** a SECoP client sends `read t1:value` to node `cryo`
- **THEN** a connected monitor receives a message event for the request and one for the `reply`, both naming node `cryo` and that client

#### Scenario: Client disconnect
- **WHEN** a SECoP client closes its connection
- **THEN** connected monitors receive a client-disconnected event

### Requirement: Traffic detail control
The endpoint SHALL send message texts only to monitors that subscribed to traffic for a node, SHALL truncate texts longer than 4 KiB (marking them as truncated), and SHALL always send per-node message counters once per second.

#### Scenario: Not subscribed to traffic
- **WHEN** a monitor did not subscribe to traffic of node `cryo`
- **THEN** it receives no message events for `cryo` but receives its counters every second

#### Scenario: Long describe reply
- **WHEN** a subscribed monitor observes a 200 KiB `describing` reply
- **THEN** the message event carries the first 4 KiB and a truncated flag

### Requirement: Log capture
The library SHALL capture log records (time, level, source node and module if known, message) from the must libraries and from the host application's `tracing` events into a per-process ring buffer of the most recent 2000 records and send new records as events.

#### Scenario: Handler error logged
- **WHEN** a module handler returns a `HardwareError`
- **THEN** a log record with level warn or error naming the node and module is buffered and sent to monitors

### Requirement: Never block the nodes
The endpoint SHALL never delay a SEC node: each monitor has a bounded queue, and when it is full the oldest events are dropped and a `lagged` event with the number of dropped events is sent instead.

#### Scenario: Stalled monitor
- **WHEN** a monitor stops reading while a node sends 10000 updates per second
- **THEN** the node's SECoP clients see no added latency and the monitor later receives a `lagged` event

### Requirement: Read-only and multiple monitors
The endpoint SHALL accept several monitors at the same time and SHALL offer no way to change node state, parameters or connections.

#### Scenario: Two monitors
- **WHEN** two monitors are connected
- **THEN** both receive the same events

### Requirement: Disable monitoring
The library SHALL not open an endpoint when monitoring is disabled through the environment variable `MUST_MONITOR=0` or the Rust API.

#### Scenario: Disabled by environment
- **WHEN** a process starts a node with `MUST_MONITOR=0`
- **THEN** no endpoint is opened and no registration is written, and the node works normally
