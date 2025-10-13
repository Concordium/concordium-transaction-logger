//! Test metrics endpoint

use crate::integration_test_helpers::{rest, server};
use reqwest::StatusCode;

/// Test scraping metrics
#[tokio::test]
async fn test_prometheus_metrics_scrape() {
    let handle = server::start_server();

    let client = rest::monitoring_client(&handle);
    let resp = client.get("metrics").send().await.unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
}
