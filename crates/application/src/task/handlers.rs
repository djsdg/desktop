use crate::task::mapper::map_task;
use crate::task::ports::{
    CreateTaskWorktreeRequest, DeleteTaskWorktreeRequest, TaskIdGenerator, TaskRepository,
    TaskWorktreeDeletionMode, TaskWorktreeProvisioner, VerifyTaskWorktreeRequest,
};
use crate::worktree::{WorktreeIdGenerator, WorktreeRepository};
use crate::{ApplicationError, Clock};
use ora_contracts::{
    CreateTaskRequest, CreateTaskResponse, DeleteTaskRequest, DeleteTaskResponse, GetTaskRequest,
    GetTaskResponse, ListTasksRequest, ListTasksResponse, TaskStatus, UpdateTaskRequest,
    UpdateTaskResponse,
};
use ora_domain::{
    AuditFields, ManagedWorktreeIdentity, ProjectId, Task as DomainTask, TaskId,
    TaskStatus as DomainTaskStatus, Worktree as DomainWorktree, WorktreeId,
    WorktreeLifecycle as DomainWorktreeLifecycle,
};
use ora_logging::{ora_error, ora_info};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

const TASK_BRANCH_PREFIX_LEN: usize = 8;
const MAX_TASK_ID_GENERATION_ATTEMPTS: usize = 3;

/// Binds task creation to one configured repository and its managed worktree directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateTaskConfiguration {
    project_id: ProjectId,
    work_dir: PathBuf,
}

impl CreateTaskConfiguration {
    /// Creates the immutable project/worktree boundary enforced by task creation.
    pub fn new(project_id: ProjectId, work_dir: PathBuf) -> Self {
        Self {
            project_id,
            work_dir,
        }
    }
}

/// Handles task creation without depending on transport-specific concerns.
pub struct CreateTaskHandler<
    TaskRepositoryPort,
    WorktreeRepositoryPort,
    TaskIdGeneratorPort,
    WorktreeIdGeneratorPort,
    WorktreeProvisioner,
    ClockSource,
> {
    task_repository: TaskRepositoryPort,
    worktree_repository: WorktreeRepositoryPort,
    task_id_generator: TaskIdGeneratorPort,
    worktree_id_generator: WorktreeIdGeneratorPort,
    worktree_provisioner: WorktreeProvisioner,
    configuration: CreateTaskConfiguration,
    clock: ClockSource,
}

impl<
    TaskRepositoryPort,
    WorktreeRepositoryPort,
    TaskIdGeneratorPort,
    WorktreeIdGeneratorPort,
    WorktreeProvisioner,
    ClockSource,
>
    CreateTaskHandler<
        TaskRepositoryPort,
        WorktreeRepositoryPort,
        TaskIdGeneratorPort,
        WorktreeIdGeneratorPort,
        WorktreeProvisioner,
        ClockSource,
    >
{
    pub fn new(
        task_repository: TaskRepositoryPort,
        worktree_repository: WorktreeRepositoryPort,
        task_id_generator: TaskIdGeneratorPort,
        worktree_id_generator: WorktreeIdGeneratorPort,
        worktree_provisioner: WorktreeProvisioner,
        configuration: CreateTaskConfiguration,
        clock: ClockSource,
    ) -> Self {
        Self {
            task_repository,
            worktree_repository,
            task_id_generator,
            worktree_id_generator,
            worktree_provisioner,
            configuration,
            clock,
        }
    }
}

impl<
    TaskRepositoryPort,
    WorktreeRepositoryPort,
    TaskIdGeneratorPort,
    WorktreeIdGeneratorPort,
    WorktreeProvisioner,
    ClockSource,
>
    CreateTaskHandler<
        TaskRepositoryPort,
        WorktreeRepositoryPort,
        TaskIdGeneratorPort,
        WorktreeIdGeneratorPort,
        WorktreeProvisioner,
        ClockSource,
    >
