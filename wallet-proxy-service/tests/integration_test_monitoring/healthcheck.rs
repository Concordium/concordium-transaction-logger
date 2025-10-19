//! Tests healthcheck endpoint

use crate::integration_test_helpers::{rest_client, run_server};
use reqwest::StatusCode;
use serde_json::value;

/// Test healthcheck endpoint
#[tokio::test]
async fn test_healthcheck() {
    let handle = run_server::start_server();

    let client = rest_client::monitoring_client(&handle);
    let resp = client.get("health").send().await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let _body: value::Value = resp.json().await.unwrap();
}
