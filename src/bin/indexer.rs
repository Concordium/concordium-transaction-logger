use std::fs;

use anyhow::Context;
use clap::AppSettings;
use concordium_rust_sdk::smart_contracts::common::{schema::VersionedModuleSchema, AccountAddress};
use serde::Deserialize;
use structopt::StructOpt;
use toml::value::Datetime;

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

fn main() -> anyhow::Result<()> {
    let args = {
        let args = Args::clap().global_setting(AppSettings::ColoredHelp);
        let matches = args.get_matches();
        Args::from_clap(&matches)
    };

    let config_file = fs::read_to_string(args.config_file.clone())
        .with_context(|| format!("Could not read file from path {}", args.config_file))?;
    let config: Config =
        toml::from_str(&config_file).context("Could not parse TOML configuration from ")?;

    let sql_schema = build_sql_schema(&config);

    println!("{:#?}", config);
    println!("{}", sql_schema);

    Ok(())
}
