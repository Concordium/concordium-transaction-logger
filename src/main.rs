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
    v2::{self, FinalizedBlockInfo},
};
use futures::TryStreamExt;
use std::{collections::HashSet, convert::TryFrom, hash::Hash};
use structopt::StructOpt;
use tokio_postgres::{
    types::{Json, ToSql},
    Transaction as DBTransaction,
};
use tonic::async_trait;
use transaction_logger::{
    run_service, BlockInsertSuccess, DatabaseHooks, NodeError, NodeHooks, PrepareStatements,
    SharedIndexerArgs,
};

type DBConn = transaction_logger::DBConn<PreparedStatements>;

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
    pub block_hash: BlockHash,
    /// Slot time of the block the row applies to.
    pub block_time: Timestamp,
    /// Block height stored in the database.
    pub block_height: AbsoluteBlockHeight,
    /// Summary of the item. Either a user-generated transaction, or a protocol
    /// event that affected the account or contract.
    pub summary: BorrowedDatabaseSummaryEntry<'a>,
}

struct BlockItemSummaryWithCanonicalAddresses {
    pub(crate) summary: BlockItemSummary,
    /// Affected addresses, resolved to canonical addresses and without
    /// duplicates.
    pub(crate) addresses: Vec<AccountAddress>,
}

/// Prepared statements for all insertions. Prepared statements are
/// per-connection so we have to re-create them each time we reconnect.
struct PreparedStatements {
    /// Insert into the summary table.
    insert_summary: tokio_postgres::Statement,
    /// Insert into the account transaction index table.
    insert_ati: tokio_postgres::Statement,
    /// Insert into the contract transaction index table.
    insert_cti: tokio_postgres::Statement,
    /// Increase the total supply of a given token.
    cis2_increase_total_supply: tokio_postgres::Statement,
    /// Decrease the total supply of a given token.
    cis2_decrease_total_supply: tokio_postgres::Statement,
}