where
    TaskRepositoryPort: TaskRepository,
    WorktreeRepositoryPort: WorktreeRepository,
    TaskIdGeneratorPort: TaskIdGenerator,
    WorktreeIdGeneratorPort: WorktreeIdGenerator,
    WorktreeProvisioner: TaskWorktreeProvisioner,
    ClockSource: Clock,
{
    /// Creates a task together with its owned linked worktree and returns the public response payload.
    pub fn handle(
        &self,
        request: CreateTaskRequest,
    ) -> Result<CreateTaskResponse, ApplicationError> {
        let project_id =
            require_configured_project(&self.configuration.project_id, request.project_id)?;
        let task_id = self
            .select_available_task_id()
            .inspect_err(|error| log_task_failure("create_task", None, error))?;
        let branch_name = branch_name_for_task(&task_id);
        let worktree_path = worktree_path_for_task(&self.configuration.work_dir, &task_id);
        let now = self.clock.now_timestamp_millis();
        let worktree_id = self.worktree_id_generator.generate_worktree_id();
        let worktree = DomainWorktree::managed(
            worktree_id,
            task_id.clone(),
            project_id.clone(),
            ManagedWorktreeIdentity::new(worktree_path.clone(), branch_name.clone()).map_err(
                |error| ApplicationError::TaskWorktree {
                    message: error.to_string(),
                },
            )?,
            ora_domain::WorktreeBaseline::unavailable(),
            DomainWorktreeLifecycle::ProvisioningPending,
            AuditFields::new(now, now, /*is_deleted*/ false),
        );
        let mut worktree = match self.worktree_repository.create_worktree(worktree) {
            Ok(worktree) => worktree,
            Err(error) => {
                return Err(ApplicationError::from_worktree_repository_error(error));
            }
        };
        let provisioned_worktree =
            match self
                .worktree_provisioner
                .create_task_worktree(CreateTaskWorktreeRequest {
                    worktree_id: worktree.id.clone(),
                    branch_name: branch_name.clone(),
                    worktree_path: worktree_path.clone(),
                }) {
                Ok(provisioned_worktree) => provisioned_worktree,
                Err(error) => {
                    return Err(self.rollback_pending_provisioning(
                        &task_id,
                        &worktree.id,
                        &worktree_path,
                        &branch_name,
                        ApplicationError::from_task_worktree_provisioner_error(error),
                    ));
                }
            };
        let baseline =
            match ora_domain::WorktreeBaseline::recorded(provisioned_worktree.base_commit_id) {
                Ok(baseline) => baseline,
                Err(error) => {
                    return Err(self.rollback_pending_provisioning(
                        &task_id,
                        &worktree.id,
                        &worktree_path,
                        &branch_name,
                        ApplicationError::TaskWorktree {
                            message: error.to_string(),
                        },
                    ));
                }
            };
        worktree.record_baseline(baseline, self.clock.now_timestamp_millis());
        let worktree_id = worktree.id.clone();
        worktree = match self.worktree_repository.update_worktree(worktree) {
            Ok(worktree) => worktree,
            Err(error) => {
                return Err(self.rollback_pending_provisioning(
                    &task_id,
                    &worktree_id,
                    &worktree_path,
                    &branch_name,
                    ApplicationError::from_worktree_repository_error(error),
                ));
            }
        };
        let task = DomainTask::new(
            task_id.clone(),
            project_id,
            request.title,
            map_contract_task_status(request.status),
            Some(worktree.id.clone()),
            AuditFields::new(now, now, /*is_deleted*/ false),
        );
        let task = match self.task_repository.create_task(task) {
            Ok(task) => task,
            Err(error) => {
                return Err(self.rollback_pending_provisioning(
                    &task_id,
                    &worktree.id,
                    &worktree_path,
                    &branch_name,
                    ApplicationError::from_task_repository_error(error),
                ));
            }
        };
        worktree.set_lifecycle(
            DomainWorktreeLifecycle::Active,
            self.clock.now_timestamp_millis(),
        );
        self.worktree_repository
            .update_worktree(worktree)
            .map_err(ApplicationError::from_worktree_repository_error)?;

        log_task_success("create_task", Some(&task.id));

        Ok(CreateTaskResponse {
            task: map_task(task),
        })
    }

    /// Generates a task id whose branch prefix does not collide with existing task worktree folders.
    fn select_available_task_id(&self) -> Result<TaskId, ApplicationError> {
        for _ in 0..MAX_TASK_ID_GENERATION_ATTEMPTS {
            let task_id = self.task_id_generator.generate_task_id();
            let branch_prefix = task_branch_prefix(&task_id);

            if task_branch_prefix_exists_in_work_dir(&self.configuration.work_dir, &branch_prefix)?
            {
                continue;
            }

            let branch_name = branch_name_for_task(&task_id);
            let branch_exists = self
                .worktree_provisioner
                .task_branch_exists(&branch_name)
                .map_err(ApplicationError::from_task_worktree_provisioner_error)?;
            if !branch_exists {
                return Ok(task_id);
            }
        }

        Err(ApplicationError::TaskWorktree {
            message: format!(
                "failed to generate a task branch prefix without collision after {MAX_TASK_ID_GENERATION_ATTEMPTS} attempts"
            ),
        })
    }

    /// Rolls back a durable provisioning intent after any later creation stage fails normally.
    fn rollback_pending_provisioning(
        &self,
        task_id: &TaskId,
        worktree_id: &WorktreeId,
        worktree_path: &Path,
        branch_name: &str,
        original_error: ApplicationError,
    ) -> ApplicationError {
        if let Err(cleanup_error) =
            self.worktree_provisioner
                .delete_task_worktree(DeleteTaskWorktreeRequest {
                    expected_worktree_id: worktree_id.clone(),
                    worktree_path: worktree_path.to_path_buf(),
                    expected_branch_name: branch_name.to_string(),
                    mode: TaskWorktreeDeletionMode::Force,
                })
        {
            // Keep the provisioning row visible so startup recovery can retry failed Git cleanup.
            let cleanup_error =
                ApplicationError::from_task_worktree_provisioner_error(cleanup_error);
            log_task_failure("create_task", Some(task_id), &cleanup_error);
            return cleanup_error;
        }

        let result = self
            .worktree_repository
            .soft_delete_worktree(worktree_id, self.clock.now_timestamp_millis())
            .map_err(ApplicationError::from_worktree_repository_error)
            .map(|_| original_error);
        match result {
            Ok(error) => {
                log_task_failure("create_task", Some(task_id), &error);
                error
            }
            Err(error) => {
                log_task_failure("create_task", Some(task_id), &error);
                error
            }
        }
    }
}

