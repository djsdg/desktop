use std::path::{Component, Path, PathBuf};

use crate::domain::paths::{GitDir, RepoRelativePath, RepoRoot, WorktreeRoot};
use crate::domain::refs::{BranchName, CommitId, WorktreeIdentityToken};
use crate::error::DomainError;

/// Distinguishes the main checkout from linked worktrees because they have different lifecycle semantics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorktreeKind {
    Main,
    Linked { name: String },
}

/// Represents one executable worktree context that belongs to a repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeHandle {
    repo_root: RepoRoot,
    worktree_root: WorktreeRoot,
    git_dir: GitDir,
    kind: WorktreeKind,
    head_commit_id: CommitId,
    branch: Option<BranchName>,
    locked_reason: Option<String>,
    identity_token: Option<WorktreeIdentityToken>,
}

impl WorktreeHandle {
    /// Creates a trusted handle exclusively from metadata discovered through Git.
    pub(crate) fn discovered(
        repo_root: RepoRoot,
        worktree_root: WorktreeRoot,
        git_dir: GitDir,
        kind: WorktreeKind,
        head_commit_id: CommitId,
        branch: Option<BranchName>,
        locked_reason: Option<String>,
    ) -> Self {
        Self {
            repo_root,
            worktree_root,
            git_dir,
            kind,
            head_commit_id,
            branch,
            locked_reason,
            identity_token: None,
        }
    }

    /// Returns the repository root that owns this worktree.
    pub fn repo_root(&self) -> &RepoRoot {
        &self.repo_root
    }

    /// Returns the checkout root where worktree-scoped Git commands should execute.
    pub fn worktree_root(&self) -> &WorktreeRoot {
        &self.worktree_root
    }

    /// Returns the gitdir backing this worktree so linked worktrees can be handled explicitly.
    pub fn git_dir(&self) -> &GitDir {
        &self.git_dir
    }

    /// Returns the worktree kind so callers can branch on main versus linked behavior deliberately.
    pub fn kind(&self) -> &WorktreeKind {
        &self.kind
    }

    /// Returns the commit checked out when Git discovered this worktree.
    pub fn head_commit_id(&self) -> &CommitId {
        &self.head_commit_id
    }

    /// Returns the checked-out local branch, or `None` for a detached worktree.
    pub fn branch(&self) -> Option<&BranchName> {
        self.branch.as_ref()
    }

    /// Returns Git's lock reason when the worktree is protected from movement or removal.
    pub fn locked_reason(&self) -> Option<&str> {
        self.locked_reason.as_deref()
    }

    /// Returns the durable marker for an Ora-managed worktree, if one was discovered.
    pub fn identity_token(&self) -> Option<&WorktreeIdentityToken> {
        self.identity_token.as_ref()
    }

    /// Attaches the marker that was durably written after linked-worktree creation.
    pub(crate) fn with_identity_token(mut self, identity_token: WorktreeIdentityToken) -> Self {
        self.identity_token = Some(identity_token);
        self
    }

    /// Attaches the optional marker observed while discovering an existing checkout.
    pub(crate) fn with_discovered_identity_token(
        mut self,
        identity_token: Option<WorktreeIdentityToken>,
    ) -> Self {
        self.identity_token = identity_token;
        self
    }

    /// Resolves a caller path into a repo-relative path while preventing traversal outside this worktree.
    pub fn resolve_repo_relative_path(
        &self,
        path: impl AsRef<Path>,
    ) -> Result<RepoRelativePath, DomainError> {
        let candidate = path.as_ref();
        let worktree_root = normalize_absolute_path(self.worktree_root.as_path());

        if candidate.is_absolute() {
            let normalized = normalize_absolute_path(candidate);
            let relative = normalized.strip_prefix(&worktree_root).map_err(|_| {
                DomainError::PathOutsideWorktree {
                    path: normalized.clone(),
                    worktree: worktree_root.clone(),
                }
            })?;

            return RepoRelativePath::new(relative);
        }

        let normalized =
            normalize_relative_path(candidate).ok_or_else(|| DomainError::PathOutsideWorktree {
                path: candidate.to_path_buf(),
                worktree: worktree_root.clone(),
            })?;

        RepoRelativePath::new(normalized)
    }
}

/// Normalizes an absolute path lexically so containment checks do not depend on filesystem existence.
fn normalize_absolute_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();

    for component in path.components() {
        match component {
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            Component::RootDir => normalized.push(component.as_os_str()),
            Component::CurDir => {}
            // Absolute paths cannot traverse above root, so extra `..` segments are ignored there.
            Component::ParentDir => {
                let _ = normalized.pop();
            }
            Component::Normal(part) => normalized.push(part),
        }
    }

    normalized
}

/// Normalizes a relative path and rejects paths whose `..` segments would escape the worktree root.
fn normalize_relative_path(path: &Path) -> Option<PathBuf> {
    let mut normalized = PathBuf::new();
    let mut depth = 0usize;

    for component in path.components() {
        match component {
            Component::Prefix(_) | Component::RootDir => return None,
            Component::CurDir => {}
            Component::ParentDir => {
                if depth == 0 {
                    return None;
                }

                let popped = normalized.pop();
                if popped {
                    depth -= 1;
                }
            }
            Component::Normal(part) => {
                normalized.push(part);
                depth += 1;
            }
        }
    }

    Some(normalized)
}
