use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Selects the old or new side of a two-way task diff for comment anchoring.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "task_diff.ts")]
pub enum TaskDiffSide {
    Old,
    New,
}

/// Captures whether a diff discussion still requires attention.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "task_diff.ts")]
pub enum TaskDiffThreadStatus {
    Open,
    Resolved,
}

/// Identifies which task diff should be computed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "task_diff.ts")]
pub struct GetTaskDiffRequest {
    pub task_id: String,
}

/// Returns one standard unified patch and the revisions needed to anchor review state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "task_diff.ts")]
pub struct GetTaskDiffResponse {
    pub base_commit_id: String,
    pub head_commit_id: String,
    pub diff_id: String,
    pub patch: String,
}

/// Anchors a root discussion to one line range in a stable diff snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "task_diff.ts")]
pub struct TaskDiffCommentAnchor {
    pub diff_id: String,
    /// Uses the old path on the old side and the new path on the new side for renamed files.
    pub path: String,
    pub side: TaskDiffSide,
    pub start_line: u32,
    pub end_line: u32,
    pub hunk_header: String,
    pub line_content: String,
}

/// Distinguishes anchored root discussions from reply messages in public responses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase")]
#[ts(export_to = "task_diff.ts")]
pub enum TaskDiffCommentKind {
    Thread {
        anchor: TaskDiffCommentAnchor,
        status: TaskDiffThreadStatus,
    },
    Reply {
        #[serde(rename = "parentCommentId")]
        #[ts(rename = "parentCommentId")]
        parent_comment_id: String,
    },
}

/// Represents one task diff discussion message returned to the frontend.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "task_diff.ts")]
pub struct TaskDiffComment {
    pub id: String,
    pub task_id: String,
    pub kind: TaskDiffCommentKind,
    pub body: String,
    #[ts(type = "number")]
    pub created_at: i64,
    #[ts(type = "number")]
    pub updated_at: i64,
}

/// Identifies which task discussion messages should be listed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "task_diff.ts")]
pub struct ListTaskDiffCommentsRequest {
    pub task_id: String,
}

/// Returns every visible root discussion and reply for one task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "task_diff.ts")]
pub struct ListTaskDiffCommentsResponse {
    pub comments: Vec<TaskDiffComment>,
}

/// Creates one line-anchored root discussion for the current task diff.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "task_diff.ts")]
pub struct CreateTaskDiffCommentRequest {
    pub task_id: String,
    pub anchor: TaskDiffCommentAnchor,
    pub body: String,
}

/// Returns the newly persisted root discussion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "task_diff.ts")]
pub struct CreateTaskDiffCommentResponse {
    pub comment: TaskDiffComment,
}

/// Adds a reply beneath an existing discussion message in the same task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "task_diff.ts")]
pub struct ReplyTaskDiffCommentRequest {
    pub task_id: String,
    pub comment_id: String,
    pub body: String,
}

/// Returns the newly persisted reply message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "task_diff.ts")]
pub struct ReplyTaskDiffCommentResponse {
    pub comment: TaskDiffComment,
}

/// Replaces the open/resolved status of one root diff discussion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "task_diff.ts")]
pub struct SetTaskDiffCommentStatusRequest {
    pub task_id: String,
    pub comment_id: String,
    pub status: TaskDiffThreadStatus,
}

/// Returns the root discussion after its status changes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "task_diff.ts")]
pub struct SetTaskDiffCommentStatusResponse {
    pub comment: TaskDiffComment,
}

#[cfg(test)]
mod tests {
    use super::{GetTaskDiffResponse, TaskDiffCommentAnchor, TaskDiffCommentKind, TaskDiffSide};
    use pretty_assertions::assert_eq;
    use serde_json::json;

    /// Verifies task diff payloads use the camel-case shape consumed by generated clients.
    #[test]
    fn serializes_task_diff_contracts() {
        let response = GetTaskDiffResponse {
            base_commit_id: "base".to_string(),
            head_commit_id: "head".to_string(),
            diff_id: "diff".to_string(),
            patch: "patch".to_string(),
        };
        let anchor = TaskDiffCommentAnchor {
            diff_id: "diff".to_string(),
            path: "src/main.rs".to_string(),
            side: TaskDiffSide::New,
            start_line: 4,
            end_line: 5,
            hunk_header: "@@ -1,3 +1,5 @@".to_string(),
            line_content: "println!()".to_string(),
        };

        assert_eq!(
            serde_json::to_value(response).unwrap(),
            json!({
                "baseCommitId": "base",
                "headCommitId": "head",
                "diffId": "diff",
                "patch": "patch",
            })
        );
        assert_eq!(serde_json::to_value(anchor).unwrap()["side"], "new");
        assert_eq!(
            serde_json::to_value(TaskDiffCommentKind::Reply {
                parent_comment_id: "comment-1".to_string(),
            })
            .unwrap(),
            json!({
                "kind": "reply",
                "parentCommentId": "comment-1",
            })
        );
    }
}
