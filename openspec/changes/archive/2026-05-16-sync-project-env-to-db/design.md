## Context

`apps/web/server` currently bootstraps a SQLite-backed runtime from `RuntimeConfig`, but that configuration only covers database, bind, and logging settings. The server can serve project CRUD requests after startup, yet startup itself does not guarantee that the database contains a project record for the workspace the process is actually serving.

The requested change introduces a workspace identity contract at bootstrap time: the runtime must know the project name and root path from environment variables, ensure a matching visible project row exists, and repair the stored path when it drifts. This touches both the server bootstrap path and the SQLite project repository, so the design should keep the reconciliation logic explicit and testable without overloading the HTTP handlers.

## Goals / Non-Goals

**Goals:**

- Extend `RuntimeConfig` with typed accessors for `ORA_PROJECT_NAME` and `ORA_PROJECT_PATH`.
- Replace repeated environment-variable string literals in `apps/web/server/src/config.rs` with file-level constants.
- Reconcile the configured workspace project into the database during startup before readiness is reported.
- Support efficient repository lookup of a visible project by name so bootstrap does not need to scan the full project table.
- Preserve the existing project row when only the configured path changes, updating `root_path` and `updated_at` without generating a new project identity.

**Non-Goals:**

- Changing the external HTTP project CRUD API.
- Introducing support for multiple configured bootstrap projects in one process.
- Backfilling or deduplicating pre-existing duplicate project names already stored in the database.
- Redesigning project identifiers, audit field semantics, or the underlying SQLite schema.

## Decisions

### Treat project identity as required runtime configuration

`RuntimeConfig` will gain a dedicated project configuration section with `name` and `path` accessors sourced from `ORA_PROJECT_NAME` and `ORA_PROJECT_PATH`. These values should be validated the same way the database path is validated today: blank values fail bootstrap with typed configuration errors rather than silently creating unusable rows.

Why:
- The runtime cannot safely reconcile a persisted project row if the workspace identity is implicit or optional.
- Keeping these keys in typed config preserves the current separation between environment parsing and bootstrap behavior.
- Defining all environment variable names as top-level constants removes repeated string literals and makes the bootstrap contract easier to audit.

Alternative considered:
- Default the project name or path from the database path or current working directory.
  Rejected because inferred workspace identity is ambiguous and would make startup behavior surprising across environments.

### Reconcile the project record inside bootstrap before constructing application state

`bootstrap.rs` will perform project reconciliation immediately after creating the repository pool and before returning `AppState`. The bootstrap flow will create a `SqliteProjectRepository`, look up a visible project by configured name, and then:

- create a new `Project` when no visible row matches the configured name
- update the existing row when the configured path differs from the stored `root_path`
- leave the row untouched when both name and path already match

Why:
- This makes the persisted project record part of readiness, which matches the user’s request that startup should guarantee the configured project exists.
- Bootstrap already owns infrastructure setup, so it is the right place for one-time reconciliation side effects.
- Running reconciliation before the HTTP router starts avoids exposing a transient state where the server is ready but its workspace project is missing.

Alternative considered:
- Trigger reconciliation lazily from the first HTTP request.
  Rejected because it couples persistence repair to traffic order and weakens readiness guarantees.

### Add a repository-level lookup by name instead of listing all projects

The project repository port and SQLite adapter will gain a `find_project_by_name` operation that returns only visible rows. Bootstrap will use this query to reconcile the configured project without loading unrelated projects into memory.

Why:
- Name-based reconciliation is the business rule being added, so the repository should expose that intent directly.
- This keeps bootstrap logic small and avoids encoding persistence filtering rules outside the repository.
- A dedicated query gives tests a clearer seam than reusing `list_projects` and searching in adapter code.

Alternative considered:
- Call `list_projects` and search for a matching name in bootstrap.
  Rejected because it pushes repository concerns upward and does unnecessary work for a single-row lookup.

### Preserve project identity when path drift is corrected

When bootstrap finds an existing project with the configured name but a different `root_path`, it will update that existing project in place, preserving `id` and `created_at` while refreshing `updated_at`. This keeps downstream references stable while letting the runtime repair stale workspace locations.

Why:
- A path drift is an update to the same logical workspace, not a reason to create a second project row.
- Preserving identifiers avoids breaking tasks, sessions, or future relations that may reference the existing project id.

Alternative considered:
- Create a new project row whenever the path changes.
  Rejected because it would duplicate logical projects and orphan relationships keyed by the old id.

## Risks / Trade-offs

- [Startup now performs a write path in addition to database bootstrap] -> Mitigation: keep reconciliation idempotent and cover create, no-op, and update cases with focused bootstrap tests.
- [Name-based matching assumes project names are unique enough for bootstrap] -> Mitigation: limit the new rule to the first visible exact-name match and document that duplicate cleanup is out of scope for this change.
- [Adding new config validation may fail environments that previously booted] -> Mitigation: make the requirement explicit in docs and tests so missing project configuration fails fast and predictably.
- [Project reconciliation in bootstrap increases coupling to repository APIs] -> Mitigation: keep the logic in a small helper that depends only on the project repository port and clock-like collaborators.
