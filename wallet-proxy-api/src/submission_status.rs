use concordium_rust_sdk::common::types::AccountAddress;
use concordium_rust_sdk::types::hashes::TransactionHash;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubmissionStatus {
    pub status: TransactionStatus,
    pub sender: AccountAddress,
    pub transaction_hash: TransactionHash,
    pub outcome: TransactionOutcome,
}

#[derive(Debug, Clone, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TransactionStatus {
    Absent,
    Received,
    Committed,
    Finalized,
}

#[derive(Debug, Clone, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TransactionOutcome {
    Success,
    Reject,
    Ambiguous,
}
