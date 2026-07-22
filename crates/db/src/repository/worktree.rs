use ora_application::{WorktreeRepository, WorktreeRepositoryError};
use ora_domain::{
    AuditFields, ManagedWorktreeIdentity, ProjectId, TaskId, Worktree, WorktreeBaseline,
    WorktreeId, WorktreeLifecycle,
};
use rusqlite::{Row, params};

use crate::repository::{RepositoryPool, connection::bool_to_sqlite};

/// Persists worktree snapshots through SQLite while hiding storage details from handlers.
#[derive(Clone, Debug)]
pub struct SqliteWorktreeRepository {
    pool: RepositoryPool,
}

impl SqliteWorktreeRepository {
    /// Builds a worktree repository from the shared repository pool.
    pub fn new(pool: RepositoryPool) -> Self {
        Self { pool }
    }
}

impl WorktreeRepository for SqliteWorktreeRepository {
    /// Inserts a new worktree row and returns the stored worktree snapshot.
    fn create_worktree(&self, worktree: Worktree) -> Result<Worktree, WorktreeRepositoryError> {
        self.pool
            .with_connection(|connection| {
                connection.execute(
                    "INSERT INTO worktrees (id, task_id, project_id, branch_name, worktree_root, base_commit_id, lifecycle, created_at, updated_at, is_deleted)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                    params![
                        worktree.id.as_ref(),
                        worktree.task_id.as_ref(),
                        worktree.project_id.as_ref(),
                        worktree.branch_name(),
                        worktree.root().map(|root| root.to_string_lossy().into_owned()),
                        baseline_value(&worktree.baseline),
                        worktree.lifecycle().database_value(),
                        worktree.audit_fields.created_at,
                        worktree.audit_fields.updated_at,
                        bool_to_sqlite(worktree.audit_fields.is_deleted),
                    ],
                )?;

                Ok(worktree)
            })
            .map_err(worktree_repository_error_from_database)
    }

    /// Loads one visible worktree row by identifier.
    fn find_worktree(
        &self,
        worktree_id: &WorktreeId,
    ) -> Result<Option<Worktree>, WorktreeRepositoryError> {
        self.pool
            .with_connection(|connection| {
                let mut statement = connection.prepare(
                    "SELECT id, task_id, project_id, branch_name, worktree_root, base_commit_id, lifecycle, created_at, updated_at, is_deleted
                     FROM worktrees
                     WHERE id = ?1 AND is_deleted = 0",
                )?;
                let mut rows = statement.query(params![worktree_id.as_ref()])?;

                match rows.next()? {
                    Some(row) => Ok(Some(map_worktree_row(row)?)),
                    None => Ok(None),
                }
            })
            .map_err(worktree_repository_error_from_database)
    }

    /// Lists every visible worktree row in stable storage order.
    fn list_worktrees(&self) -> Result<Vec<Worktree>, WorktreeRepositoryError> {
        self.pool
            .with_connection(|connection| {
                let mut statement = connection.prepare(
                    "SELECT id, task_id, project_id, branch_name, worktree_root, base_commit_id, lifecycle, created_at, updated_at, is_deleted
                     FROM worktrees
                     WHERE is_deleted = 0
                     ORDER BY created_at, id",
                )?;
                let mut rows = statement.query([])?;
                let mut worktrees = Vec::new();

                while let Some(row) = rows.next()? {
                    worktrees.push(map_worktree_row(row)?);
                }

                Ok(worktrees)
            })
            .map_err(worktree_repository_error_from_database)
    }

    /// Replaces the persisted worktree snapshot identified by the provided id.
    fn update_worktree(&self, worktree: Worktree) -> Result<Worktree, WorktreeRepositoryError> {
        self.pool
            .with_connection(|connection| {
                let updated_rows = connection.execute(
                    "UPDATE worktrees
                     SET task_id = ?2, project_id = ?3, branch_name = ?4, worktree_root = ?5, base_commit_id = ?6, lifecycle = ?7, created_at = ?8, updated_at = ?9, is_deleted = ?10
                     WHERE id = ?1 AND is_deleted = 0",
                    params![
                        worktree.id.as_ref(),
                        worktree.task_id.as_ref(),
                        worktree.project_id.as_ref(),
                        worktree.branch_name(),
                        worktree.root().map(|root| root.to_string_lossy().into_owned()),
                        baseline_value(&worktree.baseline),
                        worktree.lifecycle().database_value(),
                        worktree.audit_fields.created_at,
                        worktree.audit_fields.updated_at,
                        bool_to_sqlite(worktree.audit_fields.is_deleted),
                    ],
                )?;

                if updated_rows == 0 {
                    return Err(crate::DatabaseError::Sqlite(rusqlite::Error::QueryReturnedNoRows));
                }

                Ok(worktree)
            })
            .map_err(worktree_repository_error_from_database)
    }

    /// Soft-deletes one visible worktree row and reports whether it existed.
    fn soft_delete_worktree(
        &self,
        worktree_id: &WorktreeId,
        deleted_at: i64,
    ) -> Result<bool, WorktreeRepositoryError> {
        self.pool
            .with_connection(|connection| {
                let updated_rows = connection.execute(
                    "UPDATE worktrees
                     SET updated_at = ?2, is_deleted = 1
                     WHERE id = ?1 AND is_deleted = 0",
                    params![worktree_id.as_ref(), deleted_at],
                )?;

                Ok(updated_rows > 0)
            })
            .map_err(worktree_repository_error_from_database)
    }
}

/// Reconstructs a domain worktree from the selected worktree columns.
fn map_worktree_row(row: &Row<'_>) -> Result<Worktree, crate::DatabaseError> {
    let lifecycle = WorktreeLifecycle::from_database_value(row.get("lifecycle")?)?;
    let is_deleted = row.get::<_, i64>("is_deleted")? != 0;

    let id = WorktreeId::new(row.get::<_, String>("id")?);
    let task_id = TaskId::new(row.get::<_, String>("task_id")?);
    let project_id = ProjectId::new(row.get::<_, String>("project_id")?);
    let baseline = match row.get::<_, Option<String>>("base_commit_id")? {
        Some(commit_id) => WorktreeBaseline::recorded(commit_id)?,
        None => WorktreeBaseline::unavailable(),
    };
    let audit_fields = AuditFields::new(row.get("created_at")?, row.get("updated_at")?, is_deleted);
    let root = row.get::<_, Option<String>>("worktree_root")?;
    let branch_name = row.get::<_, Option<String>>("branch_name")?;

    match root.filter(|root| !root.is_empty()) {
        Some(root) => Ok(Worktree::managed(
            id,
            task_id,
            project_id,
            ManagedWorktreeIdentity::new(
                root.into(),
                branch_name.ok_or(ora_domain::DomainModelError::EmptyManagedWorktreeBranch)?,
            )?,
            baseline,
            lifecycle,
            audit_fields,
        )),
        None => Ok(Worktree::legacy(
            id,
            task_id,
            project_id,
            baseline,
            lifecycle,
            audit_fields,
        )),
    }
}

/// Maps the explicit domain baseline state into the nullable migration representation.
fn baseline_value(baseline: &WorktreeBaseline) -> Option<&str> {
    baseline.commit_id()
}

/// Converts shared database-layer failures into worktree repository errors.
fn worktree_repository_error_from_database(error: crate::DatabaseError) -> WorktreeRepositoryError {
    WorktreeRepositoryError::OperationFailed(error.to_string())
}
