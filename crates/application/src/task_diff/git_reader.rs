use super::ports::{ReadTaskDiffRequest, TaskDiffReader, TaskDiffReaderError, TaskDiffSnapshot};
use gitlancer::git::diff::{DiffRequest, DiffResponse};
use gitlancer::git::worktree::FindWorktreeRootRequest;
use gitlancer::{CliGitRunner, CommitId, Git, RepoRoot, WorktreeIdentityToken};
use std::path::PathBuf;

/// Reads task-scoped unified diffs through the shared Gitlancer runtime.
#[derive(Clone, Debug)]
pub struct GitTaskDiffReader {
    git: Git<CliGitRunner>,
    project_path: PathBuf,
}

impl GitTaskDiffReader {
    /// Builds a Git-backed reader for one configured project repository.
    pub fn new(project_root: PathBuf) -> Self {
        Self {
            git: Git::new(CliGitRunner::default()),
            project_path: project_root,
        }
    }
}

impl TaskDiffReader for GitTaskDiffReader {
    /// Resolves the backend-owned worktree before computing its fixed-baseline diff.
    fn read_task_diff(
        &self,
        request: ReadTaskDiffRequest,
    ) -> Result<TaskDiffSnapshot, TaskDiffReaderError> {
        let repository = self
            .git
            .discover_repository(RepoRoot::new(&self.project_path))
            .map_err(task_diff_operation_error)?;
        let worktree = self
            .git
            .find_worktree_root(FindWorktreeRootRequest {
                repository: &repository,
                worktree_root: &request.worktree_path,
            })
            .map_err(task_diff_operation_error)?;
        let expected_identity_token =
            WorktreeIdentityToken::new(request.expected_worktree_id.to_string())
                .map_err(|error| task_diff_operation_error(error.into()))?;
        if worktree.identity_token() != Some(&expected_identity_token) {
            return Err(TaskDiffReaderError::OperationFailed(format!(
                "refusing to read task diff at {:?}: expected identity {:?}, found {:?}",
                request.worktree_path,
                expected_identity_token.as_str(),
                worktree.identity_token().map(WorktreeIdentityToken::as_str),
            )));
        }
        if worktree.branch().map(gitlancer::BranchName::as_str)
            != Some(request.expected_branch_name.as_str())
        {
            return Err(TaskDiffReaderError::OperationFailed(format!(
                "refusing to read task diff at {:?}: expected branch {:?}, found {:?}",
                request.worktree_path,
                request.expected_branch_name,
                worktree.branch().map(gitlancer::BranchName::as_str),
            )));
        }
        let base_commit_id = CommitId::new(request.base_commit_id)
            .map_err(|error| task_diff_operation_error(error.into()))?;

        self.git
            .diff(DiffRequest {
                worktree: &worktree,
                base_commit_id: &base_commit_id,
            })
            .map(map_diff_response)
            .map_err(task_diff_operation_error)
    }
}

/// Maps Gitlancer's internal response into the application-owned snapshot.
fn map_diff_response(response: DiffResponse) -> TaskDiffSnapshot {
    TaskDiffSnapshot {
        head_commit_id: response.head_commit_id.as_str().to_string(),
        patch: response.patch,
    }
}

/// Hides Git and filesystem diagnostics behind a stable application-port error.
fn task_diff_operation_error(error: gitlancer::GitlancerError) -> TaskDiffReaderError {
    match error {
        gitlancer::GitlancerError::DiffTooLarge {
            byte_count,
            max_byte_count,
        } => TaskDiffReaderError::TooLarge {
            byte_count,
            max_byte_count,
        },
        error => TaskDiffReaderError::OperationFailed(error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::{GitTaskDiffReader, ReadTaskDiffRequest, TaskDiffReader, TaskDiffReaderError};

    /// Owns an isolated repository and removes it after each Git-backed reader test.
    struct TestRepository {
        root: PathBuf,
        repository: PathBuf,
    }

    impl TestRepository {
        /// Creates a committed main worktree whose nested missing paths would previously match by prefix.
        fn new() -> Self {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system time should be available")
                .as_nanos();
            let root = std::env::temp_dir().join(format!(
                "ora-task-diff-reader-{}-{unique}",
                std::process::id()
            ));
            let repository = root.join("repo");
            fs::create_dir_all(&repository).expect("create test repository");
            run_git(&repository, &["init", "--initial-branch=main", "."]);
            run_git(&repository, &["config", "user.name", "ora-test"]);
            run_git(
                &repository,
                &["config", "user.email", "ora-test@example.com"],
            );
            fs::write(repository.join("README.md"), "test\n").expect("write repository file");
            run_git(&repository, &["add", "README.md"]);
            run_git(&repository, &["commit", "--no-gpg-sign", "-m", "initial"]);

            Self { root, repository }
        }

        /// Reads the full current commit identifier used as the immutable test baseline.
        fn head_commit_id(&self) -> String {
            run_git(&self.repository, &["rev-parse", "HEAD"])
                .trim()
                .to_string()
        }
    }

    impl Drop for TestRepository {
        /// Removes only the process-unique test sandbox.
        fn drop(&mut self) {
            let _cleanup_result = fs::remove_dir_all(&self.root);
        }
    }

    /// Verifies a missing task checkout nested under the main repository cannot fall back to the main worktree.
    #[test]
    fn rejects_missing_nested_task_worktree() {
        let repository = TestRepository::new();
        let reader = GitTaskDiffReader::new(repository.repository.clone());

        let result = reader.read_task_diff(ReadTaskDiffRequest {
            expected_worktree_id: ora_domain::WorktreeId::new("worktree-1"),
            worktree_path: repository.repository.join("worktrees").join("missing"),
            expected_branch_name: "main".to_string(),
            base_commit_id: repository.head_commit_id(),
        });

        assert!(matches!(
            result,
            Err(TaskDiffReaderError::OperationFailed(_))
        ));
    }

    /// Runs Git without prompts and returns UTF-8 stdout for one repository fixture operation.
    fn run_git(cwd: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .current_dir(cwd)
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("LANG", "C")
            .args(args)
            .output()
            .expect("spawn Git fixture command");
        assert!(
            output.status.success(),
            "Git fixture command failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).expect("Git fixture stdout should be UTF-8")
    }
}
