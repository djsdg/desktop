use crate::domain::refs::CommitId;
use crate::domain::worktree::WorktreeHandle;
use crate::error::{GitExecError, GitlancerError};
use crate::exec::command::{GitCommand, GitIntent};
use crate::exec::env::GitEnv;
use crate::exec::runner::GitRunner;
use crate::git::Git;
use std::sync::atomic::{AtomicU64, Ordering};

static TEMPORARY_GIT_PATH_SEQUENCE: AtomicU64 = AtomicU64::new(0);

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
        let head_commit_id = crate::parse::commit::parse_commit_id(&head_output.stdout)?;

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
        ensure_diff_size(&patch)?;
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
            let untracked_patch = if untracked_patch.is_empty() {
                run_empty_untracked_diff(self.runner(), request.worktree, path, remaining)?
            } else {
                untracked_patch
            };
            append_patch(&mut patch, &untracked_patch);
            ensure_diff_size(&patch)?;
        }

        let verified_head_output = self.runner().run(&build_head_command(request.worktree))?;
        let verified_head_commit_id =
            crate::parse::commit::parse_commit_id(&verified_head_output.stdout)?;
        if head_commit_id != verified_head_commit_id {
            return Err(GitlancerError::DiffHeadChanged {
                before_commit_id: head_commit_id.as_str().to_string(),
                after_commit_id: verified_head_commit_id.as_str().to_string(),
            });
        }

        Ok(DiffResponse {
            head_commit_id,
            patch,
        })
    }
}

/// Generates a process-unique nonexistent Git directory so no-index ignores repository filters.
fn isolated_git_dir() -> std::path::PathBuf {
    let sequence = TEMPORARY_GIT_PATH_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "ora-no-index-git-dir-{}-{sequence}",
        std::process::id()
    ))
}

/// Owns a temporary Git index path and removes any index or lock file left by Git commands.
struct TemporaryIndex {
    path: std::path::PathBuf,
}

impl TemporaryIndex {
    /// Reserves a process-unique path without creating an invalid empty index file.
    fn new() -> Self {
        let sequence = TEMPORARY_GIT_PATH_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        Self {
            path: std::env::temp_dir().join(format!(
                "ora-empty-untracked-index-{}-{sequence}",
                std::process::id()
            )),
        }
    }

    /// Returns the path passed to Git through `GIT_INDEX_FILE`.
    fn path(&self) -> &std::path::Path {
        &self.path
    }
}

