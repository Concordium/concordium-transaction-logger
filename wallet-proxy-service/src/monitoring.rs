/// Health status enpoint
mod health;
/// Prometheus metrics endpoint
mod metrics;

use crate::monitoring::health::HealthState;
use axum::{Router, routing};
use std::sync::Arc;

/// Router exposing the Prometheus metrics and health endpoint.
pub fn monitoring_router(
    metrics_registry: prometheus_client::registry::Registry,
) -> anyhow::Result<Router> {
    let metric_routes = Router::new()
        .route("/", routing::get(metrics::metrics))
        .with_state(Arc::new(metrics_registry));
    let health_state = HealthState {};
    let health_routes = Router::new()
        .route("/", routing::get(health::health))
        .with_state(health_state);
    Ok(Router::new()
        .nest("/metrics", metric_routes)
        .nest("/health", health_routes))
}
