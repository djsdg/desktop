mod handlers;
mod id_generator;
mod mapper;
mod ports;
mod worktree_provisioner;

#[cfg(test)]
mod tests;

pub use handlers::{
    CreateTaskConfiguration, CreateTaskHandler, DeleteTaskHandler, GetTaskHandler,
    ListTasksHandler, RecoverPendingTaskWorktreesHandler, TaskWorktreeRecoveryReport,
    UpdateTaskHandler,
};
pub use id_generator::UuidTaskIdGenerator;
pub use ports::{
    CreateTaskWorktreeRequest, CreateTaskWorktreeResponse, DeleteTaskWorktreeRequest,
    TaskIdGenerator, TaskRepository, TaskRepositoryError, TaskWorktreeDeletionMode,
    TaskWorktreeProvisioner, TaskWorktreeProvisionerError, VerifyTaskWorktreeRequest,
};
pub use worktree_provisioner::GitTaskWorktreeProvisioner;
