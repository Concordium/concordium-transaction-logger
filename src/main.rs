use anyhow::Context;
use clap::AppSettings;
use concordium_rust_sdk::{
    common::{types::Timestamp, SerdeSerialize},
    endpoints,
    id::types::AccountAddress,
    postgres,
    postgres::DatabaseClient,
    types::{
        hashes::BlockHash, queries::BlockInfo, AbsoluteBlockHeight, BlockItemSummary,
        SpecialTransactionOutcome,
    },
};
use std::{
    collections::HashSet,
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
        default_value = "http://localhost:10000",
        use_delimiter = true,
        env = "TRANSACTION_LOGGER_NODES"
    )]
    endpoint:     Vec<endpoints::Endpoint>,
    #[structopt(
        long = "rpc-token",
        help = "GRPC interface access token for accessing all the nodes.",
        default_value = "rpcadmin",
        env = "TRANSACTION_LOGGER_RPC_TOKEN"
    )]
    token:        String,
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

async fn insert_summary<'a, 'b>(
    tx: &DBTransaction<'a>,
    summary: &SummaryRow<'b>,
) -> Result<i64, postgres::Error> {
    let statement = "INSERT INTO summaries (block, timestamp, height, summary) VALUES ($1, $2, \
                     $3, $4) RETURNING id";
    let values = [
        &summary.block_hash.as_ref() as &(dyn ToSql + Sync),
        &(summary.block_time.millis as i64) as &(dyn ToSql + Sync),
        &(summary.block_height.height as i64) as &(dyn ToSql + Sync),
        &Json(&summary.summary) as &(dyn ToSql + Sync),
    ];
    let res = tx.query_one(statement, &values).await?;
    let id = res.try_get::<_, i64>(0)?;
    Ok(id)
}

/// Insert an account transaction.
async fn insert_transaction(
    tx: &DBTransaction<'_>,
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
    // TODO: Use prepared statement for efficiency.
    let id = insert_summary(tx, &summary_row).await?;
    let statement = "INSERT INTO ati (account, summary) VALUES ($1, $2)";
    for affected in affected_addresses.iter() {
        let addr_bytes: &[u8; 32] = affected.as_ref();
        let values = [
            &&addr_bytes[..] as &(dyn ToSql + Sync),
            &id as &(dyn ToSql + Sync),
        ];
        tx.query_opt(statement, &values).await?;
    }
    // insert contracts
    let statement_contract = "INSERT INTO cti (index, subindex, summary) VALUES ($1, $2, $3)";
    for affected in ts.summary.affected_contracts() {
        let index = affected.index;
        let subindex = affected.subindex;
        let values = [
            &(index.index as i64) as &(dyn ToSql + Sync),
            &(subindex.sub_index as i64),
            &id,
        ];
        tx.query_opt(statement_contract, &values).await?;
    }
    Ok(())
}

