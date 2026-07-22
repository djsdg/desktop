use crate::app_state::AppState;
use crate::error::WebApiError;
use axum::Json;
use axum::extract::{Path, State};
use ora_contracts::{
    CreateTaskRequest, CreateTaskResponse, DeleteTaskRequest, DeleteTaskResponse, GetTaskRequest,
    GetTaskResponse, ListTasksRequest, ListTasksResponse, TaskStatus, UpdateTaskRequest,
    UpdateTaskResponse,
};
use serde::Deserialize;
use std::sync::Arc;

/// Carries the request path segment used by task identifier routes.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskPath {
    task_id: String,
}

/// Carries the HTTP body used for task update routes before the path identifier is applied.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateTaskBody {
    project_id: String,
    title: String,
    status: TaskStatus,
}

/// Creates one task by forwarding the request body into the application layer.
pub async fn create_task(
    State(app_state): State<AppState>,
    Json(request): Json<CreateTaskRequest>,
) -> Result<Json<CreateTaskResponse>, WebApiError> {
    let api = Arc::clone(app_state.task_api());
    run_blocking(move || api.create_task(request)).await
}

/// Loads one task by combining the path identifier into the contract request.
pub async fn get_task(
    State(app_state): State<AppState>,
    Path(path): Path<TaskPath>,
) -> Result<Json<GetTaskResponse>, WebApiError> {
    let api = Arc::clone(app_state.task_api());
    run_blocking(move || {
        api.get_task(GetTaskRequest {
            task_id: path.task_id,
        })
    })
    .await
}

/// Lists every visible task by delegating to the application handler.
pub async fn list_tasks(
    State(app_state): State<AppState>,
) -> Result<Json<ListTasksResponse>, WebApiError> {
    let api = Arc::clone(app_state.task_api());
    run_blocking(move || api.list_tasks(ListTasksRequest {})).await
}

/// Replaces one task by combining the route identifier with the JSON body payload.
pub async fn update_task(
    State(app_state): State<AppState>,
    Path(path): Path<TaskPath>,
    Json(body): Json<UpdateTaskBody>,
) -> Result<Json<UpdateTaskResponse>, WebApiError> {
    let api = Arc::clone(app_state.task_api());
    run_blocking(move || {
        api.update_task(UpdateTaskRequest {
            task_id: path.task_id,
            project_id: body.project_id,
            title: body.title,
            status: body.status,
        })
    })
    .await
}

/// Deletes one task by combining the path identifier into the contract request.
pub async fn delete_task(
    State(app_state): State<AppState>,
    Path(path): Path<TaskPath>,
) -> Result<Json<DeleteTaskResponse>, WebApiError> {
    let api = Arc::clone(app_state.task_api());
    run_blocking(move || {
        api.delete_task(DeleteTaskRequest {
            task_id: path.task_id,
        })
    })
    .await
}

/// Runs synchronous Git and SQLite task services outside Tokio's asynchronous worker pool.
async fn run_blocking<Response, Operation>(
    operation: Operation,
) -> Result<Json<Response>, WebApiError>
where
    Response: Send + 'static,
    Operation: FnOnce() -> Result<Response, ora_application::ApplicationError> + Send + 'static,
{
    tokio::task::spawn_blocking(operation)
        .await
        .map_err(|error| WebApiError::internal_error(format!("task worker failed: {error}")))?
        .map(Json)
        .map_err(WebApiError::from)
}

#[cfg(test)]
mod tests {
    use std::thread;

    use super::run_blocking;

    /// Verifies synchronous task services cannot execute on a single-threaded async worker.
    #[tokio::test(flavor = "current_thread")]
    async fn runs_task_services_on_the_blocking_pool() {
        let async_worker = thread::current().id();
        let result = run_blocking(move || {
            Ok::<_, ora_application::ApplicationError>(thread::current().id())
        })
        .await;
        let blocking_worker = match result {
            Ok(worker) => worker.0,
            Err(_) => panic!("blocking task service should complete"),
        };

        assert_ne!(blocking_worker, async_worker);
    }
}
