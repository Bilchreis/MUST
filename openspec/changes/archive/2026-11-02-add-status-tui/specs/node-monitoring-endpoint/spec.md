# Spec Delta

## ADDED Requirements

### Requirement: Log file location
The endpoint SHALL report for each node the path of the log file the node currently writes (see `node-log-files`): in the node's snapshot entry, absent while file logging is off, not yet started or stopped after an error, and as an event whenever that path changes (first file opened, day change, fallback to the per-process file name, logging stopped). This is protocol version 1.1; it only adds an optional field and an event kind, which 1.0 readers ignore.

#### Scenario: Path in the snapshot
- **WHEN** a monitor connects to a process whose node `cryo` writes `/home/u/.local/state/must/logs/cryo/2026-10-27.log`
- **THEN** the snapshot entry of `cryo` carries that path

#### Scenario: Day change
- **WHEN** node `cryo` continues in the next day's file at midnight
- **THEN** connected monitors receive a log-file event for `cryo` with the new path

#### Scenario: Logging off
- **WHEN** a node runs with file logging turned off
- **THEN** its snapshot entry has no log file path

#### Scenario: 1.0 reader
- **WHEN** a monitor that implements protocol 1.0 connects to a 1.1 endpoint
- **THEN** it is accepted, and the log file field and events are ignored by it
