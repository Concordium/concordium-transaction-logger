

use reqwest::StatusCode;
use serde_json::value;
use std::time::Duration;
use crate::MONITORING_HOST_PORT;

/// Test scraping metrics
#[tokio::test]
async fn test_submission_status1() {
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

