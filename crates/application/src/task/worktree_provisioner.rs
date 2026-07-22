use super::{
    CreateTaskWorktreeRequest, CreateTaskWorktreeResponse, DeleteTaskWorktreeRequest,
    TaskWorktreeDeletionMode, TaskWorktreeProvisioner, TaskWorktreeProvisionerError,
    VerifyTaskWorktreeRequest,
};
use gitlancer::git::branch::ListBranchesRequest;
use gitlancer::git::worktree::{
    CreateWorktreeRequest as GitCreateWorktreeRequest,
    DeleteWorktreeRequest as GitDeleteWorktreeRequest, FindWorktreeRootRequest,
    PruneWorktreesRequest, WorktreeDeletionMode as GitWorktreeDeletionMode,
};
use gitlancer::{
    BranchName, CliGitRunner, Git, RepoRoot, Repository, WorktreeIdentityToken, WorktreeRoot,
};
use std::fs;
use std::path::{Path, PathBuf};

/// Provisions and removes application-owned task worktrees through the shared Git runtime.
#[derive(Clone, Debug)]
pub struct GitTaskWorktreeProvisioner {
    git: Git<CliGitRunner>,
    project_path: PathBuf,
}

impl GitTaskWorktreeProvisioner {
    /// Builds a Git-backed task worktree provisioner for one configured project repository.
    pub fn new(project_root: PathBuf) -> Self {
        Self {
            git: Git::new(CliGitRunner::default()),
            project_path: project_root,
        }
    }

    /// Discovers the canonical owning repository before trusting configured filesystem input.
    fn discover_repository(&self) -> Result<Repository, TaskWorktreeProvisionerError> {
        self.git
            .discover_repository(RepoRoot::new(&self.project_path))
            .map_err(|_| {
                TaskWorktreeProvisionerError::OperationFailed(
                    "failed to discover task repository".to_string(),
                )
            })
    }
}

impl TaskWorktreeProvisioner for GitTaskWorktreeProvisioner {
    /// Checks local refs so orphaned task branches also participate in id collision avoidance.
    fn task_branch_exists(&self, branch_name: &str) -> Result<bool, TaskWorktreeProvisionerError> {
        let repository = self.discover_repository()?;
        self.git
            .list_branches(ListBranchesRequest {
                repository: &repository,
            })
            .map(|response| {
                response
                    .branches
                    .iter()
                    .any(|branch| branch.as_str() == branch_name)
            })
            .map_err(|_| {
                TaskWorktreeProvisionerError::OperationFailed(
                    "failed to inspect task branches".to_string(),
                )
            })
    }

    /// Creates one linked worktree while keeping Git-specific diagnostics inside the application.
    fn create_task_worktree(
        &self,
        request: CreateTaskWorktreeRequest,
    ) -> Result<CreateTaskWorktreeResponse, TaskWorktreeProvisionerError> {
        let repository = self.discover_repository()?;
        create_parent_directory(&request.worktree_path)?;
        let branch_name = BranchName::new(request.branch_name).map_err(|_| {
            TaskWorktreeProvisionerError::OperationFailed("task branch name is invalid".to_string())
        })?;
        let identity_token =
            WorktreeIdentityToken::new(request.worktree_id.to_string()).map_err(|_| {
                TaskWorktreeProvisionerError::OperationFailed(
                    "task worktree identity is invalid".to_string(),
                )
            })?;
        self.git
            .create_worktree(GitCreateWorktreeRequest {
                repository: &repository,
                worktree_root: WorktreeRoot::new(&request.worktree_path),
                branch_name,
                identity_token,
            })
            .map(|response| CreateTaskWorktreeResponse {
                base_commit_id: response.head_commit_id.as_str().to_string(),
            })
            .map_err(|_| {
                TaskWorktreeProvisionerError::OperationFailed(
                    "failed to create linked worktree".to_string(),
                )
            })
    }

