## 1. Runtime configuration

- [x] 1.1 Add file-level constants for all environment variable keys in `apps/web/server/src/config.rs`.
- [x] 1.2 Extend `RuntimeConfig` with typed bootstrap project configuration sourced from `ORA_PROJECT_NAME` and `ORA_PROJECT_PATH`, including validation and tests for blank or missing values.

## 2. Bootstrap reconciliation

- [x] 2.1 Update `apps/web/server/src/bootstrap.rs` to reconcile the configured project after repository-pool creation and before returning `AppState`.
- [x] 2.2 Implement bootstrap tests that cover project creation, no-op reconciliation when name and path already match, and path updates when the stored project path drifts.

## 3. Project repository support

- [x] 3.1 Extend the project repository port and SQLite adapter with a visible `find_project_by_name` query that ignores soft-deleted rows.
- [x] 3.2 Add repository tests for exact-name lookup and for ignoring soft-deleted project rows during name-based reads.
