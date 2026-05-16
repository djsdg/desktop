## MODIFIED Requirements

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
