# Compatibility with SHALL

must replaces SHALL's `SHALLserver` and `SHALLvariant` libraries. C and LabVIEW
programs written for SHALL use the same file names, the same function names and
signatures, and the same `SECoP_S_error`, `SECoP_S_action` and `SECoP_V_type` values,
and normally run unchanged. This page lists where must behaves differently.

## Replacing SHALL

1. Download the libraries for your platform from the
   [Releases](https://github.com/Bilchreis/MUST/releases) page
   (`must-shall-<version>-<triple>.zip` or `.tar.gz`), or build them:
   `cargo xtask dist --target <triple>` writes them to `dist/<triple>/`, and CI uploads
   the same folder as a workflow artifact named `must-shall-<triple>`.

   | Platform | Files |
   |---|---|
   | Windows 64-bit | `SHALLserver64.dll`, `SHALLvariant64.dll` (+ `.lib`, `.def`) |
   | Windows 32-bit (e.g. 32-bit LabVIEW) | `SHALLserver32.dll`, `SHALLvariant32.dll` (+ `.lib`, `.def`) |
   | Linux x86_64 | `libSHALLserver64.so`, `libSHALLvariant64.so` |

2. Replace SHALL's files with these, keeping both libraries in the same folder:
   `SHALLvariant` loads `SHALLserver` from its own folder. Remove the Qt libraries that
   SHALL needed; must has no dependency besides the operating system (the Windows
   libraries link the C runtime statically).
3. C programs: compile against the headers in `include/` (or `dist/<triple>/include/`)
   and link `SHALLserver64.lib` / `-lSHALLserver64` (and `SHALLvariant` if you used it).
   Programs that used only the C functions need no source change. LabVIEW VIs that call
   the functions by name need no change.

To go back, put SHALL's files back. For LabVIEW, [docs/labview-smoke-test.md](docs/labview-smoke-test.md)
describes a quick check of the replaced libraries.

## Protocol: SECoP 2.0

must speaks SECoP 2.0; SHALL spoke SECoP V2019-09-16. Clients that support SECoP 2.0
(e.g. current Frappy) work with must nodes. Differences a client may notice:

- **Error classes** follow SECoP 2.0. Error codes returned by the application (from a
  callback or `SECoP_S_putCommandAnswer`) are sent as:

  | `SECoP_S_error` | SECoP 2.0 error class |
  |---|---|
  | `INVALID_MODULE` | `NoSuchModule` |
  | `INVALID_PARAMETER` | `NoSuchParameter` |
  | `INVALID_COMMAND` | `NoSuchCommand` |
  | `READONLY` | `ReadOnly` |
  | `INVALID_VALUE` | `WrongType` |
  | `OUT_OF_RANGE` | `RangeError` |
  | `NOT_IMPLEMENTED` | `NotImplemented` |
  | `TIMEOUT` | `TimeoutError` |
  | `COMM_FAILED` | `CommunicationFailed` |
  | `COMMAND_FAILED` | `HardwareError` (SECoP 1.0's `CommandFailed` no longer exists) |
  | `COMMAND_RUNNING` | `CommandRunning` |
  | `IS_BUSY` | `IsBusy` |
  | `IS_ERROR` | `IsError` |
  | `DISABLED` | `Disabled` |
  | `SYNTAX` | `ProtocolError` |
  | `INTERNAL` and all others | `InternalError` |

- **`interface_classes`** is mandatory in SECoP 2.0. A module that does not set it gets
  the highest base class its accessibles satisfy (`Drivable`: `value`, `status`,
  writable `target` and command `stop`; `Writable`: without `stop`; `Readable`: `value`
  and `status`), or an empty list. `SECoP_S_nodeComplete` then returns
  `SECoP_S_WARNING_MISS_PROPERTIES`, and `SECoP_S_showErrors` names the modules.
- **Defaults** are filled in as in SHALL: a missing description becomes
  `"module without description"` (or node, parameter, command), and a command without
  datainfo is `{"type": "command"}`; `SECoP_S_nodeComplete` then returns
  `SECoP_S_WARNING_MISS_PROPERTIES`.
- **Datainfo** is checked against SECoP 2.0 when the node is complete. SHALL's 1.0-style
  descriptions are still accepted: type names in any case, and `int` without limits
  (published with the range ±2^24).
- **`visibility`** given as a number (1, 2, 3) is published as `"user"`, `"advanced"`,
  `"expert"`.
- **Custom properties** without a leading underscore are published under the given
  name, as in SHALL, and return `SECoP_S_WARNING_CUSTOM_PROPERTY`.
- The optional messages `check` and `logging` are answered with `ProtocolError`; the
  data type `matrix` is not supported.

## Library behavior

- **No Qt main thread.** Every function can be called from any thread at any time.
  `QApplication*` arguments are accepted and ignored; `SECoP_S_setManyThreads` has no
  effect (requests are always served by several threads).
- **Callbacks** (getters, setters, command functions) run on worker threads of the
  library, possibly several at once for different modules, never on the thread that
  called `SECoP_S_initLibrary`. Calls for one module are not overlapped.
- **Polled requests** wait 60 s for `SECoP_S_putCommandAnswer[2]`, as in SHALL; then the
  client gets `TimeoutError`. Answering an unknown id (timed out, answered before, never
  handed out) returns `SECoP_S_ERROR_INVALID_NAME`, as in SHALL.
- **Small buffers** in `SECoP_S_getStoredCommand[2]` work as in SHALL: the request is
  handed out anyway with `SECoP_S_WARNING_BUFFER_TOO_SMALL`, the text is cut (and
  NUL-terminated), and the size arguments receive the full text length without NUL.
- **`SECoP_S_getStoredCommand`** returns in `*ppValue` a new handle that the caller
  destroys with `SECoP_V_destroy`: the value of a `change` or the argument of a `do`;
  NULL for a `read` and a `do` without argument. A handle passed in is not used or
  freed.
- **A `change` answered without value** reports the requested value to the client.
- **Pushed values** (`SECoP_S_updateParameter[2]`) before `SECoP_S_nodeComplete` become
  the parameter's initial value. A timestamp of NaN or 0 means "now".
- **Context IDs**: `SECoP_S_initLibrary` for a context that is already initialized
  starts it afresh (its nodes stop), as in SHALL. `SECoP_S_doneLibrary(1, ctx)` stops
  the nodes of `ctx` and forgets the context, as in SHALL. **Different from SHALL:**
  `SECoP_S_doneLibrary(0, ctx)` does the same and ends the library only when no context
  remains, where SHALL ended the library, and all nodes of all contexts, at once. NULL
  is accepted as the context `""` (SHALL refused an empty context ID in
  `SECoP_S_createNode`).
- **`SECoP_S_createNode2`** takes an IP address (or `localhost`) as interface; NULL,
  empty or anything that is not an address listens on all interfaces, as in SHALL. Node
  names are unique: a second node with the same name gives
  `SECoP_S_ERROR_NAME_ALREADY_USED`.
- **`SECoP_S_nodeComplete`** returns a specific code for the first problem (e.g.
  `SECoP_S_ERROR_MISSING_MANDATORY` for a parameter without datainfo,
  `SECoP_S_ERROR_NO_GETTER` for a missing getter), where SHALL returned
  `SECoP_S_ERROR_INVALID_PROPERTY` for all; and an invalid node does not open its
  port.
- **Errors and warnings** of the building functions are kept for `SECoP_S_showErrors`,
  which prints them to stderr and clears them.
- **Status window**: SHALL's Qt status window is replaced by the terminal UI `must-tui`
  (shipped next to the libraries), which shows the nodes of all must programs of the user
  at once and only reads. `bGUI` in `SECoP_S_initLibrary` and
  `SECoP_S_showStatusWindow(1)` open it in a window of its own (a console window on
  Windows; a terminal emulator on Linux with a graphical session), unless it runs
  already, and open it again whenever it is closed. **Different from SHALL:**
  `SECoP_S_showStatusWindow(0)` does not close it, since it may show other programs'
  nodes; it stops opening it again, and the user closes it. Closing its window never
  ends the host. See [docs/status-tui.md](docs/status-tui.md).
- **Monitoring** (new): every process with nodes opens a local monitoring endpoint (a
  Unix socket in `$XDG_RUNTIME_DIR/must` or `/tmp/must-<uid>`, a named pipe on Windows,
  registered in `%LOCALAPPDATA%\must\run`), from `SECoP_S_createNode` on, with node
  states, context IDs, build errors, clients, traffic, values and log records. It is only
  accessible to the user and opens no network port. `MUST_MONITOR=0` turns it off. See
  [docs/monitoring-protocol.md](docs/monitoring-protocol.md).
- **Log files** (new): every node writes its traffic (each published value once), client
  connections and its log records to `<log dir>\<equipment_id>\<YYYY-MM-DD>.log`, by
  default under `%LOCALAPPDATA%\must\logs` (Windows) or `~/.local/state/must/logs`
  (Linux), and deletes files older than 2 days. Configure it with environment variables
  set before the program starts: `MUST_LOG_FILES=0` (off), `MUST_LOG_DIR` (directory),
  `MUST_LOG_DAYS` (retention). Logging never slows a node; there is no size limit, so
  check the disk usage estimate in [docs/log-files.md](docs/log-files.md) for nodes with
  fast polling. The files contain all values exchanged with clients.
- **Polling** depends on what the module declares:
  - **SHALL's `pollinterval` properties** work as in SHALL: the module property polls all
    readable parameters of the module at that interval (0: none), a parameter's property
    gives that parameter its own interval, and with only parameter properties only those
    parameters are polled.
  - **New: a `pollinterval` parameter** with a `double` datainfo (no negative `min`), the
    standard SECoP way, makes the module poll `value` and `status` (of a `Readable`,
    `Writable` or `Drivable` module) at that parameter's value and all other parameters
    every 60 s. The module's and its parameters' `pollinterval` properties are then
    ignored, with a warning from `SECoP_S_nodeComplete`/`SECoP_S_showErrors`. A
    client's `change` of the parameter goes to the setter (or the polling-mode request)
    as usual; the value answered, read or pushed with `SECoP_S_updateParameter` becomes
    the interval, 0 stops it, and without a value it starts at 5 s. The library never
    polls the parameter itself. SHALL treated it as an ordinary parameter.
  - **Different from SHALL: neither** means the same default: `value` and `status`
    every 5 s, all other parameters every 60 s. SHALL polled nothing. In polling mode
    the host therefore receives stored `read` requests from polling, which it answers
    like a client's. Set the module property `pollinterval` to 0 to get SHALL's
    "nothing polled".

  Every polled value is sent to activated clients, also when it did not change.
- **Error strings** (`SECoP_S_getErrorString`) have SHALL's texts, but may differ in
  detail; do not compare them.

## Not provided

- The C++ class interface of `CSECoPbaseType` and `SECoP_dataPtr`; `CSECoPbaseType` is
  an opaque type. C++ programs that used only the C functions compile unchanged.
- `SECoP_V_printString` and `SECoP_V_printStream`, which use Qt types.
- SHALL's client library (`SECoP_C_*`).
