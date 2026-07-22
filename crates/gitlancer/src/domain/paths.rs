use std::path::{Path, PathBuf};

use crate::error::DomainError;

/// Identifies the canonical root directory of a Git repository.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RepoRoot(PathBuf);

impl RepoRoot {
    /// Creates a repository root wrapper once a caller has already validated the path.
    pub fn new(path: impl AsRef<Path>) -> Self {
        Self(path.as_ref().to_path_buf())
    }

    /// Exposes the filesystem path for command construction and diagnostics.
    pub fn as_path(&self) -> &Path {
        &self.0
    }
}

/// Identifies the filesystem root where one concrete worktree is checked out.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct WorktreeRoot(PathBuf);

impl WorktreeRoot {
    /// Creates a worktree root wrapper once a caller has already validated the path.
    pub fn new(path: impl AsRef<Path>) -> Self {
        Self(path.as_ref().to_path_buf())
    }

    /// Exposes the filesystem path for command construction and diagnostics.
    pub fn as_path(&self) -> &Path {
        &self.0
    }
}

/// Identifies the gitdir associated with one concrete worktree.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GitDir(PathBuf);

impl GitDir {
    /// Creates a gitdir wrapper once a caller has already resolved indirection such as linked worktrees.
    pub fn new(path: impl AsRef<Path>) -> Self {
        Self(path.as_ref().to_path_buf())
    }

    /// Exposes the filesystem path for command construction and diagnostics.
    pub fn as_path(&self) -> &Path {
        &self.0
    }
}

/// Identifies a path relative to the worktree root so callers cannot accidentally cross repository boundaries.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RepoRelativePath(PathBuf);

impl RepoRelativePath {
    /// Creates a repository-relative path while rejecting absolute and parent-traversing inputs.
    pub(crate) fn new(path: impl AsRef<Path>) -> Result<Self, DomainError> {
        let path = path.as_ref();
        if path.is_absolute()
            || path.components().any(|component| {
                matches!(
                    component,
                    std::path::Component::ParentDir
                        | std::path::Component::RootDir
                        | std::path::Component::Prefix(_)
                )
            })
        {
            return Err(DomainError::InvalidRepoRelativePath(path.to_path_buf()));
        }

        Ok(Self(path.to_path_buf()))
    }

    /// Exposes the repo-relative path for command assembly.
    pub fn as_path(&self) -> &Path {
        &self.0
    }
}
