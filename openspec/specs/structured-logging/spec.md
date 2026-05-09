## Purpose

Define the shared structured logging infrastructure and runtime logging contract for Ora Rust services.

## Requirements

### Requirement: Rust services SHALL initialize shared structured logging through `ora-logging`
The system SHALL provide an `ora-logging` crate that exposes a shared initialization API for Rust services. That API SHALL accept a logging configuration with explicit level and output mode selection, SHALL install the process-wide subscriber, and SHALL return a guard that keeps any non-blocking file writer alive for the process lifetime. The web server integration SHALL derive that logging configuration from environment-driven inputs during Rust bootstrap using default values `ORA_LOG_LEVEL=info`, `ORA_LOG_MODE=stdout`, `ORA_LOG_PATH=./ora.log`, and `ORA_LOG_MAX_DAYS=3`. The first implementation SHALL support `ORA_LOG_MODE` values `stdout`, `file`, and `stdout_and_file`.

#### Scenario: Web server bootstraps logging
- **WHEN** `apps/web/server` starts with a valid logging configuration
- **THEN** it initializes logging through `ora-logging` before continuing service bootstrap and retains the returned guard for the rest of the process lifetime

#### Scenario: Web server reads environment-driven logging settings
- **WHEN** `apps/web/server` starts with logging-related environment variables set
- **THEN** it maps those values into `ora-logging` configuration before initializing the shared subscriber

#### Scenario: Web server uses default logging settings
- **WHEN** `apps/web/server` starts without overriding logging-related environment variables
- **THEN** it initializes logging with level `info`, mode `stdout`, path `./ora.log`, and retention window `3` days

#### Scenario: Web server enables file-only logging
- **WHEN** `ORA_LOG_MODE` is set to `file`
- **THEN** the web server initializes `ora-logging` with a file-backed sink that writes to `ORA_LOG_PATH` and applies the configured daily retention window

#### Scenario: Web server enables combined logging
- **WHEN** `ORA_LOG_MODE` is set to `stdout_and_file`
- **THEN** the web server initializes `ora-logging` with both stdout and file sinks using the same structured JSON event contract

#### Scenario: File sink initialization fails
- **WHEN** `ora-logging` cannot create or prepare the configured file output directory or writer
- **THEN** initialization fails with a typed logging setup error instead of silently degrading to another sink

### Requirement: Structured log events SHALL use the shared JSON envelope
The system SHALL emit one JSON object per log event line for all `ora-logging` managed sinks. Each event SHALL include `timestamp`, `level`, `target`, and `message`, and events with structured business metadata SHALL encode that metadata under `context` while failure events SHALL encode failure details under `error`. The `ora-logging` crate SHALL also provide helper APIs for attaching optional `span`, `trace_id`, and `request_id` fields so runtime crates can populate those reserved top-level fields consistently.

#### Scenario: Application handler logs a successful operation
- **WHEN** a handler emits a success event with `operation` and `project_id` fields
- **THEN** the resulting JSON line contains the required top-level fields plus a `context` object that includes `operation` and `project_id`

#### Scenario: Database migration logs a failure
- **WHEN** a migration step emits an error event with a failure kind and message
- **THEN** the resulting JSON line contains an `error` object and does not flatten those error fields into unrelated top-level keys

#### Scenario: Runtime code attaches correlation metadata
- **WHEN** a runtime crate uses `ora-logging` helper APIs to attach `span`, `trace_id`, or `request_id`
- **THEN** the resulting JSON line includes those fields at the reserved top level only for the values that were explicitly attached

### Requirement: File-backed logging SHALL support daily rotation and bounded day-based retention
The system SHALL support daily rotation for file-backed outputs only, and SHALL retain log files according to the configured `ORA_LOG_MAX_DAYS` window, counting the current active file toward that limit. File-backed modes SHALL write to the configured `ORA_LOG_PATH`. Stdout-only output SHALL not rotate and SHALL ignore file-path and retention settings.

#### Scenario: File output exceeds the retention limit
- **WHEN** a daily rotating file output would retain more than the configured `ORA_LOG_MAX_DAYS` window
- **THEN** the oldest matching log files are deleted first until the retained files fit within that day-based limit

#### Scenario: Stdout-only logging is configured
- **WHEN** the logging output mode is stdout only
- **THEN** the system writes structured JSON lines to standard output without creating rotating files or applying retention cleanup

#### Scenario: File-backed logging uses the configured path
- **WHEN** a file-backed logging mode is configured
- **THEN** the active log writer uses `ORA_LOG_PATH` as the base output path for the current day's log file

#### Scenario: Combined logging writes to both sinks
- **WHEN** the logging output mode is `stdout_and_file`
- **THEN** each structured log event is emitted to standard output and to the configured daily rotating file sink
