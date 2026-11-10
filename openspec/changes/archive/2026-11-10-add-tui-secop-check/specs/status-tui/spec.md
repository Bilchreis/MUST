# Spec Delta

## ADDED Requirements

### Requirement: SECoP check of the description
The detail view SHALL show, in a section of its own, what the SECoP checker (`secop-check`) finds in the selected node's structure report when checked against SECoP 2.0: the findings most severe first, each with its severity and the place in the description it concerns, under a title that counts them by severity or states that there are none. Nodes without a structure report SHALL have no such section.

#### Scenario: Findings
- **WHEN** the checker reports for node `cryo` that module `t1` lacks the property `implementation`
- **THEN** the detail view of `cryo` shows an `error` line naming `Module t1` and the missing property, under the title `SECoP check: 1 error`

#### Scenario: Conforming description
- **WHEN** the checker finds nothing in node `cryo`'s description
- **THEN** the section's title is `SECoP check: ok`

### Requirement: When the description is checked
A node SHALL be checked when the detail view shows it and its structure report was not checked yet, again when the report changes, and again when the user asks for it. The check SHALL run in the background on the report the TUI already has, without contacting the node. If the checker cannot run, the section SHALL say why.

#### Scenario: Description changes
- **WHEN** node `cryo` restarts with another structure report while its detail view is open
- **THEN** the new report is checked and its findings replace the old ones

#### Scenario: Checker not installed
- **WHEN** neither uv nor the command in `MUST_SECOP_CHECK` can be run
- **THEN** the section says that the check could not run and how to install the checker, and the rest of the detail view works as before