/// Handles one task lookup without depending on transport-specific concerns.
pub struct GetTaskHandler<Repository> {
    repository: Repository,
}

impl<Repository> GetTaskHandler<Repository> {
    pub fn new(repository: Repository) -> Self {
        Self { repository }
    }
}

impl<Repository> GetTaskHandler<Repository>
where
    Repository: TaskRepository,
{
    /// Loads one visible task or returns a stable not-found application error.
    pub fn handle(&self, request: GetTaskRequest) -> Result<GetTaskResponse, ApplicationError> {
        let task_id = TaskId::new(request.task_id);
        let task = self.repository.find_task(&task_id).map_err(|error| {
            let error = ApplicationError::from_task_repository_error(error);
            log_task_failure("get_task", Some(&task_id), &error);
            error
        })?;

        match task {
            Some(task) => {
                log_task_success("get_task", Some(&task_id));

                Ok(GetTaskResponse {
                    task: map_task(task),
                })
            }
            None => {
                let error = ApplicationError::TaskNotFound {
                    task_id: task_id.to_string(),
                };
                log_task_failure("get_task", Some(&task_id), &error);
                Err(error)
            }
        }
    }
}

/// Handles task listing without depending on transport-specific concerns.
pub struct ListTasksHandler<Repository> {
    repository: Repository,
}

impl<Repository> ListTasksHandler<Repository> {
    pub fn new(repository: Repository) -> Self {
        Self { repository }
    }
}

impl<Repository> ListTasksHandler<Repository>
where
    Repository: TaskRepository,
{
    /// Lists every visible task and maps each one into the shared contract view.
    pub fn handle(
        &self,
        _request: ListTasksRequest,
    ) -> Result<ListTasksResponse, ApplicationError> {
        let tasks = self.repository.list_tasks().map_err(|error| {
            let error = ApplicationError::from_task_repository_error(error);
            log_task_failure("list_tasks", None, &error);
            error
        })?;

        ora_info!(
            message = "listed tasks",
            operation = "list_tasks",
            task_count = tasks.len()
        );

        Ok(ListTasksResponse {
            tasks: tasks.into_iter().map(map_task).collect(),
        })
    }
}

/// Handles task updates without depending on transport-specific concerns.
pub struct UpdateTaskHandler<Repository, ClockSource> {
    repository: Repository,
    project_id: ProjectId,
    clock: ClockSource,
}

impl<Repository, ClockSource> UpdateTaskHandler<Repository, ClockSource> {
    pub fn new(repository: Repository, project_id: ProjectId, clock: ClockSource) -> Self {
        Self {
            repository,
            project_id,
            clock,
        }
    }
}

