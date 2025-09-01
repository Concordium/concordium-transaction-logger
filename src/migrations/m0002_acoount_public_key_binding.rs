use anyhow::Context;
use concordium_rust_sdk::{
    base::transactions::AccountAccessStructure,
    id::types::VerifyKey,
    v2::{self, BlockIdentifier},
};
use futures::TryStreamExt;
use tokio_postgres::Transaction;

// struct AccountKeyBindings {
//     key:               AccountAddress,
//     credential_index:  CredentialIndex,
//     key_index:         KeyIndex,
//     is_simple_account: bool,
// }

// type BindingsInsert = HashMap<AccountAccessStructure, AccountKeyBindings>;

pub async fn run(tx: &mut Transaction<'_>, endpoints: &[v2::Endpoint]) -> anyhow::Result<()> {
    // create the table bindings table
    let query = include_str!("../../resources/m0002-accounts-public-key-bindings.sql");
    tx.batch_execute(query).await?;

    // create the client
    // FIXME: weakpoint, what if there are no endpoints provided, or client cannot
    // connect, or data from the node is outdated?
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
        })
        .await?;

    // prepare statement
    let statement = tx
        .prepare(
            r#"INSERT INTO account_public_key_bindings 
        (address, public_key, credential_index, key_index, is_simple_account, active)
        VALUES
        ($1, $2, $3, $4, $5, $6)
        "#,
        )
        .await?;

    // get and insert account-key bindings info
    // TODO: make concurent run
    for account in accounts {
        let address = account.0.as_ref();
        let acc_info =
            client.get_account_info(&account.into(), BlockIdentifier::LastFinal).await?.response;
        let access_structure: AccountAccessStructure = (&acc_info).into();
        let is_simple_account = access_structure.num_keys() == 1;
        for x in access_structure.keys {
            let cred_index = x.0.index as i32;
            for y in x.1.keys {
                let key_index = y.0 .0 as i32;
                let VerifyKey::Ed25519VerifyKey(key) = y.1;
                let public_key = key.as_ref();
                let _ = tx
                    .execute(&statement, &[
                        &address,
                        &public_key,
                        &cred_index,
                        &key_index,
                        &is_simple_account,
                        &true,
                    ])
                    .await?;
            }
        }
    }

    // insert keys and accounts

    Ok(())
}
