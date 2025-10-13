use concordium_rust_sdk::base::hashes::TransactionHash;
use rand::Rng;

pub fn generate_txn_hash() -> TransactionHash {
    TransactionHash::new(rand::random())
}