impl Drop for TemporaryIndex {
    /// Best-effort cleanup keeps diff failures from accumulating temporary index files.
    fn drop(&mut self) {
        let _remove_index_result = std::fs::remove_file(&self.path);
        let lock_path = self.path.with_extension("lock");
        let _remove_lock_result = std::fs::remove_file(lock_path);
    }
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

/// Uses an isolated intent-to-add index so Git emits metadata for an empty untracked file.
fn run_empty_untracked_diff<R: GitRunner>(
    runner: &R,
    worktree: &WorktreeHandle,
    path: &str,
    max_stdout_bytes: usize,
) -> Result<String, GitlancerError> {
    let temporary_index = TemporaryIndex::new();
    runner.run(&build_initialize_temporary_index_command(
        worktree,
        temporary_index.path(),
    ))?;
    runner.run(&build_intent_to_add_command(
        worktree,
        path,
        temporary_index.path(),
    ))?;
    runner
        .run_bounded(
            &build_empty_untracked_diff_command(worktree, path, temporary_index.path()),
            max_stdout_bytes,
            MAX_DIFF_STDERR_BYTES,
        )
        .map(|output| output.stdout)
        .map_err(map_bounded_diff_error)
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

/// Enforces the response budget again after UTF-8 normalization and patch composition.
fn ensure_diff_size(patch: &str) -> Result<(), GitlancerError> {
    if patch.len() > MAX_DIFF_BYTES {
        return Err(GitlancerError::DiffTooLarge {
            byte_count: patch.len(),
            max_byte_count: MAX_DIFF_BYTES,
        });
    }

    Ok(())
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
            "--default-prefix",
            "--src-prefix=a/",
            "--dst-prefix=b/",
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
            "--literal-pathspecs",
            "diff",
            "--no-index",
            "--no-color",
            "--no-ext-diff",
            "--no-textconv",
            "--default-prefix",
            "--src-prefix=a/",
            "--dst-prefix=b/",
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

/// Initializes a temporary index from HEAD without reading or changing the real worktree index.
fn build_initialize_temporary_index_command(
    worktree: &WorktreeHandle,
    temporary_index: &std::path::Path,
) -> GitCommand {
    command_with_index(
        worktree,
        vec!["read-tree", "HEAD"],
        temporary_index,
        GitIntent::Mutating,
    )
}

/// Records only intent-to-add metadata so Git can distinguish an empty file from `/dev/null`.
fn build_intent_to_add_command(
    worktree: &WorktreeHandle,
    path: &str,
    temporary_index: &std::path::Path,
) -> GitCommand {
    command_with_index(
        worktree,
        vec!["--literal-pathspecs", "add", "--intent-to-add", "--", path],
        temporary_index,
        GitIntent::Mutating,
    )
}

/// Renders an empty intent-to-add entry as a canonical new-file patch.
fn build_empty_untracked_diff_command(
    worktree: &WorktreeHandle,
    path: &str,
    temporary_index: &std::path::Path,
) -> GitCommand {
    command_with_index(
        worktree,
        vec![
            "--literal-pathspecs",
            "diff",
            "--no-color",
            "--no-ext-diff",
            "--no-textconv",
            "--default-prefix",
            "--src-prefix=a/",
            "--dst-prefix=b/",
            "--unified=3",
            "--",
            path,
        ],
        temporary_index,
        GitIntent::ReadOnly,
    )
}

/// Creates a Git command whose mutations are isolated to a disposable index file.
fn command_with_index(
    worktree: &WorktreeHandle,
    args: Vec<&str>,
    temporary_index: &std::path::Path,
    intent: GitIntent,
) -> GitCommand {
    GitCommand::new(
        worktree.worktree_root().as_path().to_path_buf(),
        args.into_iter().map(str::to_string).collect(),
        GitEnv::default().with_variable(
            "GIT_INDEX_FILE",
            temporary_index.to_string_lossy().into_owned(),
        ),
        intent,
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
    use std::cell::RefCell;
    use std::collections::VecDeque;

    use super::{
        DiffRequest, build_diff_command, build_empty_untracked_diff_command,
        build_initialize_temporary_index_command, build_intent_to_add_command,
        build_untracked_diff_command, ensure_diff_size,
    };
    use crate::{
        CommitId, Git, GitCommand, GitDir, GitExecError, GitOutput, GitRunner, GitlancerError,
        RepoRoot, WorktreeHandle, WorktreeKind, WorktreeRoot,
    };
    use pretty_assertions::assert_eq;

    const BASE_COMMIT_ID: &str = "0123456789abcdef0123456789abcdef01234567";
    const CHANGED_COMMIT_ID: &str = "89abcdef0123456789abcdef0123456789abcdef";

    /// Verifies the public patch budget applies to the final UTF-8 string, not only captured bytes.
    #[test]
    fn rejects_final_patch_that_exceeds_response_budget() {
        let oversized_patch = "x".repeat(super::MAX_DIFF_BYTES + 1);

        assert!(matches!(
            ensure_diff_size(&oversized_patch),
            Err(GitlancerError::DiffTooLarge {
                byte_count,
                max_byte_count,
            }) if byte_count == super::MAX_DIFF_BYTES + 1
                && max_byte_count == super::MAX_DIFF_BYTES
        ));
    }

    /// Returns scripted outputs in command order so multi-command diff behavior stays deterministic.
    struct ScriptedRunner {
        outputs: RefCell<VecDeque<GitOutput>>,
    }

    impl GitRunner for ScriptedRunner {
        /// Supplies one prepared output for each Git command issued by the use case.
        fn run(&self, _command: &GitCommand) -> Result<GitOutput, GitExecError> {
            self.outputs
                .borrow_mut()
                .pop_front()
                .ok_or_else(|| GitExecError::OutputReadFailed {
                    stream: "scripted output",
                    source: std::io::Error::other("missing scripted Git output"),
                })
        }
    }

    /// Verifies tracked diffs disable executable filters and emit parser-friendly binary markers.
    #[test]
    fn builds_task_diff_command() {
        let worktree = test_worktree();
        let base_commit_id = CommitId::new(BASE_COMMIT_ID).expect("test commit id should be valid");
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
                "--default-prefix",
                "--src-prefix=a/",
                "--dst-prefix=b/",
                "--unified=3",
                BASE_COMMIT_ID,
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
                "--literal-pathspecs",
                "diff",
                "--no-index",
                "--no-color",
                "--no-ext-diff",
                "--no-textconv",
                "--default-prefix",
                "--src-prefix=a/",
                "--dst-prefix=b/",
                "--unified=3",
                "--",
                "/dev/null",
                "space name.bin",
            ]
        );
    }

    /// Verifies empty-file fallback commands share one isolated index and never touch the real one.
    #[test]
    fn builds_empty_untracked_file_commands() {
        let worktree = test_worktree();
        let temporary_index = std::path::Path::new("/tmp/empty-file-index");

        let commands = [
            build_initialize_temporary_index_command(&worktree, temporary_index),
            build_intent_to_add_command(&worktree, "empty.txt", temporary_index),
            build_empty_untracked_diff_command(&worktree, "empty.txt", temporary_index),
        ];

        assert_eq!(
            commands.map(|command| (command.args, command.env.variables)),
            [
                (
                    vec!["read-tree".to_string(), "HEAD".to_string()],
                    [(
                        "GIT_INDEX_FILE".to_string(),
                        "/tmp/empty-file-index".to_string(),
                    ),]
                    .into(),
                ),
                (
                    vec![
                        "--literal-pathspecs".to_string(),
                        "add".to_string(),
                        "--intent-to-add".to_string(),
                        "--".to_string(),
                        "empty.txt".to_string(),
                    ],
                    [(
                        "GIT_INDEX_FILE".to_string(),
                        "/tmp/empty-file-index".to_string(),
                    ),]
                    .into(),
                ),
                (
                    vec![
                        "--literal-pathspecs".to_string(),
                        "diff".to_string(),
                        "--no-color".to_string(),
                        "--no-ext-diff".to_string(),
                        "--no-textconv".to_string(),
                        "--default-prefix".to_string(),
                        "--src-prefix=a/".to_string(),
                        "--dst-prefix=b/".to_string(),
                        "--unified=3".to_string(),
                        "--".to_string(),
                        "empty.txt".to_string(),
                    ],
                    [(
                        "GIT_INDEX_FILE".to_string(),
                        "/tmp/empty-file-index".to_string(),
                    ),]
                    .into(),
                ),
            ]
        );
    }

    /// Rejects a patch whose HEAD changed while its tracked and untracked sections were generated.
    #[test]
    fn rejects_diff_when_head_changes_during_snapshot() {
        let worktree = test_worktree();
        let base_commit_id = CommitId::new(BASE_COMMIT_ID).expect("test commit id should be valid");
        let git = Git::new(ScriptedRunner {
            outputs: RefCell::new(VecDeque::from(vec![
                GitOutput::new(Some(0), format!("{BASE_COMMIT_ID}\n"), String::new(), 0),
                GitOutput::new(Some(0), "tracked patch\n".to_string(), String::new(), 0),
                GitOutput::new(Some(0), String::new(), String::new(), 0),
                GitOutput::new(Some(0), format!("{CHANGED_COMMIT_ID}\n"), String::new(), 0),
            ])),
        });

        let error = git
            .diff(DiffRequest {
                worktree: &worktree,
                base_commit_id: &base_commit_id,
            })
            .expect_err("a moving HEAD must not produce a mixed diff snapshot");

        assert!(matches!(
            error,
            GitlancerError::DiffHeadChanged {
                before_commit_id,
                after_commit_id,
            } if before_commit_id == BASE_COMMIT_ID && after_commit_id == CHANGED_COMMIT_ID
        ));
    }

    /// Builds a linked worktree fixture without touching the filesystem.
    fn test_worktree() -> WorktreeHandle {
        WorktreeHandle::discovered(
            RepoRoot::new("/repo"),
            WorktreeRoot::new("/repo/worktrees/task"),
            GitDir::new("/repo/.git/worktrees/task"),
            WorktreeKind::Linked {
                name: "task".to_string(),
            },
            CommitId::new(BASE_COMMIT_ID).expect("test commit id should be valid"),
            Some(crate::BranchName::new("ora/task").expect("test branch should be valid")),
            None,
        )
    }
}
