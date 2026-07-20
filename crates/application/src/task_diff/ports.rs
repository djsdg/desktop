use ora_domain::{TaskDiffComment, TaskDiffCommentId, TaskId};
use std::path::PathBuf;

/// Supplies task-scoped Git differences while hiding Git and filesystem implementation details.
///
/// Implementations must restrict execution to the backend-resolved worktree in each request.
pub trait TaskDiffReader {
    /// Computes all task changes against the immutable commit captured at task creation.
    fn read_task_diff(
        &self,
        request: ReadTaskDiffRequest,
    ) -> Result<TaskDiffSnapshot, TaskDiffReaderError>;
}

/// Carries the backend-owned worktree path and immutable comparison baseline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadTaskDiffRequest {
    pub worktree_path: PathBuf,
    pub base_commit_id: String,
}

/// Returns the Git revisions and unified patch used by frontend review components.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskDiffSnapshot {
    pub head_commit_id: String,
    pub patch: String,
}

/// Captures Git-backed diff failures converted into stable application errors by handlers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskDiffReaderError {
    OperationFailed(String),
    TooLarge {
        byte_count: usize,
        max_byte_count: usize,
    },
}

/// Supplies persistence operations for root diff discussions and replies.
///
/// Implementations must return only visible comments and preserve their stable creation order.
pub trait TaskDiffCommentRepository {
    /// Persists one new root discussion or reply.
    fn create_comment(
        &self,
        comment: TaskDiffComment,
    ) -> Result<TaskDiffComment, TaskDiffCommentRepositoryError>;

    /// Loads one visible comment by identifier.
    fn find_comment(
        &self,
        comment_id: &TaskDiffCommentId,
    ) -> Result<Option<TaskDiffComment>, TaskDiffCommentRepositoryError>;

    /// Lists every visible discussion message for one task.
    fn list_comments(
        &self,
        task_id: &TaskId,
    ) -> Result<Vec<TaskDiffComment>, TaskDiffCommentRepositoryError>;

    /// Persists a root discussion status replacement.
    fn update_comment(
        &self,
        comment: TaskDiffComment,
    ) -> Result<TaskDiffComment, TaskDiffCommentRepositoryError>;
}

/// Supplies identifiers for newly created diff comments and replies.
pub trait TaskDiffCommentIdGenerator {
    /// Produces a fresh comment identifier.
    fn generate_comment_id(&self) -> TaskDiffCommentId;
}

/// Captures comment persistence failures without leaking database-specific errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskDiffCommentRepositoryError {
    OperationFailed(String),
}
