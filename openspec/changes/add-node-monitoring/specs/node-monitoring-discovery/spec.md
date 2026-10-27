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
The library SHALL write, when the endpoint opens, a file `<pid>.json` in the registry directory containing protocol version, pid, process name, endpoint address and port, access token and start time, readable only by the user, written atomically, and SHALL delete it when the endpoint closes.

#### Scenario: Registration lifecycle
- **WHEN** a process with pid 4242 opens its endpoint and later stops its last node
- **THEN** `4242.json` exists while the endpoint is open and is removed afterwards

### Requirement: Listing endpoints
The client library SHALL list all registrations in the directory, skipping files it cannot parse, and report for each whether its process is still alive.

#### Scenario: Three processes
- **WHEN** three processes have registered (one 32-bit and two 64-bit on Windows)
- **THEN** the listing returns all three with their addresses and tokens

### Requirement: Stale registrations
The client library SHALL treat a registration as stale when its process no longer exists, or when its process id was reused by a process with a different start time or name, and SHALL delete stale files it finds.

#### Scenario: Host crashed
- **WHEN** a host process was killed and left `4242.json` behind
- **THEN** the next listing reports no endpoint for it and the file is deleted
