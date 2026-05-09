# Runtime Logging

Ora Rust services initialize shared structured logging through `ora-logging`.

## Ownership Boundary

- `ora-logging` owns the process-wide subscriber setup, JSON event formatting, sink selection, file rotation, and retention cleanup.
- Runtime composition roots such as `apps/web/server` own reading environment configuration, calling `ora_logging::init_logging`, and retaining the returned `LoggingGuard` for the rest of the process lifetime.
- Runtime crates such as `ora-application` and `ora-db` emit structured `tracing` events but do not configure sinks themselves.

## Environment Configuration

`apps/web/server` maps the following environment variables into `ora-logging`:

- `ORA_LOG_LEVEL`: `debug`, `info`, `warn`, or `error`. Default: `info`.
- `ORA_LOG_MODE`: `stdout`, `file`, or `stdout_and_file`. Default: `stdout`.
- `ORA_LOG_PATH`: base path for file-backed logging. Default: `./ora.log`.
- `ORA_LOG_MAX_DAYS`: retention window in days for file-backed logging, including the current active file. Default: `3`.

`ORA_LOG_MODE=stdout` ignores file path and retention settings. File-backed modes rotate daily and clean up older matching files once the retained daily window would exceed `ORA_LOG_MAX_DAYS`.

## JSON Event Contract

Every `ora-logging` sink writes one JSON object per line with these top-level fields:

- `timestamp`
- `level`
- `target`
- `message`

Optional top-level fields are emitted only when runtime code attaches them:

- `method`
- `span`
- `trace_id`
- `request_id`

Business metadata belongs under `context`, and failure details belong under `error`. For example:

```json
{
  "timestamp": "2026-05-09T12:00:00Z",
  "level": "INFO",
  "target": "ora_application::project::handlers",
  "message": "project operation completed",
  "context": {
    "operation": "create_project",
    "project_id": "project-42"
  }
}
```

`ora-logging` also provides helper APIs for correlation-aware spans so runtime crates can attach `span`, `trace_id`, and `request_id` consistently.
For runtime event calls, prefer `ora_logging::ora_debug!`, `ora_logging::ora_info!`, `ora_logging::ora_warn!`, and `ora_logging::ora_error!`; these wrappers automatically attach the current function name as the top-level `method` field.
