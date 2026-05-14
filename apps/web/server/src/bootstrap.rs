use ora_application::{
    ApplicationError, Clock, CreateProjectHandler, DeleteProjectHandler, GetProjectHandler,
    ListProjectsHandler, ProjectIdGenerator, ProjectRepository, ProjectRepositoryError,
    UpdateProjectHandler, UuidProjectIdGenerator,
};
use ora_contracts::{
    CreateProjectRequest, CreateProjectResponse, DeleteProjectRequest, DeleteProjectResponse,
    GetProjectRequest, GetProjectResponse, ListProjectsRequest, ListProjectsResponse,
    UpdateProjectRequest, UpdateProjectResponse,
};
use ora_domain::{Project, ProjectId};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::app_state::AppState;

/// Aliases the bootstrap-time project API wiring used by the first HTTP server slice.
pub type BootstrapProjectApi =
    WebProjectApi<BootstrapProjectRepositoryHandle, UuidProjectIdGenerator, SystemClock>;

/// Groups the transport-facing project entry points for the web adapter.
pub struct WebProjectApi<Repository, IdGenerator, ClockSource> {
    create_project: CreateProjectHandler<Repository, IdGenerator, ClockSource>,
    get_project: GetProjectHandler<Repository>,
    list_projects: ListProjectsHandler<Repository>,
    update_project: UpdateProjectHandler<Repository, ClockSource>,
    delete_project: DeleteProjectHandler<Repository, ClockSource>,
}

impl<Repository, IdGenerator, ClockSource> WebProjectApi<Repository, IdGenerator, ClockSource>
where
    Repository: ProjectRepository + Clone,
    IdGenerator: ProjectIdGenerator + Clone,
    ClockSource: Clock + Clone,
{
    /// Builds one transport-facing project API from application-layer handlers and shared dependencies.
    pub fn new(repository: Repository, id_generator: IdGenerator, clock: ClockSource) -> Self {
        Self {
            create_project: CreateProjectHandler::new(
                repository.clone(),
                id_generator,
                clock.clone(),
            ),
            get_project: GetProjectHandler::new(repository.clone()),
            list_projects: ListProjectsHandler::new(repository.clone()),
            update_project: UpdateProjectHandler::new(repository.clone(), clock.clone()),
            delete_project: DeleteProjectHandler::new(repository, clock),
        }
    }

    /// Accepts a create-project request and delegates the use case to the application layer.
    pub fn create_project(
        &self,
        request: CreateProjectRequest,
    ) -> Result<CreateProjectResponse, ApplicationError> {
        self.create_project.handle(request)
    }

    /// Accepts a get-project request and delegates the use case to the application layer.
    pub fn get_project(
        &self,
        request: GetProjectRequest,
    ) -> Result<GetProjectResponse, ApplicationError> {
        self.get_project.handle(request)
    }

    /// Accepts a list-projects request and delegates the use case to the application layer.
    pub fn list_projects(
        &self,
        request: ListProjectsRequest,
    ) -> Result<ListProjectsResponse, ApplicationError> {
        self.list_projects.handle(request)
    }

    /// Accepts an update-project request and delegates the use case to the application layer.
    pub fn update_project(
        &self,
        request: UpdateProjectRequest,
    ) -> Result<UpdateProjectResponse, ApplicationError> {
        self.update_project.handle(request)
    }

    /// Accepts a delete-project request and delegates the use case to the application layer.
    pub fn delete_project(
        &self,
        request: DeleteProjectRequest,
    ) -> Result<DeleteProjectResponse, ApplicationError> {
        self.delete_project.handle(request)
    }
}

/// Builds the application state used by the web runtime from the bootstrap-only dependencies.
pub fn build_app_state() -> AppState {
    AppState::new(Arc::new(build_web_project_api()))
}

/// Builds the minimal project API wiring so the web adapter can stay transport-focused.
fn build_web_project_api() -> BootstrapProjectApi {
    WebProjectApi::new(
        BootstrapProjectRepositoryHandle::new(),
        UuidProjectIdGenerator::new(),
        SystemClock,
    )
}

/// Stores the in-memory project snapshots used by the bootstrap runtime.
#[derive(Debug, Default)]
struct BootstrapProjectRepository {
    projects: Mutex<Vec<Project>>,
}

