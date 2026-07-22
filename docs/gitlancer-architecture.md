# Gitlancer Architecture

## Goals

`gitlancer` is designed as a Git CLI runtime for Ora, an AI-agent-oriented IDE.
The main goals are:

- Make multi-worktree support a first-class capability instead of an afterthought.
- Provide stable typed request/response contracts for upper layers.
- Keep the implementation strictly on top of the Git CLI, without `libgit2`.
- Make execution observable, injectable, and easy to test.
- Prefer repository- and worktree-aware domain types that prevent invalid states.

## Design Principles

1. Model repository shapes explicitly.
   Main worktrees, linked worktrees, repo roots, git dirs, and repo-relative paths are different concepts and should use different types.
2. Separate domain, execution, parsing, and Git use cases.
   Command construction should not be mixed with filesystem validation or stdout parsing.
3. Keep requests and responses explicit.
   Ora will benefit from stable typed command boundaries more than extension traits on a mutable handle.
4. Prefer static dispatch.
   `Git<R: GitRunner>` keeps the execution backend generic and testable without dynamic dispatch.
5. Parse only stable Git outputs.
   Prefer porcelain and plumbing commands such as `git worktree list --porcelain`, `git status --porcelain=v2 -z`, and `git rev-parse`.

## Layer Responsibilities

### `domain`

The domain layer owns repository facts and invariants:

- `RepoRoot`, `WorktreeRoot`, `GitDir`
- `RepoRelativePath`
- `Repository`
- `WorktreeHandle`
- `WorktreeKind`

This layer should answer questions such as:

- Is this path a repository root?
- Which repo does this worktree belong to?
- Is this path safe to pass to `git add` from this worktree?

It should not spawn processes or parse command output directly.

### `exec`

The execution layer wraps Git CLI invocation:

- `GitCommand`
- `GitIntent`
- `GitEnv`
- `GitOutput`
- `GitRunner`
- `CliGitRunner`

This layer exists so upper layers can:

- inject a fake runner in tests,
- record commands for debugging or telemetry,
- distinguish read-only, mutating, and networked Git operations.

### `git`

The Git layer exposes typed use cases that Ora can call directly:

- repository discovery,
- worktree discovery and selection,
- add / commit / status,
- branch-oriented read and lifecycle flows,
- linked-worktree lifecycle flows.

Each use case should take a typed request object and return a typed response object.
That keeps option growth manageable and produces better call boundaries for agent orchestration.

### `parse`

The parse layer converts stable Git output into typed results.
It should focus on porcelain/plumbing formats and avoid parsing human-oriented messages whenever possible.

## Core Types

### Runtime

```rust
pub struct Git<R: GitRunner> {
    runner: R,
}
```

`Git` is the entry point for all Git use cases.
It owns the execution strategy but not repository state.

### Repository and Worktree

```rust
pub struct Repository {
    root: RepoRoot,
}

pub struct WorktreeHandle {
    repo_root: RepoRoot,
    worktree_root: WorktreeRoot,
    git_dir: GitDir,
    kind: WorktreeKind,
    head_commit_id: CommitId,
    branch: Option<BranchName>,
    identity_token: Option<WorktreeIdentityToken>,
}

pub enum WorktreeKind {
    Main,
    Linked { name: String },
}
```

This structure makes multi-worktree support explicit and removes ambiguity between:

- the repository root,
- the directory where a command should run,
- the gitdir backing that worktree.

`Repository` and `WorktreeHandle` values are created only by Git-backed discovery; their constructors are not part of the public API. Listing uses NUL-delimited `git worktree list --porcelain -z`, then resolves each operational checkout with `git rev-parse --absolute-git-dir`. Ora-managed creation writes the persisted worktree id as a bounded identity marker inside the private Git administration directory. Deletion re-discovers the checkout and verifies the marker together with the resolved Git directory and checked-out branch before mutation. Path, branch, and Git administration-name reuse therefore cannot make a replacement checkout satisfy a stale handle. This prevents linked-worktree `.git` pointer files or caller-assembled metadata from being mistaken for a trusted Git identity.

Bare repository records are modeled separately because Git omits checkout-only HEAD and branch fields for them. Discovery retains the bare repository as the owning `RepoRoot`, while listing returns only its operational linked worktrees. Name-based resolution applies only to linked worktrees, so a valid linked administration name such as `main` cannot collide with the main checkout.

The application-owned persisted `ora_domain::Worktree` is a separate lifecycle record. New records store a managed absolute checkout root and branch together, begin in `ProvisioningPending`, become `Active` only after Task persistence, and enter `RemovalPending` before deletion. Startup reconciliation completes both interrupted creation and deletion. Historical rows without a persisted root decode as `LegacyUnavailable` and are never passed to destructive Git operations.