impl<Repository, ClockSource> UpdateTaskHandler<Repository, ClockSource>
where
    Repository: TaskRepository,
    ClockSource: Clock,
{
    /// Replaces the public task fields while preserving persistence-managed audit state.
    pub fn handle(
        &self,
        request: UpdateTaskRequest,
    ) -> Result<UpdateTaskResponse, ApplicationError> {
        let task_id = TaskId::new(request.task_id);
        let existing_task = self.repository.find_task(&task_id).map_err(|error| {
            let error = ApplicationError::from_task_repository_error(error);
            log_task_failure("update_task", Some(&task_id), &error);
            error
        })?;

        let existing_task = match existing_task {
            Some(existing_task) => existing_task,
            None => {
                let error = ApplicationError::TaskNotFound {
                    task_id: task_id.to_string(),
                };
                log_task_failure("update_task", Some(&task_id), &error);
                return Err(error);
            }
        };
        let requested_project_id =
            require_configured_project(&self.project_id, request.project_id)?;
        if existing_task.project_id != requested_project_id {
            return Err(ApplicationError::TaskProjectMismatch {
                expected_project_id: requested_project_id.to_string(),
                actual_project_id: existing_task.project_id.to_string(),
            });
        }

        let task = DomainTask::new(
            task_id.clone(),
            requested_project_id,
            request.title,
            map_contract_task_status(request.status),
            existing_task.worktree_id,
            AuditFields::new(
                existing_task.audit_fields.created_at,
                self.clock.now_timestamp_millis(),
                existing_task.audit_fields.is_deleted,
            ),
        );
        let task = self.repository.update_task(task).map_err(|error| {
            let error = ApplicationError::from_task_repository_error(error);
            log_task_failure("update_task", Some(&task_id), &error);
            error
        })?;

        log_task_success("update_task", Some(&task_id));

        Ok(UpdateTaskResponse {
            task: map_task(task),
        })
    }
}

/// Rejects requests that would bind the configured Git repository to another project identity.
fn require_configured_project(
    configured_project_id: &ProjectId,
    requested_project_id: String,
) -> Result<ProjectId, ApplicationError> {
    let requested_project_id = ProjectId::new(requested_project_id);
    if requested_project_id != *configured_project_id {
        return Err(ApplicationError::TaskProjectMismatch {
            expected_project_id: configured_project_id.to_string(),
            actual_project_id: requested_project_id.to_string(),
        });
    }

    Ok(requested_project_id)
}

/// Handles task deletion without exposing transport-specific cleanup details.
pub struct DeleteTaskHandler<
    TaskRepositoryPort,
    WorktreeRepositoryPort,
    WorktreeProvisioner,
    ClockSource,
> {
    task_repository: TaskRepositoryPort,
    worktree_repository: WorktreeRepositoryPort,
    worktree_provisioner: WorktreeProvisioner,
    project_id: ProjectId,
    clock: ClockSource,
}

/// Retries durable worktree removals that were interrupted after entering `RemovalPending`.
pub struct RecoverPendingTaskWorktreesHandler<
    TaskRepositoryPort,
    WorktreeRepositoryPort,
    WorktreeProvisioner,
    ClockSource,
> {
    task_repository: TaskRepositoryPort,
    worktree_repository: WorktreeRepositoryPort,
    worktree_provisioner: WorktreeProvisioner,
    project_id: ProjectId,
    clock: ClockSource,
}

/// Summarizes one best-effort recovery pass without hiding individual failures from logs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskWorktreeRecoveryReport {
    pub recovered: usize,
    pub failed: usize,
}

impl<TaskRepositoryPort, WorktreeRepositoryPort, WorktreeProvisioner, ClockSource>
    DeleteTaskHandler<TaskRepositoryPort, WorktreeRepositoryPort, WorktreeProvisioner, ClockSource>
{
    pub fn new(
        task_repository: TaskRepositoryPort,
        worktree_repository: WorktreeRepositoryPort,
        worktree_provisioner: WorktreeProvisioner,
        project_id: ProjectId,
        clock: ClockSource,
    ) -> Self {
        Self {
            task_repository,
            worktree_repository,
            worktree_provisioner,
            project_id,
            clock,
        }
    }
}

impl<TaskRepositoryPort, WorktreeRepositoryPort, WorktreeProvisioner, ClockSource>
    RecoverPendingTaskWorktreesHandler<
        TaskRepositoryPort,
        WorktreeRepositoryPort,
        WorktreeProvisioner,
        ClockSource,
    >
{
    /// Builds the recovery use case from the same dependencies as normal task deletion.
    pub fn new(
        task_repository: TaskRepositoryPort,
        worktree_repository: WorktreeRepositoryPort,
        worktree_provisioner: WorktreeProvisioner,
        project_id: ProjectId,
        clock: ClockSource,
    ) -> Self {
        Self {
            task_repository,
            worktree_repository,
            worktree_provisioner,
            project_id,
            clock,
        }
    }
}