/// Insert special outcomes.
async fn insert_special(
    tx: &DBTransaction<'_>,
    block_hash: BlockHash,
    block_time: Timestamp,
    block_height: AbsoluteBlockHeight,
    so: &SpecialTransactionOutcome,
) -> Result<(), postgres::Error> {
    // these are canonical addresses so there is no need to resolve anything
    // additional.
    let affected_addresses = match &so {
        SpecialTransactionOutcome::BakingRewards { baker_rewards, .. } => {
            baker_rewards.keys().copied().collect::<Vec<_>>()
        }
        SpecialTransactionOutcome::Mint {
            foundation_account, ..
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
    };
    let summary_row = SummaryRow {
        block_hash,
        block_time,
        block_height,
        summary: BorrowedDatabaseSummaryEntry::ProtocolEvent(so),
    };
    let id = insert_summary(tx, &summary_row).await?;
    let statement = "INSERT INTO ati (account, summary) VALUES ($1, $2)";
    for affected in affected_addresses.iter() {
        let addr_bytes: &[u8; 32] = affected.as_ref();
        let values = [
            &&addr_bytes[..] as &(dyn ToSql + Sync),
            &id as &(dyn ToSql + Sync),
        ];
        tx.query_opt(statement, &values).await?;
    }
    Ok(())
}

struct BlockItemSummaryWithCanonicalAddresses {
    pub(crate) summary:   BlockItemSummary,
    /// Affected addresses, resolved to canonical addresses and without
    /// duplicates.
    pub(crate) addresses: Vec<AccountAddress>,
}

async fn insert_block(
    db: &mut DatabaseClient,
    block_hash: BlockHash,
    block_time: Timestamp,
    block_height: AbsoluteBlockHeight,
    item_summaries: &[BlockItemSummaryWithCanonicalAddresses],
    special_events: &[SpecialTransactionOutcome],
) -> Result<(), postgres::Error> {
    let db_tx = db.as_mut().transaction().await?;
    for transaction in item_summaries.iter() {
        insert_transaction(&db_tx, block_hash, block_time, block_height, transaction).await?;
    }
    for special in special_events.iter() {
        insert_special(&db_tx, block_hash, block_time, block_height, special).await?;
    }
    db_tx.commit().await?;
    Ok(())
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

async fn create_tables(db: &DatabaseClient) -> Result<(), postgres::Error> {
    let create_summaries = "CREATE TABLE IF NOT EXISTS summaries(id SERIAL8 PRIMARY KEY UNIQUE, \
                            block BYTEA NOT NULL, timestamp INT8 NOT NULL, height INT8 NOT NULL, \
                            summary JSONB NOT NULL)";
    let create_ati = "CREATE TABLE IF NOT EXISTS ati(id SERIAL8, account BYTEA NOT NULL, summary \
                      INT8 NOT NULL, CONSTRAINT ati_pkey PRIMARY KEY (account, id), CONSTRAINT \
                      ati_summary_fkey FOREIGN KEY(summary) REFERENCES summaries(id) ON DELETE \
                      RESTRICT  ON UPDATE RESTRICT)";
    let create_cti = "CREATE TABLE IF NOT EXISTS cti(id SERIAL8, index INT8 NOT NULL,subindex \
                      INT8 NOT NULL,summary INT8 NOT NULL, CONSTRAINT cti_pkey PRIMARY KEY \
                      (index, subindex, id), CONSTRAINT cti_summary_fkey FOREIGN KEY(summary) \
                      REFERENCES summaries(id) ON DELETE RESTRICT ON UPDATE RESTRICT)";
    db.as_ref().batch_execute(create_summaries).await?;
    db.as_ref().batch_execute(create_ati).await?;
    db.as_ref().batch_execute(create_cti).await
}

const WAIT_MILLISECONDS: u64 = 500;
const MAX_CONNECT_ATTEMPTS: u64 = 6;

async fn check_node_and_wait(node: &mut endpoints::Client, max_behind: u32) -> anyhow::Result<()> {
    let info = node.get_consensus_status().await?;
    let now = chrono::Utc::now();
    if let Some((lft, duration)) = info
        .last_finalized_time
        .map(|t| (t, t.signed_duration_since(now)))
    {
        let secs = duration.num_seconds();
        if !(0..=max_behind.into()).contains(&secs) {
            anyhow::bail!("Node is likely behind. Its last finalized time is {}.", lft);
        } else {
            tokio::time::sleep(std::time::Duration::from_millis(WAIT_MILLISECONDS)).await;
            Ok(())
        }
    } else {
        anyhow::bail!("Node is likely behind since it has never witnessed a finalization.");
    }
}

enum QueryFailure {
    NoBlocks,
    NotFinalized,
    NotFound,
}

/// Data sent by the node query task to the transaction insertion task.
/// Contains all the information needed to build the transaction index.
type TransactionLogData = (
    BlockInfo,
    Vec<BlockItemSummaryWithCanonicalAddresses>,
    Vec<SpecialTransactionOutcome>,
);

#[repr(transparent)]
#[derive(Eq, Debug, Clone, Copy)]
struct AccountAddressEq(AccountAddress);

impl From<AccountAddressEq> for AccountAddress {
    fn from(aae: AccountAddressEq) -> Self { aae.0 }
}

impl PartialEq for AccountAddressEq {
    fn eq(&self, other: &Self) -> bool {
        let bytes_1: &[u8; 32] = &self.0.as_ref();
        let bytes_2: &[u8; 32] = &other.0.as_ref();
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
    /// Query errors, etc.
    #[error("Error querying the node {0}.")]
    OtherError(#[from] anyhow::Error),
}

/// Return Err if querying the node failed.
/// Return Ok(()) if the channel to the database was closed.
async fn use_node(
    node_ep: endpoints::Endpoint,
    token: &str,
    sender: &tokio::sync::mpsc::Sender<TransactionLogData>,
    height: &mut AbsoluteBlockHeight, // start height
    max_parallel: u32,
    stop_flag: &AtomicBool,
    canonical_cache: &mut HashSet<AccountAddressEq>,
    max_behind: u32, // maximum number of seconds a node can be behind before it is deemed "behind"
) -> Result<(), NodeError> {
    let mut node = endpoints::Client::connect(node_ep, token.into())
        .await
        .map_err(NodeError::ConnectionError)?;
    // if the cache is empty we seed it with all accounts at the last finalized
    // block. This will only happen the first time this function is successfully
    // invoked.
    if canonical_cache.is_empty() {
        let info = node
            .get_consensus_status()
            .await
            .context("Error querying consensus status.")?;
        let accounts = node
            .get_account_list(&info.last_finalized_block)
            .await
            .context("Error querying account list.")?;
        // this relies on the fact that get_account_list returns canonical addresses.
        log::debug!(
            "Initializing the address cache with {} accounts.",
            accounts.len()
        );
        for addr in accounts {
            canonical_cache.insert(AccountAddressEq(addr));
        }
    }
    let mut parallel = 1;
    while !stop_flag.load(Ordering::Acquire) {
        let mut handles = Vec::with_capacity(parallel as usize);
        for h in u64::from(*height)..u64::from(*height) + parallel {
            let mut node = node.clone();
            handles.push(async move {
                let height: AbsoluteBlockHeight = h.into();
                let blocks_at_height = node.get_blocks_at_height(height.into()).await?;
                if blocks_at_height.len() != 1 {
                    return Ok(Err(QueryFailure::NoBlocks));
                }
                let block = blocks_at_height[0];
                let info = node.get_block_info(&block).await;
                let info = match info {
                    Ok(info) if !info.finalized => {
                        return Ok(Err(QueryFailure::NotFinalized));
                    }
                    Ok(info) => info,
                    Err(endpoints::QueryError::NotFound) => {
                        return Ok(Err(QueryFailure::NotFound));
                    }
                    Err(endpoints::QueryError::RPCError(e)) => return Err(e.into()),
                };
                let summary = node.get_block_summary(&block).await?;
                Ok::<_, anyhow::Error>(Ok((info, summary)))
            });
        }
        let mut success = true;
        for result in futures::future::join_all(handles).await {
            match result? {
                Ok((info, summary)) => {
                    let mut with_addresses =
                        Vec::with_capacity(summary.transaction_summaries.len());
                    for summary in summary.transaction_summaries.into_iter() {
                        let affected_addresses = summary.affected_addresses();
                        let mut addresses = Vec::with_capacity(affected_addresses.len());
                        // resolve canonical addresses. This part is only needed because the index
                        // is currently expected by "canonical address",
                        // which is only possible to resolve by querying the node.
                        let mut seen = HashSet::with_capacity(affected_addresses.len());
                        for address in affected_addresses.into_iter() {
                            if let Some(addr) = canonical_cache.get(address.as_ref()) {
                                let addr = AccountAddress::from(*addr);
                                if !seen.insert(addr) {
                                    addresses.push(addr);
                                }
                            } else {
                                let ainfo = node
                                    .get_account_info(address, &info.block_hash)
                                    .await
                                    .context("Error querying account info.")?;
                                let addr = ainfo.account_address();
                                if !seen.insert(addr) {
                                    log::debug!("Discovered new address {}", addr);
                                    addresses.push(addr);
                                    canonical_cache.insert(AccountAddressEq(addr));
                                } else {
                                    log::debug!("Canonical address {} already listed.", addr);
                                }
                            }
                        }
                        with_addresses
                            .push(BlockItemSummaryWithCanonicalAddresses { summary, addresses })
                    }
                    if sender
                        .send((info, with_addresses, summary.special_events))
                        .await
                        .is_err()
                    {
                        log::warn!(
                            "The database connection has been closed. Terminating node queries."
                        );
                        return Ok(());
                    }
                    *height = height.next();
                }
                Err(_) => {
                    success = false;
                    // if we failed one height we drop the rest since we likely failed those as
                    // well. In any case we must do things in order.
                    check_node_and_wait(&mut node, max_behind).await?;
                    break;
                }
            }
        }
        if success {
            parallel = std::cmp::min(parallel + 1, max_parallel.into());
        } else {
            parallel = 1;
        }
    }
    Ok(())
}

/// Try to reconnect to the database with exponential backoff, at most
/// MAX_CONNECT_ATTEMPTS times.
async fn try_reconnect(
    config: &postgres::Config,
    stop_flag: &AtomicBool,
) -> anyhow::Result<DatabaseClient> {
    let mut i = 1;
    while !stop_flag.load(Ordering::Acquire) {
        match DatabaseClient::create(config.clone(), postgres::NoTls).await {
            Ok(c) => return Ok(c),
            Err(e) if i < MAX_CONNECT_ATTEMPTS => {
                let delay = std::time::Duration::from_millis(500 * (1 << i));
                log::error!(
                    "Could not connect to the database due to {}. Reconnecting in {}ms",
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
                     reason {}.",
                    MAX_CONNECT_ATTEMPTS,
                    e
                );
                return Err(e.into());
            }
        }
    }
    anyhow::bail!("The node was requested to stop.")
}

async fn write_to_db(
    config: postgres::Config,
    height_sender: tokio::sync::oneshot::Sender<AbsoluteBlockHeight>, // start height
    mut receiver: tokio::sync::mpsc::Receiver<TransactionLogData>,
    stop_flag: Arc<AtomicBool>,
) -> anyhow::Result<()> {
    let mut db = try_reconnect(&config, &stop_flag).await?;
    create_tables(&db).await?;

    let height = get_last_block_height(&db)
        .await?
        .map_or(0.into(), |h| h.next());
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
            if let Err(e) = insert_block(
                &mut db,
                bi.block_hash,
                (bi.block_slot_time.timestamp_millis() as u64).into(),
                bi.block_height,
                &item_summaries,
                &special_events,
            )
            .await
            {
                successive_errors += 1;
                // wait for 2^(min(successive_errors - 1, 7)) seconds before attempting.
                // The reason for the min is that we bound the time between reconnects.
                let delay = std::time::Duration::from_millis(
                    500 * (1 << std::cmp::min(successive_errors, 8)),
                );
                log::error!(
                    "Database connection lost due to {}. Will attempt to reconnect in {}ms.",
                    e,
                    delay.as_millis()
                );
                tokio::time::sleep(delay).await;
                let new_db = match try_reconnect(&config, &stop_flag).await {
                    Ok(db) => db,
                    Err(e) => {
                        receiver.close();
                        return Err(e);
                    }
                };
                // and drop the old database.
                let old_db = std::mem::replace(&mut db, new_db);
                match old_db.stop().await {
                    Ok(v) => {
                        if let Err(e) = v {
                            log::warn!(
                                "Could not correctly stop the old database connection due to: {}.",
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
            } else {
                successive_errors = 0;
                log::debug!(
                    "Processed block {} at height {}.",
                    bi.block_hash,
                    bi.block_height
                );
            }
        } else {
            break;
        }
    }
    // stop the database connection.
    receiver.close();
    db.stop().await??;
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
    anyhow::ensure!(
        !app.endpoint.is_empty(),
        "At least one node must be provided."
    );
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

    let db_write_handle = tokio::spawn(write_to_db(
        config,
        height_sender,
        receiver,
        stop_flag.clone(),
    ));
    // The height we should start querying the node at.
    // If the sender died we simply terminate the program.
    let mut height = height_receiver.await?;

    // Cache where we store the mapping from account aliases to canonical account
    // addresses. This is done by storing just equivalence classes represented
    // by the canonical address.
    let mut canonical_cache = HashSet::new();
    // To make sure we do not end up in an infinite loop in case all reconnects fail
    // we count reconnects.
    let mut last_success: u64 = 0;
    let num_nodes = app.endpoint.len() as u64;
    for (node_ep, idx) in app.endpoint.into_iter().cycle().zip(0u64..) {
        if stop_flag.load(Ordering::Acquire) {
            break;
        }
        if idx.saturating_sub(last_success) >= num_nodes {
            // we skipped all the nodes without success.
            let delay = std::time::Duration::from_secs(5);
            log::debug!(
                "Connections to all nodes have failed. Pausing for {}s before trying node {} \
                 again.",
                delay.as_secs(),
                node_ep.uri()
            );
            tokio::time::sleep(delay).await;
        }
        // connect to the node.
        log::debug!("Attempting to use node {}", node_ep.uri());
        match use_node(
            node_ep,
            &app.token,
            &sender,
            &mut height,
            app.num_parallel,
            stop_flag.as_ref(),
            &mut canonical_cache,
            app.max_behind,
        )
        .await
        {
            Err(NodeError::ConnectionError(e)) => {
                log::error!(
                    "Failed to connect to node due to {}. Will attempt another node.",
                    e
                );
            }
            Err(NodeError::OtherError(e)) => {
                last_success = idx;
                log::error!(
                    "Node query failed due to: {}. Will attempt another node.",
                    e
                );
            }
            Ok(()) => break,
        }
    }
    db_write_handle.abort();
    shutdown_handler_handle.abort();
    Ok(())
}
