use crate::{AuditFields, DomainModelError, ProjectId, TaskId};
use serde::{Deserialize, Deserializer, Serialize};
use std::path::{Path, PathBuf};

/// Represents an optional immutable baseline without exposing an empty recorded state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WorktreeBaseline(Option<String>);

impl WorktreeBaseline {
    /// Creates a recorded baseline only when a Git object identifier was actually supplied.
    pub fn recorded(commit_id: impl Into<String>) -> Result<Self, DomainModelError> {
        let commit_id = commit_id.into();
        if commit_id.trim().is_empty() {
            return Err(DomainModelError::EmptyWorktreeBaseline);
        }
        Ok(Self(Some(commit_id)))
    }

    /// Represents historical worktrees whose creation commit was never recorded.
    pub fn unavailable() -> Self {
        Self(None)
    }

    /// Returns the recorded commit identifier when this worktree supports stable diffs.
    pub fn commit_id(&self) -> Option<&str> {
        self.0.as_deref()
    }
}

impl<'de> Deserialize<'de> for WorktreeBaseline {
    /// Reuses the validated constructor so deserialization cannot create an empty baseline.
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        match Option::<String>::deserialize(deserializer)? {
            Some(commit_id) => Self::recorded(commit_id).map_err(serde::de::Error::custom),
            None => Ok(Self::unavailable()),
        }
    }
}

/// Models the durable lifecycle used to resume worktree removal after interruption.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorktreeLifecycle {
    ProvisioningPending,
    Active,
    RemovalPending,
}

impl WorktreeLifecycle {
    /// Returns the integer code used by persistence adapters for this lifecycle state.
    pub fn database_value(self) -> i64 {
        match self {
            Self::ProvisioningPending => 0,
            Self::Active => 1,
            Self::RemovalPending => 2,
        }
    }

    /// Converts a persisted integer into a strongly typed worktree lifecycle state.
    pub fn from_database_value(value: i64) -> Result<Self, DomainModelError> {
        match value {
            0 => Ok(Self::ProvisioningPending),
            1 => Ok(Self::Active),
            2 => Ok(Self::RemovalPending),
            _ => Err(DomainModelError::InvalidWorktreeLifecycle(value)),
        }
    }
}

/// Stores the trusted filesystem and Git identity for an application-managed worktree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ManagedWorktreeIdentity {
    root: PathBuf,
    branch_name: String,
}

impl<'de> Deserialize<'de> for ManagedWorktreeIdentity {
    /// Reuses the validated constructor so deserialization cannot bypass identity invariants.
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct SerializedIdentity {
            root: PathBuf,
            branch_name: String,
        }

        let identity = SerializedIdentity::deserialize(deserializer)?;
        Self::new(identity.root, identity.branch_name).map_err(serde::de::Error::custom)
    }
}

impl ManagedWorktreeIdentity {
    /// Validates the root and branch together so a managed worktree cannot have partial identity.
    pub fn new(root: PathBuf, branch_name: String) -> Result<Self, DomainModelError> {
        if root.as_os_str().is_empty() {
            return Err(DomainModelError::EmptyManagedWorktreeRoot);
        }
        if !root.is_absolute() {
            return Err(DomainModelError::RelativeManagedWorktreeRoot(root));
        }
        if branch_name.trim().is_empty() {
            return Err(DomainModelError::EmptyManagedWorktreeBranch);
        }

        Ok(Self { root, branch_name })
    }

    /// Returns the persisted absolute checkout root used for Git operations and recovery.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Returns the local branch owned by the task worktree.
    pub fn branch_name(&self) -> &str {
        &self.branch_name
    }
}

/// Distinguishes fully managed worktrees from historical rows that lack a trusted filesystem identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorktreeIdentity {
    Managed(ManagedWorktreeIdentity),
    LegacyUnavailable,
}

impl TryFrom<i64> for WorktreeLifecycle {
    type Error = DomainModelError;

    fn try_from(value: i64) -> Result<Self, Self::Error> {
        Self::from_database_value(value)
    }
}

/// Represents the physical git worktree that backs a task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Worktree {
    pub id: crate::WorktreeId,
    pub task_id: TaskId,
    pub project_id: ProjectId,
    pub baseline: WorktreeBaseline,
    pub audit_fields: AuditFields,
    identity: WorktreeIdentity,
    lifecycle: WorktreeLifecycle,
}

impl Worktree {
    /// Creates a managed worktree whose root and branch can be trusted for later recovery.
    pub fn managed(
        id: crate::WorktreeId,
        task_id: TaskId,
        project_id: ProjectId,
        identity: ManagedWorktreeIdentity,
        baseline: WorktreeBaseline,
        lifecycle: WorktreeLifecycle,
        audit_fields: AuditFields,
    ) -> Self {
        Self {
            id,
            task_id,
            project_id,
            baseline,
            audit_fields,
            identity: WorktreeIdentity::Managed(identity),
            lifecycle,
        }
    }

    /// Reconstructs a historical row that predates trusted worktree identity persistence.
    pub fn legacy(
        id: crate::WorktreeId,
        task_id: TaskId,
        project_id: ProjectId,
        baseline: WorktreeBaseline,
        lifecycle: WorktreeLifecycle,
        audit_fields: AuditFields,
    ) -> Self {
        Self {
            id,
            task_id,
            project_id,
            baseline,
            audit_fields,
            identity: WorktreeIdentity::LegacyUnavailable,
            lifecycle,
        }
    }

    /// Returns the trusted root only for worktrees created by the managed lifecycle.
    pub fn root(&self) -> Option<&Path> {
        match &self.identity {
            WorktreeIdentity::Managed(identity) => Some(identity.root()),
            WorktreeIdentity::LegacyUnavailable => None,
        }
    }

    /// Returns the trusted local branch only for worktrees created by the managed lifecycle.
    pub fn branch_name(&self) -> Option<&str> {
        match &self.identity {
            WorktreeIdentity::Managed(identity) => Some(identity.branch_name()),
            WorktreeIdentity::LegacyUnavailable => None,
        }
    }

    /// Returns the durable lifecycle state used by creation and deletion recovery.
    pub fn lifecycle(&self) -> WorktreeLifecycle {
        self.lifecycle
    }

    /// Replaces the lifecycle after the caller has durably completed the corresponding transition.
    pub fn set_lifecycle(&mut self, lifecycle: WorktreeLifecycle, updated_at: i64) {
        self.lifecycle = lifecycle;
        self.audit_fields = AuditFields::new(
            self.audit_fields.created_at,
            updated_at,
            self.audit_fields.is_deleted,
        );
    }

    /// Records the immutable baseline captured by Git while provisioning remains recoverable.
    pub fn record_baseline(&mut self, baseline: WorktreeBaseline, updated_at: i64) {
        self.baseline = baseline;
        self.audit_fields = AuditFields::new(
            self.audit_fields.created_at,
            updated_at,
            self.audit_fields.is_deleted,
        );
    }
}
