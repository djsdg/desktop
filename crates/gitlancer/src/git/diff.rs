use crate::domain::refs::CommitId;
use crate::domain::worktree::WorktreeHandle;
use crate::error::{GitExecError, GitlancerError};
use crate::exec::command::{GitCommand, GitIntent};
use crate::exec::env::GitEnv;
use crate::exec::runner::GitRunner;
use crate::git::Git;
use std::sync::atomic::{AtomicU64, Ordering};

static ISOLATED_GIT_DIR_SEQUENCE: AtomicU64 = AtomicU64::new(0);

const MAX_DIFF_BYTES: usize = 10 * 1024 * 1024;
const MAX_DIFF_STDERR_BYTES: usize = 1024 * 1024;

/// Carries the fixed baseline and worktree used to compute all task changes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffRequest<'a> {
    pub worktree: &'a WorktreeHandle,
    pub base_commit_id: &'a CommitId,
}

/// Returns a standard unified patch that frontend diff parsers can consume directly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffResponse {
    pub head_commit_id: CommitId,
    pub patch: String,
}

impl<R: GitRunner> Git<R> {
    /// Computes tracked and untracked changes without staging files or invoking clean filters.
    pub fn diff(&self, request: DiffRequest<'_>) -> Result<DiffResponse, GitlancerError> {
        let head_output = self.runner().run(&build_head_command(request.worktree))?;
        let head_commit_id = head_output.stdout.trim();
        if head_commit_id.is_empty() {
            return Err(crate::ParseError::MissingLine.into());
        }

        let tracked = self
            .runner()
            .run_bounded(
                &build_diff_command(&request),
                MAX_DIFF_BYTES,
                MAX_DIFF_STDERR_BYTES,
            )
            .map_err(map_bounded_diff_error)?
            .stdout;
        let mut patch = tracked;
        let untracked_output = self
            .runner()
            .run_bounded(
                &build_untracked_command(request.worktree),
                MAX_DIFF_BYTES,
                MAX_DIFF_STDERR_BYTES,
            )
            .map_err(map_bounded_diff_error)?;
        let isolated_git_dir = isolated_git_dir();

        for path in untracked_output
            .stdout
            .split('\0')
            .filter(|path| !path.is_empty())
        {
            let separator_bytes = usize::from(!patch.is_empty() && !patch.ends_with('\n'));
            let remaining = MAX_DIFF_BYTES.saturating_sub(patch.len() + separator_bytes);
            if remaining == 0 {
                return Err(diff_too_large());
            }
            let untracked_patch = run_untracked_diff(
                self.runner(),
                &build_untracked_diff_command(request.worktree, path, &isolated_git_dir),
                remaining,
            )?;
            append_patch(&mut patch, &untracked_patch);
        }

        Ok(DiffResponse {
            head_commit_id: CommitId::new(head_commit_id),
            patch,
        })
    }
}

/// Generates a process-unique nonexistent Git directory so no-index ignores repository filters.
fn isolated_git_dir() -> std::path::PathBuf {
    let sequence = ISOLATED_GIT_DIR_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "ora-no-index-git-dir-{}-{sequence}",
        std::process::id()
    ))
}

/// Accepts exit code one because `git diff --no-index` uses it to report that files differ.
fn run_untracked_diff<R: GitRunner>(
    runner: &R,
    command: &GitCommand,
    max_stdout_bytes: usize,
) -> Result<String, GitlancerError> {
    match runner.run_bounded(command, max_stdout_bytes, MAX_DIFF_STDERR_BYTES) {
        Ok(output) => Ok(output.stdout),
        Err(GitExecError::NonZeroExit {
            code: Some(1),
            stdout,
            ..
        }) => Ok(stdout),
        Err(error) => Err(map_bounded_diff_error(error)),
    }
}

/// Converts bounded runner failures into the public diff-size error when appropriate.
fn map_bounded_diff_error(error: GitExecError) -> GitlancerError {
    match error {
        GitExecError::OutputTooLarge {
            stream: "stdout", ..
        } => diff_too_large(),
        error => GitlancerError::Exec(error),
    }
}

/// Builds the stable size error without pretending the discarded byte count is known exactly.
fn diff_too_large() -> GitlancerError {
    GitlancerError::DiffTooLarge {
        byte_count: MAX_DIFF_BYTES + 1,
        max_byte_count: MAX_DIFF_BYTES,
    }
}

