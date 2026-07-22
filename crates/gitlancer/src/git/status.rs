use crate::domain::paths::RepoRelativePath;
use crate::domain::worktree::WorktreeHandle;
use crate::error::GitlancerError;
use crate::exec::command::{GitCommand, GitIntent};
use crate::exec::env::GitEnv;
use crate::exec::runner::GitRunner;
use crate::git::Git;
use crate::parse;

/// Carries the information needed to read structured status information from one worktree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusRequest<'a> {
    pub worktree: &'a WorktreeHandle,
}

/// Represents the high-level status view returned to upper layers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusResponse {
    pub entries: Vec<StatusEntry>,
}

/// Represents one structured worktree status entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StatusEntry {
    /// Describes a tracked path whose index or worktree state differs from `HEAD`.
    Ordinary {
        status: TrackedStatus,
        submodule: SubmoduleStatus,
        head_mode: FileMode,
        index_mode: FileMode,
        worktree_mode: FileMode,
        head_object_id: StatusObjectId,
        index_object_id: StatusObjectId,
        path: RepoRelativePath,
    },
    /// Describes a tracked path Git classified as a rename or copy.
    RenamedOrCopied {
        status: TrackedStatus,
        submodule: SubmoduleStatus,
        head_mode: FileMode,
        index_mode: FileMode,
        worktree_mode: FileMode,
        head_object_id: StatusObjectId,
        index_object_id: StatusObjectId,
        operation: RenameOrCopy,
        similarity: SimilarityScore,
        path: RepoRelativePath,
        original_path: RepoRelativePath,
    },
    /// Describes a path with unmerged index stages after a conflicted operation.
    Unmerged {
        status: TrackedStatus,
        submodule: SubmoduleStatus,
        stage_one_mode: FileMode,
        stage_two_mode: FileMode,
        stage_three_mode: FileMode,
        worktree_mode: FileMode,
        stage_one_object_id: StatusObjectId,
        stage_two_object_id: StatusObjectId,
        stage_three_object_id: StatusObjectId,
        path: RepoRelativePath,
    },
    /// Describes a path that is not tracked by the index.
    Untracked { path: RepoRelativePath },
    /// Describes a path excluded by an ignore rule.
    Ignored { path: RepoRelativePath },
}

impl StatusEntry {
    /// Returns the current repository-relative path shared by every status record kind.
    pub fn path(&self) -> &RepoRelativePath {
        match self {
            Self::Ordinary { path, .. }
            | Self::RenamedOrCopied { path, .. }
            | Self::Unmerged { path, .. }
            | Self::Untracked { path }
            | Self::Ignored { path } => path,
        }
    }
}

/// Captures the independent index and worktree state encoded by porcelain v2's `XY` field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrackedStatus {
    pub index: ChangeKind,
    pub worktree: ChangeKind,
}

/// Identifies the kind of change Git reports for one side of a tracked path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    Unmodified,
    Modified,
    FileTypeChanged,
    Added,
    Deleted,
    Renamed,
    Copied,
    Unmerged,
}

/// Captures porcelain v2's four-character submodule state without exposing positional flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubmoduleStatus {
    NotSubmodule,
    Submodule {
        commit_changed: bool,
        tracked_changes: bool,
        untracked_changes: bool,
    },
}

/// Preserves a Git file mode after the status parser has validated its octal representation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileMode(String);

impl FileMode {
    /// Exposes the validated mode exactly as Git emitted it.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Creates a file mode after the parser has validated the porcelain field.
    pub(crate) fn from_validated(value: &str) -> Self {
        Self(value.to_string())
    }
}

/// Preserves an object ID from a status record, including the all-zero missing-object sentinel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusObjectId(String);

impl StatusObjectId {
    /// Exposes the validated object ID exactly as Git emitted it.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Creates an object ID after the parser has validated the porcelain field.
    pub(crate) fn from_validated(value: &str) -> Self {
        Self(value.to_string())
    }
}

/// Distinguishes Git's rename and copy classifications for a type `2` status record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenameOrCopy {
    Rename,
    Copy,
}

/// Stores Git's validated rename/copy similarity percentage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SimilarityScore(u8);

impl SimilarityScore {
    /// Returns the similarity percentage in the inclusive range `0..=100`.
    pub fn get(self) -> u8 {
        self.0
    }

    /// Creates a similarity score after the parser has checked Git's range constraint.
    pub(crate) fn from_validated(value: u8) -> Self {
        Self(value)
    }
}

impl<R: GitRunner> Git<R> {
    /// Returns worktree status using porcelain v2 so callers can reason about changes without ad-hoc parsing.
    pub fn status(&self, request: StatusRequest<'_>) -> Result<StatusResponse, GitlancerError> {
        let command = GitCommand::new(
            request.worktree.worktree_root().as_path().to_path_buf(),
            vec![
                "status".to_string(),
                "--porcelain=v2".to_string(),
                "-z".to_string(),
            ],
            GitEnv::default(),
            GitIntent::ReadOnly,
        );
        let output = self.runner().run(&command)?;
        let entries = parse::status::parse_status_v2(&output.stdout)?;

        Ok(StatusResponse { entries })
    }
}
