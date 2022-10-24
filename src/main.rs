use anyhow::Context;
use clap::AppSettings;
use concordium_rust_sdk::{
    cis2::{self, TokenAmount, TokenId},
    common::{types::Timestamp, SerdeSerialize},
    id::types::AccountAddress,
    postgres,
    postgres::DatabaseClient,
    types::{
        hashes::BlockHash, queries::BlockInfo, AbsoluteBlockHeight, BlockItemSummary,
        ContractAddress, SpecialTransactionOutcome,
    },
    v2,
};
use futures::{StreamExt, TryStreamExt};
use std::{
    collections::HashSet,
    convert::TryFrom,
    hash::Hash,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
use structopt::StructOpt;
use thiserror::Error;
use tokio_postgres::{
    types::{Json, ToSql},
    Transaction as DBTransaction,
};

#[derive(StructOpt)]
struct App {
    #[structopt(
        long = "node",
        help = "GRPC interface of the node(s).",
        default_value = "http://localhost:20000",
        use_delimiter = true,
        env = "TRANSACTION_LOGGER_NODES"
    )]
    endpoint:     Vec<v2::Endpoint>,
    #[structopt(
        long = "db",
        default_value = "host=localhost dbname=transaction-outcome user=postgres \
                         password=password port=5432",
        help = "Database connection string.",
        env = "TRANSACTION_LOGGER_DB_STRING"
    )]
    config:       postgres::Config,
    #[structopt(
        long = "log-level",
        default_value = "off",
        help = "Maximum log level.",
        env = "TRANSACTION_LOGGER_LOG_LEVEL"
    )]
    log_level:    log::LevelFilter,
    #[structopt(
        long = "num-parallel",
        default_value = "1",
        help = "Maximum number of parallel queries to make to the node. Usually 1 is the correct \
                number, but during initial catchup it is useful to increase this to, say 8 to \
                take advantage of parallelism in queries.",
        env = "TRANSACTION_LOGGER_NUM_PARALLEL_QUERIES"
    )]
    num_parallel: u32,
    #[structopt(
        long = "max-behind-seconds",
        default_value = "240",
        help = "Maximum number of seconds the node's last finalization can be behind before the \
                node is given up and another one is tried.",
        env = "TRANSACTION_LOGGER_MAX_BEHIND_SECONDS"
    )]
    max_behind:   u32,
}

#[derive(SerdeSerialize, Debug)]
pub enum BorrowedDatabaseSummaryEntry<'a> {
    #[serde(rename = "Left")]
    /// An item that is explicitly included in the block. This is always a
    /// result of user actions, e.g., transfers, account creations.
    BlockItem(&'a BlockItemSummary),
    #[serde(rename = "Right")]
    /// Protocol genereated event, such as baking and finalization rewards, and
    /// minting.
    ProtocolEvent(&'a SpecialTransactionOutcome),
}

pub struct SummaryRow<'a> {
    /// Hash of the block the row applies to.
    pub block_hash:   BlockHash,
    /// Slot time of the block the row applies to.
    pub block_time:   Timestamp,
    /// Block height stored in the database.
    pub block_height: AbsoluteBlockHeight,
    /// Summary of the item. Either a user-generated transaction, or a protocol
    /// event that affected the account or contract.
    pub summary:      BorrowedDatabaseSummaryEntry<'a>,
}

struct BlockItemSummaryWithCanonicalAddresses {
    pub(crate) summary:   BlockItemSummary,
    /// Affected addresses, resolved to canonical addresses and without
    /// duplicates.
    pub(crate) addresses: Vec<AccountAddress>,
}

/// Prepared statements for all insertions. Prepared statements are
/// per-connection so we have to re-create them each time we reconnect.
struct PreparedStatements {
    /// Insert into the summary table.
    insert_summary:             tokio_postgres::Statement,
    /// Insert into the account transaction index table.
    insert_ati:                 tokio_postgres::Statement,
    /// Insert into the contract transaction index table.
    insert_cti:                 tokio_postgres::Statement,
    /// Increase the total supply of a given token.
    cis2_increase_total_supply: tokio_postgres::Statement,
    /// Decrease the total supply of a given token.
    cis2_decrease_total_supply: tokio_postgres::Statement,
}

