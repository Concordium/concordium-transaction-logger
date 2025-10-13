use crate::integration_test_helpers::{rest, server};
use reqwest::StatusCode;
use wallet_proxy_api::{SubmissionStatus, TransactionStatus};

#[tokio::test]
async fn test_submission_status() {
    let handle = server::start_server();

    let rest_client = rest::rest_client(&handle);
    let resp = rest_client.get("v0/submissionStatus").send().await.unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let submission_status: SubmissionStatus = resp.json().await.unwrap();
    assert_eq!(submission_status.status, TransactionStatus::Finalized);
}
