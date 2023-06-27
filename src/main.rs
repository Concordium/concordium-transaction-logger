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
    run_service, set_shutdown, BlockInsertSuccess, DatabaseError, DatabaseHooks, NodeError,
    NodeHooks, PrepareStatements, SharedIndexerArgs,
};

type DBConn = transaction_logger::DBConn<PreparedStatements>;

const MAX_CONNECT_ATTEMPTS: u32 = 6;

/// A collection on arguments necessary to run the service. These are supplied
/// via the command line.
#[derive(StructOpt)]
struct Args {
    #[structopt(
        long = "node",
        help = "GRPC interface of the node(s).",
        default_value = "http://localhost:20000",
        use_delimiter = true,
        env = "TRANSACTION_LOGGER_NODES"
    )]
    endpoint:        Vec<v2::Endpoint>,
    #[structopt(
        long = "db",
        default_value = "host=localhost dbname=transaction-outcome user=postgres \
                         password=password port=5432",
        help = "Database connection string.",
        env = "TRANSACTION_LOGGER_DB_STRING"
    )]
    config:          postgres::Config,
    #[structopt(
        long = "log-level",
        default_value = "off",
        help = "Maximum log level.",
        env = "TRANSACTION_LOGGER_LOG_LEVEL"
    )]
    log_level:       log::LevelFilter,
    #[structopt(
        long = "num-parallel",
        default_value = "1",
        help = "Maximum number of parallel queries to make to the node. Usually 1 is the correct \
                number, but during initial catchup it is useful to increase this to, say 8 to \
                take advantage of parallelism in queries.",
        env = "TRANSACTION_LOGGER_NUM_PARALLEL_QUERIES"
    )]
    num_parallel:    u32,
    #[structopt(
        long = "max-behind-seconds",
        default_value = "240",
        help = "Maximum number of seconds the node's last finalization can be behind before the \
                node is given up and another one is tried.",
        env = "TRANSACTION_LOGGER_MAX_BEHIND_SECONDS"
    )]
    max_behind:      u32,
    #[structopt(
        long = "connect-timeout",
        default_value = "10",
        help = "Connection timeout for connecting to the node, in seconds",
        env = "TRANSACTION_LOGGER_CONNECT_TIMEOUT"
    )]
    connect_timeout: u32,
    #[structopt(
        long = "request-timeout",
        default_value = "60",
        help = "Request timeout for node requests, in seconds",
        env = "TRANSACTION_LOGGER_REQUEST_TIMEOUT"
    )]
    request_timeout: u32,
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

struct BlockItemSummaryWithCanonicalAddresses {
    pub(crate) summary:   BlockItemSummary,
    /// Affected addresses, resolved to canonical addresses and without
    /// duplicates.
    pub(crate) addresses: Vec<AccountAddress>,
}

