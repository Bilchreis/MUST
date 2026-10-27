# Spec Delta

## Purpose

Lets a process hosting must nodes, including a GUI host such as LabVIEW that has no terminal, open the status TUI in its own terminal window, as SHALL's GUI flags did for the Qt status window.

## ADDED Requirements

### Requirement: Launch on request
The library SHALL launch `must-tui` when `SECoP_S_initLibrary` is called with `bGUI` set, when `SECoP_S_showStatusWindow(1)` is called, or when a Rust host calls the launch API, unless a TUI is already running for the current user.

#### Scenario: First init with GUI
- **WHEN** a host calls `SECoP_S_initLibrary(NULL, 1, 1, "ctx")` and no TUI is running
- **THEN** a `must-tui` window opens and shows the host's nodes once they start

#### Scenario: Already running
- **WHEN** a second process calls `SECoP_S_showStatusWindow(1)` while a TUI is open
- **THEN** no second TUI is started and the call returns normally

### Requirement: Own window on Windows
On Windows the library SHALL start `must-tui.exe` as a separate process with its own new console window, so that closing that window ends only the TUI and never the host.

#### Scenario: Close the console window
- **WHEN** the user closes the TUI's console window
- **THEN** the TUI process ends and the host process and its nodes keep running

### Requirement: Terminal emulator on Linux
On Linux the library SHALL, when `DISPLAY` or `WAYLAND_DISPLAY` is set, start `must-tui` in a terminal emulator chosen from `MUST_TUI_TERMINAL`, `$TERMINAL`, `x-terminal-emulator`, `gnome-terminal`, `konsole`, `xfce4-terminal` and `xterm` (first found), detached from the host; without a graphical session or terminal emulator it SHALL log how to start `must-tui` manually.

#### Scenario: No graphical session
- **WHEN** a headless host calls `SECoP_S_showStatusWindow(1)`
- **THEN** nothing is started, the call returns normally, and an info log says to run `must-tui`

### Requirement: Finding the executable
The library SHALL look for the TUI executable first in the directory of the loaded must library, then at the path in `MUST_TUI`, then on `PATH`, and SHALL log a warning naming the searched locations when none is found.

#### Scenario: Shipped next to the DLL
- **WHEN** `SHALLserver32.dll` and `must-tui.exe` are in the same directory, which is not on `PATH`
- **THEN** the launch finds and starts that `must-tui.exe`

### Requirement: Launch never disturbs the host
The launch SHALL never block the calling thread for more than a short process-spawn time, never fail the calling function, and the TUI SHALL keep running independently when the host exits.

#### Scenario: Missing executable
- **WHEN** no `must-tui` executable can be found
- **THEN** `SECoP_S_initLibrary` still returns `SECoP_S_SUCCESS` and only a warning is logged

### Requirement: Hide request
The library SHALL accept `SECoP_S_showStatusWindow(0)` and leave a running TUI open, since it may show other processes' nodes; the user closes the TUI.

#### Scenario: Hide while open
- **WHEN** a host calls `SECoP_S_showStatusWindow(0)` while a TUI is open
- **THEN** the call returns and the TUI stays open

### Requirement: Focus the launching process
The library SHALL pass the host's process id to the TUI it starts, and the TUI SHALL select that process's first node once it appears.

#### Scenario: Focus on launch
- **WHEN** a host with pid 4242 launches the TUI
- **THEN** the TUI starts with pid 4242's first node selected once its endpoint is discovered
