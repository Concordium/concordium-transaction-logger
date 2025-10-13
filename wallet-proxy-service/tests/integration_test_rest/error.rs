//! Test of generic errors

use concordium_rust_sdk::v2::generated;
use crate::integration_test_helpers::{fixtures, node_mock, rest, server};
use reqwest::StatusCode;
use wallet_proxy_api::{ErrorCode, ErrorResponse};

/// Test that request to node fails
#[tokio::test]
async fn test_node_request_fails() {
    let handle = server::start_server();
    let rest_client = rest::rest_client(&handle);
    let node_mock = node_mock::mock(&handle);

    let txn_hash = fixtures::generate_txn_hash();

    node_mock.mock(|when, then| {
        when.path("/concordium.v2.Queries/GetBlockItemStatus")
            .pb(generated::TransactionHash::from(&txn_hash));
        then.internal_server_error();
    });


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