## Request / Response Style

Instead of attaching methods directly to `Worktree`, gitlancer favors explicit request objects:

```rust
pub struct AddRequest<'a> {
    pub worktree: &'a WorktreeHandle,
    pub paths: Vec<RepoRelativePath>,
}

pub struct CommitRequest<'a> {
    pub worktree: &'a WorktreeHandle,
    pub message: &'a str,
    pub allow_empty: bool,
}

pub struct ListWorktreesRequest<'a> {
    pub repository: &'a Repository,
}

pub struct CreateBranchRequest<'a> {
    pub repository: &'a Repository,
    pub branch_name: BranchName,
}

pub struct CreateWorktreeRequest<'a> {
    pub repository: &'a Repository,
    pub worktree_root: WorktreeRoot,
    pub branch_name: BranchName,
    pub identity_token: WorktreeIdentityToken,
}

pub struct DeleteWorktreeRequest<'a> {
    pub repository: &'a Repository,
    pub worktree: &'a WorktreeHandle,
    pub mode: WorktreeDeletionMode,
}
```

This is a better fit for Ora because requests are:

- easier to log,
- easier to extend with options,
- easier to serialize into agent tool payloads,
- easier to validate before execution.

## Execution Semantics

`GitCommand` should carry enough metadata for policy and observability:

```rust
pub struct GitCommand {
    pub cwd: PathBuf,
    pub args: Vec<String>,
    pub env: GitEnv,
    pub intent: GitIntent,
}
```

Suggested intents:

- `ReadOnly`
- `Mutating`
- `Network`

Ora can use those intents to decide whether a command can run automatically, needs confirmation, or should be retried.

`CliGitRunner` applies a finite execution deadline, closes stdin, and bounds captured stdout and stderr by default. Diff operations may select smaller per-command limits through `run_bounded`. The deadline remains active until Git has exited and both captured streams have closed, including when a detached hook or alias descendant inherits a pipe. On timeout or output overflow, the runner terminates Git's complete process tree by reusing `ora-process` (Unix process groups or Windows Job Objects), so hooks and aliases cannot leave descendants running. Every attempt reports command and completion events through the optional `GitlancerLogger`. These controls keep hooks, malformed repositories, and unexpectedly large command output from blocking, leaking processes, or exhausting the host process.

Task diff generation forces stable `a/` and `b/` path prefixes, reads `HEAD` before collecting tracked and untracked patches, and verifies it again afterward. A concurrent commit or checkout therefore produces a typed snapshot-change error instead of returning a patch paired with stale revision metadata. Git commands that accept caller-selected repository paths use literal pathspec mode so pathspec-like filenames cannot widen the requested operation.

## Parsing Strategy

gitlancer should rely on stable machine-readable outputs:

- `git rev-parse --show-toplevel`
- `git rev-parse --git-dir`
- `git worktree list --porcelain`
- `git status --porcelain=v2 -z`
- `git rev-parse HEAD`
- `git log -1 --pretty=%s`

Human-readable stderr remains useful for diagnostics, but it should not be the primary source of structured state.

## Error Model

The public error hierarchy should clearly distinguish:

- domain validation failures,
- process spawning or execution failures,
- parsing failures.

Suggested shape:

```rust
GitlancerError
  - Domain(DomainError)
  - Exec(GitExecError)
  - Parse(ParseError)
```

Key examples:

- `DomainError::NotARepository`
- `DomainError::PathOutsideWorktree`
- `DomainError::WorktreeMismatch`
- `GitExecError::GitNotFound`
- `GitExecError::SpawnFailed`
- `GitExecError::NonZeroExit`
- `ParseError::InvalidWorktreeList`

## Testing Strategy

gitlancer should be tested at three levels:

1. Unit tests for parsers and path/domain validation.
2. Fake-runner tests for command assembly and option handling.
3. Real Git integration tests for multi-worktree scenarios.

Priority integration scenarios:

- open repository from nested directory,
- list main and linked worktrees,
- add and commit from a linked worktree,
- detect worktree mismatch,
- parse `status --porcelain=v2 -z`,
- handle linked-worktree `.git` indirection correctly.

## Implementation Status

The v1 runtime implements repository discovery, trusted worktree discovery, branch lifecycle operations, structured status and commit parsing, linked-worktree creation and identity-safe deletion, bounded task diffs, and optional structured logging. Ora's task integration resolves task worktrees by exact checkout root and verifies the persisted branch plus worktree-id marker before reading a diff or deleting a checkout.

Network operations are intentionally outside the current runtime specification. `GitIntent::Network` remains available for a future fetch, pull, push, or clone design, but its presence does not imply those operations are implemented.
