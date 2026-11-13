# Spec Delta

## REMOVED Requirements

### Requirement: Logging not supported
**Reason**: The node now implements `logging`, specified in `sec-node-remote-logging`.
**Migration**: Clients that treated `error_logging` with `ProtocolError` as "unsupported" now receive `logging <module> <level>` replies and `log` events. Nothing is sent unless they ask for it.
