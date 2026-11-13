# Spec Delta

## Purpose

Runs SEC nodes from config files on the command line, with `must` for the built-in module types and with the same commands in binaries that driver collections build with their own types.

## ADDED Requirements

### Requirement: Serve a config
`must serve <file>` SHALL load the config with the default registry, print the warnings, start the node, print the address it listens on, and serve until it receives Ctrl-C, then stop the node and exit with status 0. If loading or starting fails, it SHALL print every error and exit with a non-zero status without serving.

#### Scenario: Serving the gas rack example
- **WHEN** `must serve` is run on the gas rack example config
- **THEN** a SECoP client connecting to the printed address gets a `describe` with the flow controllers `mfc1` to `mfc5`

#### Scenario: Invalid config
- **WHEN** `must serve` is run on a config with an unknown module type
- **THEN** it prints the error with file and line, exits with a non-zero status, and no port is opened

#### Scenario: Port in use
- **WHEN** the config's interface port is already in use
- **THEN** `must serve` prints an error naming the address and exits with a non-zero status

### Requirement: Check a config
`must check <file>` SHALL load the config and build the node without opening any port, print every error and warning, and exit with status 0 if loading succeeded (also with warnings) and non-zero otherwise. On success it SHALL print the node's modules with their types, one per line.

#### Scenario: Valid config
- **WHEN** `must check` is run on the gas rack example config
- **THEN** it exits with status 0 and lists the modules with their types

#### Scenario: Errors are reported
- **WHEN** `must check` is run on a config with two errors
- **THEN** it prints both errors and exits with a non-zero status

### Requirement: List module types
`must types` SHALL print the names of the module types in the registry, one per line, in sorted order.

#### Scenario: Built-in types
- **WHEN** `must types` is run
- **THEN** the output contains `must::sim::FlowController` and `must::sim::Temperature`

### Requirement: Runner logging
While serving, the runner SHALL log to the terminal at level `info` unless the `RUST_LOG` environment variable sets a filter. It SHALL also send log messages to the monitoring endpoint, so that `must monitor` shows the node and its log.

#### Scenario: Node in the status TUI
- **WHEN** a node is started with `must serve` and `must monitor` is opened
- **THEN** the TUI lists the node with its equipment_id and shows its log messages

### Requirement: Runner for driver collections
The library SHALL offer the `serve`, `check` and `types` commands as an entry point that takes a registry, so that a driver collection can build a binary with its own module types and the built-in ones. That binary SHALL behave as `must serve`, `must check` and `must types` do, with its registry.

#### Scenario: Custom binary
- **WHEN** a binary registers the type `acme::Valve` and calls the entry point with `check` on a config using `acme::Valve`
- **THEN** the check succeeds and lists the module with type `acme::Valve`
