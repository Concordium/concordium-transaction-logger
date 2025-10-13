use std::sync::Arc;
use axum::extract::State;
use prometheus_client::registry::Registry;

/// GET Handler for route `/metrics`.
/// Exposes the metrics in the registry in the Prometheus format.
pub async fn metrics(State(metrics_registry): State<Arc<Registry>>) -> Result<String, String> {
    let mut buffer = String::new();
    prometheus_client::encoding::text::encode(&mut buffer, &metrics_registry)
        .map_err(|err| err.to_string())?;
    Ok(buffer)
}
