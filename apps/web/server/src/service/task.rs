use crate::bootstrap::SystemClock;
use ora_application::{
    ApplicationError, CreateTaskConfiguration, CreateTaskHandler, DeleteTaskHandler,
    GetTaskHandler, GitTaskWorktreeProvisioner, ListTasksHandler,
    RecoverPendingTaskWorktreesHandler, UpdateTaskHandler, UuidTaskIdGenerator,
    UuidWorktreeIdGenerator,
};
use ora_contracts::{
    CreateTaskRequest, CreateTaskResponse, DeleteTaskRequest, DeleteTaskResponse, GetTaskRequest,
    GetTaskResponse, ListTasksRequest, ListTasksResponse, UpdateTaskRequest, UpdateTaskResponse,
};
use ora_db::{RepositoryPool, SqliteTaskRepository, SqliteWorktreeRepository};
use ora_domain::ProjectId;
use ora_logging::{ora_error, ora_info};
use std::path::PathBuf;

/// Groups the transport-facing task entry points for the web adapter.
pub struct TaskApi {
    create_task: CreateTaskHandler<
        SqliteTaskRepository,
        SqliteWorktreeRepository,
        UuidTaskIdGenerator,
        UuidWorktreeIdGenerator,
        GitTaskWorktreeProvisioner,
        SystemClock,
    >,
    get_task: GetTaskHandler<SqliteTaskRepository>,
    list_tasks: ListTasksHandler<SqliteTaskRepository>,
    update_task: UpdateTaskHandler<SqliteTaskRepository, SystemClock>,
    delete_task: DeleteTaskHandler<
        SqliteTaskRepository,
        SqliteWorktreeRepository,
        GitTaskWorktreeProvisioner,
        SystemClock,
    >,
}

impl TaskApi {
    /// Builds the task transport API from the shared repository pool and clock source.
    pub fn new(
        pool: RepositoryPool,
        project_root: PathBuf,
        project_id: ProjectId,
        work_dir: PathBuf,
        clock: SystemClock,
    ) -> Self {
        let task_repository = SqliteTaskRepository::new(pool.clone());
        let worktree_repository = SqliteWorktreeRepository::new(pool);
        let worktree_provisioner = GitTaskWorktreeProvisioner::new(project_root);
        let recovery = RecoverPendingTaskWorktreesHandler::new(
            task_repository.clone(),
            worktree_repository.clone(),
            worktree_provisioner.clone(),
            project_id.clone(),
            clock,
        );
        match recovery.handle() {
            Ok(report) => ora_info!(
                message = "task worktree recovery completed",
                recovered = report.recovered,
                failed = report.failed,
            ),
            Err(error) => ora_error!(
                message = "task worktree recovery failed",
                error = %error,
            ),
        }

        Self {
            create_task: CreateTaskHandler::new(
                task_repository.clone(),
                worktree_repository.clone(),
                UuidTaskIdGenerator::new(),
                UuidWorktreeIdGenerator::new(),
                worktree_provisioner.clone(),
                CreateTaskConfiguration::new(project_id.clone(), work_dir.clone()),
                clock,
            ),
            get_task: GetTaskHandler::new(task_repository.clone()),
            list_tasks: ListTasksHandler::new(task_repository.clone()),
            update_task: UpdateTaskHandler::new(task_repository.clone(), project_id.clone(), clock),
            delete_task: DeleteTaskHandler::new(
                task_repository,
                worktree_repository,
                worktree_provisioner,
                project_id,
                clock,
            ),
        }
    }

    /// Accepts a create-task request and delegates the use case to the application layer.
    pub fn create_task(
        &self,
        request: CreateTaskRequest,
    ) -> Result<CreateTaskResponse, ApplicationError> {
        self.create_task.handle(request)
    }

    /// Accepts a get-task request and delegates the use case to the application layer.
    pub fn get_task(&self, request: GetTaskRequest) -> Result<GetTaskResponse, ApplicationError> {
        self.get_task.handle(request)
    }

    /// Accepts a list-tasks request and delegates the use case to the application layer.
    pub fn list_tasks(
        &self,
        request: ListTasksRequest,
    ) -> Result<ListTasksResponse, ApplicationError> {
        self.list_tasks.handle(request)
    }

    /// Accepts an update-task request and delegates the use case to the application layer.
    pub fn update_task(
        &self,
        request: UpdateTaskRequest,
    ) -> Result<UpdateTaskResponse, ApplicationError> {
        self.update_task.handle(request)
    }

    /// Accepts a delete-task request and delegates the use case to the application layer.
    pub fn delete_task(
        &self,
        request: DeleteTaskRequest,
    ) -> Result<DeleteTaskResponse, ApplicationError> {
        self.delete_task.handle(request)
    }
}
