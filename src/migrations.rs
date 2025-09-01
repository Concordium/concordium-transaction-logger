use crate::postgres::DatabaseClient;
use anyhow::Context;
use concordium_rust_sdk::v2;
use std::cmp::Ordering;
use tokio_postgres::Transaction;

mod m0002_acoount_public_key_binding;

/// Ensure the current database schema version is compatible with the supported
/// schema version.
pub async fn ensure_compatible_schema_version(
    database_client: &DatabaseClient,
    supported: SchemaVersion,
) -> anyhow::Result<()> {
    if !has_migration_table(database_client).await? {
        anyhow::bail!(
            "Failed to find database schema version.

The `transaction-logger` binary should have initialized the database schema at start."
        )
    }
    let current = current_schema_version(database_client).await?;
    match current.cmp(&supported) {
        // If the database schema version is exactly the supported one, we are done.
        Ordering::Equal => (),
        _ => anyhow::bail!(
            "Database is using an older schema version not supported by this version of \
             `transaction-logger`.

The `transaction-logger` binary should have migrated the database schema at start to the latest \
             version."
        ),
    }
    Ok(())
}

/// Migrate the database schema to the latest version.
pub async fn run_migrations(
    db_connection: &mut DatabaseClient,
    endpoints: &[v2::Endpoint],
) -> anyhow::Result<()> {
    ensure_migrations_table(db_connection).await?;
    let mut current = current_schema_version(db_connection).await?;
    log::info!("Current database schema version {}", current.as_i64());
    log::info!(
        "Latest database schema version {}",
        SchemaVersion::LATEST.as_i64()
    );
    while current < SchemaVersion::LATEST {
        log::info!(
            "Running migration from database schema version {}",
            current.as_i64()
        );
        let new_version = current.migration_to_next(db_connection, endpoints).await?;
        log::info!(
            "Migrated database schema to version {} successfully",
            new_version.as_i64()
        );
        current = new_version
    }
    Ok(())
}

/// Check whether the current database schema version matches the latest known
/// database schema version.
pub async fn ensure_latest_schema_version(
    db_connection: &mut DatabaseClient,
) -> anyhow::Result<()> {
    if !has_migration_table(db_connection).await? {
        anyhow::bail!(
            "Failed to find the database schema version.
            The `transaction-logger` binary should have initialized the database schema at start."
        )
    }
    let current = current_schema_version(db_connection).await?;
    if current != SchemaVersion::LATEST {
        anyhow::bail!(
            "Current database schema version is not the latest
    Current: {}
     Latest: {}
 The `transaction-logger` binary should have migrated the database schema at start to the latest \
             version.",
            current.as_i64(),
            SchemaVersion::LATEST.as_i64()
        )
    }
    Ok(())
}

#[derive(Debug, derive_more::Display)]
#[display("Migration {version}:{description}")]
struct Migration {
    /// Version number for the database schema.
    version: i64,
    /// Short description of the point of the migration.
    description: String,
    /// Whether the migration does a breaking change to the database schema.
    /// This can be used for the API to ensure no breaking change has happened
    /// since the supported schema version.
    destructive: bool,
}

impl From<SchemaVersion> for Migration {
    fn from(value: SchemaVersion) -> Self {
        Migration {
            version: value.as_i64(),
            description: value.to_string(),
            destructive: value.is_destructive(),
        }
    }
}

/// Represents database schema versions.
/// Whenever migrating the database a new variant should be introduced.
#[derive(
    Debug,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Clone,
    Copy,
    derive_more::Display,
    num_derive::FromPrimitive,
)]
#[repr(i64)]
pub enum SchemaVersion {
    #[display("0000:Empty database with no tables yet.")]
    Empty,
    #[display(
        "0001:Initial schema version with tables for tracking affected accounts, contracts, and \
         cis2 tokens."
    )]
    InitialSchema,
    #[display("0002: Adding account address bindings to public keys.")]
    AccountsPublicKeyBindings,
}
impl SchemaVersion {
    /// The latest known version of the schema.
    const LATEST: SchemaVersion = SchemaVersion::AccountsPublicKeyBindings;

