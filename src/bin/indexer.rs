use core::fmt;
use std::{fs, str::FromStr};

use anyhow::Context;
use base64::{engine::general_purpose, Engine as _};
use clap::AppSettings;
use concordium_rust_sdk::smart_contracts::common::{
    schema::VersionedModuleSchema, AccountAddress, Cursor, Deserial,
};
use serde::{
    de::{self, Visitor},
    Deserialize,
};
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
#[derive(Deserialize, Debug)]
enum SupportedStandards {
    /// CIS-2 standard
    CIS2,
}

/// [`VersionedModuleSchema`] newtype
#[derive(Debug)]
struct ModuleSchema(VersionedModuleSchema);

impl FromStr for ModuleSchema {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let bytes: Vec<u8> = general_purpose::STANDARD_NO_PAD
            .decode(s)
            .context("Failed to decode string as base64")?;
        let mut cursor = Cursor::new(bytes);
        let schema = VersionedModuleSchema::deserial(&mut cursor)
            .context("Failed to deserialize base64 string as VersionedModuleSchema")?;
        Ok(ModuleSchema(schema))
    }
}

/// To make it possible to specify a module schema as base64 in [`Config`]
impl<'de> Deserialize<'de> for ModuleSchema {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>, {
        struct Base64Visitor;

        impl<'de> Visitor<'de> for Base64Visitor {
            type Value = ModuleSchema;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                write!(formatter, "A base64 string.")
            }

            fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
                v.parse::<ModuleSchema>()
                    .map_err(|_| de::Error::invalid_value(de::Unexpected::Str(v), &self))
            }
        }
        deserializer.deserialize_str(Base64Visitor)
    }
}

/// Configuration of a tracked smart contract
#[derive(Deserialize, Debug)]
struct TrackedContract {
    /// Index of the contract
    index:     u64,
    /// Subindex of the contract. Defaults to 0.
    subindex:  Option<u64>,
    /// Schema of the contract. If not specified, an attempt to get it from
    /// chain will be made.
    schema:    Option<ModuleSchema>,
    /// Contract events to parse. Defaults to all events. These should be
    /// specified as strings corresponding to the event names specified in
    /// the contract.
    events:    Option<Vec<String>>,
    /// Standards to apply to the contract.
    standards: Option<Vec<SupportedStandards>>,
}

/// Structure of the configuration expected from the configuration file.
#[derive(Deserialize, Debug)]
struct Config {
    /// Which accounts to track. Defaults to [`TrackedAddresses::Declared`]
    tracked_accounts:     Option<TrackedAddresses>,
    /// Which contracts to track. Defaults to [`TrackedAddresses::Declared`]
    tracked_contracts:    Option<TrackedAddresses>,
    /// Which transactions to track. Defaults to [`TrackedTransactions::All`]
    tracked_transactions: Option<TrackedTransactions>,
    /// When to start tracking from. If not specified, the service will try to
    /// figure out the earliest possible time to track from, which includes
    /// everything specified in the configuration.
    track_from:           Option<TrackFrom>,
    /// Accounts to track if `tracked_accounts` is set to
    /// [`TrackedAddresses::Declared`]
    accounts:             Option<Vec<TrackedAccount>>,
    /// Contracts to track if `tracked_contracts` is set to
    /// [`TrackedAddresses::Declared`]
    contracts:            Option<Vec<TrackedContract>>,
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

    println!("{:#?}", config);

    Ok(())
}
