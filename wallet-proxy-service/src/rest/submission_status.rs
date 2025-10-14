use crate::rest::{AppPath, RestResult, RestState};
use anyhow::Context;
use axum::extract::State;
use axum::Json;
use concordium_rust_sdk::base::hashes::TransactionHash;
use concordium_rust_sdk::types;
use wallet_proxy_api::{SubmissionStatus, TransactionStatus};

/// GET Handler for route `/v0/submissionStatus`.
pub async fn submission_status(
    State(mut state): State<RestState>,
    AppPath(txn_hash): AppPath<TransactionHash>,
) -> RestResult<Json<SubmissionStatus>> {
    let sdk_transaction_status = state
        .node_client
        .get_block_item_status(&txn_hash)
        .await
        .context("get_block_item_status")?;

    Ok(Json(to_submission_status(sdk_transaction_status)))
}

fn to_submission_status(txn_status: types::TransactionStatus) -> SubmissionStatus {
    let status = match &txn_status {
        types::TransactionStatus::Received => TransactionStatus::Received,
        types::TransactionStatus::Finalized(_) => TransactionStatus::Finalized,
        types::TransactionStatus::Committed(_) => TransactionStatus::Committed,
    };

    SubmissionStatus { status }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_submission_status_received() {
        let status = types::TransactionStatus::Received;

        let submission = to_submission_status(status);
        assert_eq!(submission.status, TransactionStatus::Received);
    }
}
