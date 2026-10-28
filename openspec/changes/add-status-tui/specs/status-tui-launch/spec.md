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
The library SHALL look for the TUI executable first in the directory of the loaded must library, then at the path in `MUST_TUI`, then on `PATH`, and SHALL log a warning naming the searched locations when none is found; `MUST_TUI=0` SHALL turn launching off.

#### Scenario: Shipped next to the DLL
- **WHEN** `SHALLserver32.dll` and `must-tui.exe` are in the same directory, which is not on `PATH`
- **THEN** the launch finds and starts that `must-tui.exe`

### Requirement: Launch never disturbs the host
The launch SHALL never block the calling thread for more than a short process-spawn time, never fail the calling function, and the TUI SHALL keep running independently when the host exits.

#### Scenario: Missing executable
- **WHEN** no `must-tui` executable can be found
- **THEN** `SECoP_S_initLibrary` still returns `SECoP_S_SUCCESS` and only a warning is logged

### Requirement: Keep the TUI open
After a launch request, the library SHALL look at least every 5 seconds whether a TUI runs for the current user and, if none does, launch it again, until the host hides the status display or ends the library. A launch request while it keeps the TUI open SHALL look at once.

#### Scenario: User closes the TUI
- **WHEN** a host called `SECoP_S_initLibrary` with `bGUI` set and the user closes the TUI window
- **THEN** a new TUI window opens within about 5 seconds, focused on the host

#### Scenario: Library done
- **WHEN** a host calls `SECoP_S_doneLibrary(0, ...)` for its last context and the user then closes the TUI
- **THEN** no new TUI is opened

### Requirement: One launch at a time
While a process starts a TUI, until the TUI runs or 20 seconds have passed, no other process or thread SHALL start one.

#### Scenario: Two hosts notice the TUI is gone
- **WHEN** two processes that keep the TUI open both look after the user closed it
- **THEN** exactly one new TUI is started

### Requirement: Back off from failing launches
When a launch fails or the started TUI does not come up within 20 seconds, the library SHALL try again at doubling intervals starting at 5 seconds and up to 5 minutes, and SHALL log each kind of outcome once when it changes rather than at every attempt.

#### Scenario: Launch blocked
- **WHEN** endpoint protection kills every `must-tui.exe` the host starts
- **THEN** the host tries again at growing intervals, logs one warning, and keeps running normally

### Requirement: Hide request
The library SHALL accept `SECoP_S_showStatusWindow(0)`, stop opening the TUI again, and leave a running TUI open, since it may show other processes' nodes; the user closes the TUI.

#### Scenario: Hide while open
- **WHEN** a host calls `SECoP_S_showStatusWindow(0)` while a TUI is open
- **THEN** the call returns and the TUI stays open

#### Scenario: Close after hide
- **WHEN** a host called `SECoP_S_showStatusWindow(0)` and the user then closes the TUI
- **THEN** no new TUI is opened

### Requirement: Focus the launching process
The library SHALL pass the host's process id to the TUI it starts, and the TUI SHALL select that process's first node once it appears.

#### Scenario: Focus on launch
- **WHEN** a host with pid 4242 launches the TUI
- **THEN** the TUI starts with pid 4242's first node selected once its endpoint is discovered
