# Spec Delta

## Purpose

Lets tools find the monitoring endpoints of all processes of the current user that run must nodes, on Linux and Windows, regardless of the processes' bitness.

## ADDED Requirements

### Requirement: Per-user runtime directory
The library SHALL use a per-user registry directory: `$XDG_RUNTIME_DIR/must` on Linux, falling back to `/tmp/must-<uid>`, and `%LOCALAPPDATA%\must\run` on Windows; on Linux it SHALL create the directory with mode 0700 and refuse to use one owned by another user or accessible by others.

#### Scenario: Fallback directory on Linux
- **WHEN** `XDG_RUNTIME_DIR` is not set for a user with uid 1000
- **THEN** registrations go to `/tmp/must-1000`, created with mode 0700

#### Scenario: Unsafe directory
- **WHEN** `/tmp/must-1000` exists but is owned by another user
- **THEN** no registration is written, monitoring is disabled for the process, and a warning is logged

### Requirement: Registration file
The library SHALL write, when the endpoint opens, a file `<pid>.json` in the registry directory containing protocol version, pid, process name, start time and an endpoint description (see "Endpoint description"), readable only by the user, written atomically, and SHALL delete it, and on Linux the endpoint's socket file, when the endpoint closes.

#### Scenario: Registration lifecycle
- **WHEN** a process with pid 4242 opens its endpoint and later stops its last node
- **THEN** `4242.json` exists while the endpoint is open and is removed afterwards

### Requirement: Listing endpoints
The client library SHALL list all registrations in the directory, skipping files it cannot parse, and report for each whether its process is still alive.

#### Scenario: Three processes
- **WHEN** three processes have registered (one 32-bit and two 64-bit on Windows)
- **THEN** the listing returns all three with their endpoint descriptions

### Requirement: Endpoint description
A registration SHALL describe its endpoint as an object with a `kind` and the fields of that kind: `unix` with the socket `path` on Linux, `pipe` with the pipe `name` on Windows; the client library SHALL skip, and report as unsupported, registrations whose kind it does not know, without deleting them.

#### Scenario: Linux registration
- **WHEN** process 4242 registers on Linux
- **THEN** its file contains an endpoint like `{"kind":"unix","path":"/run/user/1000/must/4242.sock"}`

#### Scenario: Unknown kind
- **WHEN** a registration contains an endpoint of kind `tcp` and the client library does not support it
- **THEN** the listing reports that registration as unsupported, lists the other registrations normally, and keeps the file

### Requirement: Stale registrations
The client library SHALL treat a registration as stale when its process no longer exists, or when its process id was reused by a process with a different start time or name, and SHALL delete stale files it finds.

#### Scenario: Host crashed
- **WHEN** a host process was killed and left `4242.json` behind
- **THEN** the next listing reports no endpoint for it, and the file (and on Linux its socket file) is deleted
