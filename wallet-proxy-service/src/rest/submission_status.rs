use crate::rest::{RestResult, RestState};
use anyhow::Context;
use axum::Json;
use axum::extract::{Path, State};
use concordium_rust_sdk::base::hashes::TransactionHash;
use concordium_rust_sdk::types;
use wallet_proxy_api::SubmissionStatus;

/// GET Handler for route `/v0/submissionStatus`.
pub async fn submission_status(
    State(mut state): State<RestState>,
    Path(txn_hash): Path<TransactionHash>,
) -> RestResult<Json<SubmissionStatus>> {
    let sdk_transaction_status = state
        .node_client
        .get_block_item_status(&txn_hash)
        .await
        .context("get_block_item_status")?;

    Ok(Json(from_sdk(sdk_transaction_status)))
}

fn from_sdk(txn_status: types::TransactionStatus) -> SubmissionStatus {
    todo!()
}