impl<TaskRepositoryPort, WorktreeRepositoryPort, WorktreeProvisioner, ClockSource>
    RecoverPendingTaskWorktreesHandler<
        TaskRepositoryPort,
        WorktreeRepositoryPort,
        WorktreeProvisioner,
        ClockSource,
    >
where
    TaskRepositoryPort: TaskRepository,
    WorktreeRepositoryPort: WorktreeRepository,
    WorktreeProvisioner: TaskWorktreeProvisioner,
    ClockSource: Clock,
{
    /// Reconciles interrupted provisioning and removal states while leaving failures visible.
    pub fn handle(&self) -> Result<TaskWorktreeRecoveryReport, ApplicationError> {
        let pending_worktrees = self
            .worktree_repository
            .list_worktrees()
            .map_err(ApplicationError::from_worktree_repository_error)?
            .into_iter()
            .filter(|worktree| {
                worktree.project_id == self.project_id
                    && worktree.lifecycle() != ora_domain::WorktreeLifecycle::Active
            });
        let mut report = TaskWorktreeRecoveryReport {
            recovered: 0,
            failed: 0,
        };

        for worktree in pending_worktrees {
            let task_id = worktree.task_id.clone();
            let result = match worktree.lifecycle() {
                DomainWorktreeLifecycle::ProvisioningPending => recover_pending_provisioning(
                    &self.task_repository,
                    &self.worktree_repository,
                    &self.worktree_provisioner,
                    &self.clock,
                    &worktree,
                ),
                DomainWorktreeLifecycle::RemovalPending => finalize_pending_worktree_removal(
                    &self.task_repository,
                    &self.worktree_repository,
                    &self.worktree_provisioner,
                    &self.clock,
                    &worktree,
                ),
                DomainWorktreeLifecycle::Active => Ok(()),
            };
            match result {
                Ok(()) => {
                    report.recovered += 1;
                    log_task_success("recover_task_worktree", Some(&task_id));
                }
                Err(error) => {
                    report.failed += 1;
                    log_task_failure("recover_task_worktree", Some(&task_id), &error);
                }
            }
        }

        Ok(report)
    }
}

impl<TaskRepositoryPort, WorktreeRepositoryPort, WorktreeProvisioner, ClockSource>
    DeleteTaskHandler<TaskRepositoryPort, WorktreeRepositoryPort, WorktreeProvisioner, ClockSource>
where
    TaskRepositoryPort: TaskRepository,
    WorktreeRepositoryPort: WorktreeRepository,
    WorktreeProvisioner: TaskWorktreeProvisioner,
    ClockSource: Clock,
{
    /// Deletes one task and removes its owned linked worktree before returning the CRUD-shaped response.
    pub fn handle(
        &self,
        request: DeleteTaskRequest,
    ) -> Result<DeleteTaskResponse, ApplicationError> {
        let task_id = TaskId::new(request.task_id);
        let existing_task = self.task_repository.find_task(&task_id).map_err(|error| {
            let error = ApplicationError::from_task_repository_error(error);
            log_task_failure("delete_task", Some(&task_id), &error);
            error
        })?;
        let existing_task = match existing_task {
            Some(task) => task,
            None => {
                let error = ApplicationError::TaskNotFound {
                    task_id: task_id.to_string(),
                };
                log_task_failure("delete_task", Some(&task_id), &error);
                return Err(error);
            }
        };
        if existing_task.project_id != self.project_id {
            return Err(ApplicationError::TaskProjectMismatch {
                expected_project_id: self.project_id.to_string(),
                actual_project_id: existing_task.project_id.to_string(),
            });
        }
        if let Some(worktree_id) = existing_task.worktree_id {
            let existing_worktree = self
                .worktree_repository
                .find_worktree(&worktree_id)
                .map_err(|error| {
                    let error = ApplicationError::from_worktree_repository_error(error);
                    log_task_failure("delete_task", Some(&task_id), &error);
                    error
                })?;
            let mut existing_worktree = match existing_worktree {
                Some(worktree) => worktree,
                None => {
                    let error = ApplicationError::WorktreeNotFound {
                        worktree_id: worktree_id.to_string(),
                    };
                    log_task_failure("delete_task", Some(&task_id), &error);
                    return Err(error);
                }
            };
            if existing_worktree.task_id != existing_task.id {
                let error = ApplicationError::TaskWorktree {
                    message: format!(
                        "worktree {} ownership does not match task {}",
                        existing_worktree.id, existing_task.id
                    ),
                };
                log_task_failure("delete_task", Some(&task_id), &error);
                return Err(error);
            }
            if existing_worktree.project_id != existing_task.project_id {
                return Err(ApplicationError::TaskProjectMismatch {
                    expected_project_id: existing_task.project_id.to_string(),
                    actual_project_id: existing_worktree.project_id.to_string(),
                });
            }
            managed_worktree_identity(&existing_worktree)?;
            if existing_worktree.lifecycle() != ora_domain::WorktreeLifecycle::RemovalPending {
                existing_worktree.set_lifecycle(
                    DomainWorktreeLifecycle::RemovalPending,
                    self.clock.now_timestamp_millis(),
                );
                existing_worktree = self
                    .worktree_repository
                    .update_worktree(existing_worktree)
                    .map_err(|error| {
                        let error = ApplicationError::from_worktree_repository_error(error);
                        log_task_failure("delete_task", Some(&task_id), &error);
                        error
                    })?;
            }
            finalize_pending_worktree_removal(
                &self.task_repository,
                &self.worktree_repository,
                &self.worktree_provisioner,
                &self.clock,
                &existing_worktree,
            )
            .inspect_err(|error| log_task_failure("delete_task", Some(&task_id), error))?;

            log_task_success("delete_task", Some(&task_id));
            return Ok(DeleteTaskResponse {
                task_id: task_id.to_string(),
            });
        }

        let deleted = self
            .task_repository
            .soft_delete_task(&task_id, self.clock.now_timestamp_millis())
            .map_err(|error| {
                let error = ApplicationError::from_task_repository_error(error);
                log_task_failure("delete_task", Some(&task_id), &error);
                error
            })?;

        if deleted {
            log_task_success("delete_task", Some(&task_id));

            Ok(DeleteTaskResponse {
                task_id: task_id.to_string(),
            })
        } else {
            let error = ApplicationError::TaskNotFound {
                task_id: task_id.to_string(),
            };
            log_task_failure("delete_task", Some(&task_id), &error);
            Err(error)
        }
    }
}

