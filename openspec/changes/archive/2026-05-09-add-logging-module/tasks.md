## 1. Workspace and logging crate scaffolding

- [x] 1.1 Add `crates/logging` as a workspace member and declare the shared `tracing` ecosystem dependencies needed for structured logging, JSON formatting, rotation, and tests.
- [x] 1.2 Scaffold `ora-logging` with a private module layout and explicit public exports for `LoggingConfig`, `LogLevel`, `LogOutput`, `FileLoggingConfig`, `RotationPolicy`, `LoggingGuard`, and `LoggingInitError`.
- [x] 1.3 Add crate-level tests that exercise the public configuration model and preserve the intended public API surface.

## 2. Implement structured logging infrastructure

- [x] 2.1 Implement `ora_logging::init_logging` so it installs the shared subscriber, builds the configured sinks, and returns a guard that keeps file-backed writers alive for the process lifetime.
- [x] 2.2 Implement the JSON event formatter so every event emits one line with the required top-level fields plus `context` and `error` objects when structured fields are present.
- [x] 2.3 Add helper APIs in `ora-logging` for attaching optional `span`, `trace_id`, and `request_id` fields consistently across runtime crates.
- [x] 2.4 Implement daily file rotation and bounded retention cleanup based on `ORA_LOG_MAX_DAYS` while keeping stdout-only mode non-rotating.
- [x] 2.5 Add focused logging crate tests that verify JSON output shape, helper API field emission, `stdout`, `file`, and `stdout_and_file` sink-selection behavior, typed initialization failures, and retention cleanup semantics.

## 3. Wire logging into runtime crates

- [x] 3.1 Update `apps/web/server` to read `ORA_LOG_LEVEL`, `ORA_LOG_MODE`, `ORA_LOG_PATH`, and `ORA_LOG_MAX_DAYS`, map `ORA_LOG_MODE` across `stdout`, `file`, and `stdout_and_file`, construct the initial `LoggingConfig`, initialize `ora-logging` during bootstrap, and retain the returned guard for the process lifetime.
- [x] 3.2 Add structured informational and error events to `ora-application` project handlers so CRUD operations emit operation names and project identifiers when available.
- [x] 3.3 Add structured informational and error events to `ora-db` bootstrap and migration flows for database open lifecycle, reconciliation decisions, migration execution, and migration failures.

## 4. Verification and documentation

- [x] 4.1 Add or update integration-style tests that capture emitted events from `ora-application` and `ora-db` under scoped subscribers and assert the expected success and failure behavior.
- [x] 4.2 Update the relevant `docs/` pages to describe the new logging crate, the JSON event contract, the environment variables, and the ownership boundary for runtime logging setup.
- [x] 4.3 Run `task test` and fix any failures introduced by the logging crate, runtime wiring, and logging assertions.
