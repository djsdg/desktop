use crate::ApplicationError;
use crate::project::mapper::map_project;
use crate::project::ports::{Clock, ProjectIdGenerator, ProjectRepository};
use ora_contracts::{
    CreateProjectRequest, CreateProjectResponse, DeleteProjectRequest, DeleteProjectResponse,
    GetProjectRequest, GetProjectResponse, ListProjectsRequest, ListProjectsResponse,
    UpdateProjectRequest, UpdateProjectResponse,
};
use ora_domain::{AuditFields, Project as DomainProject, ProjectId};
use ora_logging::{ora_error, ora_info};

/// Handles project creation without depending on transport-specific concerns.
pub struct CreateProjectHandler<Repository, IdGenerator, ClockSource> {
    repository: Repository,
    id_generator: IdGenerator,
    clock: ClockSource,
}

impl<Repository, IdGenerator, ClockSource>
    CreateProjectHandler<Repository, IdGenerator, ClockSource>
{
    pub fn new(repository: Repository, id_generator: IdGenerator, clock: ClockSource) -> Self {
        Self {
            repository,
            id_generator,
            clock,
        }
    }
}

impl<Repository, IdGenerator, ClockSource>
    CreateProjectHandler<Repository, IdGenerator, ClockSource>
where
    Repository: ProjectRepository,
    IdGenerator: ProjectIdGenerator,
    ClockSource: Clock,
{
    /// Creates a new project snapshot and returns the public response payload.
    pub fn handle(
        &self,
        request: CreateProjectRequest,
    ) -> Result<CreateProjectResponse, ApplicationError> {
        let now = self.clock.now_timestamp_millis();
        let project = DomainProject::new(
            self.id_generator.generate_project_id(),
            request.name,
            request.root_path,
            AuditFields::new(now, now, false),
        );
        let project = self.repository.create_project(project).map_err(|error| {
            let error = ApplicationError::from_project_repository_error(error);
            log_project_failure("create_project", None, &error);
            error
        })?;

        log_project_success("create_project", Some(&project.id));

        Ok(CreateProjectResponse {
            project: map_project(project),
        })
    }
}

/// Handles one project lookup without depending on transport-specific concerns.
pub struct GetProjectHandler<Repository> {
    repository: Repository,
}

impl<Repository> GetProjectHandler<Repository> {
    pub fn new(repository: Repository) -> Self {
        Self { repository }
    }
}

impl<Repository> GetProjectHandler<Repository>
where
    Repository: ProjectRepository,
{
    /// Loads one visible project or returns a stable not-found application error.
    pub fn handle(
        &self,
        request: GetProjectRequest,
    ) -> Result<GetProjectResponse, ApplicationError> {
        let project_id = ProjectId::new(request.project_id);
        let project = self.repository.find_project(&project_id).map_err(|error| {
            let error = ApplicationError::from_project_repository_error(error);
            log_project_failure("get_project", Some(&project_id), &error);
            error
        })?;

        match project {
            Some(project) => {
                log_project_success("get_project", Some(&project_id));

                Ok(GetProjectResponse {
                    project: map_project(project),
                })
            }
            None => {
                let error = ApplicationError::ProjectNotFound {
                    project_id: project_id.to_string(),
                };
                log_project_failure("get_project", Some(&project_id), &error);
                Err(error)
            }
        }
    }
}

/// Handles project listing without depending on transport-specific concerns.
pub struct ListProjectsHandler<Repository> {
    repository: Repository,
}

impl<Repository> ListProjectsHandler<Repository> {
    pub fn new(repository: Repository) -> Self {
        Self { repository }
    }
}

impl<Repository> ListProjectsHandler<Repository>
where
    Repository: ProjectRepository,
{
    /// Lists every visible project and maps each one into the shared contract view.
    pub fn handle(
        &self,
        _request: ListProjectsRequest,
    ) -> Result<ListProjectsResponse, ApplicationError> {
        let projects = self.repository.list_projects().map_err(|error| {
            let error = ApplicationError::from_project_repository_error(error);
            log_project_failure("list_projects", None, &error);
            error
        })?;

        ora_info!(
            message = "listed projects",
            operation = "list_projects",
            project_count = projects.len()
        );

        Ok(ListProjectsResponse {
            projects: projects.into_iter().map(map_project).collect(),
        })
    }
}

/// Handles project updates without depending on transport-specific concerns.
pub struct UpdateProjectHandler<Repository, ClockSource> {
    repository: Repository,
    clock: ClockSource,
}

impl<Repository, ClockSource> UpdateProjectHandler<Repository, ClockSource> {
    pub fn new(repository: Repository, clock: ClockSource) -> Self {
        Self { repository, clock }
    }
}

