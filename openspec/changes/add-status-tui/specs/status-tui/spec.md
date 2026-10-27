# Spec Delta

## Purpose

A terminal UI that shows, in one place, all must SEC nodes running on the machine for the current user, with their status, connected clients, message traffic and logs, replacing SHALL's Qt status window.

## ADDED Requirements

### Requirement: Show all running nodes
The TUI SHALL discover all monitoring endpoints of the current user, connect to each, and list every node of every process with process name and pid, equipment_id, listen address, state, number of connected clients, messages per second, and the most severe module status.

#### Scenario: Several hosts
- **WHEN** a LabVIEW process runs nodes `cryo` and `magnet` and a second process runs node `test`
- **THEN** the overview lists all three nodes, grouped by process

### Requirement: Processes come and go
The TUI SHALL rescan for endpoints at least every 2 seconds, add new processes without user action, and mark a process whose endpoint closed or whose process ended as gone, removing it after 60 seconds unless the user pins it.

#### Scenario: New node appears
- **WHEN** a new process starts a node while the TUI is open
- **THEN** the node appears in the overview within 3 seconds

#### Scenario: Host exits
- **WHEN** a host process exits
- **THEN** its nodes are shown as gone, and disappear after 60 seconds

### Requirement: Node detail
The TUI SHALL show for a selected node its modules with their parameters (value formatted with the datainfo's `fmtstr` and unit, timestamp age, or error), each module's status colored by status group (DISABLED, IDLE, WARN, BUSY, ERROR), and the connected SECoP clients with address, connect time and activation state.

#### Scenario: Busy module
- **WHEN** module `t1` has status `[300, "ramping"]`
- **THEN** the detail view shows `t1` in the BUSY color with the text `ramping`

#### Scenario: Parameter error
- **WHEN** the last read of `t1:value` failed with `HardwareError`
- **THEN** the detail view shows the error class and message instead of a value

### Requirement: Traffic view
The TUI SHALL show the SECoP messages of a selected node in a scrolling list with time, direction, client and text, subscribing to traffic only while the view is open, and SHALL support pausing and a text filter.

#### Scenario: Live traffic
- **WHEN** the traffic view of `cryo` is open and a client sends `read t1:value`
- **THEN** the request and the reply appear within a second

#### Scenario: Pause
- **WHEN** the user pauses the traffic view
- **THEN** the list stops scrolling while new messages are kept, and resumes with them on unpause

### Requirement: Log view
The TUI SHALL show the log records of all processes merged by time, filterable by minimum level, process, node and a text search.

#### Scenario: Filter errors of one node
- **WHEN** the user sets level filter to error and node filter to `cryo`
- **THEN** only error records attributed to `cryo` are shown

### Requirement: Lag and connection problems are visible
The TUI SHALL indicate when an endpoint reported dropped events (`lagged`), could not be connected, or rejected the protocol version.

#### Scenario: Incompatible endpoint
- **WHEN** an endpoint answers with an unsupported protocol version
- **THEN** its process is listed with a version error instead of nodes

### Requirement: Keyboard operation
The TUI SHALL be fully usable with the keyboard (arrow keys, Enter, Tab between views, `/` search, `p` pause, `?` help, `q` quit) and SHALL show the key bindings on `?`.

#### Scenario: Help
- **WHEN** the user presses `?`
- **THEN** a help overlay lists all key bindings

### Requirement: Terminal compatibility
The TUI SHALL work in Windows console hosts (conhost and Windows Terminal) and common Linux terminals at 80×24 or larger, redraw on resize, honor `NO_COLOR`, and restore the terminal state on exit, also after a panic.

#### Scenario: Resize
- **WHEN** the terminal is resized from 120×40 to 80×24
- **THEN** the TUI redraws without artifacts and all views stay usable

#### Scenario: Panic restores terminal
- **WHEN** the TUI panics
- **THEN** the terminal is back in normal mode and the panic message is printed

### Requirement: Read-only
The TUI SHALL not offer any action that changes nodes, parameters or connections.

#### Scenario: No write actions
- **WHEN** the user opens the help overlay
- **THEN** no listed key binding changes node state
