use crate::app_state::AppState;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;

/// Returns a lightweight liveness response that only confirms the process is up.
pub async fn liveness() -> impl IntoResponse {
    StatusCode::OK
}

/// Returns readiness success only after application-state bootstrap completes.
pub async fn readiness(State(app_state): State<AppState>) -> impl IntoResponse {
    if app_state.is_ready() {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    }
}
