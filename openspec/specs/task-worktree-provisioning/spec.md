## Purpose

Define the backend-owned linked-worktree lifecycle that task creation and deletion must enforce.

## Requirements

### Requirement: Task creation SHALL provision exactly one internal linked worktree
The system SHALL treat linked worktree provisioning as part of task creation. When a new task is created, the backend SHALL reject a project id other than the configured Git repository's project, derive a task-owned branch name and worktree root from the generated task identifier, persist that root and branch in a `ProvisioningPending` worktree record, create one linked Git worktree, persist its immutable baseline, persist the task with the new internal `worktree_id`, and finally activate the worktree.

#### Scenario: A request names another project
- **WHEN** task creation supplies a project id other than the project bound to the configured Git repository
- **THEN** the backend rejects the request before persisting or provisioning a worktree

#### Scenario: Creating a task provisions and links a worktree
- **WHEN** the backend handles a valid task creation request for the configured project
- **THEN** it creates one linked Git worktree, persists one worktree record, and stores the task-to-worktree linkage internally without exposing the internal `worktree_id` in the public task payload

#### Scenario: Branch and path are derived from the generated task identity
- **WHEN** the backend generates a new task identifier during task creation
- **THEN** it derives the Git branch name from a short task-id prefix and the worktree directory from the full task identifier under the configured worktree root

#### Scenario: Short branch-prefix collisions regenerate the task identity
- **WHEN** the generated short task-id prefix is already used by a task worktree directory or a local `ora/<prefix>` branch
- **THEN** the backend generates another task identifier before provisioning, because the collision applies to the shortened branch name rather than the full-identifier worktree directory

#### Scenario: Orphaned task branches still reserve their prefix
- **WHEN** a task worktree directory has been removed but its local `ora/<prefix>` branch remains
- **THEN** the backend treats that prefix as unavailable and generates another task identifier

### Requirement: Task creation SHALL keep Git and persistence state aligned
The system SHALL avoid exposing partially created task workspaces. It SHALL persist provisioning intent before mutating Git and SHALL write the persisted worktree id into the linked worktree's private Git directory as its instance identity. If linked-worktree provisioning or later persistence fails normally, the backend SHALL attempt compensating cleanup. If the process stops between stages, startup recovery SHALL either activate a pending worktree already owned by a persisted task or delete an orphaned checkout and soft-delete its pending record. Recovery SHALL leave the pending record visible instead of deleting a checkout whose identity marker is missing or mismatched.

#### Scenario: Git provisioning fails before persistence
- **WHEN** linked-worktree creation fails for a new task
- **THEN** the backend returns a task-creation failure and no task or worktree record is persisted

#### Scenario: Task persistence fails after Git worktree creation
- **WHEN** the backend has already created the linked worktree but later fails to persist the worktree or task record
- **THEN** it attempts to delete the created linked worktree before returning a failure outcome

#### Scenario: Runtime stops during worktree creation
- **WHEN** a `ProvisioningPending` worktree remains after an interrupted create flow
- **THEN** startup recovery activates it only when a matching task, project, baseline, and worktree id were persisted and the exact checkout still carries the persisted identity marker and branch
- **THEN** otherwise startup recovery removes the orphaned checkout and soft-deletes the pending worktree row

### Requirement: Task deletion SHALL remove the task-owned linked worktree with force mode
The system SHALL treat the linked worktree as backend-owned task state. When a task is deleted, the backend SHALL remove the linked worktree associated with that task before finalizing deletion, and it SHALL use force mode for the first version so dirty worktrees do not block cleanup.

#### Scenario: Deleting a task removes its linked worktree
- **WHEN** the backend deletes a task that has an associated linked worktree
- **THEN** it removes that linked worktree and completes task deletion

#### Scenario: Dirty linked worktree does not block task deletion
- **WHEN** the linked worktree associated with a task contains uncommitted changes during task deletion
- **THEN** the backend uses force-mode worktree removal so cleanup still proceeds

### Requirement: Task worktree removal SHALL be durable and recoverable
Before mutating Git, the backend SHALL persist the worktree lifecycle as `RemovalPending`. Git removal SHALL use the absolute worktree root persisted at creation rather than deriving it from current configuration, SHALL be idempotent, SHALL verify both the persisted task branch and the worktree-id identity marker before mutation, and the runtime SHALL retry visible pending removals for the configured project during startup. A failed retry SHALL leave the pending row visible for a later recovery pass.

#### Scenario: Git removal fails after durable intent is recorded
- **WHEN** a task worktree enters `RemovalPending` and Git removal fails
- **THEN** the task and worktree remain visible and the worktree remains `RemovalPending`

#### Scenario: Runtime starts with pending worktree removals
- **WHEN** startup finds one or more visible `RemovalPending` worktrees
- **THEN** it retries Git removal only for worktrees whose persisted `project_id` matches the configured project and soft-deletes the corresponding task and worktree rows after cleanup succeeds

#### Scenario: A worktree path is occupied by another branch
- **WHEN** deletion or recovery finds the expected checkout path registered to a branch other than the persisted task branch
- **THEN** it rejects Git removal and leaves the pending worktree visible for investigation or a later retry

#### Scenario: Git worktree was removed before a retry
- **WHEN** recovery retries a pending removal whose trusted worktree identity is already absent
- **THEN** Git prunes stale worktree administration metadata
- **THEN** cleanup is treated as successful and persistence cleanup continues

### Requirement: Worktree-backed tasks SHALL remain in their configured project
The system SHALL reject task updates that would change a worktree-backed task's project identity. Diff and deletion flows SHALL verify that the task, persisted worktree, and configured Git repository project agree before accessing Git.

#### Scenario: A task update attempts project reassignment
- **WHEN** an update supplies a project id different from the configured worktree project
- **THEN** the backend returns a project-mismatch conflict
- **THEN** neither the task nor its worktree ownership changes