/// Data sent by the node query task to the transaction insertion task.
/// Contains all the information needed to build the transaction index.
type TransactionLogData =
    (BlockInfo, Vec<BlockItemSummaryWithCanonicalAddresses>, Vec<SpecialTransactionOutcome>);

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

        Ok(Self {
            insert_summary,
            insert_ati,
            insert_cti,
            cis2_increase_total_supply,
            cis2_decrease_total_supply,
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

/// Insert block into the database. Inserts transactionally by block to
/// facilitate easy recovery if database connection is lost.
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

/// Get the last recorded block height from the database.
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

/// Handles database-related execution delegated from service infrastructure.
struct DatabaseDelegate;

#[async_trait]
impl DatabaseHooks<TransactionLogData, PreparedStatements> for DatabaseDelegate {
    async fn insert_into_db(
        db_conn: &mut DBConn,
        (bi, item_summaries, special_events): &TransactionLogData,
    ) -> Result<BlockInsertSuccess, DatabaseError> {
        let time = insert_block(
            db_conn,
            bi.block_hash,
            (bi.block_slot_time.timestamp_millis() as u64).into(),
            bi.block_height,
            item_summaries,
            special_events,
        )
        .await?;

        Ok(BlockInsertSuccess {
            time,
            block_height: bi.block_height,
            block_hash: bi.block_hash,
        })
    }

    async fn on_request_max_height(
        db: &DatabaseClient,
    ) -> Result<Option<AbsoluteBlockHeight>, DatabaseError> {
        let height = get_last_block_height(db).await?;
        Ok(height)
    }
}

/// Handles node-related execution delegated from service infrastructure.
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
        node: &mut v2::Client,
        finalized_block_info: &FinalizedBlockInfo,
    ) -> Result<TransactionLogData, NodeError> {
        // Collect necessary block-specific information from the `node`
        let binfo = node.get_block_info(finalized_block_info.block_hash).await?.response;
        let transaction_summaries = if binfo.transaction_count == 0 {
            Vec::new()
        } else {
            node.get_block_transaction_events(finalized_block_info.block_hash)
                .await?
                .response
                .try_collect()
                .await?
        };
        let special_events = node
            .get_block_special_events(finalized_block_info.block_hash)
            .await?
            .response
            .try_collect()
            .await?;

        // Map account addresses affected by each summary to their respective canonical
        // address
        let mut with_addresses = Vec::with_capacity(transaction_summaries.len());
        for summary in transaction_summaries {
            let affected_addresses = summary.affected_addresses();
            let mut addresses = Vec::with_capacity(affected_addresses.len());
            // resolve canonical addresses. This part is only needed because the index
            // is currently expected by "canonical address",
            // which is only possible to resolve by querying the node.
            let mut seen = HashSet::with_capacity(affected_addresses.len());
            for address in affected_addresses.into_iter() {
                if let Some(addr) = self.canonical_cache.get(address.as_ref()) {
                    let addr = AccountAddress::from(*addr);
                    if seen.insert(addr) {
                        addresses.push(addr);
                    }
                } else {
                    let ainfo = node
                        .get_account_info(&address.into(), binfo.block_hash)
                        .await
                        .context("Error querying account info.")?
                        .response;
                    let addr = ainfo.account_address;
                    if seen.insert(addr) {
                        log::debug!("Discovered new address {}", addr);
                        addresses.push(addr);
                        self.canonical_cache.insert(AccountAddressEq(addr));
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

        Ok((binfo, with_addresses, special_events))
    }
}

#[tokio::main(flavor = "multi_thread")]
async fn main() -> anyhow::Result<()> {
    let args = {
        let args = Args::clap()
            // .setting(AppSettings::ArgRequiredElseHelp)
            .global_setting(AppSettings::ColoredHelp);
        let matches = args.get_matches();
        Args::from_clap(&matches)
    };

    let mut log_builder = env_logger::Builder::from_env("TRANSACTION_LOGGER_LOG");
    log_builder.filter_module(module_path!(), args.log_level);
    log_builder.init();

    let (shutdown_send, shutdown_receive) = tokio::sync::watch::channel("init".to_string());

    let shutdown_handler_handle = tokio::spawn(set_shutdown(shutdown_send));

    let sql_schema = include_str!("../resources/schema.sql");
    let node_hooks = NodeDelegate {
        canonical_cache: HashSet::new(),
    };

    let run_service_args = SharedIndexerArgs {
        max_connect_attemps: MAX_CONNECT_ATTEMPTS,
        max_behind:          args.max_behind,
        num_parallel:        args.num_parallel,
        connect_timeout:     args.connect_timeout,
        request_timeout:     args.request_timeout,
        db_config:           args.config,
        endpoint:            args.endpoint,
    };

    run_service::<TransactionLogData, PreparedStatements, DatabaseDelegate, NodeDelegate>(
        sql_schema,
        run_service_args,
        shutdown_receive,
        node_hooks,
    )
    .await?;

    shutdown_handler_handle.abort();

    Ok(())
}
