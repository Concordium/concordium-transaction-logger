

use reqwest::StatusCode;
use serde_json::value;
use std::time::Duration;
use crate::MONITORING_HOST_PORT;

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

/// Test healthcheck endpoint
#[tokio::test]
async fn test_healthcheck() {
    crate::start_server();


    let client = reqwest::Client::new();
    let resp = client
        .get(format!("http://{}/health", MONITORING_HOST_PORT))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let _body: value::Value = resp.json().await.unwrap();
}
