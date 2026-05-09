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
use ora_logging::{
    FileLoggingConfig, LogLevel, LogOutput, LoggingConfig, LoggingGuard, RotationPolicy,
    init_logging,
};
use std::cell::RefCell;
use std::env;
use std::num::NonZeroUsize;
use std::rc::Rc;
use thiserror::Error;

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

    /// Accepts a create-project contract request and delegates the use case to the application layer.
    pub fn create_project(
        &self,
        request: CreateProjectRequest,
    ) -> Result<CreateProjectResponse, ApplicationError> {
        self.create_project.handle(request)
    }

    /// Accepts a get-project contract request and delegates the use case to the application layer.
    pub fn get_project(
        &self,
        request: GetProjectRequest,
    ) -> Result<GetProjectResponse, ApplicationError> {
        self.get_project.handle(request)
    }

    /// Accepts a list-projects contract request and delegates the use case to the application layer.
    pub fn list_projects(
        &self,
        request: ListProjectsRequest,
    ) -> Result<ListProjectsResponse, ApplicationError> {
        self.list_projects.handle(request)
    }

    /// Accepts an update-project contract request and delegates the use case to the application layer.
    pub fn update_project(
        &self,
        request: UpdateProjectRequest,
    ) -> Result<UpdateProjectResponse, ApplicationError> {
        self.update_project.handle(request)
    }

    /// Accepts a delete-project contract request and delegates the use case to the application layer.
    pub fn delete_project(
        &self,
        request: DeleteProjectRequest,
    ) -> Result<DeleteProjectResponse, ApplicationError> {
        self.delete_project.handle(request)
    }
}

#[derive(Debug, Default)]
struct BootstrapProjectRepository {
    projects: RefCell<Vec<Project>>,
}

impl BootstrapProjectRepository {
    /// Returns the visible project snapshot for one identifier.
    fn find_visible_project(&self, project_id: &ProjectId) -> Option<Project> {
        self.projects
            .borrow()
            .iter()
            .find(|project| project.id == *project_id && !project.audit_fields.is_deleted)
            .cloned()
    }
}

#[derive(Clone, Debug, Default)]
struct BootstrapProjectRepositoryHandle {
    repository: Rc<BootstrapProjectRepository>,
}

impl BootstrapProjectRepositoryHandle {
    /// Creates a shared repository handle that can be cloned across application handlers.
    fn new() -> Self {
        Self {
            repository: Rc::new(BootstrapProjectRepository::default()),
        }
    }
}

impl ProjectRepository for BootstrapProjectRepositoryHandle {
    fn create_project(&self, project: Project) -> Result<Project, ProjectRepositoryError> {
        self.repository.projects.borrow_mut().push(project.clone());
        Ok(project)
    }

    fn find_project(
        &self,
        project_id: &ProjectId,
    ) -> Result<Option<Project>, ProjectRepositoryError> {
        Ok(self.repository.find_visible_project(project_id))
    }

    fn list_projects(&self) -> Result<Vec<Project>, ProjectRepositoryError> {
        Ok(self
            .repository
            .projects
            .borrow()
            .iter()
            .filter(|project| !project.audit_fields.is_deleted)
            .cloned()
            .collect())
    }

    fn update_project(&self, project: Project) -> Result<Project, ProjectRepositoryError> {
        let mut projects = self.repository.projects.borrow_mut();
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

    fn soft_delete_project(
        &self,
        project_id: &ProjectId,
        deleted_at: i64,
    ) -> Result<bool, ProjectRepositoryError> {
        let mut projects = self.repository.projects.borrow_mut();
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

#[derive(Clone, Copy, Debug)]
struct BootstrapClock;

impl Clock for BootstrapClock {
    fn now_timestamp_millis(&self) -> i64 {
        0
    }
}

/// Builds the minimal project API wiring so the web adapter can stay transport-focused.
fn build_web_project_api()
-> WebProjectApi<BootstrapProjectRepositoryHandle, UuidProjectIdGenerator, BootstrapClock> {
    WebProjectApi::new(
        BootstrapProjectRepositoryHandle::new(),
        UuidProjectIdGenerator::new(),
        BootstrapClock,
    )
}

/// Loads the logging configuration from the environment contract defined for the web server bootstrap.
fn read_logging_config() -> Result<LoggingConfig, WebBootstrapError> {
    let level = match env::var("ORA_LOG_LEVEL")
        .unwrap_or_else(|_| "info".to_string())
        .to_ascii_lowercase()
        .as_str()
    {
        "debug" => LogLevel::Debug,
        "info" => LogLevel::Info,
        "warn" => LogLevel::Warn,
        "error" => LogLevel::Error,
        value => {
            return Err(WebBootstrapError::InvalidLogLevel {
                value: value.to_string(),
            });
        }
    };
    let file_config = FileLoggingConfig::new(
        env::var("ORA_LOG_PATH").unwrap_or_else(|_| "./ora.log".to_string()),
        RotationPolicy::Daily,
        read_log_max_days()?,
    );
    let output = match env::var("ORA_LOG_MODE")
        .unwrap_or_else(|_| "stdout".to_string())
        .to_ascii_lowercase()
        .as_str()
    {
        "stdout" => LogOutput::Stdout,
        "file" => LogOutput::File(file_config),
        "stdout_and_file" => LogOutput::StdoutAndFile(file_config),
        value => {
            return Err(WebBootstrapError::InvalidLogMode {
                value: value.to_string(),
            });
        }
    };

    Ok(LoggingConfig::new(level, output))
}

/// Parses the configured retention window and rejects zero-day values explicitly.
fn read_log_max_days() -> Result<NonZeroUsize, WebBootstrapError> {
    let raw_value = env::var("ORA_LOG_MAX_DAYS").unwrap_or_else(|_| "3".to_string());
    let parsed_value =
        raw_value
            .parse::<usize>()
            .map_err(|source| WebBootstrapError::InvalidLogMaxDays {
                value: raw_value.clone(),
                source,
            })?;

    NonZeroUsize::new(parsed_value).ok_or(WebBootstrapError::InvalidLogMaxDaysZero)
}

/// Boots the web adapter with shared logging plus the application-layer project API wiring.
fn main() -> Result<(), WebBootstrapError> {
    let _logging_guard = initialize_logging()?;
    let _web_project_api = build_web_project_api();

    Ok(())
}

/// Initializes structured logging during process bootstrap and returns the guard that owns writer lifetimes.
fn initialize_logging() -> Result<LoggingGuard, WebBootstrapError> {
    let config = read_logging_config()?;

    init_logging(config).map_err(WebBootstrapError::LoggingInit)
}

/// Reports bootstrap-time configuration and logging failures for the web server entry point.
#[derive(Debug, Error)]
enum WebBootstrapError {
    #[error("invalid ORA_LOG_LEVEL value `{value}`")]
    InvalidLogLevel { value: String },
    #[error("invalid ORA_LOG_MODE value `{value}`")]
    InvalidLogMode { value: String },
    #[error("invalid ORA_LOG_MAX_DAYS value `{value}`")]
    InvalidLogMaxDays {
        value: String,
        #[source]
        source: std::num::ParseIntError,
    },
    #[error("ORA_LOG_MAX_DAYS must be greater than zero")]
    InvalidLogMaxDaysZero,
    #[error(transparent)]
    LoggingInit(#[from] ora_logging::LoggingInitError),
}