#[async_trait]
impl PrepareStatements for PreparedStatements {
    async fn prepare_all(client: &mut DatabaseClient) -> Result<Self, postgres::Error> {
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

        return Ok(Self {
            insert_summary,
            insert_ati,
            insert_cti,
            cis2_increase_total_supply,
            cis2_decrease_total_supply,
        });
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

/// Data sent by the node query task to the transaction insertion task.
/// Contains all the information needed to build the transaction index.
type TransactionLogData =
    (BlockInfo, Vec<BlockItemSummaryWithCanonicalAddresses>, Vec<SpecialTransactionOutcome>);

#[repr(transparent)]
#[derive(Eq, Debug, Clone, Copy)]
struct AccountAddressEq(AccountAddress);

impl From<AccountAddressEq> for AccountAddress {
    fn from(aae: AccountAddressEq) -> Self {
        aae.0
    }
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
    fn as_ref(&self) -> &AccountAddressEq {
        unsafe { std::mem::transmute(self) }
    }
}

/// Return Err if querying the node failed.
/// Return Ok(()) if the channel to the database was closed.
// #[allow(clippy::too_many_arguments)]
// async fn use_node(
//     node_ep: v2::Endpoint,
//     sender: &tokio::sync::mpsc::Sender<TransactionLogData>,
//     height: &mut AbsoluteBlockHeight, // start height
//     max_parallel: u32,
//     stop_flag: &AtomicBool,
//     canonical_cache: &mut HashSet<AccountAddressEq>,
//     max_behind: u32, // maximum number of seconds a node can be behind before it is deemed "behind"
// ) -> Result<(), NodeError> {
//     // Use TLS if the URI scheme is HTTPS.
//     // This uses whatever system certificates have been installed as trusted roots.
//     let node_ep = if node_ep.uri().scheme().map_or(false, |x| x == &http::uri::Scheme::HTTPS) {
//         node_ep.tls_config(ClientTlsConfig::new()).map_err(NodeError::ConnectionError)?
//     } else {
//         node_ep
//     };

//     let mut node = v2::Client::new(node_ep).await.map_err(NodeError::ConnectionError)?;
//     // if the cache is empty we seed it with all accounts at the last finalized
//     // block. This will only happen the first time this function is successfully
//     // invoked.
//     if canonical_cache.is_empty() {
//         let accounts_response = node
//             .get_account_list(&v2::BlockIdentifier::LastFinal)
//             .await
//             .context("Error querying account list.")?;
//         let accounts = accounts_response.response;
//         let r = accounts
//             .try_fold(HashSet::new(), |mut cc, addr| async move {
//                 let _ = cc.insert(AccountAddressEq(addr));
//                 Ok(cc)
//             })
//             .await?;
//         *canonical_cache = r;
//     }
//     let mut finalized_blocks = node.get_finalized_blocks_from(*height).await?;
//     let timeout = std::time::Duration::from_secs(max_behind.into());
//     while !stop_flag.load(Ordering::Acquire) {
//         let (error, chunk) = finalized_blocks
//             .next_chunk_timeout(max_parallel as usize, timeout)
//             .await
//             .map_err(|_| NodeError::Timeout)?;
//         let mut futures = futures::stream::FuturesOrdered::new();
//         for fb in chunk {
//             let mut node = node.clone();
//             futures.push_back(async move {
//                 let binfo = node.get_block_info(fb.block_hash).await?;
//                 let events = if binfo.response.transaction_count == 0 {
//                     Vec::new()
//                 } else {
//                     node.get_block_transaction_events(fb.block_hash)
//                         .await?
//                         .response
//                         .try_collect()
//                         .await?
//                 };
//                 let special = node
//                     .get_block_special_events(fb.block_hash)
//                     .await?
//                     .response
//                     .try_collect()
//                     .await?;
//                 Ok::<
//                     (BlockInfo, Vec<BlockItemSummary>, Vec<SpecialTransactionOutcome>),
//                     anyhow::Error,
//                 >((binfo.response, events, special))
//             });
//         }
//         while let Some(result) = futures.next().await {
//             let (binfo, transaction_summaries, special_events) = result?;
//             let mut with_addresses = Vec::with_capacity(transaction_summaries.len());
//             for summary in transaction_summaries {
//                 let affected_addresses = summary.affected_addresses();
//                 let mut addresses = Vec::with_capacity(affected_addresses.len());
//                 // resolve canonical addresses. This part is only needed because the index
//                 // is currently expected by "canonical address",
//                 // which is only possible to resolve by querying the node.
//                 let mut seen = HashSet::with_capacity(affected_addresses.len());
//                 for address in affected_addresses.into_iter() {
//                     if let Some(addr) = canonical_cache.get(address.as_ref()) {
//                         let addr = AccountAddress::from(*addr);
//                         if seen.insert(addr) {
//                             addresses.push(addr);
//                         }
//                     } else {
//                         let ainfo = node
//                             .get_account_info(&address.into(), binfo.block_hash)
//                             .await
//                             .context("Error querying account info.")?
//                             .response;
//                         let addr = ainfo.account_address;
//                         if seen.insert(addr) {
//                             log::debug!("Discovered new address {}", addr);
//                             addresses.push(addr);
//                             canonical_cache.insert(AccountAddressEq(addr));
//                         } else {
//                             log::debug!("Canonical address {} already listed.", addr);
//                         }
//                     }
//                 }
//                 with_addresses.push(BlockItemSummaryWithCanonicalAddresses {
//                     summary,
//                     addresses,
//                 })
//             }
//             if sender.send((binfo, with_addresses, special_events)).await.is_err() {
//                 log::error!("The database connection has been closed. Terminating node queries.");
//                 return Ok(());
//             }
//             *height = height.next();
//         }
//         if error {
//             // we have processed the blocks we can, but further queries on the same stream
//             // will fail since the stream signalled an error.
//             return Err(NodeError::OtherError(anyhow::anyhow!("Finalized block stream dropped.")));
//         }
//     }
//     Ok(())
// }

/// Attempt to extract CIS2 events from the block item.
/// If the transaction is a smart contract init or update transaction then
/// attempt to parse the events as CIS2 events. If any of the events fail
/// parsing then the logs for that section of execution are ignored, since it
/// indicates an error in the contract.
///
/// The return value of [`None`] means there are no understandable CIS2 logs
/// produced.
fn get_cis2_events(bi: &BlockItemSummary) -> Option<Vec<(ContractAddress, Vec<cis2::Event>)>> {
    match bi.contract_update_logs() {
        Some(log_iter) => Some(
            log_iter
                .flat_map(|(ca, logs)| {
                    match logs
                        .iter()
                        .map(cis2::Event::try_from)
                        .collect::<Result<Vec<cis2::Event>, _>>()
                    {
                        Ok(events) => Some((ca, events)),
                        Err(_) => None,
                    }
                })
                .collect(),
        ),
        None => {
            let init = bi.contract_init()?;
            let cis2 = init
                .events
                .iter()
                .map(cis2::Event::try_from)
                .collect::<Result<Vec<cis2::Event>, _>>()
                .ok()?;
            Some(vec![(init.address, cis2)])
        }
    }
}

struct DatabaseDelegate;

#[async_trait]
impl DatabaseHooks<TransactionLogData, PreparedStatements> for DatabaseDelegate {
    async fn insert_into_db(
        db_conn: &mut DBConn,
        (bi, item_summaries, special_events): &TransactionLogData,
    ) -> Result<BlockInsertSuccess, postgres::Error> {
        let res = insert_block(
            db_conn,
            bi.block_hash,
            (bi.block_slot_time.timestamp_millis() as u64).into(),
            bi.block_height,
            item_summaries,
            special_events,
        )
        .await;

        res.map(|time| BlockInsertSuccess {
            time,
            block_height: bi.block_height,
            block_hash: bi.block_hash,
        })
    }

    async fn on_request_max_height(
        db: &DatabaseClient,
    ) -> Result<Option<AbsoluteBlockHeight>, postgres::Error> {
        get_last_block_height(db).await
    }
}

struct NodeDelegate {
    canonical_cache: HashSet<AccountAddressEq>,
}

#[async_trait]
impl NodeHooks<TransactionLogData> for NodeDelegate {
    async fn on_use_node(&mut self, client: &mut v2::Client) -> Result<(), NodeError> {
        if self.canonical_cache.is_empty() {
            let accounts_response = client
                .get_account_list(&v2::BlockIdentifier::LastFinal)
                .await
                .context("Error querying account list.")?;
            let accounts = accounts_response.response;
            let accounts = accounts
                .try_fold(HashSet::new(), |mut cc, addr| async move {
                    let _ = cc.insert(AccountAddressEq(addr));
                    Ok(cc)
                })
                .await?;

            self.canonical_cache = accounts;
        }

        Ok(())
    }

    async fn on_finalized_block(
        &mut self,
        client: &mut v2::Client,
        finalized_block_info: &FinalizedBlockInfo,
    ) -> Result<TransactionLogData, NodeError> {
        todo!()
    }
}

#[tokio::main(flavor = "multi_thread")]
async fn main() -> anyhow::Result<()> {
    let app = {
        let app = SharedIndexerArgs::clap()
            // .setting(AppSettings::ArgRequiredElseHelp)
            .global_setting(AppSettings::ColoredHelp);
        let matches = app.get_matches();
        SharedIndexerArgs::from_clap(&matches)
    };
    anyhow::ensure!(!app.endpoint.is_empty(), "At least one node must be provided.");

    let sql_schema = include_str!("../resources/schema.sql");
    let node_hooks = NodeDelegate {
        canonical_cache: HashSet::new(),
    };

    run_service::<TransactionLogData, PreparedStatements, DatabaseDelegate, NodeDelegate>(
        sql_schema, app, node_hooks,
    )
    .await?;

    Ok(())
}

// #[tokio::main(flavor = "multi_thread")]
// async fn main() -> anyhow::Result<()> {
//     let app = {
//         let app = SharedIndexerArgs::clap()
//             // .setting(AppSettings::ArgRequiredElseHelp)
//             .global_setting(AppSettings::ColoredHelp);
//         let matches = app.get_matches();
//         SharedIndexerArgs::from_clap(&matches)
//     };
//     anyhow::ensure!(!app.endpoint.is_empty(), "At least one node must be provided.");
//     let config = app.config;

//     let mut log_builder = env_logger::Builder::from_env("TRANSACTION_LOGGER_LOG");
//     // only log the current module (main).
//     log_builder.filter_module(module_path!(), app.log_level);
//     log_builder.init();

//     // This program is set up as follows.
//     // It uses tokio to manage tasks. The reason for this is that it is the
//     // expectation that the main bottleneck is IO, either writing to the database or
//     // waiting for responses from the node.
//     // A background task is spawned whose only purpose is to write to the database.
//     // That task only terminates if the connection to the database is lost and
//     // cannot be established within half a minute or so (duration is governed by
//     // MAX_CONNECT_ATTEMPTS and exponential backoff).
//     //
//     // In the main task we query the nodes for block summaries. If querying the node
//     // fails, or the node is deemed too behind then we try the next node. The given
//     // nodes are tried in sequence and cycled when the end of the sequence is
//     // reached.
//     //
//     // The main task and the database writer task communicate via a bounded channel,
//     // with the main task sending block info and block summaries to the database
//     // writer thread.

//     // Since the database connection is managed by the background task we use a
//     // oneshot channel to get the height we should start querying at. First the
//     // background database task is started which then sends the height over this
//     // channel.
//     let (height_sender, height_receiver) = tokio::sync::oneshot::channel();
//     // Create a channel between the task querying the node and the task logging
//     // transactions.
//     let (sender, receiver) = tokio::sync::mpsc::channel(100);

//     let stop_flag = Arc::new(AtomicBool::new(false));

//     let shutdown_handler_handle = tokio::spawn(set_shutdown(stop_flag.clone()));

//     let db_write_handle =
//         tokio::spawn(write_to_db(config, height_sender, receiver, stop_flag.clone()));
//     // The height we should start querying the node at.
//     // If the sender died we simply terminate the program.
//     let mut height = height_receiver.await?;

//     // Cache where we store the mapping from account aliases to canonical account
//     // addresses. This is done by storing just equivalence classes represented
//     // by the canonical address.
//     let mut canonical_cache = HashSet::new();
//     // To make sure we do not end up in an infinite loop in case all reconnects fail
//     // we count reconnects. We deem a node connection successful if it increases
//     // maximum achieved height by at least 1.
//     let mut max_height = height;
//     let mut last_success = 0;
//     let num_nodes = app.endpoint.len() as u64;
//     for (node_ep, idx) in app.endpoint.into_iter().cycle().zip(0u64..) {
//         if stop_flag.load(Ordering::Acquire) {
//             break;
//         }
//         if idx.saturating_sub(last_success) >= num_nodes {
//             // we skipped all the nodes without success.
//             let delay = std::time::Duration::from_secs(5);
//             log::error!(
//                 "Connections to all nodes have failed. Pausing for {}s before trying node {} \
//                  again.",
//                 delay.as_secs(),
//                 node_ep.uri()
//             );
//             tokio::time::sleep(delay).await;
//         }
//         // connect to the node.
//         log::info!("Attempting to use node {}", node_ep.uri());

//         let node_ep = node_ep
//             .connect_timeout(std::time::Duration::from_secs(app.connect_timeout.into()))
//             .timeout(std::time::Duration::from_secs(app.request_timeout.into()));

//         let node_result = use_node(
//             node_ep,
//             &sender,
//             &mut height,
//             app.num_parallel,
//             stop_flag.as_ref(),
//             &mut canonical_cache,
//             app.max_behind,
//         )
//         .await;

//         match node_result {
//             Err(NodeError::ConnectionError(e)) => {
//                 log::warn!("Failed to connect to node due to {:#}. Will attempt another node.", e);
//             }
//             Err(NodeError::OtherError(e)) => {
//                 log::warn!("Node query failed due to: {:#}. Will attempt another node.", e);
//             }
//             Err(NodeError::NetworkError(e)) => {
//                 log::warn!("Failed to connect to node due to {:#}. Will attempt another node.", e);
//             }
//             Err(NodeError::QueryError(e)) => {
//                 log::warn!("Failed to connect to node due to {:#}. Will attempt another node.", e);
//             }
//             Err(NodeError::Timeout) => {
//                 log::warn!("Node too far behind. Will attempt another node.");
//             }
//             Ok(()) => break,
//         }
//         if height > max_height {
//             last_success = idx;
//             max_height = height;
//         }
//     }
//     db_write_handle.abort();
//     shutdown_handler_handle.abort();
//     Ok(())
// }
