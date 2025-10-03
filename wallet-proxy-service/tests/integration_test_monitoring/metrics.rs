//! Test metrics endpoint

use crate::MONITORING_HOST_PORT;
use reqwest::StatusCode;

/// Test scraping metrics
#[tokio::test]
async fn test_prometheus_metrics_scrape() {
    crate::start_server();

    let client = reqwest::Client::new();
    let status = client
        .get(format!("http://{}/metrics", MONITORING_HOST_PORT))
        .send()
        .await
        .unwrap()
        .status();

    assert_eq!(status, StatusCode::OK);
}
