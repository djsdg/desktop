### Requirement: Database bootstrap SHALL track applied migration versions
The system SHALL create and maintain a dedicated `migrations` table in each managed SQLite database. The table SHALL record one row per applied migration version plus its execution timestamp and SHALL be the source of truth for migration state reconciliation.

#### Scenario: Bootstrap initializes migration tracking
- **WHEN** the migration runner targets a database that does not yet contain the migration bookkeeping table
- **THEN** it creates the `migrations` table before recording applied migration versions

### Requirement: Migrations SHALL define explicit up and down steps
Every bundled database migration SHALL provide an ordered version identifier, an `up` operation, and a `down` operation defined in Rust code so the runner can move schema state in both directions.

#### Scenario: Migration catalog is loaded
- **WHEN** the application enumerates bundled migrations during bootstrap
- **THEN** each migration includes a version plus executable `up` and `down` definitions sourced from Rust code

### Requirement: Missing bundled versions SHALL be applied in ascending order
If the target database records fewer applied versions than are present in the bundled migration set, the runner SHALL execute each missing migration's `up` step in ascending version order and record the version together with its execution timestamp after the step succeeds.

#### Scenario: Database is behind the bundled migration set
- **WHEN** the database has applied versions `0001` and the bundled set contains `0001`, `0002`, and `0003`
- **THEN** the runner executes `0002` up before `0003` up and records each version as it completes

### Requirement: Extra recorded versions SHALL be rolled back in descending order
If the target database records applied versions that are not present in the bundled migration set, the runner SHALL execute the corresponding `down` steps in descending version order and remove each rolled back version from the `migrations` table.

#### Scenario: Database is ahead of the bundled migration set
- **WHEN** the database records `0001`, `0002`, and `0003` but the bundled set contains only `0001` and `0002`
- **THEN** the runner executes `0003` down before finishing reconciliation and removes `0003` from the migration table

### Requirement: Shared-prefix mismatches SHALL fail explicitly
The runner SHALL treat any mismatch within the shared prefix of applied and bundled versions as an error instead of attempting automatic repair.

#### Scenario: Applied history diverges from available history
- **WHEN** the database records `0001`, `0003` and the bundled set contains `0001`, `0002`
- **THEN** reconciliation fails with an explicit version mismatch error and does not execute further migrations

### Requirement: The initial migration SHALL reproduce the current base schema
The first bundled migration SHALL create the schema currently defined in `docs/schema.sql` and SHALL also create the `migrations` tracking table needed for version bookkeeping, including an execution timestamp column.

#### Scenario: Fresh database is initialized from the first migration
- **WHEN** the runner bootstraps an empty SQLite database with only the initial migration available
- **THEN** the resulting schema includes the tables from `docs/schema.sql` plus the `migrations` table

### Requirement: Failed migration steps SHALL not advance recorded state
If an `up` or `down` step fails, the runner SHALL stop execution and SHALL NOT record the failed target version as applied or removed unless the corresponding schema change succeeded.

#### Scenario: Up migration fails during reconciliation
- **WHEN** a missing migration's `up` step returns an execution error
- **THEN** the runner reports the failure and leaves the failed version absent from the `migrations` table

### Requirement: Database bootstrap SHALL emit structured migration reconciliation events
The system SHALL require `ora-db` database bootstrap and migration reconciliation flows to emit structured operational logs through the shared logging envelope. These events SHALL cover database open and bootstrap lifecycle, migration reconciliation decisions, migration step execution, and migration failures with context such as the active operation and migration version when applicable.

#### Scenario: Bootstrap reconciles pending migrations
- **WHEN** the database bootstrap path finds bundled migrations that have not yet been applied
- **THEN** `ora-db` emits informational events that describe the reconciliation decision and each migration version as it is executed

#### Scenario: Migration step fails during reconciliation
- **WHEN** an `up` or `down` migration step fails
- **THEN** `ora-db` emits an error event that includes the migration version and failure details before returning the migration error to the caller
