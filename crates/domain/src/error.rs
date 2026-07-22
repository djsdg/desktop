use std::path::PathBuf;

use thiserror::Error;

/// Enumerates domain-model conversion failures that adapters must handle explicitly.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum DomainModelError {
    #[error("managed worktree root must not be empty")]
    EmptyManagedWorktreeRoot,
    #[error("managed worktree root must be absolute: {0:?}")]
    RelativeManagedWorktreeRoot(PathBuf),
    #[error("managed worktree branch must not be empty")]
    EmptyManagedWorktreeBranch,
    #[error("worktree baseline commit must not be empty")]
    EmptyWorktreeBaseline,
    #[error("invalid project work context surface value: {0}")]
    InvalidProjectWorkContextSurface(String),
    #[error("invalid task status value: {0}")]
    InvalidTaskStatus(i64),
    #[error("invalid worktree lifecycle value: {0}")]
    InvalidWorktreeLifecycle(i64),
    #[error("invalid virtual entry kind value: {0}")]
    InvalidVirtualEntryKind(i64),
    #[error("invalid session status value: {0}")]
    InvalidSessionStatus(i64),
    #[error("skill name must not be blank")]
    EmptySkillName,
    #[error("agent definition name must not be blank")]
    EmptyAgentDefinitionName,
}
