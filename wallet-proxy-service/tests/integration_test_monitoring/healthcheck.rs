//! Tests healthcheck endpoint

use crate::MONITORING_HOST_PORT;
use reqwest::StatusCode;
use serde_json::value;


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
