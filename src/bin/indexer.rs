use std::fs;

use anyhow::Context;
use async_trait::async_trait;
use clap::AppSettings;
use concordium_rust_sdk::{
    postgres::{self, DatabaseClient},
    smart_contracts::common::{schema::VersionedModuleSchema, AccountAddress},
    types::AbsoluteBlockHeight,
    v2::{self, FinalizedBlockInfo},
};
use serde::Deserialize;
use structopt::StructOpt;
use toml::value::Datetime;
use transaction_logger::{
    run_service, set_shutdown, BlockInsertSuccess, DatabaseError, DatabaseHooks, NodeError,
    NodeHooks, PrepareStatements, SharedIndexerArgs,
};

/// Used to configure which transactions to track
#[derive(Deserialize, Debug)]
enum TrackedTransactions {
    /// Track all transactions
    All,
    /// Track no transactions
    None,
    /// Track the transactions included in the list
    Selected(Vec<u32>),
}

impl Default for TrackedTransactions {
    fn default() -> Self { TrackedTransactions::All }
}

/// Used to configure which accounts/contracts to track.
#[derive(Deserialize, Debug)]
enum TrackedAddresses {
    /// Track all addresses
    All,
    /// Track addresses in the configuration
    Declared,
}

impl Default for TrackedAddresses {
    fn default() -> Self { TrackedAddresses::Declared }
}

/// Used to configure at which point in the lifetime of the chain to start
/// tracking from
#[derive(Deserialize, Debug)]
enum TrackFrom {
    /// Track from date
    Date(Datetime),
    /// Track from block height
    Height(u64),
    /// Track from block hash
    BlockHash(String),
}

/// Configuration of a tracked account
#[derive(Deserialize, Debug)]
struct TrackedAccount {
    /// Address of account to track
    address:      AccountAddress,
    /// Transactions to track for account. Defaults to `tracked_transactions`
    /// for [`Config`]
    transactions: Option<TrackedTransactions>,
}

/// Supported CIS's
#[derive(Deserialize, Debug, PartialEq)]
enum SupportedStandards {
    /// CIS-2 standard
    CIS2,
}

/// [`VersionedModuleSchema`] newtype
#[derive(Debug, Deserialize)]
#[serde(try_from = "String")]
struct ModuleSchema(VersionedModuleSchema);

impl TryFrom<String> for ModuleSchema {
    type Error = anyhow::Error;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        let schema = VersionedModuleSchema::from_base64_str(&value)
            .context("Failed parsing VersionedModuleSchema from provided base64 string")?
            .into();
        Ok(schema)
    }
}

impl From<VersionedModuleSchema> for ModuleSchema {
    fn from(value: VersionedModuleSchema) -> Self { ModuleSchema(value) }
}

impl AsRef<VersionedModuleSchema> for ModuleSchema {
    fn as_ref(&self) -> &VersionedModuleSchema { &self.0 }
}

/// Default value for `subindex` field on [`TrackedContract`]
fn default_tracked_contract_subindex() -> u64 { 0 }

/// Default value for `standards` field on [`TrackedContract`]
fn default_tracked_contract_standards() -> Vec<SupportedStandards> { Vec::new() }

/// Configuration of a tracked smart contract
#[derive(Deserialize, Debug)]
struct TrackedContract {
    /// Index of the contract
    index:     u64,
    /// Subindex of the contract. Defaults to 0.
    #[serde(default = "default_tracked_contract_subindex")]
    subindex:  u64,
    /// Schema of the contract. If not specified, an attempt to get it from
    /// chain will be made.
    schema:    Option<ModuleSchema>,
    /// Contract events to parse. Defaults to all events. These should be
    /// specified as strings corresponding to the event names specified in
    /// the contract.
    events:    Option<Vec<String>>,
    /// Standards to apply to the contract. If not specified, no standards are
    /// applied.
    #[serde(default = "default_tracked_contract_standards")]
    standards: Vec<SupportedStandards>,
}

/// Default value for `accounts` field on [`Config`]
fn default_config_accounts() -> Vec<TrackedAccount> { Vec::new() }

/// Default value for `contracts` field on [`Config`]
fn default_config_contracts() -> Vec<TrackedContract> { Vec::new() }

/// Structure of the configuration expected from the configuration file.
#[derive(Deserialize, Debug)]
struct Config {
    /// Which accounts to track. Defaults to [`TrackedAddresses::Declared`]
    #[serde(default)]
    tracked_accounts:     TrackedAddresses,
    /// Which contracts to track. Defaults to [`TrackedAddresses::Declared`]
    #[serde(default)]
    tracked_contracts:    TrackedAddresses,
    /// Which transactions to track. Defaults to [`TrackedTransactions::All`]
    #[serde(default)]
    tracked_transactions: TrackedTransactions,
    /// When to start tracking from. If not specified, the service will try to
    /// figure out the earliest possible time to track from, which includes
    /// everything specified in the configuration.
    track_from:           Option<TrackFrom>,
    /// Accounts to track if `tracked_accounts` is set to
    /// [`TrackedAddresses::Declared`]
    #[serde(default = "default_config_accounts")]
    accounts:             Vec<TrackedAccount>,
    /// Contracts to track if `tracked_contracts` is set to
    /// [`TrackedAddresses::Declared`]
    #[serde(default = "default_config_contracts")]
    contracts:            Vec<TrackedContract>,
}

