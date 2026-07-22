use super::Migration;

const UP_STATEMENTS: &[&str] = &[
    "ALTER TABLE worktrees ADD COLUMN worktree_root TEXT",
    "CREATE INDEX idx_worktrees_project_lifecycle ON worktrees (project_id, lifecycle, is_deleted)",
];

const DOWN_STATEMENTS: &[&str] = &[
    "DROP INDEX IF EXISTS idx_worktrees_project_lifecycle",
    "ALTER TABLE worktrees DROP COLUMN worktree_root",
];

/// Adds the trusted checkout root required for configuration-independent worktree recovery.
pub fn migration() -> Migration {
    Migration::new("0006", UP_STATEMENTS, DOWN_STATEMENTS)
}