/// Completes the retry-safe portion of deletion after durable state is `RemovalPending`.
fn finalize_pending_worktree_removal<
    TaskRepositoryPort,
    WorktreeRepositoryPort,
    WorktreeProvisioner,
    ClockSource,
>(
    task_repository: &TaskRepositoryPort,
    worktree_repository: &WorktreeRepositoryPort,
    worktree_provisioner: &WorktreeProvisioner,
    clock: &ClockSource,
    worktree: &DomainWorktree,
) -> Result<(), ApplicationError>
where
    TaskRepositoryPort: TaskRepository,
    WorktreeRepositoryPort: WorktreeRepository,
    WorktreeProvisioner: TaskWorktreeProvisioner,
    ClockSource: Clock,
{
    let (worktree_path, branch_name) = managed_worktree_identity(worktree)?;
    worktree_provisioner
        .delete_task_worktree(DeleteTaskWorktreeRequest {
            expected_worktree_id: worktree.id.clone(),
            worktree_path: worktree_path.to_path_buf(),
            expected_branch_name: branch_name.to_string(),
            mode: TaskWorktreeDeletionMode::Force,
        })
        .map_err(ApplicationError::from_task_worktree_provisioner_error)?;

    let deleted_at = clock.now_timestamp_millis();
    task_repository
        .soft_delete_task(&worktree.task_id, deleted_at)
        .map_err(ApplicationError::from_task_repository_error)?;
    worktree_repository
        .soft_delete_worktree(&worktree.id, deleted_at)
        .map_err(ApplicationError::from_worktree_repository_error)?;

    Ok(())
}

/// Resolves a pending creation by activating an owned task or cleaning an orphaned checkout.
fn recover_pending_provisioning<
    TaskRepositoryPort,
    WorktreeRepositoryPort,
    WorktreeProvisioner,
    ClockSource,
