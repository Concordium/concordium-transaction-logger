mod submission_status;
mod middleware;

use crate::configuration::Cli;
use anyhow::Context;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::{Json, Router, routing};
use concordium_rust_sdk::v2;
use wallet_proxy_api::{ErrorCode, ErrorResponse};

/// Router exposing the REST API
pub async fn rest_router(cli: &Cli, metrics_registry: &mut prometheus_client::registry::Registry) -> anyhow::Result<Router> {
    let node_client = v2::Client::new(cli.node.clone())
        .await
        .context("create node client")?;

    let rest_state = RestState { node_client };

    let metrics_layer = middleware::metrics::MetricsLayer::new(metrics_registry);

    Ok(Router::new()
        .route(
            "/v0/submissionStatus/{txn_hash}",
            routing::get(submission_status::submission_status),
        )
        .layer(metrics_layer)
        .with_state(rest_state))
}

/// Represents the state required by the REST endpoint router.
#[derive(Clone)]
struct RestState {
    node_client: v2::Client,
}

type RestResult<A> = Result<A, RestError>;

#[derive(Debug, thiserror::Error)]
enum RestError {
    #[error("resource not found")]
    NotFound,
    #[error("{0:#}")]
    Anyhow(#[from] anyhow::Error),
}

impl IntoResponse for RestError {
    fn into_response(self) -> axum::response::Response {
        let (status, error_code) = match self {
            RestError::NotFound => (StatusCode::NOT_FOUND, ErrorCode::NotFound),
            RestError::Anyhow(_) => (StatusCode::INTERNAL_SERVER_ERROR, ErrorCode::Internal),
        };

        let error_resp = ErrorResponse {
            error_message: self.to_string(),
            error: error_code,
        };

        (status, Json(error_resp)).into_response()
    }
}

// todo ar handle mapping to error for internal invalid requests parse