/// A wrapper around a [`DatabaseClient`] that maintains prepared statements.
struct DBConn {
    client:   DatabaseClient,
    prepared: PreparedStatements,
}

impl DBConn {
    /// Create a new database connection. If the second argument is `true` then
    /// the table creation script will be run (see the file
    /// `resources/schema.sql`). This script is in principle idempotent so
    /// running it twice should be safe, but it is also needless,
    /// so this should only be requested on a first connection on startup.
    async fn create(config: &postgres::Config, try_create_tables: bool) -> anyhow::Result<Self> {
        let mut client = DatabaseClient::create(config.clone(), postgres::NoTls).await?;

        if try_create_tables {
            let create = include_str!("../resources/schema.sql");
            client.as_ref().batch_execute(create).await?
        }

        let insert_ati =
            client.as_mut().prepare("INSERT INTO ati (account, summary) VALUES ($1, $2)").await?;
        let insert_cti = client
            .as_mut()
            .prepare("INSERT INTO cti (index, subindex, summary) VALUES ($1, $2, $3)")
            .await?;
        let insert_summary = client
            .as_mut()
            .prepare(
                "INSERT INTO summaries (block, timestamp, height, summary) VALUES ($1, $2, $3, \
                 $4) RETURNING id",
            )
            .await?;
        let cis2_increase_total_supply = client
            .as_mut()
            .prepare(
                "INSERT INTO cis2_tokens (index, subindex, token_id, total_supply)
VALUES ($1, $2, $3, CAST($4 AS TEXT):: NUMERIC)
ON CONFLICT (index, subindex, token_id)
DO UPDATE SET total_supply = cis2_tokens.total_supply + EXCLUDED.total_supply
RETURNING id",
            )
            .await?;
        let cis2_decrease_total_supply = client
            .as_mut()
            .prepare(
                "INSERT INTO cis2_tokens (index, subindex, token_id, total_supply)
VALUES ($1, $2, $3, CAST($4 AS TEXT):: NUMERIC)
ON CONFLICT (index, subindex, token_id)
DO UPDATE SET total_supply = cis2_tokens.total_supply - EXCLUDED.total_supply
RETURNING id",
            )
            .await?;
        Ok(Self {
            client,
            prepared: PreparedStatements {
                insert_summary,
                insert_ati,
                insert_cti,
                cis2_increase_total_supply,
                cis2_decrease_total_supply,
            },
        })
    }
}