>(
    task_repository: &TaskRepositoryPort,
    worktree_repository: &WorktreeRepositoryPort,
    worktree_provisioner: &WorktreeProvisioner,
    clock: &ClockSource,
    worktree: &DomainWorktree,
) -> Result<(), ApplicationError>
where
    TaskRepositoryPort: TaskRepository,
    WorktreeRepositoryPort: WorktreeRepository,
    WorktreeProvisioner: TaskWorktreeProvisioner,
    ClockSource: Clock,
{
    let task = task_repository
        .find_task(&worktree.task_id)
        .map_err(ApplicationError::from_task_repository_error)?;
    if let Some(task) = task {
        if task.project_id != worktree.project_id
            || task.worktree_id.as_ref() != Some(&worktree.id)
            || worktree.baseline.commit_id().is_none()
        {
            return Err(ApplicationError::TaskWorktree {
                message: format!(
                    "pending worktree {} does not match its persisted task",
                    worktree.id
                ),
            });
        }

        let (worktree_path, branch_name) = managed_worktree_identity(worktree)?;
        worktree_provisioner
            .verify_task_worktree(VerifyTaskWorktreeRequest {
                expected_worktree_id: worktree.id.clone(),
                worktree_path: worktree_path.to_path_buf(),
                expected_branch_name: branch_name.to_string(),
            })
            .map_err(ApplicationError::from_task_worktree_provisioner_error)?;

        let mut active_worktree = worktree.clone();
        active_worktree.set_lifecycle(
            DomainWorktreeLifecycle::Active,
            clock.now_timestamp_millis(),
        );
        worktree_repository
            .update_worktree(active_worktree)
            .map_err(ApplicationError::from_worktree_repository_error)?;
        return Ok(());
    }

    let (worktree_path, branch_name) = managed_worktree_identity(worktree)?;
    worktree_provisioner
        .delete_task_worktree(DeleteTaskWorktreeRequest {
            expected_worktree_id: worktree.id.clone(),
            worktree_path: worktree_path.to_path_buf(),
            expected_branch_name: branch_name.to_string(),
            mode: TaskWorktreeDeletionMode::Force,
        })
        .map_err(ApplicationError::from_task_worktree_provisioner_error)?;
    worktree_repository
        .soft_delete_worktree(&worktree.id, clock.now_timestamp_millis())
        .map_err(ApplicationError::from_worktree_repository_error)?;

    Ok(())
}

/// Returns the persisted identity required before any filesystem mutation is allowed.
fn managed_worktree_identity(worktree: &DomainWorktree) -> Result<(&Path, &str), ApplicationError> {
    match (worktree.root(), worktree.branch_name()) {
        (Some(root), Some(branch_name)) => Ok((root, branch_name)),
        (None, None) => Err(ApplicationError::TaskWorktree {
            message: format!(
                "worktree {} predates trusted root and branch persistence",
                worktree.id
            ),
        }),
        (Some(_), None) | (None, Some(_)) => Err(ApplicationError::TaskWorktree {
            message: format!(
                "worktree {} has an incomplete trusted identity",
                worktree.id
            ),
        }),
    }
}

/// Emits the shared informational event shape for successful task CRUD operations.
fn log_task_success(operation: &'static str, task_id: Option<&TaskId>) {
    match task_id {
        Some(task_id) => {
            ora_info!(
                message = "task operation completed",
                operation,
                task_id = task_id.to_string()
            );
        }
        None => {
            ora_info!(message = "task operation completed", operation);
        }
    }
}

/// Emits the shared error event shape for failed task CRUD operations.
fn log_task_failure(operation: &'static str, task_id: Option<&TaskId>, error: &ApplicationError) {
    match (task_id, error) {
        (Some(task_id), ApplicationError::TaskNotFound { .. }) => {
            ora_error!(
                message = "task operation failed",
                operation,
                task_id = task_id.to_string(),
                error.kind = "task_not_found",
                error.message = error.to_string()
            );
        }
        (Some(task_id), ApplicationError::TaskRepository { .. }) => {
            ora_error!(
                message = "task operation failed",
                operation,
                task_id = task_id.to_string(),
                error.kind = "task_repository",
                error.message = error.to_string()
            );
        }
        (Some(task_id), ApplicationError::TaskWorktree { .. }) => {
            ora_error!(
                message = "task operation failed",
                operation,
                task_id = task_id.to_string(),
                error.kind = "task_worktree",
                error.message = error.to_string()
            );
        }
        (Some(task_id), ApplicationError::WorktreeNotFound { .. }) => {
            ora_error!(
                message = "task operation failed",
                operation,
                task_id = task_id.to_string(),
                error.kind = "worktree_not_found",
                error.message = error.to_string()
            );
        }
        (Some(task_id), ApplicationError::WorktreeRepository { .. }) => {
            ora_error!(
                message = "task operation failed",
                operation,
                task_id = task_id.to_string(),
                error.kind = "worktree_repository",
                error.message = error.to_string()
            );
        }
        (None, ApplicationError::TaskRepository { .. }) => {
            ora_error!(
                message = "task operation failed",
                operation,
                error.kind = "task_repository",
                error.message = error.to_string()
            );
        }
        (None, ApplicationError::TaskNotFound { .. }) => {
            ora_error!(
                message = "task operation failed",
                operation,
                error.kind = "task_not_found",
                error.message = error.to_string()
            );
        }
        (None, ApplicationError::TaskWorktree { .. }) => {
            ora_error!(
                message = "task operation failed",
                operation,
                error.kind = "task_worktree",
                error.message = error.to_string()
            );
        }
        _ => {}
    }
}

