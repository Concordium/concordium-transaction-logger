use crate::integration_test_helpers::{fixtures, node_mock, rest, server};
use concordium_rust_sdk::v2::generated;
use reqwest::StatusCode;
use wallet_proxy_api::{SubmissionStatus, TransactionStatus};

#[tokio::test]
async fn test_submission_status() {
    let handle = server::start_server();
    let rest_client = rest::rest_client(&handle);
    let node_mock = node_mock::mock(&handle);

    let txn_hash = fixtures::generate_txn_hash();

    node_mock.mock(|when, then| {
        when.path("/concordium.v2.Queries/GetBlockItemStatus")
            .pb(generated::TransactionHash::from(&txn_hash));
        then.pb(generated::BlockItemStatus {
            status: Some(generated::block_item_status::Status::Received(
                Default::default(),
            )),
        });
    });

    let resp = rest_client
        .get(format!("v0/submissionStatus/{}", txn_hash))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let submission_status: SubmissionStatus = resp.json().await.unwrap();
    assert_eq!(submission_status.status, TransactionStatus::Received);
}