impl BootstrapProjectRepository {
    /// Returns the visible project snapshot for one identifier.
    fn find_visible_project(
        &self,
        project_id: &ProjectId,
    ) -> Result<Option<Project>, ProjectRepositoryError> {
        let projects = self.projects.lock().map_err(|error| {
            ProjectRepositoryError::OperationFailed(format!(
                "bootstrap project store poisoned: {error}"
            ))
        })?;

        Ok(projects
            .iter()
            .find(|project| project.id == *project_id && !project.audit_fields.is_deleted)
            .cloned())
    }
}

/// Wraps the in-memory project store in a cloneable handle that application handlers can share.
#[derive(Clone, Debug, Default)]
pub struct BootstrapProjectRepositoryHandle {
    repository: Arc<BootstrapProjectRepository>,
}

impl BootstrapProjectRepositoryHandle {
    /// Creates a shared repository handle that can be cloned across application handlers.
    fn new() -> Self {
        Self {
            repository: Arc::new(BootstrapProjectRepository::default()),
        }
    }
}

impl ProjectRepository for BootstrapProjectRepositoryHandle {
    /// Persists one new project snapshot in the bootstrap store.
    fn create_project(&self, project: Project) -> Result<Project, ProjectRepositoryError> {
        let mut projects = self.repository.projects.lock().map_err(|error| {
            ProjectRepositoryError::OperationFailed(format!(
                "bootstrap project store poisoned: {error}"
            ))
        })?;

        projects.push(project.clone());

        Ok(project)
    }

    /// Looks up one visible project snapshot by identifier.
    fn find_project(
        &self,
        project_id: &ProjectId,
    ) -> Result<Option<Project>, ProjectRepositoryError> {
        self.repository.find_visible_project(project_id)
    }

    /// Lists every visible project snapshot from the bootstrap store.
    fn list_projects(&self) -> Result<Vec<Project>, ProjectRepositoryError> {
        let projects = self.repository.projects.lock().map_err(|error| {
            ProjectRepositoryError::OperationFailed(format!(
                "bootstrap project store poisoned: {error}"
            ))
        })?;

        Ok(projects
            .iter()
            .filter(|project| !project.audit_fields.is_deleted)
            .cloned()
            .collect())
    }

    /// Replaces one visible project snapshot in the bootstrap store.
    fn update_project(&self, project: Project) -> Result<Project, ProjectRepositoryError> {
        let mut projects = self.repository.projects.lock().map_err(|error| {
            ProjectRepositoryError::OperationFailed(format!(
                "bootstrap project store poisoned: {error}"
            ))
        })?;

        if let Some(existing_project) = projects.iter_mut().find(|existing_project| {
            existing_project.id == project.id && !existing_project.audit_fields.is_deleted
        }) {
            *existing_project = project.clone();
            Ok(project)
        } else {
            Err(ProjectRepositoryError::OperationFailed(format!(
                "missing bootstrap project during update: {}",
                project.id
            )))
        }
    }

    /// Marks one project as deleted while preserving CRUD-shaped delete semantics for callers.
    fn soft_delete_project(
        &self,
        project_id: &ProjectId,
        deleted_at: i64,
    ) -> Result<bool, ProjectRepositoryError> {
        let mut projects = self.repository.projects.lock().map_err(|error| {
            ProjectRepositoryError::OperationFailed(format!(
                "bootstrap project store poisoned: {error}"
            ))
        })?;

        if let Some(project) = projects
            .iter_mut()
            .find(|project| project.id == *project_id && !project.audit_fields.is_deleted)
        {
            project.audit_fields.updated_at = deleted_at;
            project.audit_fields.is_deleted = true;
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

/// Reads the current wall-clock time for audit fields in the bootstrap runtime.
#[derive(Clone, Copy, Debug)]
pub(crate) struct SystemClock;

impl Clock for SystemClock {
    /// Returns the current Unix timestamp in milliseconds for handler audit fields.
    fn now_timestamp_millis(&self) -> i64 {
        match SystemTime::now().duration_since(UNIX_EPOCH) {
            Ok(duration) => duration.as_millis() as i64,
            Err(_) => 0,
        }
    }
}
