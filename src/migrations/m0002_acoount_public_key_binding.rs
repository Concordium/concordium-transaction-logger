use anyhow::Context;
use concordium_rust_sdk::{base::transactions::AccountAccessStructure, v2::{self, BlockIdentifier}};
use futures::TryStreamExt;
use tokio_postgres::Transaction;
use std::collections::HashMap;

use crate::migrations::SchemaVersion;

struct AccountKeyBindings {
    key:               AccountAddress,
    credential_index:  CredentialIndex,
    key_index:         KeyIndex,
    is_simple_account: bool,
}

type BindingsInsert = HashMap<AccountAccessStructure, AccountKeyBindings>;


pub async fn run(
    tx: &mut Transaction<'_>,
    endpoints: &[v2::Endpoint],
) -> anyhow::Result<SchemaVersion> {
    // create the table
    let statement = "CREATE TABLE IF NOT EXISTS account_public_key_bindings(
        idx BIGINT PRIMARY KEY,
        public_key CHAR(64),
        address BYTEA UNIQUE NOT NULL,
        credential_index INT NOT NULL,
        key_index INT NOT NULL,
        is_simple_account BOOLEAN NOT NULL
    )";
    let _ = tx.execute(statement, Vec::new()).await?;

    // create the client
    // FIXME: weakpoint, what if there are no endpoints provided, or client cannot connect,
    // or data from the node is outdated?
    let endpoint = endpoints.first().context("This is bad")?;
    let mut client = v2::Client::new(endpoint.clone()).await?;

    // get list of accounts
    let accounts = client
        .get_account_list(BlockIdentifier::LastFinal)
        .await?
        .response
        .try_fold(Vec::new(), |mut cc, address| async move {
            cc.push(address);
            Ok(cc)
        }).await?;

    // create account to key binding
    // TODO: make concurent run
    let key_binding = HashMap::new();
    for account in accounts {
        let acc_info = client.get_account_info(account.into(), BlockIdentifier::LastFinal).await?.response;
        let access_structure: AccountAccessStructure = acc_info.clone().into();
        let is_simple_account = access_structure.num_keys() == 1;
        let binding = AccountKeyBinding{};
    }
    

    Ok(SchemaVersion::AccountsPublicKeyBindings)
}