impl PreparedStatements {
    /// Insert a new summary row, containing a single transaction summary.
    async fn insert_summary<'a, 'b, 'c>(
        &'a self,
        tx: &DBTransaction<'b>,
        summary: &SummaryRow<'c>,
    ) -> Result<i64, postgres::Error> {
        let values = [
            &summary.block_hash.as_ref() as &(dyn ToSql + Sync),
            &(summary.block_time.millis as i64) as &(dyn ToSql + Sync),
            &(summary.block_height.height as i64) as &(dyn ToSql + Sync),
            &Json(&summary.summary) as &(dyn ToSql + Sync),
        ];
        let res = tx.query_one(&self.insert_summary, &values).await?;
        let id = res.try_get::<_, i64>(0)?;
        Ok(id)
    }

    /// Insert an account transaction.
    async fn insert_transaction<'a, 'b>(
        &'a self,
        tx: &DBTransaction<'b>,
        block_hash: BlockHash,
        block_time: Timestamp,
        block_height: AbsoluteBlockHeight,
        ts: &BlockItemSummaryWithCanonicalAddresses,
    ) -> Result<(), postgres::Error> {
        let affected_addresses = &ts.addresses;
        let summary_row = SummaryRow {
            block_hash,
            block_time,
            block_height,
            summary: BorrowedDatabaseSummaryEntry::BlockItem(&ts.summary),
        };
        let id = self.insert_summary(tx, &summary_row).await?;
        for affected in affected_addresses.iter() {
            let addr_bytes: &[u8; 32] = affected.as_ref();
            let values = [&&addr_bytes[..] as &(dyn ToSql + Sync), &id as &(dyn ToSql + Sync)];
            tx.query_opt(&self.insert_ati, &values).await?;
        }
        // insert contracts
        for affected in ts.summary.affected_contracts() {
            let index = affected.index;
            let subindex = affected.subindex;
            let values = [&(index as i64) as &(dyn ToSql + Sync), &(subindex as i64), &id];
            tx.query_opt(&self.insert_cti, &values).await?;
        }
        Ok(())
    }

    /// Insert special outcomes.
    async fn insert_special<'a, 'b>(
        &'a self,
        tx: &DBTransaction<'b>,
        block_hash: BlockHash,
        block_time: Timestamp,
        block_height: AbsoluteBlockHeight,
        so: &SpecialTransactionOutcome,
    ) -> Result<(), postgres::Error> {
        // these are canonical addresses so there is no need to resolve anything
        // additional.
        let affected_addresses = match so {
            SpecialTransactionOutcome::BakingRewards {
                baker_rewards,
                ..
            } => baker_rewards.keys().copied().collect::<Vec<_>>(),
            SpecialTransactionOutcome::Mint {
                foundation_account,
                ..
            } => vec![*foundation_account],
            SpecialTransactionOutcome::FinalizationRewards {
                finalization_rewards,
                ..
            } => finalization_rewards.keys().copied().collect::<Vec<_>>(),
            SpecialTransactionOutcome::BlockReward {
                baker,
                foundation_account,
                ..
            } => vec![*baker, *foundation_account],
            SpecialTransactionOutcome::PaydayFoundationReward {
                foundation_account,
                ..
            } => vec![*foundation_account],
            SpecialTransactionOutcome::PaydayAccountReward {
                account,
                ..
            } => vec![*account],
            // the following two are only administrative events, they don't transfer to the account,
            // only to the virtual account.
            SpecialTransactionOutcome::BlockAccrueReward {
                ..
            } => Vec::new(),
            SpecialTransactionOutcome::PaydayPoolReward {
                ..
            } => Vec::new(),
        };
        let summary_row = SummaryRow {
            block_hash,
            block_time,
            block_height,
            summary: BorrowedDatabaseSummaryEntry::ProtocolEvent(so),
        };
        let id = self.insert_summary(tx, &summary_row).await?;
        for affected in affected_addresses.iter() {
            let addr_bytes: &[u8; 32] = affected.as_ref();
            let values = [&&addr_bytes[..] as &(dyn ToSql + Sync), &id as &(dyn ToSql + Sync)];
            tx.query_opt(&self.insert_ati, &values).await?;
        }
        Ok(())
    }

    /// Increase the total supply of the given token, and return the primary key
    /// in the `cis2_tokens` table for that token.
    async fn cis2_increase_total_supply<'a, 'b>(
        &'a self,
        tx: &DBTransaction<'b>,
        ca: ContractAddress,
        token_id: &TokenId,
        amount: &TokenAmount,
    ) -> Result<i64, postgres::Error> {
        let values = [
            &(ca.index as i64) as &(dyn ToSql + Sync),
            &(ca.subindex as i64),
            &token_id.as_ref(),
            &amount.0.to_string(),
        ];
        let id = tx.query_one(&self.cis2_increase_total_supply, &values).await?;
        let id = id.try_get::<_, i64>(0)?;
        Ok(id)
    }

    /// Decrease the total supply of the given token, and return the primary key
    /// in the `cis2_tokens` table for that token.
    async fn cis2_decrease_total_supply<'a, 'b>(
        &'a self,
        tx: &DBTransaction<'b>,
        ca: ContractAddress,
        token_id: &TokenId,
        amount: &TokenAmount,
    ) -> Result<i64, postgres::Error> {
        let values = [
            &(ca.index as i64) as &(dyn ToSql + Sync),
            &(ca.subindex as i64),
            &token_id.as_ref(),
            &amount.0.to_string(),
        ];
        let id = tx.query_one(&self.cis2_decrease_total_supply, &values).await?;
        let id = id.try_get::<_, i64>(0)?;
        Ok(id)
    }

    /// Check whether the summary contains any CIS2 events, and if so,
    /// parse them and insert them into the `cis2_tokens` table.
    async fn insert_cis2(
        &self,
        tx: &DBTransaction<'_>,
        ts: &BlockItemSummary,
    ) -> Result<(), postgres::Error> {
        if let Some(effects) = get_cis2_events(ts) {
            for (ca, events) in effects {
                for event in events {
                    match event {
                        cis2::Event::Transfer {
                            ..
                        } => {
                            // do nothing, tokens are not created here.
                        }
                        cis2::Event::Mint {
                            ref token_id,
                            ref amount,
                            ..
                        } => {
                            self.cis2_increase_total_supply(tx, ca, token_id, amount).await?;
                        }
                        cis2::Event::Burn {
                            ref token_id,
                            ref amount,
                            ..
                        } => {
                            self.cis2_decrease_total_supply(tx, ca, token_id, amount).await?;
                        }
                        cis2::Event::UpdateOperator {
                            ..
                        } => {
                            // do nothing, updating operators does not change
                            // token suply
                        }
                        cis2::Event::TokenMetadata {
                            ..
                        } => {
                            // do nothing, updating token metadata does not
                            // change token supply.
                        }
                        cis2::Event::Unknown => {
                            // do nothing, not a CIS2 event
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

impl AsRef<DatabaseClient> for DBConn {
    fn as_ref(&self) -> &DatabaseClient { &self.client }
}

impl AsMut<DatabaseClient> for DBConn {
    fn as_mut(&mut self) -> &mut DatabaseClient { &mut self.client }
}

async fn insert_block(
    db: &mut DBConn,
    block_hash: BlockHash,
    block_time: Timestamp,
    block_height: AbsoluteBlockHeight,
    item_summaries: &[BlockItemSummaryWithCanonicalAddresses],
    special_events: &[SpecialTransactionOutcome],
) -> Result<chrono::Duration, postgres::Error> {
    let start = chrono::Utc::now();
    let db_tx = db.client.as_mut().transaction().await?;
    let prepared = &db.prepared;
    for transaction in item_summaries.iter() {
        prepared
            .insert_transaction(&db_tx, block_hash, block_time, block_height, transaction)
            .await?;
        prepared.insert_cis2(&db_tx, &transaction.summary).await?;
    }
    for special in special_events.iter() {
        prepared.insert_special(&db_tx, block_hash, block_time, block_height, special).await?;
    }
    db_tx.commit().await?;
    let end = chrono::Utc::now().signed_duration_since(start);
    Ok(end)
}

async fn get_last_block_height(
    db: &DatabaseClient,
) -> Result<Option<AbsoluteBlockHeight>, postgres::Error> {
    let statement = "SELECT summaries.height FROM summaries ORDER BY summaries.id DESC LIMIT 1;";
    let row = db.as_ref().query_opt(statement, &[]).await?;
    if let Some(row) = row {
        let raw = row.try_get::<_, i64>(0)?;
        Ok(Some((raw as u64).into()))
    } else {
        Ok(None)
    }
}

const MAX_CONNECT_ATTEMPTS: u64 = 6;

/// Data sent by the node query task to the transaction insertion task.
/// Contains all the information needed to build the transaction index.
type TransactionLogData =
    (BlockInfo, Vec<BlockItemSummaryWithCanonicalAddresses>, Vec<SpecialTransactionOutcome>);

#[repr(transparent)]
#[derive(Eq, Debug, Clone, Copy)]
struct AccountAddressEq(AccountAddress);

impl From<AccountAddressEq> for AccountAddress {
    fn from(aae: AccountAddressEq) -> Self { aae.0 }
}

impl PartialEq for AccountAddressEq {
    fn eq(&self, other: &Self) -> bool {
        let bytes_1: &[u8; 32] = self.0.as_ref();
        let bytes_2: &[u8; 32] = other.0.as_ref();
        bytes_1[0..29] == bytes_2[0..29]
    }
}

impl Hash for AccountAddressEq {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        let bytes: &[u8; 32] = self.0.as_ref();
        bytes[0..29].hash(state);
    }
}

impl AsRef<AccountAddressEq> for AccountAddress {
    fn as_ref(&self) -> &AccountAddressEq { unsafe { std::mem::transmute(self) } }
}

#[derive(Debug, Error)]
enum NodeError {
    /// Error establishing connection.
    #[error("Error connecting to the node {0}.")]
    ConnectionError(tonic::transport::Error),
    /// No finalization in some time.
    #[error("Timeout.")]
    Timeout,
    /// Error establishing connection.
    #[error("Error during query {0}.")]
    NetworkError(#[from] v2::Status),
    /// Query error.
    #[error("Error querying the node {0}.")]
    QueryError(#[from] v2::QueryError),
    /// Query errors, etc.
    #[error("Error querying the node {0}.")]
    OtherError(#[from] anyhow::Error),
}

/// Return Err if querying the node failed.
/// Return Ok(()) if the channel to the database was closed.
#[allow(clippy::too_many_arguments)]
async fn use_node(
    node_ep: v2::Endpoint,
    sender: &tokio::sync::mpsc::Sender<TransactionLogData>,
    height: &mut AbsoluteBlockHeight, // start height
    max_parallel: u32,
    stop_flag: &AtomicBool,
    canonical_cache: &mut HashSet<AccountAddressEq>,
    max_behind: u32, // maximum number of seconds a node can be behind before it is deemed "behind"
) -> Result<(), NodeError> {
    let mut node = v2::Client::new(node_ep).await.map_err(NodeError::ConnectionError)?;
    // if the cache is empty we seed it with all accounts at the last finalized
    // block. This will only happen the first time this function is successfully
    // invoked.
    if canonical_cache.is_empty() {
        let accounts_response = node
            .get_account_list(&v2::BlockIdentifier::LastFinal)
            .await
            .context("Error querying account list.")?;
        let accounts = accounts_response.response;
        let r = accounts
            .try_fold(HashSet::new(), |mut cc, addr| async move {
                let _ = cc.insert(AccountAddressEq(addr));
                Ok(cc)
            })
            .await?;
        *canonical_cache = r;
    }
    let mut finalized_blocks = node.get_finalized_blocks_from(*height).await?;
    let timeout = std::time::Duration::from_secs(max_behind.into());
    while !stop_flag.load(Ordering::Acquire) {
        let (error, chunk) = finalized_blocks
            .next_chunk_timeout(max_parallel as usize, timeout)
            .await
            .map_err(|_| NodeError::Timeout)?;
        let mut futures = futures::stream::FuturesOrdered::new();
        for fb in chunk {
            let mut node = node.clone();
            futures.push_back(async move {
                let binfo = node.get_block_info(&fb.block_hash.into()).await?;
                let events = if binfo.response.transaction_count == 0 {
                    Vec::new()
                } else {
                    node.get_block_transaction_events(&fb.block_hash.into())
                        .await?
                        .response
                        .try_collect()
                        .await?
                };
                let special = node
                    .get_block_special_events(&fb.block_hash.into())
                    .await?
                    .response
                    .try_collect()
                    .await?;
                Ok::<
                    (BlockInfo, Vec<BlockItemSummary>, Vec<SpecialTransactionOutcome>),
                    anyhow::Error,
                >((binfo.response, events, special))
            });
        }
        while let Some(result) = futures.next().await {
            let (binfo, transaction_summaries, special_events) = result?;
            let mut with_addresses = Vec::with_capacity(transaction_summaries.len());
            for summary in transaction_summaries {
                let affected_addresses = summary.affected_addresses();
                let mut addresses = Vec::with_capacity(affected_addresses.len());
                // resolve canonical addresses. This part is only needed because the index
                // is currently expected by "canonical address",
                // which is only possible to resolve by querying the node.
                let mut seen = HashSet::with_capacity(affected_addresses.len());
                for address in affected_addresses.into_iter() {
                    if let Some(addr) = canonical_cache.get(address.as_ref()) {
                        let addr = AccountAddress::from(*addr);
                        if seen.insert(addr) {
                            addresses.push(addr);
                        }
                    } else {
                        let ainfo = node
                            .get_account_info(&address.into(), &binfo.block_hash.into())
                            .await
                            .context("Error querying account info.")?
                            .response;
                        let addr = ainfo.account_address;
                        if seen.insert(addr) {
                            log::debug!("Discovered new address {}", addr);
                            addresses.push(addr);
                            canonical_cache.insert(AccountAddressEq(addr));
                        } else {
                            log::debug!("Canonical address {} already listed.", addr);
                        }
                    }
                }
                with_addresses.push(BlockItemSummaryWithCanonicalAddresses {
                    summary,
                    addresses,
                })
            }
            if sender.send((binfo, with_addresses, special_events)).await.is_err() {
                log::error!("The database connection has been closed. Terminating node queries.");
                return Ok(());
            }
            *height = height.next();
        }
        if error {
            // we have processed the blocks we can, but further queries on the same stream
            // will fail since the stream signalled an error.
            return Err(NodeError::OtherError(anyhow::anyhow!("Finalized block stream dropped.")));
        }
    }
    Ok(())
}

/// Try to reconnect to the database with exponential backoff, at most
/// MAX_CONNECT_ATTEMPTS times.
async fn try_reconnect(
    config: &postgres::Config,
    stop_flag: &AtomicBool,
    create_tables: bool,
) -> anyhow::Result<DBConn> {
    let mut i = 1;
    while !stop_flag.load(Ordering::Acquire) {
        match DBConn::create(config, create_tables).await {
            Ok(c) => return Ok(c),
            Err(e) if i < MAX_CONNECT_ATTEMPTS => {
                let delay = std::time::Duration::from_millis(500 * (1 << i));
                log::error!(
                    "Could not connect to the database due to {:#}. Reconnecting in {}ms",
                    e,
                    delay.as_millis()
                );
                // wait for 2^(i-1) seconds before attempting to reconnect.
                tokio::time::sleep(delay).await;
                i += 1;
            }
            Err(e) => {
                log::error!(
                    "Could not connect to the database in {} attempts. Last attempt failed with \
                     reason {:#}.",
                    MAX_CONNECT_ATTEMPTS,
                    e
                );
                return Err(e);
            }
        }
    }
    anyhow::bail!("The node was requested to stop.")
}

fn get_cis2_events(
    bi: &BlockItemSummary,
) -> Option<impl Iterator<Item = (ContractAddress, Vec<cis2::Event>)> + '_> {
    let log_iter = bi.contract_update_logs()?;
    let ret = log_iter.flat_map(|(ca, logs)| {
        match logs.iter().map(cis2::Event::try_from).collect::<Result<Vec<cis2::Event>, _>>() {
            Ok(events) => Some((ca, events)),
            Err(_) => None,
        }
    });
    Some(ret)
}

async fn write_to_db(
    config: postgres::Config,
    height_sender: tokio::sync::oneshot::Sender<AbsoluteBlockHeight>, // start height
    mut receiver: tokio::sync::mpsc::Receiver<TransactionLogData>,
    stop_flag: Arc<AtomicBool>,
) -> anyhow::Result<()> {
    let mut db = try_reconnect(&config, &stop_flag, true).await?;

    let height = get_last_block_height(&db.client).await?.map_or(0.into(), |h| h.next());
    height_sender
        .send(height)
        .map_err(|_| anyhow::anyhow!("Cannot send height to the node worker."))?;
    let mut retry = None;
    // How many successive insertion errors were encountered.
    // This is used to slow down attempts to not spam the database
    let mut successive_errors = 0;
    while !stop_flag.load(Ordering::Acquire) {
        let next_item = if let Some(v) = retry.take() {
            Some(v)
        } else {
            receiver.recv().await
        };
        if let Some((bi, item_summaries, special_events)) = next_item {
            match insert_block(
                &mut db,
                bi.block_hash,
                (bi.block_slot_time.timestamp_millis() as u64).into(),
                bi.block_height,
                &item_summaries,
                &special_events,
            )
            .await
            {
                Ok(time) => {
                    successive_errors = 0;
                    log::info!(
                        "Processed block {} at height {} in {}ms.",
                        bi.block_hash,
                        bi.block_height,
                        time.num_milliseconds()
                    );
                }
                Err(e) => {
                    successive_errors += 1;
                    // wait for 2^(min(successive_errors - 1, 7)) seconds before attempting.
                    // The reason for the min is that we bound the time between reconnects.
                    let delay = std::time::Duration::from_millis(
                        500 * (1 << std::cmp::min(successive_errors, 8)),
                    );
                    log::error!(
                        "Database connection lost due to {:#}. Will attempt to reconnect in {}ms.",
                        e,
                        delay.as_millis()
                    );
                    tokio::time::sleep(delay).await;
                    let new_db = match try_reconnect(&config, &stop_flag, false).await {
                        Ok(db) => db,
                        Err(e) => {
                            receiver.close();
                            return Err(e);
                        }
                    };
                    // and drop the old database.
                    let old_db = std::mem::replace(&mut db, new_db);
                    match old_db.client.stop().await {
                        Ok(v) => {
                            if let Err(e) = v {
                                log::warn!(
                                    "Could not correctly stop the old database connection due to: \
                                     {}.",
                                    e
                                );
                            }
                        }
                        Err(e) => {
                            if e.is_panic() {
                                log::warn!(
                                    "Could not correctly stop the old database connection. The \
                                     connection thread panicked."
                                );
                            } else {
                                log::warn!("Could not correctly stop the old database connection.");
                            }
                        }
                    }
                    retry = Some((bi, item_summaries, special_events));
                }
            }
        } else {
            break;
        }
    }
    // stop the database connection.
    receiver.close();
    db.client.stop().await??;
    Ok(())
}

/// Construct a future for shutdown signals (for unix: SIGINT and SIGTERM) (for
/// windows: ctrl c and ctrl break). The signal handler is set when the future
/// is polled and until then the default signal handler.
async fn set_shutdown(flag: Arc<AtomicBool>) -> anyhow::Result<()> {
    #[cfg(unix)]
    {
        use tokio::signal::unix as unix_signal;
        let mut terminate_stream = unix_signal::signal(unix_signal::SignalKind::terminate())?;
        let mut interrupt_stream = unix_signal::signal(unix_signal::SignalKind::interrupt())?;
        let terminate = Box::pin(terminate_stream.recv());
        let interrupt = Box::pin(interrupt_stream.recv());
        futures::future::select(terminate, interrupt).await;
        flag.store(true, Ordering::Release);
    }
    #[cfg(windows)]
    {
        use tokio::signal::windows as windows_signal;
        let mut ctrl_break_stream = windows_signal::ctrl_break()?;
        let mut ctrl_c_stream = windows_signal::ctrl_c()?;
        let ctrl_break = Box::pin(ctrl_break_stream.recv());
        let ctrl_c = Box::pin(ctrl_c_stream.recv());
        futures::future::select(ctrl_break, ctrl_c).await;
        flag.store(true, Ordering::Release);
    }
    Ok(())
}

#[tokio::main(flavor = "multi_thread")]
async fn main() -> anyhow::Result<()> {
    let app = {
        let app = App::clap()
            // .setting(AppSettings::ArgRequiredElseHelp)
            .global_setting(AppSettings::ColoredHelp);
        let matches = app.get_matches();
        App::from_clap(&matches)
    };
    anyhow::ensure!(!app.endpoint.is_empty(), "At least one node must be provided.");
    let config = app.config;

    let mut log_builder = env_logger::Builder::from_env("TRANSACTION_LOGGER_LOG");
    // only log the current module (main).
    log_builder.filter_module(module_path!(), app.log_level);
    log_builder.init();

    // This program is set up as follows.
    // It uses tokio to manage tasks. The reason for this is that it is the
    // expectation that the main bottleneck is IO, either writing to the database or
    // waiting for responses from the node.
    // A background task is spawned whose only purpose is to write to the database.
    // That task only terminates if the connection to the database is lost and
    // cannot be established within half a minute or so (duration is governed by
    // MAX_CONNECT_ATTEMPTS and exponential backoff).
    //
    // In the main task we query the nodes for block summaries. If querying the node
    // fails, or the node is deemed too behind then we try the next node. The given
    // nodes are tried in sequence and cycled when the end of the sequence is
    // reached.
    //
    // The main task and the database writer task communicate via a bounded channel,
    // with the main task sending block info and block summaries to the database
    // writer thread.

    // Since the database connection is managed by the background task we use a
    // oneshot channel to get the height we should start querying at. First the
    // background database task is started which then sends the height over this
    // channel.
    let (height_sender, height_receiver) = tokio::sync::oneshot::channel();
    // Create a channel between the task querying the node and the task logging
    // transactions.
    let (sender, receiver) = tokio::sync::mpsc::channel(100);

    let stop_flag = Arc::new(AtomicBool::new(false));

    let shutdown_handler_handle = tokio::spawn(set_shutdown(stop_flag.clone()));

    let db_write_handle =
        tokio::spawn(write_to_db(config, height_sender, receiver, stop_flag.clone()));
    // The height we should start querying the node at.
    // If the sender died we simply terminate the program.
    let mut height = height_receiver.await?;

    // Cache where we store the mapping from account aliases to canonical account
    // addresses. This is done by storing just equivalence classes represented
    // by the canonical address.
    let mut canonical_cache = HashSet::new();
    // To make sure we do not end up in an infinite loop in case all reconnects fail
    // we count reconnects. We deem a node connection successful if it increases
    // maximum achieved height by at least 1.
    let mut max_height = height;
    let mut last_success = 0;
    let num_nodes = app.endpoint.len() as u64;
    for (node_ep, idx) in app.endpoint.into_iter().cycle().zip(0u64..) {
        if stop_flag.load(Ordering::Acquire) {
            break;
        }
        if idx.saturating_sub(last_success) >= num_nodes {
            // we skipped all the nodes without success.
            let delay = std::time::Duration::from_secs(5);
            log::error!(
                "Connections to all nodes have failed. Pausing for {}s before trying node {} \
                 again.",
                delay.as_secs(),
                node_ep.uri()
            );
            tokio::time::sleep(delay).await;
        }
        // connect to the node.
        log::info!("Attempting to use node {}", node_ep.uri());

        let node_result = use_node(
            node_ep,
            &sender,
            &mut height,
            app.num_parallel,
            stop_flag.as_ref(),
            &mut canonical_cache,
            app.max_behind,
        )
        .await;

        match node_result {
            Err(NodeError::ConnectionError(e)) => {
                log::warn!("Failed to connect to node due to {:#}. Will attempt another node.", e);
            }
            Err(NodeError::OtherError(e)) => {
                log::warn!("Node query failed due to: {:#}. Will attempt another node.", e);
            }
            Err(NodeError::NetworkError(e)) => {
                log::warn!("Failed to connect to node due to {:#}. Will attempt another node.", e);
            }
            Err(NodeError::QueryError(e)) => {
                log::warn!("Failed to connect to node due to {:#}. Will attempt another node.", e);
            }
            Err(NodeError::Timeout) => {
                log::warn!("Node too far behind. Will attempt another node.");
            }
            Ok(()) => break,
        }
        if height > max_height {
            last_success = idx;
            max_height = height;
        }
    }
    db_write_handle.abort();
    shutdown_handler_handle.abort();
    Ok(())
}
