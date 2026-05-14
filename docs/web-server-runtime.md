# Web Server Runtime

`apps/web/server` is the first HTTP backend runtime for Ora.

## Purpose

- It boots shared structured logging through `ora-logging`.
- It exposes health endpoints for process liveness and runtime readiness.
- It serves the first HTTP-backed `project` CRUD slice by delegating to `ora-application`.

## Bind Configuration

The web server reads its listener configuration from:

- `ORA_HOST`: bind host. Default: `0.0.0.0`
- `ORA_PORT`: bind port. Default: `32578`

When unset, the server binds `0.0.0.0:32578`.

Invalid host or port values fail startup during bootstrap.

## Health Endpoints

- `GET /health/live`: confirms that the process is running
- `GET /health/ready`: confirms that application-state bootstrap completed successfully

`/health/ready` remains unavailable until the runtime finishes constructing its application state.

## Project HTTP API

The first server slice exposes project CRUD routes:

- `POST /api/projects`
- `GET /api/projects`
- `GET /api/projects/{project_id}`
- `PUT /api/projects/{project_id}`
- `DELETE /api/projects/{project_id}`

Request and response payloads use `ora-contracts` DTO shapes, so transport behavior stays aligned with the shared application contract.

## Current Storage Behavior

The current runtime uses an in-memory bootstrap repository.

- Data is not persisted across process restarts.
- The module boundaries are intentionally shaped so a future database-backed composition root can replace the bootstrap store without changing the HTTP route surface.
