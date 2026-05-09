## Why

The Rust workspace can execute application and database flows, but it does not yet have a shared logging foundation for operational visibility. We need a structured logging capability now so the web composition root, application handlers, and database bootstrap paths can emit consistent JSON events before more adapters and runtime behavior arrive.

## What Changes

- Add a new `ora-logging` infrastructure crate that owns structured JSON log formatting, sink configuration, and file-rotation setup for Rust services.
- Introduce a shared logging configuration model with `debug`, `info`, and `error` levels plus stdout-only, file-only, and stdout-plus-file output modes.
- Initialize process-wide logging from `apps/web/server` and define the logging lifecycle around bootstrap success, bootstrap failure, and long-lived writer guard ownership.
- Add logging behavior to `ora-application` project handlers and `ora-db` database bootstrap and migration flows so operational events include consistent targets, messages, context fields, and structured error payloads.
- Document the new logging boundary and configuration expectations in `docs/` alongside the Rust workspace architecture.

## Capabilities

### New Capabilities
- `structured-logging`: Defines the shared Rust logging infrastructure, JSON event shape, configuration model, and runtime sink behavior for Ora services.

### Modified Capabilities
- `application-handlers`: Adds requirement-level logging expectations for application handlers so use-case execution emits transport-agnostic operational events with business context.
- `database-migrations`: Adds requirement-level logging expectations for database bootstrap reconciliation and migration step execution.

## Impact

- Affected code: `Cargo.toml`, `crates/logging`, `crates/application`, `crates/db`, `apps/web/server`, and related tests.
- Affected APIs: new public Rust API for `ora-logging` initialization and configuration, plus application-layer constructor changes needed to inject logging dependencies cleanly.
- Affected dependencies: adds the `tracing` ecosystem crates needed for structured JSON logging and rotating file output.