/// Adds a file patch while preserving exactly one separator between patch streams.
fn append_patch(patch: &mut String, addition: &str) {
    if addition.is_empty() {
        return;
    }
    if !patch.is_empty() && !patch.ends_with('\n') {
        patch.push('\n');
    }
    patch.push_str(addition);
}

/// Builds the HEAD lookup command so uncommitted changes still carry their branch revision.
fn build_head_command(worktree: &WorktreeHandle) -> GitCommand {
    command(worktree, vec!["rev-parse", "HEAD"])
}

/// Builds the tracked-file comparison without external diff or text-conversion processes.
pub fn build_diff_command(request: &DiffRequest<'_>) -> GitCommand {
    command(
        request.worktree,
        vec![
            "diff",
            "--no-color",
            "--no-ext-diff",
            "--no-textconv",
            "--find-renames",
            "--unified=3",
            request.base_commit_id.as_str(),
            "--",
        ],
    )
}

/// Lists ignored-aware untracked paths in a machine-readable representation.
fn build_untracked_command(worktree: &WorktreeHandle) -> GitCommand {
    command(
        worktree,
        vec!["ls-files", "--others", "--exclude-standard", "-z"],
    )
}

/// Lets Git render one untracked file with correct quoting, modes, symlinks, and binary markers.
fn build_untracked_diff_command(
    worktree: &WorktreeHandle,
    path: &str,
    isolated_git_dir: &std::path::Path,
) -> GitCommand {
    GitCommand::new(
        worktree.worktree_root().as_path().to_path_buf(),
        vec![
            "diff",
            "--no-index",
            "--no-color",
            "--no-ext-diff",
            "--no-textconv",
            "--unified=3",
            "--",
            "/dev/null",
            path,
        ]
        .into_iter()
        .map(str::to_string)
        .collect(),
        GitEnv::default().with_variable("GIT_DIR", isolated_git_dir.to_string_lossy().into_owned()),
        GitIntent::ReadOnly,
    )
}

/// Creates a read-only Git command from borrowed arguments.
fn command(worktree: &WorktreeHandle, args: Vec<&str>) -> GitCommand {
    GitCommand::new(
        worktree.worktree_root().as_path().to_path_buf(),
        args.into_iter().map(str::to_string).collect(),
        GitEnv::default(),
        GitIntent::ReadOnly,
    )
}

#[cfg(test)]
mod tests {
    use super::{DiffRequest, build_diff_command, build_untracked_diff_command};
    use crate::{CommitId, GitDir, RepoRoot, WorktreeHandle, WorktreeKind, WorktreeRoot};
    use pretty_assertions::assert_eq;

    /// Verifies tracked diffs disable executable filters and emit parser-friendly binary markers.
    #[test]
    fn builds_task_diff_command() {
        let worktree = test_worktree();
        let base_commit_id = CommitId::new("base-commit");
        let command = build_diff_command(&DiffRequest {
            worktree: &worktree,
            base_commit_id: &base_commit_id,
        });

        assert_eq!(
            command.args,
            vec![
                "diff",
                "--no-color",
                "--no-ext-diff",
                "--no-textconv",
                "--find-renames",
                "--unified=3",
                "base-commit",
                "--",
            ]
        );
    }

    /// Verifies untracked files use Git's no-index renderer without clean or textconv filters.
    #[test]
    fn builds_untracked_file_diff_command() {
        let worktree = test_worktree();

        assert_eq!(
            build_untracked_diff_command(
                &worktree,
                "space name.bin",
                std::path::Path::new("/tmp/missing-git-dir"),
            )
            .args,
            vec![
                "diff",
                "--no-index",
                "--no-color",
                "--no-ext-diff",
                "--no-textconv",
                "--unified=3",
                "--",
                "/dev/null",
                "space name.bin",
            ]
        );
    }

    /// Builds a linked worktree fixture without touching the filesystem.
    fn test_worktree() -> WorktreeHandle {
        WorktreeHandle::new(
            RepoRoot::new("/repo"),
            WorktreeRoot::new("/repo/worktrees/task"),
            GitDir::new("/repo/.git/worktrees/task"),
            WorktreeKind::Linked {
                name: "task".to_string(),
            },
        )
    }
}
