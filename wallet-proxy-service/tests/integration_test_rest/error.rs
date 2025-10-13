//! Test of generic errors

use crate::integration_test_helpers::{rest, server};
use reqwest::StatusCode;
use wallet_proxy_api::{ErrorCode, ErrorResponse};

/// Test that request to node fails
#[tokio::test]
async fn test_node_request_fails() {
    let handle = server::start_server();

    let txn_hash = "9abc8cafb44957e8f8ce59c8128934e3533c69d84fd60c157147e68e7b72577e"; // todo ar generate

    let rest_client = rest::rest_client(&handle);
    let resp = rest_client.get(format!("v0/submissionStatus/{}", txn_hash)).send().await.unwrap();

    assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let err_resp: ErrorResponse = resp.json().await.unwrap();
    assert_eq!(err_resp.error, ErrorCode::Internal);
    assert!(err_resp.error_message.contains("RPC error"), "message: {}", err_resp.error_message);
}

/// Test that parsing request fails
#[tokio::test]
async fn test_invalid_request() {
    let handle = server::start_server();

    // invalid transaction hash
    let txn_hash = "aaa";

    let rest_client = rest::rest_client(&handle);
    let resp = rest_client.get(format!("v0/submissionStatus/{}", txn_hash)).send().await.unwrap();

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let err_resp: ErrorResponse = resp.json().await.unwrap();
    assert_eq!(err_resp.error, ErrorCode::InvalidRequest);
    assert!(err_resp.error_message.contains("hest"), "message: {}", err_resp.error_message);
}

// todo ar test not found