/// Custom middleware applied by the REST router
mod middleware;

// Modules for specific REST endpoints follows here:
mod submission_status;

use crate::configuration::Cli;
use anyhow::Context;
use axum::extract::rejection::{PathRejection, QueryRejection};
use axum::extract::{FromRequestParts, Request};
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::{Json, Router, routing};
use concordium_rust_sdk::v2;
use std::fmt::Display;
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

/// Error returned by REST endpoint handlers. Will
/// be mapped to the right HTTP response (HTTP code and custom
/// error body) by the axum middleware
#[derive(Debug, thiserror::Error)]
enum RestError {
    #[error("resource not found")]
    NotFound,
    #[error("{0:#}")]
    Anyhow(#[from] anyhow::Error),
}

/// Error for handling rejections of invalid requests.
/// Will be mapped to the right HTTP response (HTTP code and custom
/// error body) by the axum middleware.
///
/// See <https://docs.rs/axum/latest/axum/extract/index.html#customizing-extractor-responses>
#[derive(Debug, thiserror::Error)]
enum RejectionError {
    #[error("invalid path parameters")]
    PathRejection(#[from] PathRejection),
    #[error("invalid query parameters")]
    QueryRejection(#[from] QueryRejection),
}

/// Extractor with build in error handling. Like [axum::extract::Path](Path) but will use [`RejectionError`] for rejection errors
#[derive(FromRequestParts)]
#[from_request(via(axum::extract::Path), rejection(RejectionError))]
struct AppPath<T>(T);

/// Extractor with build in error handling. Like [axum::extract::Query](Query) but will use [`RejectionError`] for rejection errors
#[derive(FromRequestParts)]
#[from_request(via(axum::extract::Query), rejection(RejectionError))]
struct AppQuery<T>(T);

fn error_response(err: &impl Display, http_status: StatusCode, error_code: ErrorCode) -> Response {
    let error_resp = ErrorResponse {
        error_message: err.to_string(),
        error: error_code,
    };

    (http_status, Json(error_resp)).into_response()
}

impl IntoResponse for RestError {
    fn into_response(self) -> Response {
        let (status, error_code) = match self {
            RestError::NotFound => (StatusCode::NOT_FOUND, ErrorCode::NotFound),
            RestError::Anyhow(_) => (StatusCode::INTERNAL_SERVER_ERROR, ErrorCode::Internal),
        };

        let mut response = error_response(&self, status, error_code);
        response.extensions_mut().insert(Arc::new(self));
        response
    }
}

impl IntoResponse for RejectionError {
    fn into_response(self) -> Response {
        let (status, error_code) = match self {
            RejectionError::PathRejection(_) => {
                (StatusCode::BAD_REQUEST, ErrorCode::InvalidRequest)
            }
        };

        error_response(&self, status, error_code)
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
