use crate::rest::{AppPath, RestResult, RestState};
use anyhow::Context;
use axum::Json;
use axum::extract::State;
use concordium_rust_sdk::base::hashes::TransactionHash;
use concordium_rust_sdk::types;
use wallet_proxy_api::{SubmissionStatus, TransactionStatus};

/// GET Handler for route `/v0/submissionStatus`.
pub async fn submission_status(
    State(mut state): State<RestState>,
    AppPath(txn_hash): AppPath<TransactionHash>,
) -> RestResult<Json<SubmissionStatus>> {
    let result = state.node_client.get_block_item_status(&txn_hash).await;

    if result.as_ref().is_err_and(|err| err.is_not_found()) {
        return Ok(Json(submission_status_absent()));
    }

    let transaction_status = result.context("get_block_item_status")?;

    Ok(Json(to_submission_status(transaction_status)))
}

fn submission_status_absent() -> SubmissionStatus {
    SubmissionStatus {
        status: TransactionStatus::Absent,
    }
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
