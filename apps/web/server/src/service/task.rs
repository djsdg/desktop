use crate::bootstrap::SystemClock;
use ora_application::{
    ApplicationError, CreateTaskHandler, DeleteTaskHandler, GetTaskHandler,
    GitTaskWorktreeProvisioner, ListTasksHandler, UpdateTaskHandler, UuidTaskIdGenerator,
    UuidWorktreeIdGenerator,
};
use ora_contracts::{
    CreateTaskRequest, CreateTaskResponse, DeleteTaskRequest, DeleteTaskResponse, GetTaskRequest,
    GetTaskResponse, ListTasksRequest, ListTasksResponse, UpdateTaskRequest, UpdateTaskResponse,
};
use ora_db::{RepositoryPool, SqliteTaskRepository, SqliteWorktreeRepository};
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
        work_dir: PathBuf,
        clock: SystemClock,
    ) -> Self {
        let task_repository = SqliteTaskRepository::new(pool.clone());
        let worktree_repository = SqliteWorktreeRepository::new(pool);
        let worktree_provisioner = GitTaskWorktreeProvisioner::new(project_root);

        Self {
            create_task: CreateTaskHandler::new(
                task_repository.clone(),
                worktree_repository.clone(),
                UuidTaskIdGenerator::new(),
                UuidWorktreeIdGenerator::new(),
                worktree_provisioner.clone(),
                work_dir.clone(),
                clock,
            ),
            get_task: GetTaskHandler::new(task_repository.clone()),
            list_tasks: ListTasksHandler::new(task_repository.clone()),
            update_task: UpdateTaskHandler::new(task_repository.clone(), clock),
            delete_task: DeleteTaskHandler::new(
                task_repository,
                worktree_repository,
                worktree_provisioner,
                work_dir,
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
