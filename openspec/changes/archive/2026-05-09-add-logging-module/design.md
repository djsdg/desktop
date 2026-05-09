## Context

The Rust workspace currently has no shared logging foundation for service-style runtime behavior. `apps/web/server` is the active composition root for backend wiring, `ora-application` owns transport-agnostic use-case orchestration, and `ora-db` owns SQLite bootstrap and migration reconciliation. The desktop Tauri shell already uses a development-only plugin logger, but that path is adapter-specific and does not establish the JSON log contract needed for Rust crates that will run outside the Tauri runtime.

This change introduces a reusable logging crate and a small set of logging conventions before more infrastructure and adapter behavior accumulates. The design needs to keep domain code free of side effects, preserve testability, and avoid scattering sink setup logic across crates.

## Goals / Non-Goals

**Goals:**
- Provide one shared `ora-logging` crate that initializes structured JSON logging for Rust services.
- Keep the public logging configuration small and explicit: level, output mode, file rotation, and retention.
- Ensure `ora-application` and `ora-db` can emit consistent operational events without taking a dependency on adapter-specific code.
- Make file output rotation and retention deterministic for local development and long-running service processes.
- Keep the JSON event shape stable enough for future ingestion and test assertions.

**Non-Goals:**
- Replacing the Tauri plugin logging path in `apps/desktop/src-tauri`.
- Introducing distributed tracing, span propagation across processes, or remote log shipping.
- Adding domain-layer logging side effects inside `ora-domain`.
- Designing a user-facing settings surface for logging configuration in this change.

## Decisions

### 1. Add `ora-logging` as a dedicated infrastructure crate

`ora-logging` will be added as a new workspace crate and will own the shared logging API. Its public surface will expose `LoggingConfig`, `LogLevel`, `LogOutput`, `FileLoggingConfig`, `RotationPolicy`, `LoggingGuard`, and `init_logging`.

Why:
- It keeps logging setup concerns out of `ora-application` and `ora-db`.
- It gives the workspace one place to encode formatting, writer lifecycle, and retention behavior.
- It matches the existing architecture direction of introducing focused crates such as `ora-application` and `ora-contracts`.

Alternatives considered:
- Initialize `tracing_subscriber` directly inside `apps/web/server` with no shared crate. Rejected because formatting, rotation, and retention rules would be trapped in the adapter.
- Use the `log` facade and `env_logger`. Rejected because the desired JSON structure and file-rotation behavior align better with the `tracing` ecosystem.

### 2. Use the `tracing` ecosystem, but keep a custom JSON event contract

The implementation will use `tracing`, `tracing-subscriber`, and `tracing-appender`. `ora-logging` will provide a formatter layer that writes one JSON object per line with the agreed top-level shape: `timestamp`, `level`, `target`, `message`, optional `span`, `trace_id`, `request_id`, plus `context` and `error` objects when present.

Why:
- `tracing` gives us structured fields at call sites, flexible subscribers, and mature sink support.
- A custom event formatter is the cleanest way to preserve the exact JSON envelope from the brainstorm instead of depending on the default `tracing-subscriber` JSON field layout.
- Keeping `context` and `error` as dedicated objects prevents uncontrolled top-level key growth.

Alternatives considered:
- Use the default `tracing_subscriber::fmt().json()` output. Rejected because it does not naturally enforce the desired top-level `context` and `error` objects.
- Serialize ad hoc logging structs at each call site. Rejected because it would duplicate formatting logic and make sink behavior inconsistent.

### 3. Initialize logging once in `apps/web/server` and keep the guard there

`apps/web/server/src/main.rs` will remain the composition root for backend startup. It will build a `LoggingConfig`, call `ora_logging::init_logging`, and hold the returned `LoggingGuard` for the process lifetime.

Why:
- Process-wide subscriber initialization is inherently a composition-root concern.
- Holding the guard in `main` makes the non-blocking file writer lifetime explicit and prevents accidental log loss.
- It avoids requiring downstream crates to know anything about sink setup.

Alternatives considered:
- Lazily initialize logging inside `ora-db` or `ora-application`. Rejected because those crates should emit events, not own process-global bootstrap.
- Hide the guard inside a global singleton. Rejected because it obscures lifetime management and complicates tests.

### 4. Emit logs directly from `ora-application` and `ora-db`, but keep `ora-domain` side-effect free

`ora-application` handlers and `ora-db` bootstrap and migration flows will emit `tracing` events directly. `ora-domain` will continue to return typed values and errors only, leaving outer layers responsible for adding operational context such as `operation`, `project_id`, and `migration_version`.

Why:
- Application and persistence layers own the operational context that makes logs useful.
- Avoiding domain logging preserves the current separation of pure business rules from infrastructure concerns.
- Direct event emission keeps the code simple while still allowing scoped subscriber-based tests.

Alternatives considered:
- Add a logging trait and inject it into every handler and migration component. Rejected for now because process-wide structured events are already provided by `tracing`, and an extra abstraction would increase constructor noise without a clear behavior gap.

### 5. Treat retention as a responsibility of file-backed logging only

`LogOutput::Stdout` will never rotate. `LogOutput::File(...)` and `LogOutput::StdoutAndFile(...)` will use daily rotation plus post-initialization retention cleanup that deletes log files older than the configured retention window. The environment contract for the first web-server integration is:

- `ORA_LOG_LEVEL=info`
- `ORA_LOG_MODE=stdout`
- `ORA_LOG_PATH=./ora.log`
- `ORA_LOG_MAX_DAYS=3`

`ORA_LOG_PATH` names the active log file path for file-backed modes. `ORA_LOG_MAX_DAYS` defines the number of daily log files to retain, including the current day's active file. `ORA_LOG_MODE=stdout` remains the default, so file-path and retention settings are ignored unless a file-backed mode is selected.

Why:
- It matches the brainstorm decisions and keeps stdout behavior predictable in local shells and container logs.
- Daily retention matches the `ORA_LOG_MAX_DAYS` operational setting more directly than a raw file-count limit.
- A single `ORA_LOG_PATH` variable is simpler for operators than separate directory and prefix variables.

Alternatives considered:
- Express retention as `max_files` instead of days. Rejected because the chosen environment contract is day-oriented and should stay self-describing.
- Apply retention only to archived files and ignore the active file. Rejected because the resulting retained daily window would exceed the configured limit.
- Build custom rotation instead of using `tracing-appender`. Rejected because the ecosystem crate already covers the rotation trigger cleanly.

### 6. Reserve helper APIs for optional correlation fields from the start

The first implementation will reserve helper APIs in `ora-logging` for optional `span`, `trace_id`, and `request_id` fields instead of leaving those fields as purely ad hoc call-site conventions. The formatter will still only emit those fields when values are attached, but the crate will define the helper surface that runtime crates use to attach them consistently.

Why:
- It prevents drift in field naming and placement once more adapters and request-oriented flows arrive.
- It gives the first implementation a stable extension point without forcing every current call site to populate those fields immediately.
- It keeps future correlation work additive instead of requiring formatter and call-site rewrites later.

Alternatives considered:
- Allow each call site to attach optional fields with no shared helper APIs. Rejected because it risks inconsistent naming and envelope shape.
- Fully defer helper APIs to a follow-up change. Rejected because the schema already reserves these fields and the first implementation should define how they enter the envelope.

### 7. Support environment-driven logging configuration in the first web server integration

`apps/web/server` will still initialize logging in Rust bootstrap code, but it will derive `LoggingConfig` from environment-driven inputs in this change rather than hard-coding all values in `main`. Rust bootstrap remains the composition root, while environment variables become the external configuration surface for level, output mode, log path, and daily retention settings. The first implementation will support all logging modes through `ORA_LOG_MODE`: `stdout`, `file`, and `stdout_and_file`.

Why:
- Logging behavior is operational configuration, and environment-driven inputs make the first service integration usable across local development and deployment contexts.
- It keeps the initialization ownership in the composition root while avoiding a follow-up refactor just to make logging configurable.
- It aligns with the service-style nature of `apps/web/server` better than compile-time-only defaults.
- Supporting the full mode set in the initial implementation avoids a second pass through the formatter and sink builder just to add the combined sink path later.

Alternatives considered:
- Hard-code logging configuration in Rust bootstrap for the first version. Rejected because it would make file output and level changes unnecessarily invasive.
- Introduce a broader application settings system first. Rejected because this change only needs a narrow configuration reader for logging.
- Delay `file` or `stdout_and_file` support to follow-up work. Rejected because this feature already defines the full sink model and should ship it coherently.

### 8. Verify behavior with formatter tests and scoped subscriber integration tests

`ora-logging` will own unit tests for JSON serialization, timestamp formatting, sink selection, and retention cleanup. `ora-application` and `ora-db` will add targeted tests that run operations under a test subscriber and assert that expected events are emitted for success and failure paths.

Why:
- The JSON envelope is a public contract for this feature and should be asserted close to the formatter.
- Scoped subscribers keep application and database tests deterministic without mutating process-wide environment state.

## Risks / Trade-offs

- [Custom formatter complexity] -> Keep the public event contract small and cover formatter output with snapshot-like assertions over full JSON objects.
- [Global subscriber initialization is hard to undo in tests] -> Keep `init_logging` tests inside `ora-logging` focused on one-time initialization boundaries, and use scoped subscribers for most behavior verification elsewhere.
- [Retention cleanup can delete the wrong files if matching is too broad] -> Restrict cleanup to files owned by the configured prefix within the configured directory.
- [Logging volume may grow quickly once handler-level events are added] -> Start with `debug`, `info`, and `error` only, and define narrow event conventions around bootstrap, CRUD operations, and migration steps.

## Migration Plan

1. Add `ora-logging` and workspace dependencies for the `tracing` stack.
2. Implement the formatter, sink builder, and retention cleanup inside `ora-logging`.
3. Implement helper APIs for optional `span`, `trace_id`, and `request_id` attachment and cover them with formatter tests.
4. Initialize logging in `apps/web/server`, derive `LoggingConfig` from `ORA_LOG_LEVEL`, `ORA_LOG_MODE`, `ORA_LOG_PATH`, and `ORA_LOG_MAX_DAYS`, and keep the guard alive in `main`.
5. Add structured events to `ora-application` handlers and `ora-db` bootstrap and migration paths.
6. Update documentation to describe the logging contract, helper APIs, environment configuration, and runtime ownership.
7. Run `task test` to validate the new crate and the changed runtime behavior.

Rollback is straightforward during development: remove the `ora-logging` wiring from the composition root and revert the direct event calls if the implementation proves too noisy or unstable before wider adoption.

## Open Questions

- None.
