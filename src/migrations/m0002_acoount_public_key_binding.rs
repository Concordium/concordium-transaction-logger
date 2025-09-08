use anyhow::Context;
use concordium_rust_sdk::{
    base::transactions::AccountAccessStructure,
    common::Serial,
    types::AbsoluteBlockHeight,
    v2::{self, BlockIdentifier},
};
use futures::TryStreamExt;
use tokio_postgres::Transaction;

pub async fn run(tx: &mut Transaction<'_>, endpoints: &[v2::Endpoint]) -> anyhow::Result<()> {
    // create the table bindings table
    let query = include_str!("../../resources/m0002-accounts-public-key-bindings.sql");
    tx.batch_execute(query).await?;

    // get the latest block in the db
    let last_block_height = tx
        .query_opt(
            "SELECT summaries.height FROM summaries ORDER BY summaries.id DESC LIMIT 1",
            &[],
        )
        .await?
        .map(|row| row.try_get::<_, i64>(0))
        .transpose()?
        .map(|h| AbsoluteBlockHeight::from(h as u64).into())
        .unwrap_or(0.into());

    // create the client
    let endpoint = endpoints.first().context(
        "First node endpoint is not available. The indexer/migration file needs access to a node.",
    )?;
    let mut client = v2::Client::new(endpoint.clone()).await?;

    // get all account infos
    let accounts = client
        .get_account_list(BlockIdentifier::AbsoluteHeight(last_block_height))
        .await?
        .response
        .try_collect::<Vec<_>>()
        .await?;

    let futures = accounts.into_iter().map(|account| {
        let mut client = client.clone();
        async move {
            client
                .get_account_info(&account.into(), last_block_height)
                .await
        }
    });

    let account_infos = futures::future::try_join_all(futures)
        .await?
        .into_iter()
        .map(|resp| resp.response);

    // prepare statement
    let statement = tx
        .prepare(
            r#"INSERT INTO account_public_key_bindings 
        (address, public_key, credential_index, key_index, is_simple_account)
        VALUES
        ($1, $2, $3, $4, $5)"#,
        )
        .await?;

    // insert account-key bindings info
    for info in account_infos {
        let address: &[u8] = info.account_address.as_ref();
        let access_structure: AccountAccessStructure = (&info).into();
        let is_simple_account = access_structure.num_keys() == 1;
        for (cred_index, credential_keys) in access_structure.keys {
            let credential_index: u8 = cred_index.into();
            for (key_index, key) in credential_keys.keys {
                let key_index: u8 = key_index.into();
                let mut public_key = Vec::new();
                key.serial(&mut public_key);
                let _ = tx
                    .execute(
                        &statement,
                        &[
                            &address,
                            &public_key,
                            &(credential_index as i32),
                            &(key_index as i32),
                            &is_simple_account,
                        ],
                    )
                    .await?;
            }
        }
    }
    Ok(())
}
