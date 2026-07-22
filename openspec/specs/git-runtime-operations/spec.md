## Purpose

Define the typed Git runtime operations that Gitlancer exposes for repository, branch, status, commit, and linked-worktree workflows.

## Requirements

### Requirement: Repository worktree queries return multi-worktree-aware handles
The runtime SHALL list repository worktrees from `git worktree list --porcelain -z` and return trusted `WorktreeHandle` values whose `repo_root` points to the owning repository, whose `worktree_root` points to the checkout root, whose `git_dir` is resolved through `git rev-parse --absolute-git-dir`, whose optional identity token is read from the private Git administration directory, and whose `kind` distinguishes the main worktree from linked worktrees. Callers SHALL NOT be able to construct arbitrary worktree handles.

#### Scenario: Listing main and linked worktrees
- **WHEN** a repository contains its main checkout and one linked worktree
- **THEN** `list_worktrees` returns two worktrees
- **THEN** exactly one returned worktree is `WorktreeKind::Main`
- **THEN** the linked worktree is returned as `WorktreeKind::Linked`

### Requirement: Worktrees can be resolved by name and by nested path
The runtime SHALL resolve linked worktrees by their configured worktree name and SHALL locate which worktree contains an arbitrary nested filesystem path.

#### Scenario: Resolving a linked worktree by name
- **WHEN** a caller requests a linked worktree name that exists in the repository
- **THEN** `resolve_worktree` returns the corresponding `WorktreeHandle`

#### Scenario: A linked worktree uses the name main
- **WHEN** a linked worktree's Git administration name is `main`
- **THEN** `resolve_worktree` returns that linked worktree rather than the repository's main checkout

#### Scenario: Finding a worktree from a nested file path
- **WHEN** a caller provides a path nested under a linked worktree checkout
- **THEN** `find_worktree` returns the linked worktree that contains that path

#### Scenario: A bare repository owns linked worktrees
- **WHEN** a bare repository has no checkout HEAD record and owns one linked worktree
- **THEN** repository discovery retains the bare repository root
- **THEN** `list_worktrees` returns the operational linked worktree without treating the bare repository as a checkout

### Requirement: Runtime inspection commands return typed branch, status, and commit data
The runtime SHALL expose local branches, status entries, and commit metadata through typed responses backed by machine-readable Git output.

Typed branch names and commit IDs SHALL reject values that do not satisfy Git's local-ref rules or full SHA-1/SHA-256 object-ID shape. Status parsing SHALL reject absolute and parent-relative paths instead of constructing an invalid `RepoRelativePath`.
Commands that accept repository-relative caller paths SHALL disable Git pathspec magic, and generated patches SHALL force stable `a/` and `b/` prefixes instead of inheriting repository diff-prefix configuration.

#### Scenario: Listing local branches
- **WHEN** a repository contains multiple local branches
- **THEN** `list_branches` returns each local branch as a `BranchName`

#### Scenario: Reading structured status data
- **WHEN** a worktree contains tracked or untracked changes
- **THEN** `status` returns at least one `StatusEntry` for each porcelain-v2 status record

#### Scenario: Reading commit metadata after a successful commit
- **WHEN** `commit` succeeds in a worktree
- **THEN** the response includes the `HEAD` commit ID
- **THEN** the response includes the latest commit summary

#### Scenario: HEAD changes while a task diff is generated
- **WHEN** the worktree `HEAD` differs before and after tracked and untracked patches are collected
- **THEN** diff generation returns a typed snapshot-change error
- **THEN** the runtime does not return a patch paired with a stale `HEAD` commit ID

#### Scenario: Rejecting an unsafe status path
- **WHEN** malformed status output contains an absolute path or a parent-relative path
- **THEN** status parsing returns a typed parse error
- **THEN** no `RepoRelativePath` is constructed for that record

### Requirement: CLI execution is bounded by default
The CLI runner SHALL close child stdin, apply a finite execution deadline, and bound captured stdout and stderr for every command. A caller MAY select smaller output limits for operations such as task diff generation.

