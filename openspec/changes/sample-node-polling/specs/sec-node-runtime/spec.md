# Spec Delta

## MODIFIED Requirements

### Requirement: Updates to activated clients
The library SHALL send an `update` (or `error_update`) to every client that has the parameter's module activated whenever a parameter value is obtained by a read, a change, a poll or an application push, and SHALL let the application push new values at any time. Every published value SHALL be sent, even when it equals the previous value of the parameter.

#### Scenario: Application push
- **WHEN** the application pushes `t1:value` 295.2 while two clients are activated and one is not
- **THEN** both activated clients receive `update t1:value [295.2, {...}]` and the third receives nothing

#### Scenario: Unchanged value
- **WHEN** a poll of `t1:value` returns 295.2 twice in a row while a client is activated
- **THEN** the client receives two `update t1:value` messages, each with its own timestamp

### Requirement: Polling
The library SHALL poll parameters only when the application configures it, and SHALL publish each poll result (value or error) as an update. A module configured with a tiered poll interval SHALL poll a fast tier at that interval and a slow tier every 60 s unless configured otherwise:
- fast tier: `value` and `status` of a module whose base class is `Readable`, `Writable` or `Drivable`
- slow tier: every other parameter that is neither constant nor cached

A module configured with a uniform poll interval SHALL poll all non-constant, non-cached parameters at that one interval. For each parameter the effective interval SHALL be, in order of precedence: the parameter's own interval, the module's `pollinterval` parameter (for the fast tier or the uniform interval), the module's configured interval, and otherwise no polling. Constant and cached parameters and the `pollinterval` parameter itself SHALL never be polled.

#### Scenario: Periodic read
- **WHEN** `t1:value` is configured to poll every second and a client is activated
- **THEN** the client receives about one `update t1:value` per second

#### Scenario: Fast and slow tier
- **WHEN** a `Drivable` module with `value`, `status`, `target` and `ramp` is configured with a tiered interval of 1 s and runs for 130 s
- **THEN** `value` and `status` are each polled about 130 times, and `target` and `ramp` about twice

#### Scenario: No polling unless configured
- **WHEN** a `Readable` module has no poll interval and no `pollinterval` parameter
- **THEN** no parameter of it is polled after the startup reads

#### Scenario: Parameter interval wins
- **WHEN** a tiered module polls at 1 s and its `ramp` parameter has its own interval of 5 s
- **THEN** `ramp` is polled every 5 s, not every 60 s

#### Scenario: Uniform interval
- **WHEN** a module with `value`, `status` and `target` is configured with a uniform interval of 2 s
- **THEN** all three are polled about every 2 s

### Requirement: Poll interval parameter
The library SHALL drive a module's polling by its `pollinterval` parameter when the module declares one. A `change` of it SHALL go to the module's handler first, and the value the handler returns SHALL be published as usual. Every published value of it (from a change, the initial value or an application push) SHALL re-time the module's polling: its fast tier, or its uniform interval. A value of 0 SHALL stop that polling until a positive value is published. Parameters with an interval of their own and the slow tier SHALL keep their intervals. Without an initial value, the parameter SHALL start with the module's configured interval, or 0 if none is configured.

#### Scenario: Client tunes the rate
- **WHEN** a client changes `cryo:pollinterval` from 1 to 0.2 on a module polled at 1 s
- **THEN** the handler's change is called with 0.2, the client receives `changed cryo:pollinterval [0.2, ...]`, and `cryo:value` is polled about every 0.2 s from then on

#### Scenario: Handler adjusts the value
- **WHEN** a client changes `cryo:pollinterval` to 0.01 and the handler returns 0.1, its hardware minimum
- **THEN** the client receives `changed cryo:pollinterval [0.1, ...]` and polling runs every 0.1 s

#### Scenario: Handler refuses
- **WHEN** the handler fails the change of `cryo:pollinterval`
- **THEN** the client receives the error and the polling interval is unchanged

#### Scenario: Zero stops polling
- **WHEN** `cryo:pollinterval` is set to 0
- **THEN** `cryo:value` and `cryo:status` are no longer polled, while a parameter with its own interval keeps being polled

#### Scenario: Application push re-times
- **WHEN** the application pushes `cryo:pollinterval` 5.0
- **THEN** activated clients receive the update and the fast tier polls every 5 s

#### Scenario: Never polled
- **WHEN** a tiered module with a `pollinterval` parameter runs for several minutes
- **THEN** the handler is never asked to read `pollinterval` by a poll

### Requirement: Poll hook
The library SHALL, at each poll tick of a module, call the module handler's poll hook once with the names of all parameters of that module due at that tick, in declaration order. The default hook SHALL read each parameter through the handler's read and publish the value or error. A handler that overrides the hook is responsible for publishing the values it obtains, and parameters it does not publish SHALL produce no update. A tick SHALL be skipped while the module's request backlog is full, and the hook SHALL be subject to the node's handler timeout.

#### Scenario: Parameters due together
- **WHEN** a tiered module's fast and slow tiers fall due at the same tick
- **THEN** the hook is called once with `value`, `status` and the slow-tier parameters

#### Scenario: Default hook
- **WHEN** a handler implements only `read` and its module is polled
- **THEN** each due parameter is read and its value sent as an update

#### Scenario: One hardware transaction
- **WHEN** a handler overrides the hook and publishes `value` and `status` through its context from one controller query
- **THEN** activated clients receive both updates and the handler's `read` is not called by the poll

#### Scenario: Hook times out
- **WHEN** the hook does not return within the node timeout
- **THEN** the hook is abandoned, the error is logged, and the next tick polls again
