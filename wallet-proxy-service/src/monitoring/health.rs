use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use serde_json::json;

/// Represents the state required by the health endpoint router.
///
/// This struct provides access to essential resources needed to determine
/// system health and readiness.
#[derive(Clone)]
pub struct HealthState {}

/// GET Handler for route `/health`.
/// Verifying the API service state is as expected.
pub async fn health(State(_state): State<HealthState>) -> (StatusCode, Json<serde_json::Value>) {
    // TODO: database check as part of COR-1809
    // TODO: node check as part of COR-1810

    let is_healthy = true;

    let status_code = if is_healthy {
        StatusCode::OK
    } else {
        StatusCode::INTERNAL_SERVER_ERROR
    };
    (status_code, Json(json!({})))
}
