## ADDED Requirements

### Requirement: Database bootstrap SHALL emit structured migration reconciliation events
The system SHALL require `ora-db` database bootstrap and migration reconciliation flows to emit structured operational logs through the shared logging envelope. These events SHALL cover database open and bootstrap lifecycle, migration reconciliation decisions, migration step execution, and migration failures with context such as the active operation and migration version when applicable.

#### Scenario: Bootstrap reconciles pending migrations
- **WHEN** the database bootstrap path finds bundled migrations that have not yet been applied
- **THEN** `ora-db` emits informational events that describe the reconciliation decision and each migration version as it is executed

#### Scenario: Migration step fails during reconciliation
- **WHEN** an `up` or `down` migration step fails
- **THEN** `ora-db` emits an error event that includes the migration version and failure details before returning the migration error to the caller