impl<Repository, ClockSource> UpdateProjectHandler<Repository, ClockSource>
where
    Repository: ProjectRepository,
    ClockSource: Clock,
{
    /// Replaces the public project fields while preserving persistence-managed audit state.
    pub fn handle(
        &self,
        request: UpdateProjectRequest,
    ) -> Result<UpdateProjectResponse, ApplicationError> {
        let project_id = ProjectId::new(request.project_id);
        let existing_project = self.repository.find_project(&project_id).map_err(|error| {
            let error = ApplicationError::from_project_repository_error(error);
            log_project_failure("update_project", Some(&project_id), &error);
            error
        })?;

        let existing_project = match existing_project {
            Some(existing_project) => existing_project,
            None => {
                let error = ApplicationError::ProjectNotFound {
                    project_id: project_id.to_string(),
                };
                log_project_failure("update_project", Some(&project_id), &error);
                return Err(error);
            }
        };

        let project = DomainProject::new(
            project_id.clone(),
            request.name,
            request.root_path,
            AuditFields::new(
                existing_project.audit_fields.created_at,
                self.clock.now_timestamp_millis(),
                existing_project.audit_fields.is_deleted,
            ),
        );
        let project = self.repository.update_project(project).map_err(|error| {
            let error = ApplicationError::from_project_repository_error(error);
            log_project_failure("update_project", Some(&project_id), &error);
            error
        })?;

        log_project_success("update_project", Some(&project_id));

        Ok(UpdateProjectResponse {
            project: map_project(project),
        })
    }
}

/// Handles project deletion without exposing storage-specific soft-delete semantics.
pub struct DeleteProjectHandler<Repository, ClockSource> {
    repository: Repository,
    clock: ClockSource,
}

impl<Repository, ClockSource> DeleteProjectHandler<Repository, ClockSource> {
    pub fn new(repository: Repository, clock: ClockSource) -> Self {
        Self { repository, clock }
    }
}

impl<Repository, ClockSource> DeleteProjectHandler<Repository, ClockSource>
where
    Repository: ProjectRepository,
    ClockSource: Clock,
{
    /// Deletes one project through a CRUD-shaped contract while letting storage soft-delete it.
    pub fn handle(
        &self,
        request: DeleteProjectRequest,
    ) -> Result<DeleteProjectResponse, ApplicationError> {
        let project_id = ProjectId::new(request.project_id);
        let deleted = self
            .repository
            .soft_delete_project(&project_id, self.clock.now_timestamp_millis())
            .map_err(|error| {
                let error = ApplicationError::from_project_repository_error(error);
                log_project_failure("delete_project", Some(&project_id), &error);
                error
            })?;

        if deleted {
            log_project_success("delete_project", Some(&project_id));

            Ok(DeleteProjectResponse {
                project_id: project_id.to_string(),
            })
        } else {
            let error = ApplicationError::ProjectNotFound {
                project_id: project_id.to_string(),
            };
            log_project_failure("delete_project", Some(&project_id), &error);
            Err(error)
        }
    }
}

/// Emits the shared informational event shape for successful project CRUD operations.
fn log_project_success(operation: &'static str, project_id: Option<&ProjectId>) {
    match project_id {
        Some(project_id) => {
            ora_info!(
                message = "project operation completed",
                operation,
                project_id = project_id.to_string()
            );
        }
        None => {
            ora_info!(message = "project operation completed", operation);
        }
    }
}

/// Emits the shared error event shape for failed project CRUD operations.
fn log_project_failure(
    operation: &'static str,
    project_id: Option<&ProjectId>,
    error: &ApplicationError,
) {
    match (project_id, error) {
        (Some(project_id), ApplicationError::ProjectNotFound { .. }) => {
            ora_error!(
                message = "project operation failed",
                operation,
                project_id = project_id.to_string(),
                error.kind = "project_not_found",
                error.message = error.to_string()
            );
        }
        (Some(project_id), ApplicationError::ProjectRepository { .. }) => {
            ora_error!(
                message = "project operation failed",
                operation,
                project_id = project_id.to_string(),
                error.kind = "project_repository",
                error.message = error.to_string()
            );
        }
        (None, ApplicationError::ProjectRepository { .. }) => {
            ora_error!(
                message = "project operation failed",
                operation,
                error.kind = "project_repository",
                error.message = error.to_string()
            );
        }
        (None, ApplicationError::ProjectNotFound { .. }) => {
            ora_error!(
                message = "project operation failed",
                operation,
                error.kind = "project_not_found",
                error.message = error.to_string()
            );
        }
    }
}
