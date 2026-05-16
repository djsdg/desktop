## Why

The web server currently reads only the database path and network/logging settings from environment variables, so the runtime has no bootstrap contract for the workspace project it is serving. We need startup to reconcile a configured project name and root path into the database automatically so the persisted project list stays aligned with the active Ora workspace without requiring a separate API call.

## What Changes

- Add `ORA_PROJECT_NAME` and `ORA_PROJECT_PATH` to the web server runtime configuration contract.
- Refactor `apps/web/server/src/config.rs` to define environment variable key constants at the top of the file instead of repeating string literals inline.
- Make web-server bootstrap reconcile the configured project against persisted projects by name.
- Create a project row when no existing project uses the configured name.
- Update the stored project path when a project with the configured name exists but its persisted path differs from the configured path.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `web-server-runtime`: Startup configuration now includes a required project identity contract and bootstraps the persisted project record before the runtime is considered ready.
- `database-repositories`: Project persistence must support looking up a visible project by name and replacing its stored root path when bootstrap detects drift.

## Impact

- Affected code: `apps/web/server/src/config.rs`, web server bootstrap/composition code, and `ora-db` project repository implementation and tests.
- Affected behavior: server startup now mutates the projects table to reflect the configured project name and path.
- Dependencies: no new external dependencies, but startup logic will depend on existing project repository and domain construction paths.
