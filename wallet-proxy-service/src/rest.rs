mod middleware;
mod submission_status;

use crate::configuration::Cli;
use anyhow::Context;
use axum::extract::rejection::PathRejection;
use axum::extract::{FromRequestParts, Request};
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::{Json, Router, routing};
use concordium_rust_sdk::v2;
use std::sync::Arc;
use tower::layer;
use tracing::{error, warn};
use wallet_proxy_api::{ErrorCode, ErrorResponse};

/// Router exposing the REST API
pub async fn rest_router(
    cli: &Cli,
    metrics_registry: &mut prometheus_client::registry::Registry,
) -> anyhow::Result<Router> {
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
        .layer(axum::middleware::from_fn(log_app_errors))
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
    #[error("invalid path parameters")]
    PathRejection(#[from] PathRejection),
}

#[derive(FromRequestParts)]
#[from_request(via(axum::extract::Path), rejection(RestError))]
struct AppPath<T>(T);

impl IntoResponse for RestError {
    fn into_response(self) -> Response {
        let (status, error_code) = match self {
            RestError::NotFound => (StatusCode::NOT_FOUND, ErrorCode::NotFound),
            RestError::Anyhow(_) => (StatusCode::INTERNAL_SERVER_ERROR, ErrorCode::Internal),
            RestError::PathRejection(_) => (StatusCode::BAD_REQUEST, ErrorCode::InvalidRequest),
        };

        let error_resp = ErrorResponse {
            error_message: self.to_string(),
            error: error_code,
        };

        let mut response = (status, Json(error_resp)).into_response();
        response.extensions_mut().insert(Arc::new(self));
        response
    }
}

async fn log_app_errors(request: Request, next: Next) -> Response {
    let response = next.run(request).await;
    if let Some(err) = response.extensions().get::<Arc<RestError>>() {
        // log "internal" errors
        if matches!(err.as_ref(), RestError::Anyhow(_)) {
            warn!(%err, "error processing request");
        }
    }
    response
}
