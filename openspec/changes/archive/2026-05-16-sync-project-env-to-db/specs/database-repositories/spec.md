## ADDED Requirements

### Requirement: Project repositories SHALL support visible lookup by project name
The system SHALL allow `ora-db` project repositories to load one visible `Project` by its exact persisted `name` so bootstrap flows can reconcile a configured workspace identity without listing the full project table. Name-based lookup SHALL ignore soft-deleted rows the same way identifier-based reads do.

#### Scenario: Visible project is queried by exact name
- **WHEN** a caller asks an `ora-db` project repository to find a project by a name stored on one visible row
- **THEN** the repository returns that full `Project` snapshot, including its existing identifier, root path, and audit fields

#### Scenario: Soft-deleted project shares the queried name
- **WHEN** a caller asks an `ora-db` project repository to find a project by name and only soft-deleted rows match that name
- **THEN** the repository returns `None` instead of exposing deleted project rows to bootstrap or application code

#### Scenario: No visible project matches the queried name
- **WHEN** a caller asks an `ora-db` project repository to find a project by name that does not exist among visible rows
- **THEN** the repository returns `None`
