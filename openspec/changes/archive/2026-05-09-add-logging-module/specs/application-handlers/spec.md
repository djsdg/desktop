## ADDED Requirements

### Requirement: Project CRUD handlers SHALL emit structured operational events
The system SHALL require `CreateProjectHandler`, `GetProjectHandler`, `ListProjectsHandler`, `UpdateProjectHandler`, and `DeleteProjectHandler` to emit structured operational logs from `ora-application` without introducing transport-specific concerns. These events SHALL use the shared JSON logging envelope and SHALL include business context such as the use-case operation name and relevant project identifiers when available.

#### Scenario: Handler completes a project use case successfully
- **WHEN** a project CRUD handler completes successfully
- **THEN** `ora-application` emits an informational event that identifies the operation and includes the relevant `project_id` when that identifier is available for the use case

#### Scenario: Handler encounters an application-layer failure
- **WHEN** a project CRUD handler returns a not-found or repository failure outcome
- **THEN** `ora-application` emits an error event that records the operation context and failure details without requiring an HTTP or Tauri adapter to add the log entry itself
