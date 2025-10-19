//! Test metrics endpoint

use crate::integration_test_helpers::{rest_client, run_server};
use reqwest::StatusCode;

/// Test scraping metrics
#[tokio::test]
async fn test_prometheus_metrics_scrape() {
    let handle = run_server::start_server();

    let client = rest_client::monitoring_client(&handle);
    let resp = client.get("metrics").send().await.unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
}
