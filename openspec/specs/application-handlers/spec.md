## Purpose

Define transport-agnostic application handlers and handler-owned ports for the first `project` CRUD vertical slice.

## Requirements

### Requirement: Project CRUD handlers SHALL be transport-agnostic
The system SHALL define `CreateProjectHandler`, `GetProjectHandler`, `ListProjectsHandler`, `UpdateProjectHandler`, and `DeleteProjectHandler` in `ora-application` as transport-agnostic entry points. Each handler SHALL accept exactly one request contract type and SHALL return exactly one response contract type or an application error without referencing HTTP, Tauri, or database-specific types.

#### Scenario: HTTP adapter invokes a handler
- **WHEN** an HTTP route receives a `project` CRUD request
- **THEN** it can deserialize transport input into one `ora-contracts` request value, call the corresponding `ora-application` handler, and serialize the returned contract response or application error without additional use-case orchestration in the adapter

#### Scenario: Tauri adapter invokes a handler
- **WHEN** a Tauri command needs to perform a `project` CRUD action
- **THEN** it can translate command input into one `ora-contracts` request value and delegate the use case to the same `ora-application` handler API used by other adapters

### Requirement: Application handlers SHALL own project repository ports
The system SHALL define the repository and supporting dependency traits required by the `project` CRUD handlers inside `ora-application`, and handlers SHALL depend on those ports rather than concrete persistence implementations.

#### Scenario: Unit test executes a project handler
- **WHEN** a test constructs a `project` handler with in-memory fake dependencies
- **THEN** the handler can execute the full use case without a database, HTTP server, or Tauri runtime

#### Scenario: Persistence adapter integrates later
- **WHEN** `ora-db` implements project persistence for the application layer
- **THEN** it implements `ora-application` ports instead of changing handler signatures or moving orchestration logic into the database crate

### Requirement: DeleteProjectHandler SHALL preserve CRUD semantics externally
The system SHALL expose deletion through `DeleteProjectHandler` as a normal CRUD delete use case even if the underlying repository implementation performs a soft delete internally.

#### Scenario: Adapter requests project deletion
- **WHEN** an adapter invokes `DeleteProjectHandler`
- **THEN** it interacts with a delete-oriented request and response contract rather than transport-visible soft-delete or archive semantics

### Requirement: Project CRUD handlers SHALL emit structured operational events
The system SHALL require `CreateProjectHandler`, `GetProjectHandler`, `ListProjectsHandler`, `UpdateProjectHandler`, and `DeleteProjectHandler` to emit structured operational logs from `ora-application` without introducing transport-specific concerns. These events SHALL use the shared JSON logging envelope and SHALL include business context such as the use-case operation name and relevant project identifiers when available.

#### Scenario: Handler completes a project use case successfully
- **WHEN** a project CRUD handler completes successfully
- **THEN** `ora-application` emits an informational event that identifies the operation and includes the relevant `project_id` when that identifier is available for the use case

#### Scenario: Handler encounters an application-layer failure
- **WHEN** a project CRUD handler returns a not-found or repository failure outcome
- **THEN** `ora-application` emits an error event that records the operation context and failure details without requiring an HTTP or Tauri adapter to add the log entry itself
