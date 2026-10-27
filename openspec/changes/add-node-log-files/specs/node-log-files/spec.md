# Spec Delta

## Purpose

Keeps a rolling on-disk record per SEC node of everything it exchanged with its clients and of what it reported, so problems can be investigated after the fact.

## ADDED Requirements

### Requirement: One log file per node and day
The library SHALL write the log of each running node into `<log dir>/<node folder>/<YYYY-MM-DD>.log`, where the node folder is the node's `equipment_id` with characters not allowed in file names replaced by `_`, and the date is the local date at which the line was written.

#### Scenario: Two nodes
- **WHEN** nodes `cryo` and `magnet` run on 2026-10-27 with the default log directory on Linux
- **THEN** their lines go to `~/.local/state/must/logs/cryo/2026-10-27.log` and `~/.local/state/must/logs/magnet/2026-10-27.log`

#### Scenario: Midnight
- **WHEN** a node is running at local midnight
- **THEN** lines written after midnight go to the new day's file and the previous day's file is closed

#### Scenario: Unusual equipment_id
- **WHEN** a node has `equipment_id` `lab:cryo/1`
- **THEN** its folder is `lab_cryo_1`

### Requirement: Same node in two processes
The library SHALL NOT interleave the lines of two processes in one file: when another process is already writing a node's file for the day, the second process SHALL write to `<YYYY-MM-DD>-<pid>.log` in the same folder.

#### Scenario: Node name used twice
- **WHEN** two LabVIEW processes each run a node with `equipment_id` `cryo` on the same day
- **THEN** one writes `cryo/<date>.log` and the other `cryo/<date>-<pid>.log`

### Requirement: Logged content
The library SHALL write one line for every message the node received from or sent to a client (complete, including `describing` replies and every `update` to every client), for every client connect and disconnect (with the peer address and reason), for node start and stop, and for every library log record of level info or higher concerning the node.

#### Scenario: A request and its reply
- **WHEN** client 3 sends `read t1:value` and the node replies
- **THEN** the file contains a line for the received `read t1:value` followed by a line for the sent `reply t1:value [...]`, both naming client 3

#### Scenario: Handler error
- **WHEN** a module handler fails and the node logs a warning for module `t1`
- **THEN** the file contains a line with level `WARN`, module `t1` and the message

### Requirement: Line format
Each line SHALL start with the local time with milliseconds and UTC offset in ISO 8601, followed by a kind marker (`>` received, `<` sent, `*` connection or node event, or a log level), the client id and address for client-related lines, and the text; message texts SHALL be written unchanged on one line, and the file SHALL be UTF-8.

#### Scenario: Readable lines
- **WHEN** client 3 from `192.168.1.20:51234` sends `change t1:target 300`
- **THEN** the line reads like `2026-10-27T16:42:10.123+01:00 > [3 192.168.1.20:51234] change t1:target 300`

### Requirement: Retention
The library SHALL delete a node's log files whose day ended more than the retention ago (default 2 days, so that at least the last 2 × 24 hours are kept), when a node starts and then about once per hour, and only files whose names match the log file pattern.

#### Scenario: Default retention
- **WHEN** a node runs on 2026-10-27 and its folder holds files for 2026-10-24, 2026-10-25 and 2026-10-26
- **THEN** the file of 2026-10-24 is deleted and the others are kept

#### Scenario: Foreign files kept
- **WHEN** the node folder also contains `notes.txt`
- **THEN** that file is not deleted

#### Scenario: Longer retention
- **WHEN** the retention is configured to 30 days
- **THEN** files are kept for 30 days after their day ended

### Requirement: Configuration
File logging SHALL be on by default; the environment variables `MUST_LOG_FILES=0` (off), `MUST_LOG_DIR` (directory) and `MUST_LOG_DAYS` (retention in days, at least 1) and the Rust API SHALL configure it, the Rust API taking precedence; invalid values SHALL be reported as a warning and replaced by the default.

#### Scenario: C host with environment variables
- **WHEN** a LabVIEW process using the SHALL C ABI starts with `MUST_LOG_DIR=D:\logs` and `MUST_LOG_DAYS=7`
- **THEN** its nodes write to `D:\logs\<node folder>\` and keep 7 days

#### Scenario: Turned off
- **WHEN** a process starts a node with `MUST_LOG_FILES=0`
- **THEN** no log directory or file is created and the node works normally

### Requirement: Default location
The default log directory SHALL be `$XDG_STATE_HOME/must/logs` (falling back to `~/.local/state/must/logs`) on Linux and `%LOCALAPPDATA%\must\logs` on Windows, created on first use and readable only by the user on Linux.

#### Scenario: Windows default
- **WHEN** a node starts on Windows without `MUST_LOG_DIR`
- **THEN** its log goes under `%LOCALAPPDATA%\must\logs`

### Requirement: Never affect the node
File logging SHALL never block or slow a node: lines are handed to a background writer through a bounded buffer; when the buffer is full, lines are dropped and the next written line is preceded by a line stating how many were dropped; when the directory cannot be created or written, a warning is logged, file logging stops for that node, and the node keeps working.

#### Scenario: Slow disk
- **WHEN** the disk stalls while a node answers requests
- **THEN** the node's clients see no added latency and the file later contains a line stating how many lines were dropped

#### Scenario: Read-only directory
- **WHEN** the log directory is not writable
- **THEN** the node starts and serves clients normally and one warning names the directory

### Requirement: Flushing
The library SHALL flush written lines to the file at least once per second and when a node stops, so that a log is complete up to about one second before a crash of the host process.

#### Scenario: Stopped node
- **WHEN** a node is stopped
- **THEN** its log file contains every line up to the stop, ending with the node-stopped line