    /// Parse version number into a database schema version.
    /// None if the version is unknown.
    fn from_version(version: i64) -> Option<SchemaVersion> {
        num_traits::FromPrimitive::from_i64(version)
    }

    /// Convert to the integer representation used as version in the database.
    fn as_i64(self) -> i64 {
        self as i64
    }

    /// Whether introducing the database schema version is destructive, meaning
    /// not backwards compatible.
    /// Note: We use match statements here to catch missing variants at
    /// compile-time. This enforces explicit evaluation when adding a new
    /// database schema, ensuring awareness of whether the change is
    /// destructive.
    fn is_destructive(self) -> bool {
        match self {
            SchemaVersion::Empty => false,
            SchemaVersion::InitialSchema => false,
            SchemaVersion::AccountsPublicKeyBindings => false,
        }
    }

    /// Run migrations for this schema version to the next.
    async fn migration_to_next(
        &self,
        database_client: &mut DatabaseClient,
        endpoints: &[v2::Endpoint],
    ) -> anyhow::Result<SchemaVersion> {
        let start_time = chrono::Utc::now();
        let mut tx = database_client.as_mut().transaction().await?;
        let new_version = match self {
            SchemaVersion::Empty => {
                tx.batch_execute(include_str!("../resources/m0001-initial.sql"))
                    .await?;
                SchemaVersion::InitialSchema
            }
            SchemaVersion::InitialSchema => {
                m0002_acoount_public_key_binding::run(&mut tx, endpoints).await?;
                SchemaVersion::AccountsPublicKeyBindings
            }
            SchemaVersion::AccountsPublicKeyBindings => unimplemented!(
                "No migration implemented for database schema version {}",
                self.as_i64()
            ),
        };

        let end_time = chrono::Utc::now();
        insert_migration(&mut tx, &new_version.into(), start_time, end_time).await?;
        tx.commit().await?;

        Ok(new_version)
    }
}

/// Set up the migrations tables if not already there.
async fn ensure_migrations_table(database_client: &mut DatabaseClient) -> anyhow::Result<()> {
    let statement = "
        CREATE TABLE IF NOT EXISTS migrations (
            version BIGINT PRIMARY KEY,
            description TEXT NOT NULL,
            destructive BOOL NOT NULL,
            start_time TIMESTAMPTZ NOT NULL,
            end_time TIMESTAMPTZ NOT NULL)";

    database_client.as_ref().execute(statement, &[]).await?;

    Ok(())
}

/// Check the existence of the 'migration' table.
async fn has_migration_table(database_client: &DatabaseClient) -> anyhow::Result<bool> {
    let statement = "
      SELECT EXISTS (
            SELECT FROM information_schema.tables
            WHERE  table_schema = 'public'
            AND    table_name   = 'migrations'
        );";

    let row_opt = database_client.as_ref().query_opt(statement, &[]).await?;

    let has_migration_table = match row_opt {
        Some(row) => row.get::<_, bool>(0),
        None => false,
    };

    Ok(has_migration_table)
}

/// Query the migrations table for the current database schema version.
/// Results in an error if not migrations table found.
pub async fn current_schema_version(
    database_client: &DatabaseClient,
) -> anyhow::Result<SchemaVersion> {
    let statement = "SELECT MAX(version) FROM migrations";

    let row_opt = database_client.as_ref().query_opt(statement, &[]).await?;
    let version = match row_opt {
        Some(row) => row.get::<_, Option<i64>>(0).unwrap_or(0),
        None => 0,
    };

    SchemaVersion::from_version(version).context("Unknown database schema version")
}

/// Update the migrations table with a new migration.
async fn insert_migration(
    tx: &mut Transaction<'_>,
    migration: &Migration,
    start_time: chrono::DateTime<chrono::Utc>,
    end_time: chrono::DateTime<chrono::Utc>,
) -> anyhow::Result<()> {
    let statement = "
        INSERT INTO migrations (
            version, 
            description,
            destructive, 
            start_time,
            end_time
        )
        VALUES ($1, $2, $3, $4, $5)";

    let params: &[&(dyn tokio_postgres::types::ToSql + Sync)] = &[
        &migration.version,
        &migration.description,
        &migration.destructive,
        &start_time,
        &end_time,
    ];

    tx.execute(statement, params).await?;
    Ok(())
}
