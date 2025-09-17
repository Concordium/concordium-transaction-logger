use anyhow::Context;
use concordium_rust_sdk::{
    base::transactions::AccountAccessStructure,
    common::Serial,
    types::{AbsoluteBlockHeight, AccountInfo},
    v2::{self, BlockIdentifier},
};
use futures::{stream, StreamExt, TryStreamExt};
use tokio::time::Instant;
use tokio_postgres::types::ToSql;
use tokio_postgres::Transaction;

/// Represents one pending insertion row for the `account_public_key_bindings` table to be inserted
type PendingPublicKeyBindingInsertionRow = (Vec<u8>, Vec<u8>, i32, i32, bool);

pub async fn run(tx: &mut Transaction<'_>, endpoints: &[v2::Endpoint]) -> anyhow::Result<()> {
    println!("Starting migration now for public keys");

    // Get time of the start of the migration - so that we can measure how long it takes
    let start = Instant::now();

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
        .map(|h| AbsoluteBlockHeight::from(h as u64))
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

    // create variables for querying the node, concurrency limit and pending rows to be bulk inserted
    let mut query_count = 0;
    let accounts_length = accounts.len();
    let batch_size = 1000;
    let concurrent_query_limit = 50usize;
    let mut pending_rows: Vec<PendingPublicKeyBindingInsertionRow> = Vec::with_capacity(batch_size);
    let mut rows_inserted_count = 0;
    println!(
        "Details -- accounts to fetch and insert: {}, batch size: {}",
        accounts_length, batch_size
    );

    // Create a buffer for querying the account info's - `concurrent_query_limit` defines how many are done in parallel with the node
    let account_infos: Vec<AccountInfo> = stream::iter(accounts.into_iter())
        .map(|account| {

            let mut client = client.clone();
            query_count += 1;

            async move {
                let account_info = client
                    .get_account_info(&account.into(), last_block_height)
                    .await
                    .map(|resp| resp.response)
                    .expect("Expected account info here");

                println!(
                    "account info query with node: {} out of: {}, account: {:?}",
                    query_count, accounts_length, &account.0
                );

                account_info
            }
        })
        .buffer_unordered(concurrent_query_limit)
        .collect()
        .await;

    // For all our account info's we need to find the credentials and keys in the access structure and build them into a pending row to be inserted. Once we have reached the batch size for pending rows, they will get inserted together
    for (index, account_info) in account_infos.into_iter().enumerate() {
        let address: Vec<u8> = account_info.account_address.0.clone().to_vec();
        let access_structure: AccountAccessStructure = (&account_info).into();
        let is_simple_account = access_structure.num_keys() == 1;

        for (cred_index, credential_keys) in access_structure.keys {
            let credential_index: u8 = cred_index.into();
            for (key_index, key) in credential_keys.keys {
                let key_index: u8 = key_index.into();
                let mut public_key = Vec::new();
                key.serial(&mut public_key);

                // create and push a pending row into the vector (later, they will be bulk inserted)
                pending_rows.push((
                    address.clone(),
                    public_key.clone(),
                    credential_index as i32,
                    key_index as i32,
                    is_simple_account,
                ));
            }
        }

        // Flush batch and write to the database once we have reached the batch size
        if pending_rows.len() >= batch_size {
            bulk_insert_pending_rows(tx, &pending_rows).await?;
            rows_inserted_count += pending_rows.len();
            println!("Bulk insert done now for account index: {} out of: {} accounts. Rows inserted so far: {}", index, accounts_length, rows_inserted_count);
            pending_rows.clear();
        }
    }

    // Flush any remaining pending rows to the DB (occurs when less than the batch limit was reached at the end)
    if !pending_rows.is_empty() {
        bulk_insert_pending_rows(tx, &pending_rows).await?;
        println!("Finalized last bulk insert for {} rows", pending_rows.len());
        pending_rows.clear();
    }

    println!("elasped time was: {:?}", start.elapsed());
    Ok(())
}

/// helper function to bulk insert the pending rows to the DB
pub async fn bulk_insert_pending_rows(
    tx: &tokio_postgres::Transaction<'_>,
    rows: &[PendingPublicKeyBindingInsertionRow],
) -> Result<u64, tokio_postgres::Error> {
    if rows.is_empty() {
        return Ok(0);
    }

    let mut placeholders = Vec::with_capacity(rows.len());
    let mut params: Vec<&(dyn ToSql + Sync)> = Vec::with_capacity(rows.len() * 5);

    for (i, row) in rows.iter().enumerate() {
        let base = i * 5;
        placeholders.push(format!(
            "(${}, ${}, ${}, ${}, ${})",
            base + 1,
            base + 2,
            base + 3,
            base + 4,
            base + 5
        ));

        params.push(&row.0);
        params.push(&row.1);
        params.push(&row.2);
        params.push(&row.3);
        params.push(&row.4);
    }

    let sql = format!(
        "INSERT INTO account_public_key_bindings \
         (address, public_key, credential_index, key_index, is_simple_account) \
         VALUES {}",
        placeholders.join(", ")
    );

    tx.execute(sql.as_str(), &params).await
}
