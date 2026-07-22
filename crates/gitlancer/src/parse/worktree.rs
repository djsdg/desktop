use std::path::PathBuf;

use crate::domain::paths::{RepoRoot, WorktreeRoot};
use crate::domain::refs::{BranchName, CommitId};
use crate::error::ParseError;

/// Represents one complete machine-readable worktree record before runtime identity resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParsedWorktree {
    /// Represents a bare repository entry, which has no checked-out HEAD or branch state.
    Bare { repository_root: RepoRoot },
    /// Represents an operational main or linked checkout with a discoverable Git identity.
    Checkout {
        worktree_root: WorktreeRoot,
        head_commit_id: CommitId,
        branch: Option<BranchName>,
        detached: bool,
        locked_reason: Option<String>,
        prunable_reason: Option<String>,
    },
}

#[derive(Debug, Default)]
struct WorktreeRecordBuilder {
    worktree_root: Option<PathBuf>,
    head_commit_id: Option<String>,
    branch: Option<String>,
    detached: bool,
    bare: bool,
    locked_reason: Option<String>,
    prunable_reason: Option<String>,
}

impl WorktreeRecordBuilder {
    /// Converts one completed porcelain record into the typed parser result.
    fn finish(self) -> Result<ParsedWorktree, ParseError> {
        let worktree_root = self.worktree_root.ok_or(ParseError::InvalidWorktreeList)?;
        if self.bare {
            if self.head_commit_id.is_some() || self.branch.is_some() || self.detached {
                return Err(ParseError::InvalidWorktreeList);
            }

            return Ok(ParsedWorktree::Bare {
                repository_root: RepoRoot::new(worktree_root),
            });
        }

        let head_commit_id = self.head_commit_id.ok_or(ParseError::InvalidWorktreeList)?;

        let head_commit_id =
            CommitId::new(head_commit_id).map_err(|_| ParseError::InvalidWorktreeList)?;
        let branch = self
            .branch
            .map(BranchName::new)
            .transpose()
            .map_err(|_| ParseError::InvalidWorktreeList)?;

        if self.detached == branch.is_some() {
            return Err(ParseError::InvalidWorktreeList);
        }

        Ok(ParsedWorktree::Checkout {
            worktree_root: WorktreeRoot::new(worktree_root),
            head_commit_id,
            branch,
            detached: self.detached,
            locked_reason: self.locked_reason,
            prunable_reason: self.prunable_reason,
        })
    }
}

/// Parses NUL-delimited `git worktree list --porcelain -z` output without losing unusual paths.
pub fn parse_worktree_list(stdout: &str) -> Result<Vec<ParsedWorktree>, ParseError> {
    let mut worktrees = Vec::new();

    for record in stdout.split("\0\0").filter(|record| !record.is_empty()) {
        let mut builder = WorktreeRecordBuilder::default();
        for field in record.split('\0').filter(|field| !field.is_empty()) {
            if let Some(path) = field.strip_prefix("worktree ") {
                builder.worktree_root = Some(PathBuf::from(path));
            } else if let Some(head) = field.strip_prefix("HEAD ") {
                builder.head_commit_id = Some(head.to_string());
            } else if let Some(branch) = field.strip_prefix("branch refs/heads/") {
                builder.branch = Some(branch.to_string());
            } else if field == "detached" {
                builder.detached = true;
            } else if field == "bare" {
                builder.bare = true;
            } else if field == "locked" {
                builder.locked_reason = Some(String::new());
            } else if let Some(reason) = field.strip_prefix("locked ") {
                builder.locked_reason = Some(reason.trim_start().to_string());
            } else if field == "prunable" {
                builder.prunable_reason = Some(String::new());
            } else if let Some(reason) = field.strip_prefix("prunable ") {
                builder.prunable_reason = Some(reason.trim_start().to_string());
            }
        }
        worktrees.push(builder.finish()?);
    }

    if worktrees.is_empty() {
        return Err(ParseError::InvalidWorktreeList);
    }

    Ok(worktrees)
}
