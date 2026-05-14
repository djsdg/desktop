use crate::app_state::AppState;
use crate::handlers::{health, projects};
use axum::Router;
use axum::routing::{get, post};

/// Builds the top-level router for health checks and project CRUD routes.
pub fn build_router(app_state: AppState) -> Router {
    Router::new()
        .route("/health/live", get(health::liveness))
        .route("/health/ready", get(health::readiness))
        .route(
            "/api/projects",
            post(projects::create_project).get(projects::list_projects),
        )
        .route(
            "/api/projects/{project_id}",
            get(projects::get_project)
                .put(projects::update_project)
                .delete(projects::delete_project),
        )
        .with_state(app_state)
}

#[cfg(test)]
mod tests {
    use super::build_router;
    use crate::bootstrap::build_app_state;
    use axum::body::{Body, to_bytes};
    use axum::http::{Method, Request, StatusCode};
    use pretty_assertions::assert_eq;
    use serde_json::{Value, json};
    use tower::util::ServiceExt;

    /// Verifies the liveness route reports process health without bootstrap state.
    #[tokio::test]
    async fn serves_liveness_route() {
        let app = test_router();
        let response = match app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/health/live")
                    .body(Body::empty())
                    .unwrap_or_else(|error| panic!("failed to build request: {error}")),
            )
            .await
        {
            Ok(response) => response,
            Err(error) => panic!("request failed: {error}"),
        };

        assert_eq!(response.status(), StatusCode::OK);
    }

    /// Verifies readiness stays unavailable until bootstrap marks the state as ready.
    #[tokio::test]
    async fn serves_unready_status_before_bootstrap_completion() {
        let app_state = build_app_state();
        let app = build_router(app_state);
        let response = match app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/health/ready")
                    .body(Body::empty())
                    .unwrap_or_else(|error| panic!("failed to build request: {error}")),
            )
            .await
        {
            Ok(response) => response,
            Err(error) => panic!("request failed: {error}"),
        };

        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    /// Verifies the router supports the first project CRUD slice end to end.
    #[tokio::test]
    async fn serves_project_crud_routes() {
        let app = test_router();
        let create_response = match app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/projects")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({
                            "name": "Ora",
                            "rootPath": "/workspace/ora",
                        })
                        .to_string(),
                    ))
                    .unwrap_or_else(|error| panic!("failed to build request: {error}")),
            )
            .await
        {
            Ok(response) => response,
            Err(error) => panic!("request failed: {error}"),
        };
        let created_project = response_json(create_response).await["project"].clone();
        let project_id = match created_project["id"].as_str() {
            Some(project_id) => project_id.to_string(),
            None => panic!("response did not include a project id"),
        };
        let list_response = match app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/api/projects")
                    .body(Body::empty())
                    .unwrap_or_else(|error| panic!("failed to build request: {error}")),
            )
            .await
        {
            Ok(response) => response,
            Err(error) => panic!("request failed: {error}"),
        };
        let get_response = match app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri(format!("/api/projects/{project_id}"))
                    .body(Body::empty())
                    .unwrap_or_else(|error| panic!("failed to build request: {error}")),
            )
            .await
        {
            Ok(response) => response,
            Err(error) => panic!("request failed: {error}"),
        };
        let update_response = match app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::PUT)
                    .uri(format!("/api/projects/{project_id}"))
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({
                            "name": "Ora Updated",
                            "rootPath": "/workspace/ora-next",
                        })
                        .to_string(),
                    ))
                    .unwrap_or_else(|error| panic!("failed to build request: {error}")),
            )
            .await
        {
            Ok(response) => response,
            Err(error) => panic!("request failed: {error}"),
        };
        let delete_response = match app
            .oneshot(
                Request::builder()
                    .method(Method::DELETE)
                    .uri(format!("/api/projects/{project_id}"))
                    .body(Body::empty())
                    .unwrap_or_else(|error| panic!("failed to build request: {error}")),
            )
            .await
        {
            Ok(response) => response,
            Err(error) => panic!("request failed: {error}"),
        };

        assert_eq!(
            created_project,
            json!({
                "id": project_id,
                "name": "Ora",
                "rootPath": "/workspace/ora",
            })
        );
        assert_eq!(
            response_json(list_response).await,
            json!({
                "projects": [
                    {
                        "id": project_id,
                        "name": "Ora",
                        "rootPath": "/workspace/ora",
                    },
                ],
            })
        );
        assert_eq!(
            response_json(get_response).await,
            json!({
                "project": {
                    "id": project_id,
                    "name": "Ora",
                    "rootPath": "/workspace/ora",
                },
            })
        );
        assert_eq!(
            response_json(update_response).await,
            json!({
                "project": {
                    "id": project_id,
                    "name": "Ora Updated",
                    "rootPath": "/workspace/ora-next",
                },
            })
        );
        assert_eq!(
            response_json(delete_response).await,
            json!({
                "projectId": project_id,
            })
        );
    }

    /// Verifies missing projects surface the shared HTTP error payload.
    #[tokio::test]
    async fn serves_not_found_payload_for_missing_project() {
        let app = test_router();
        let response = match app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/api/projects/missing-project")
                    .body(Body::empty())
                    .unwrap_or_else(|error| panic!("failed to build request: {error}")),
            )
            .await
        {
            Ok(response) => response,
            Err(error) => panic!("request failed: {error}"),
        };

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert_eq!(
            response_json(response).await,
            json!({
                "error": {
                    "code": "project_not_found",
                    "message": "project not found: missing-project",
                },
            })
        );
    }

    /// Builds a ready router for tests that need the full project route surface.
    fn test_router() -> axum::Router {
        let app_state = build_app_state();
        app_state.mark_ready();
        build_router(app_state)
    }

    /// Decodes one JSON response body so route tests can compare the full payload.
    async fn response_json(response: axum::response::Response) -> Value {
        let bytes = match to_bytes(response.into_body(), usize::MAX).await {
            Ok(bytes) => bytes,
            Err(error) => panic!("failed to read response body: {error}"),
        };

        match serde_json::from_slice::<Value>(&bytes) {
            Ok(value) => value,
            Err(error) => panic!("failed to decode JSON body: {error}"),
        }
    }
}
