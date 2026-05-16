## Purpose

Define the persisted web server runtime contract for Ora's HTTP backend, including SQLite-backed bootstrap, CRUD route exposure, and stable readiness and error behavior.

## Requirements

### Requirement: Web server runtime SHALL bootstrap a file-backed SQLite application state
The system SHALL make `apps/web/server` construct its shared runtime state from a file-backed SQLite database through `ora-db` during startup. The runtime SHALL load a database path and a configured bootstrap project identity from typed bootstrap configuration, SHALL run database bootstrap and repository-pool construction before marking the server ready, SHALL reconcile the configured project into persistent storage before application state is returned, and SHALL fail startup with a typed bootstrap error when the database path or bootstrap project configuration is invalid or the SQLite bootstrap sequence cannot complete.

#### Scenario: Server starts with a usable database path and a missing configured project
- **WHEN** `ora-web-server` starts with a valid file-backed database path plus `ORA_PROJECT_NAME` and `ORA_PROJECT_PATH`, and no visible project row exists with that configured name
- **THEN** startup bootstraps SQLite, creates one project row with the configured name and path, constructs the shared runtime state, and only then reports readiness success

#### Scenario: Server starts with an existing configured project whose stored path drifted
- **WHEN** `ora-web-server` starts with a valid bootstrap configuration and a visible project row already exists for the configured project name but its stored `root_path` differs from `ORA_PROJECT_PATH`
- **THEN** startup updates that existing project row in place to the configured path before the runtime is considered ready

#### Scenario: Server starts with a usable database path and an already reconciled configured project
- **WHEN** `ora-web-server` starts with a valid bootstrap configuration and a visible project row already exists whose name and path match the configured project identity
- **THEN** startup leaves the existing row unchanged, constructs shared repositories and handlers, and reports readiness success

#### Scenario: Bootstrap project configuration is invalid
- **WHEN** `ora-web-server` starts with a blank or missing configured bootstrap project name or path
- **THEN** startup fails with a typed bootstrap error instead of serving requests with an unknown workspace identity

#### Scenario: Database bootstrap fails during startup
- **WHEN** the configured SQLite database cannot be opened, migrated, or pooled during web-server bootstrap
- **THEN** startup fails with a typed bootstrap error instead of serving requests with a partially initialized runtime

### Requirement: Web server runtime SHALL expose HTTP CRUD routes for all core persisted models
The system SHALL expose create, get, list, update, and delete HTTP routes for `project`, `task`, `worktree`, and `session`. Each route SHALL translate transport input into the matching `ora-contracts` request DTO, SHALL delegate to the corresponding `ora-application` handler, and SHALL serialize the returned `ora-contracts` response DTO without adding adapter-local response shapes.

#### Scenario: Client performs task CRUD over HTTP
- **WHEN** a caller creates, fetches, lists, updates, or deletes a task through the web server
- **THEN** the server delegates to the matching task application handler and returns the matching `ora-contracts` task response payload

#### Scenario: Client performs worktree CRUD over HTTP
- **WHEN** a caller creates, fetches, lists, updates, or deletes a worktree through the web server
- **THEN** the server delegates to the matching worktree application handler and returns the matching `ora-contracts` worktree response payload

#### Scenario: Client performs session CRUD over HTTP
- **WHEN** a caller creates, fetches, lists, updates, or deletes a session through the web server
- **THEN** the server delegates to the matching session application handler and returns the matching `ora-contracts` session response payload

#### Scenario: Existing project CRUD uses persisted storage
- **WHEN** a caller creates a project and later fetches or lists projects through the same SQLite-backed runtime
- **THEN** the project routes read and write through the persistent repository-backed application handlers instead of an in-memory bootstrap store

### Requirement: Web server runtime SHALL keep HTTP readiness and error semantics stable across all model routes
The system SHALL return readiness success only after the database-backed runtime state is fully initialized, and it SHALL map application-layer not-found and repository failures for `project`, `task`, `worktree`, and `session` into stable structured HTTP error responses.

#### Scenario: Resource route requests a missing entity
- **WHEN** any project, task, worktree, or session get, update, or delete route delegates to an application handler that returns a not-found outcome
- **THEN** the server responds with an HTTP not-found status and a structured error payload that identifies the missing entity family

#### Scenario: Resource route encounters a repository failure
- **WHEN** any project, task, worktree, or session route delegates to an application handler that returns a repository failure
- **THEN** the server responds with an HTTP server-error status and a structured error payload instead of leaking raw infrastructure error formatting
