use super::Migration;

const UP_STATEMENTS: &[&str] = &[
    "ALTER TABLE worktrees ADD COLUMN lifecycle INTEGER NOT NULL DEFAULT 1",
    "ALTER TABLE worktrees ADD COLUMN project_id TEXT NOT NULL DEFAULT ''",
    "UPDATE worktrees
     SET project_id = COALESCE(
         (SELECT tasks.project_id FROM tasks WHERE tasks.id = worktrees.task_id),
         ''
     )",
    "ALTER TABLE worktrees DROP COLUMN is_active",
];

const DOWN_STATEMENTS: &[&str] = &[
    "ALTER TABLE worktrees ADD COLUMN is_active INTEGER NOT NULL DEFAULT 0",
    "UPDATE worktrees SET is_active = 1 WHERE lifecycle = 1",
    "ALTER TABLE worktrees DROP COLUMN project_id",
    "ALTER TABLE worktrees DROP COLUMN lifecycle",
];

/// Replaces the legacy activity flag with a durable removal lifecycle state.
pub fn migration() -> Migration {
    Migration::new("0005", UP_STATEMENTS, DOWN_STATEMENTS)
}