    /// Confirms that startup recovery is about to activate the checkout owned by the persisted row.
    fn verify_task_worktree(
        &self,
        request: VerifyTaskWorktreeRequest,
    ) -> Result<(), TaskWorktreeProvisionerError> {
        let repository = self.discover_repository()?;
        let worktree = self
            .git
            .find_worktree_root(FindWorktreeRootRequest {
                repository: &repository,
                worktree_root: &request.worktree_path,
            })
            .map_err(|_| {
                TaskWorktreeProvisionerError::OperationFailed(
                    "failed to verify linked worktree".to_string(),
                )
            })?;

        verify_worktree_identity(
            &worktree,
            &request.expected_worktree_id,
            &request.worktree_path,
            &request.expected_branch_name,
        )
    }

    /// Deletes one linked worktree while keeping Git-specific diagnostics inside the application.
    fn delete_task_worktree(
        &self,
        request: DeleteTaskWorktreeRequest,
    ) -> Result<(), TaskWorktreeProvisionerError> {
        let repository = self.discover_repository()?;
        let worktree = match self.git.find_worktree_root(FindWorktreeRootRequest {
            repository: &repository,
            worktree_root: &request.worktree_path,
        }) {
            Ok(worktree) => worktree,
            Err(gitlancer::GitlancerError::Domain(gitlancer::DomainError::NotAWorktree(_))) => {
                // Git may still retain a prunable administration entry after the checkout path
                // disappears, so an idempotent delete also reconciles that stale metadata.
                self.git
                    .prune_worktrees(PruneWorktreesRequest {
                        repository: &repository,
                    })
                    .map_err(|_| {
                        TaskWorktreeProvisionerError::OperationFailed(
                            "failed to prune stale linked worktree metadata".to_string(),
                        )
                    })?;
                return Ok(());
            }
            Err(_) => {
                return Err(TaskWorktreeProvisionerError::OperationFailed(
                    "failed to delete linked worktree".to_string(),
                ));
            }
        };
        verify_worktree_identity(
            &worktree,
            &request.expected_worktree_id,
            &request.worktree_path,
            &request.expected_branch_name,
        )?;
        let mode = match request.mode {
            TaskWorktreeDeletionMode::Force => GitWorktreeDeletionMode::Force,
        };

        self.git
            .delete_worktree(GitDeleteWorktreeRequest {
                repository: &repository,
                worktree: &worktree,
                mode,
            })
            .map(|_| ())
            .map_err(|_| {
                TaskWorktreeProvisionerError::OperationFailed(
                    "failed to delete linked worktree".to_string(),
                )
            })
    }
}

/// Validates the durable marker and branch shared by recovery and destructive cleanup.
fn verify_worktree_identity(
    worktree: &gitlancer::WorktreeHandle,
    expected_worktree_id: &ora_domain::WorktreeId,
    worktree_path: &Path,
    expected_branch_name: &str,
) -> Result<(), TaskWorktreeProvisionerError> {
    let expected_identity_token = WorktreeIdentityToken::new(expected_worktree_id.to_string())
        .map_err(|_| {
            TaskWorktreeProvisionerError::OperationFailed(
                "task worktree identity is invalid".to_string(),
            )
        })?;
    if worktree.identity_token() != Some(&expected_identity_token) {
        return Err(TaskWorktreeProvisionerError::OperationFailed(format!(
            "refusing to use worktree at {worktree_path:?}: expected identity {:?}, found {:?}",
            expected_identity_token.as_str(),
            worktree.identity_token().map(WorktreeIdentityToken::as_str),
        )));
    }
    if worktree.branch().map(BranchName::as_str) != Some(expected_branch_name) {
        return Err(TaskWorktreeProvisionerError::OperationFailed(format!(
            "refusing to use worktree at {worktree_path:?}: expected branch {expected_branch_name:?}, found {:?}",
            worktree.branch().map(BranchName::as_str),
        )));
    }

    Ok(())
}

/// Creates the parent eagerly because Git expects the worktree path's ancestor to exist.
fn create_parent_directory(worktree_path: &Path) -> Result<(), TaskWorktreeProvisionerError> {
    match worktree_path.parent() {
        Some(parent_directory) => fs::create_dir_all(parent_directory).map_err(|_| {
            TaskWorktreeProvisionerError::OperationFailed(
                "failed to create linked worktree".to_string(),
            )
        }),
        None => Ok(()),
    }
}
