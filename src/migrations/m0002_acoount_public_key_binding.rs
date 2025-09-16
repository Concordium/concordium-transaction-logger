use anyhow::Context;
use concordium_rust_sdk::{
    base::transactions::AccountAccessStructure,
    common::Serial,
    types::AbsoluteBlockHeight,
    v2::{self, BlockIdentifier},
};
use futures::TryStreamExt;
use tokio_postgres::Transaction;
use tokio_postgres::types::ToSql;

pub async fn run(tx: &mut Transaction<'_>, endpoints: &[v2::Endpoint]) -> anyhow::Result<()> {
    println!("Starting migration now for public keys");

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


    let mut query_count = 0;
    let accounts_length = accounts.len();
    let mut batch_size = 1000;
    let mut pending_rows: Vec<(Vec<u8>, Vec<u8>, i32, i32, bool)> = Vec::with_capacity(batch_size);
    println!("Details -- accounts to fetch and insert: {}, batch size: {}", accounts_length, batch_size);


    // Start looping through the accounts
    for account in accounts {
        let account_info = client
            .get_account_info(&account.into(), last_block_height)
            .await?
            .response;

        // logging progress
        println!("Queried for: {} out of: {}", query_count, accounts_length);
        query_count += 1;

        let address: Vec<u8> = account_info.account_address.0.clone().to_vec();
        let access_structure: AccountAccessStructure = (&account_info).into();
        let is_simple_account = access_structure.num_keys() == 1;

        for (cred_index, credential_keys) in access_structure.keys {
            let credential_index: u8 = cred_index.into();
            for (key_index, key) in credential_keys.keys {
                let key_index: u8 = key_index.into();
                let mut public_key = Vec::new();
                key.serial(&mut public_key);

                pending_rows.push((
                    address.clone(),
                    public_key.clone(),
                    credential_index as i32,
                    key_index as i32,
                    is_simple_account,
                ));
            }
        }

        // Flush batch when it reaches batch_size
        if pending_rows.len() >= batch_size {
            insert_bindings(tx, &pending_rows).await?;
            println!("Bulk insert done now for {} rows", pending_rows.len());
            pending_rows.clear();
        }
    }

    // Flush any remaining rows at the very end
    if !pending_rows.is_empty() {
        insert_bindings(tx, &pending_rows).await?;
        println!("Finalized last bulk insert for {} rows", pending_rows.len());
        pending_rows.clear();
    }

    Ok(())
}



pub async fn insert_bindings(
    tx: &tokio_postgres::Transaction<'_>,
    rows: &[(Vec<u8>, Vec<u8>, i32, i32, bool)],
) -> Result<u64, tokio_postgres::Error> {
    if rows.is_empty() {
        return Ok(0);
    }

    // Build the VALUES part: ($1, $2, $3, $4, $5), ($6, $7, $8, $9, $10), ...
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

        // Flatten the row values into params
        params.push(&row.0); // Vec<u8> → bytea
        params.push(&row.1); // Vec<u8> → bytea
        params.push(&row.2); // i32 → int
        params.push(&row.3); // i32 → int
        params.push(&row.4); // bool
    }

    let sql = format!(
        "INSERT INTO account_public_key_bindings \
         (address, public_key, credential_index, key_index, is_simple_account) \
         VALUES {}",
        placeholders.join(", ")
    );

    tx.execute(sql.as_str(), &params).await
}