/// Translates the transport-facing task status into the domain enum.
fn map_contract_task_status(status: TaskStatus) -> DomainTaskStatus {
    match status {
        TaskStatus::Todo => DomainTaskStatus::Todo,
        TaskStatus::Doing => DomainTaskStatus::Doing,
        TaskStatus::Done => DomainTaskStatus::Done,
    }
}

/// Derives the stable task branch name from the first eight characters of the generated task id.
fn branch_name_for_task(task_id: &TaskId) -> String {
    format!("ora/{}", task_branch_prefix(task_id))
}

/// Derives the short branch prefix used to keep task branch names readable.
fn task_branch_prefix(task_id: &TaskId) -> String {
    task_id
        .to_string()
        .chars()
        .take(TASK_BRANCH_PREFIX_LEN)
        .collect()
}

/// Derives the owned linked-worktree path from the configured worktree root and full task id.
fn worktree_path_for_task(work_dir: &Path, task_id: &TaskId) -> PathBuf {
    work_dir.join(task_id.to_string())
}

/// Checks existing task worktree folders before branch creation because branch names use short ids.
fn task_branch_prefix_exists_in_work_dir(
    work_dir: &Path,
    branch_prefix: &str,
) -> Result<bool, ApplicationError> {
    let entries = match fs::read_dir(work_dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(_) => {
            return Err(ApplicationError::TaskWorktree {
                message: "failed to inspect task worktree directory".to_string(),
            });
        }
    };

    for entry in entries {
        let entry = entry.map_err(|_| ApplicationError::TaskWorktree {
            message: "failed to inspect task worktree directory".to_string(),
        })?;
        let file_type = entry
            .file_type()
            .map_err(|_| ApplicationError::TaskWorktree {
                message: "failed to inspect task worktree directory".to_string(),
            })?;

        if file_type.is_dir()
            && entry
                .file_name()
                .to_string_lossy()
                .starts_with(branch_prefix)
        {
            return Ok(true);
        }
    }

    Ok(false)
}

#[cfg(test)]
mod task_branch_prefix_tests {
    use super::task_branch_prefix_exists_in_work_dir;
    use pretty_assertions::assert_eq;
    use std::fs;
    use std::path::PathBuf;

    /// Verifies an absent worktree root is treated as the first task creation, not an inspection failure.
    #[test]
    fn reports_no_collision_when_work_dir_does_not_exist() {
        let work_dir = unique_test_work_dir("missing");

        assert_eq!(
            task_branch_prefix_exists_in_work_dir(&work_dir, "12345678"),
            Ok(false)
        );
    }

    /// Verifies only directories with the requested prefix reserve a task branch prefix.
    #[test]
    fn detects_matching_directory_prefixes() {
        let work_dir = unique_test_work_dir("matching-directory");
        fs::create_dir_all(work_dir.join("12345678-existing"))
            .unwrap_or_else(|error| panic!("failed to create collision fixture: {error}"));

        assert_eq!(
            task_branch_prefix_exists_in_work_dir(&work_dir, "12345678"),
            Ok(true)
        );

        fs::remove_dir_all(&work_dir)
            .unwrap_or_else(|error| panic!("failed to remove collision fixture: {error}"));
    }

    /// Verifies ordinary files and unrelated directories do not reserve a task branch prefix.
    #[test]
    fn ignores_files_and_unrelated_directories() {
        let work_dir = unique_test_work_dir("unrelated-entries");
        fs::create_dir_all(work_dir.join("87654321-existing"))
            .unwrap_or_else(|error| panic!("failed to create unrelated directory: {error}"));
        fs::write(work_dir.join("12345678-file"), b"not a worktree")
            .unwrap_or_else(|error| panic!("failed to create ordinary file: {error}"));

        assert_eq!(
            task_branch_prefix_exists_in_work_dir(&work_dir, "12345678"),
            Ok(false)
        );

        fs::remove_dir_all(&work_dir)
            .unwrap_or_else(|error| panic!("failed to remove unrelated entries: {error}"));
    }

    /// Builds an isolated filesystem location for branch-prefix unit tests.
    fn unique_test_work_dir(name: &str) -> PathBuf {
        let work_dir = std::env::temp_dir().join(format!(
            "ora-application-prefix-unit-{name}-{}",
            std::process::id()
        ));
        if work_dir.exists() {
            fs::remove_dir_all(&work_dir)
                .unwrap_or_else(|error| panic!("failed to reset unit-test work dir: {error}"));
        }
        work_dir
    }
}