#### Scenario: A Git command exceeds its execution deadline
- **WHEN** a Git process or hook does not exit before the configured runner deadline
- **THEN** the runner terminates Git and its descendant process tree
- **THEN** it returns `GitExecError::TimedOut`

#### Scenario: A descendant retains output pipes after Git exits
- **WHEN** the direct Git process exits but a hook or alias descendant keeps stdout or stderr open beyond the deadline
- **THEN** the runner continues enforcing the deadline until both streams close
- **THEN** it terminates the descendant process tree and returns `GitExecError::TimedOut`

#### Scenario: Default command output exceeds its capture budget
- **WHEN** stdout or stderr exceeds the runner's default capture limit
- **THEN** the runner terminates Git and its descendant process tree
- **THEN** it returns `GitExecError::OutputTooLarge`

#### Scenario: Bounded execution is observable
- **WHEN** a CLI command succeeds, exceeds an output limit, or times out
- **THEN** the registered logger receives the command event and its corresponding completion event

### Requirement: Runtime branch lifecycle commands are exposed through typed repository APIs
The runtime SHALL expose typed APIs to create and delete local branches from repository-aware inputs without requiring callers to assemble raw Git arguments.

#### Scenario: Creating a local branch
- **WHEN** a caller requests creation of a new local branch in a repository
- **THEN** the runtime creates that branch through the Git CLI
- **THEN** the response identifies the created branch as a `BranchName`

#### Scenario: Deleting a local branch
- **WHEN** a caller requests deletion of an existing local branch in a repository
- **THEN** the runtime deletes that branch through the Git CLI
- **THEN** the deleted branch no longer appears in `list_branches`

### Requirement: Runtime worktree lifecycle commands manage linked worktrees explicitly
The runtime SHALL expose typed APIs to create and delete linked worktrees while preserving the distinction between the main worktree and linked worktrees. Destructive deletion SHALL require an Ora-managed identity marker; a discovered unmanaged checkout remains readable but cannot be removed through this safe lifecycle API.

#### Scenario: Creating a linked worktree
- **WHEN** a caller requests creation of a linked worktree for a repository at a target checkout path
- **THEN** the runtime creates the linked worktree through the Git CLI
- **THEN** `list_worktrees` returns the new worktree as `WorktreeKind::Linked`

#### Scenario: Deleting a linked worktree
- **WHEN** a caller requests deletion of an existing linked worktree that belongs to a repository
- **THEN** the runtime removes that linked worktree through the Git CLI
- **THEN** `list_worktrees` no longer returns the removed worktree

### Requirement: Lifecycle mutations reject invalid destructive targets through typed errors
The runtime SHALL reject unsupported or mismatched lifecycle requests with typed validation errors before invoking Git whenever the invalid state can be determined from repository and worktree metadata.

#### Scenario: Rejecting deletion of the main worktree
- **WHEN** a caller requests deletion of the repository's main worktree
- **THEN** the runtime returns a domain validation error
- **THEN** no Git deletion command is invoked

#### Scenario: Rejecting removal of a worktree from another repository
- **WHEN** a caller requests deletion of a linked worktree that does not belong to the supplied repository
- **THEN** the runtime returns a domain validation error
- **THEN** no Git deletion command is invoked

#### Scenario: Repeating deletion of an already removed worktree identity
- **WHEN** a caller retries deletion with a previously discovered linked-worktree handle after that identity has already been removed
- **THEN** the runtime returns an already-absent successful outcome without invoking another destructive Git command

#### Scenario: A checkout path is reused by a different worktree identity
- **WHEN** a stale handle names a path whose identity token, resolved Git directory, or checked-out branch no longer matches the handle
- **THEN** the runtime rejects deletion instead of removing the replacement worktree

#### Scenario: A checkout path and branch are both reused
- **WHEN** a linked worktree is removed and another checkout reuses its path, branch, and Git administration name
- **THEN** the replacement does not inherit the removed worktree's identity marker
- **THEN** deletion through the stale handle is rejected