/// A collection on arguments necessary to run the service. These are supplied
/// via the command line.
#[derive(StructOpt)]
struct Args {
    #[structopt(
        long = "config-file",
        help = "Configuration file (TOML) for the service.",
        default_value = "./resources/indexer/default-config.toml",
        env = "INDEXER_CONFIG_FILE"
    )]
    config_file: String,
    #[structopt(
        long = "node",
        help = "GRPC interface of the node(s).",
        default_value = "http://localhost:20002",
        use_delimiter = true,
        env = "TRANSACTION_LOGGER_NODES"
    )]
    endpoint:    Vec<v2::Endpoint>,
    #[structopt(
        long = "db",
        default_value = "host=localhost dbname=indexer user=postgres password=password port=5432",
        help = "Database connection string.",
        env = "INDEXER_LOGGER_DB_STRING"
    )]
    db_config:   postgres::Config,
    #[structopt(
        long = "log-level",
        default_value = "off",
        help = "Maximum log level.",
        env = "INDEXER_LOGGER_LOG_LEVEL"
    )]
    log_level:   log::LevelFilter,
}

type DBConn = transaction_logger::DBConn<PreparedStatements>;

struct BlockData;

struct PreparedStatements;

#[async_trait]
impl PrepareStatements for PreparedStatements {
    async fn prepare_all(client: &mut DatabaseClient) -> Result<Self, postgres::Error> {
        println!("prepare_all");
        todo!()
    }
}

struct DatabaseState<'a> {
    config_hash: &'a [u8],
}

#[async_trait]
impl<'a> DatabaseHooks<BlockData, PreparedStatements> for DatabaseState<'a> {
    async fn on_request_max_height(
        &self,
        db: &DatabaseClient,
    ) -> Result<Option<AbsoluteBlockHeight>, DatabaseError> {
        let height = get_last_block_height(db).await?;
        Ok(height)
    }

    async fn insert_into_db(
        &self,
        db_conn: &mut DBConn,
        bd: &BlockData,
    ) -> Result<BlockInsertSuccess, DatabaseError> {
        println!("insert_into_db");
        todo!()
    }

    /// Checks that the configuration used matches what is in the database (if
    /// any). Panics if configuration hash mismatch is found.
    async fn on_create_client(&self, db_client: &DatabaseClient) -> Result<(), DatabaseError> {
        println!("on_create_client {:?}", self.config_hash);
        Ok(())
    }
}

struct NodeState;

#[async_trait]
impl NodeHooks<BlockData> for NodeState {
    async fn on_use_node(&mut self, client: &mut v2::Client) -> Result<(), NodeError> {
        println!("on_use_node");
        todo!()
    }

    async fn on_finalized_block(
        &mut self,
        node: &mut v2::Client,
        fb: &FinalizedBlockInfo,
    ) -> Result<BlockData, NodeError> {
        println!("on_finalized_block {}", fb.block_hash);
        todo!()
    }
}

///
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

/// Builds an sql schema conditionally depending on the `config` passed
fn build_sql_schema(config: &Config) -> String {
    let base_sql = include_str!("../../resources/indexer/sql/base.sql");

    let mut schema = String::from(base_sql);

    let needs_accounts = match config.tracked_accounts {
        TrackedAddresses::All => true,
        TrackedAddresses::Declared => !config.accounts.is_empty(),
    };
    if needs_accounts {
        let accounts_sql = include_str!("../../resources/indexer/sql/accounts.sql");
        schema.push_str(accounts_sql);
    }

    let needs_contracts = match config.tracked_contracts {
        TrackedAddresses::All => true,
        TrackedAddresses::Declared => !config.contracts.is_empty(),
    };
    if needs_contracts {
        let contracts_sql = include_str!("../../resources/indexer/sql/contracts.sql");
        schema.push_str(contracts_sql);
    }

    let needs_cis2 =
        config.contracts.iter().any(|c| c.standards.contains(&SupportedStandards::CIS2));
    if needs_cis2 {
        let cis2_sql = include_str!("../../resources/indexer/sql/cis2.sql");
        schema.push_str(cis2_sql);
    }

    schema
}

#[tokio::main(flavor = "multi_thread")]
async fn main() -> anyhow::Result<()> {
    let args = {
        let args = Args::clap().global_setting(AppSettings::ColoredHelp);
        let matches = args.get_matches();
        Args::from_clap(&matches)
    };

    let mut log_builder = env_logger::Builder::from_env("INDEXER_LOGGER_LOG");
    log_builder.filter_module(module_path!(), args.log_level);
    log_builder.init();

    let config_file = fs::read_to_string(args.config_file.clone())
        .with_context(|| format!("Could not read file from path {}", args.config_file))?;
    let config: Config =
        toml::from_str(&config_file).context("Could not parse TOML configuration from ")?;

    let sql_schema = build_sql_schema(&config);

    // TODO: get these values from config
    let app_config = SharedIndexerArgs {
        db_config:           args.db_config,
        max_behind:          240,
        num_parallel:        1,
        connect_timeout:     10,
        request_timeout:     60,
        max_connect_attemps: 8,
        endpoint:            args.endpoint,
    };

    let (shutdown_send, shutdown_receive) = tokio::sync::watch::channel(());
    let shutdown_handler_handle = tokio::spawn(set_shutdown(shutdown_send));

    let node_state = NodeState;
    let db_state = DatabaseState {
        config_hash: &[0; 10], // TODO: get proper hash of config_file
    };

    println!("{:#?}", config);
    println!("{}", sql_schema);

    run_service(sql_schema, app_config, shutdown_receive, node_state, db_state).await?;

    shutdown_handler_handle.abort();
    Ok(())
}
