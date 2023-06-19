use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use concordium_rust_sdk::{
    postgres::{self, DatabaseClient},
    types::{hashes::BlockHash, AbsoluteBlockHeight},
    v2::{self, FinalizedBlockInfo},
};
use structopt::StructOpt;
use thiserror::Error;
use tonic::{async_trait, transport::ClientTlsConfig};

const MAX_CONNECT_ATTEMPTS: u64 = 6;

/// A collection of variables supplied to [`run_service`]. These determine how the service runs
/// with regards to connections to concordium node(s), db, and logging.
#[derive(StructOpt)]
pub struct SharedIndexerArgs {
    #[structopt(
        long = "node",
        help = "GRPC interface of the node(s).",
        default_value = "http://localhost:20000",
        use_delimiter = true,
        env = "TRANSACTION_LOGGER_NODES"
    )]
    pub endpoint: Vec<v2::Endpoint>,
    #[structopt(
        long = "db",
        default_value = "host=localhost dbname=transaction-outcome user=postgres \
                         password=password port=5432",
        help = "Database connection string.",
        env = "TRANSACTION_LOGGER_DB_STRING"
    )]
    pub config: postgres::Config,
    #[structopt(
        long = "log-level",
        default_value = "off",
        help = "Maximum log level.",
        env = "TRANSACTION_LOGGER_LOG_LEVEL"
    )]
    pub log_level: log::LevelFilter,
    #[structopt(
        long = "num-parallel",
        default_value = "1",
        help = "Maximum number of parallel queries to make to the node. Usually 1 is the correct \
                number, but during initial catchup it is useful to increase this to, say 8 to \
                take advantage of parallelism in queries.",
        env = "TRANSACTION_LOGGER_NUM_PARALLEL_QUERIES"
    )]
    pub num_parallel: u32,
    #[structopt(
        long = "max-behind-seconds",
        default_value = "240",
        help = "Maximum number of seconds the node's last finalization can be behind before the \
                node is given up and another one is tried.",
        env = "TRANSACTION_LOGGER_MAX_BEHIND_SECONDS"
    )]
    pub max_behind: u32,
    #[structopt(
        long = "connect-timeout",
        default_value = "10",
        help = "Connection timeout for connecting to the node, in seconds",
        env = "TRANSACTION_LOGGER_CONNECT_TIMEOUT"
    )]
    pub connect_timeout: u32,
    #[structopt(
        long = "request-timeout",
        default_value = "60",
        help = "Request timeout for node requests, in seconds",
        env = "TRANSACTION_LOGGER_REQUEST_TIMEOUT"
    )]
    pub request_timeout: u32,
}

/// Defines necessary interface to be used with [`DBConn`].
#[async_trait]
pub trait PrepareStatements {
    /// Supplies [`DatabaseClient`] with the purpose of preparing a collection of
    /// [`tokio_postgres::Statement`]s to be used when interacting with the database later in the
    /// execution.
    async fn prepare_all(client: &mut DatabaseClient) -> Result<Self, postgres::Error>
    where
        Self: Sized;
}

/// A wrapper around a [`DatabaseClient`] that maintains prepared statements.
pub struct DBConn<P> {
    /// DatabaseClient to be used when interacting with the database.
    pub client: DatabaseClient,
    /// A collection of prepared statements for the associtated [`client`] to be used when interacting with the
    /// database
    pub prepared: P,
}

impl<P: PrepareStatements> DBConn<P> {
    /// Create a new database connection. If the second argument is `true` then
    /// the table creation script will be run (see the file
    /// `resources/schema.sql`). This script is in principle idempotent so
    /// running it twice should be safe, but it is also needless,
    /// so this should only be requested on a first connection on startup.
    pub async fn create(
        config: &postgres::Config,
        try_run_sql: Option<&str>,
    ) -> anyhow::Result<Self> {
        let mut client = DatabaseClient::create(config.clone(), postgres::NoTls).await?;

        if let Some(sql) = try_run_sql {
            client.as_ref().batch_execute(sql).await?
        }

        let prepared = P::prepare_all(&mut client).await?;

        Ok(Self {
            client,
            prepared,
        })
    }
}

impl<P> AsRef<DatabaseClient> for DBConn<P> {
    fn as_ref(&self) -> &DatabaseClient {
        &self.client
    }
}

impl<P> AsMut<DatabaseClient> for DBConn<P> {
    fn as_mut(&mut self) -> &mut DatabaseClient {
        &mut self.client
    }
}

/// Holds information pertaining to block insertion into database.
pub struct BlockInsertSuccess {
    /// The time it took to insert the block.
    pub time: chrono::Duration,
    /// The hash of the inserted block.
    pub block_hash: BlockHash,
    /// The height of the inserted block.
    pub block_height: AbsoluteBlockHeight,
}

/// A collection of possible errors that can happen while using the database.
#[derive(Debug, Error)]
pub enum DatabaseError {
    /// Database error.
    #[error("Error using the database {0}.")]
    PostgresError(#[from] postgres::Error),
    /// Other errors while processing database data.
    #[error("Error using the database {0}.")]
    OtherError(#[from] anyhow::Error),
}

/// Defines a set of necessary callbacks used by the database thread.
#[async_trait]
pub trait DatabaseHooks<D, P> {
    /// Invoked by database thread every time a block has been received to be inserted into the
    /// database.
    async fn insert_into_db(
        db_conn: &mut DBConn<P>,
        data: &D,
    ) -> Result<BlockInsertSuccess, DatabaseError>;

    /// Invoked by the database thread to request the latest recorded height in the database.
    async fn on_request_max_height(
        db: &DatabaseClient,
    ) -> Result<Option<AbsoluteBlockHeight>, DatabaseError>;
}

/// A collection of possible errors that can happen while using the node to query data.
#[derive(Debug, Error)]
pub enum NodeError {
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

/// Defines a set of necessary callbacks used while interacting with a node.
#[async_trait]
pub trait NodeHooks<D> {
    /// Invoked when a new node is being used. Should be used for one-time exectution each time a node
    /// is being cycled for use.
    async fn on_use_node(&mut self, client: &mut v2::Client) -> Result<(), NodeError>;

    /// Invoked when a finalized block is received when traversing the chain.
    async fn on_finalized_block(
        &mut self,
        client: &mut v2::Client,
        finalized_block_info: &FinalizedBlockInfo,
    ) -> Result<D, NodeError>;
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

/// Try to reconnect to the database with exponential backoff, at most
/// [`MAX_CONNECT_ATTEMPTS`] times.
async fn try_reconnect<P>(
    config: &postgres::Config,
    sql_schema: &str,
    stop_flag: &AtomicBool,
    create_tables: bool,
) -> anyhow::Result<DBConn<P>>
where
    P: PrepareStatements,
{
    let mut i = 1;
    let sql_opt: Option<&str> = if create_tables {
        Some(sql_schema)
    } else {
        None
    };

    while !stop_flag.load(Ordering::Acquire) {
        match DBConn::create(config, sql_opt).await {
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
                    "Could not connect to the database in {} attempts. Last attempt failed \
                         with reason {:#}.",
                    MAX_CONNECT_ATTEMPTS,
                    e
                );
                return Err(e);
            }
        }
    }
    anyhow::bail!("The node was requested to stop.")
}

/// Handles database related execution, using `H` for domain-specific database queries. Will attempt to reconnect to database on errors. Runs until `stop_flag` is triggered.
async fn write_to_db<D, P, H>(
    config: postgres::Config,
    sql_schema: &str,
    start_from_sender: tokio::sync::oneshot::Sender<AbsoluteBlockHeight>, // start height
    mut receiver: tokio::sync::mpsc::Receiver<D>,
    stop_flag: Arc<AtomicBool>,
) -> anyhow::Result<()>
where
    P: PrepareStatements,
    H: DatabaseHooks<D, P>,
{
    let mut db = try_reconnect(&config, sql_schema, &stop_flag, true).await?;

    let start_from = H::on_request_max_height(&db.client).await?.map_or(0.into(), |h| h.next());
    start_from_sender
        .send(start_from)
        .map_err(|_| anyhow::anyhow!("Cannot send start height value to the node worker."))?;
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
        if let Some(data) = next_item {
            match H::insert_into_db(&mut db, &data).await {
                Ok(success) => {
                    successive_errors = 0;
                    log::info!(
                        "Processed block {} at height {} in {}ms.",
                        success.block_hash,
                        success.block_height,
                        success.time.num_milliseconds()
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
                        "Database connection lost due to {:#}. Will attempt to reconnect in \
                             {}ms.",
                        e,
                        delay.as_millis()
                    );
                    tokio::time::sleep(delay).await;
                    let new_db = match try_reconnect(&config, sql_schema, &stop_flag, false).await {
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
                                    "Could not correctly stop the old database connection due \
                                         to: {}.",
                                    e
                                );
                            }
                        }
                        Err(e) => {
                            if e.is_panic() {
                                log::warn!(
                                    "Could not correctly stop the old database connection. \
                                         The connection thread panicked."
                                );
                            } else {
                                log::warn!("Could not correctly stop the old database connection.");
                            }
                        }
                    }
                    retry = Some(data);
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

/// Handles single-node connection and traversing the chain, delegating domain-specific processing
/// to `hooks` and sends data of type `D` to database thread.
/// Return Err if querying the node failed.
/// Return Ok(()) if the channel to the database was closed.
#[allow(clippy::too_many_arguments)]
async fn node_process<D, H>(
    node_ep: v2::Endpoint,
    sender: &tokio::sync::mpsc::Sender<D>,
    height: &mut AbsoluteBlockHeight, // start height
    max_parallel: u32,
    stop_flag: &AtomicBool,
    max_behind: u32, /* maximum number of seconds a node can be behind before it is deemed "behind" */
    hooks: &mut H,
) -> Result<(), NodeError>
where
    H: NodeHooks<D>,
{
    // Use TLS if the URI scheme is HTTPS.
    // This uses whatever system certificates have been installed as trusted roots.
    let node_ep = if node_ep.uri().scheme().map_or(false, |x| x == &http::uri::Scheme::HTTPS) {
        node_ep.tls_config(ClientTlsConfig::new()).map_err(NodeError::ConnectionError)?
    } else {
        node_ep
    };

    let mut node = v2::Client::new(node_ep).await.map_err(NodeError::ConnectionError)?;
    while !stop_flag.load(Ordering::Acquire) {
        hooks.on_use_node(&mut node).await?;

        let timeout = std::time::Duration::from_secs(max_behind.into());
        let mut finalized_blocks = node.get_finalized_blocks_from(*height).await?;

        let (has_error, chunks) = finalized_blocks
            .next_chunk_timeout(max_parallel as usize, timeout)
            .await
            .map_err(|_| NodeError::Timeout)?;

        for fb in chunks {
            let d = hooks.on_finalized_block(&mut node, &fb).await?;

            if sender.send(d).await.is_err() {
                log::error!("The database connection has been closed. Terminating node queries.");
                return Ok(());
            }

            *height = height.next();
        }

        if has_error {
            // we have processed the blocks we can, but further queries on the same stream
            // will fail since the stream signalled an error.
            return Err(NodeError::OtherError(anyhow::anyhow!("Finalized block stream dropped.")));
        }
    }

    Ok(())
}

/// Executes service infrastructure. Handles connections to a set of nodes and a database
/// (configured with `app_config`) on their
/// own separate threads.
/// Implementation of `NH` defines hooks used by the node thread, while implementation of `DH`
/// defines hooks used by the database thread.
pub async fn run_service<D, P, DH, NH>(
    sql_schema: &'static str,
    app_config: SharedIndexerArgs,
    mut node_hooks: NH,
    config_logger: Option<impl FnOnce(&mut env_logger::Builder, log::LevelFilter)>,
) -> Result<(), anyhow::Error>
where
    P: PrepareStatements + Send + 'static,
    D: Send + Sync + 'static,
    DH: DatabaseHooks<D, P> + Send + Sync + 'static,
    NH: NodeHooks<D>,
{
    anyhow::ensure!(!app_config.endpoint.is_empty(), "At least one node must be provided.");
    let config = app_config.config;

    let mut log_builder = env_logger::Builder::from_env("TRANSACTION_LOGGER_LOG");
    log_builder.filter_module(module_path!(), app_config.log_level);
    if let Some(config_logger) = config_logger {
        config_logger(&mut log_builder, app_config.log_level);
    }
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

    let db_write_handle = tokio::spawn(write_to_db::<D, P, DH>(
        config,
        sql_schema,
        height_sender,
        receiver,
        stop_flag.clone(),
    ));
    // The height we should start querying the node at.
    // If the sender died we simply terminate the program.
    let mut height = height_receiver.await?;

    // To make sure we do not end up in an infinite loop in case all reconnects fail
    // we count reconnects. We deem a node connection successful if it increases
    // maximum achieved height by at least 1.
    let mut max_height = height;
    let mut last_success = 0;
    let num_nodes = app_config.endpoint.len() as u64;
    for (node_ep, idx) in app_config.endpoint.into_iter().cycle().zip(0u64..) {
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

        let node_ep = node_ep
            .connect_timeout(std::time::Duration::from_secs(app_config.connect_timeout.into()))
            .timeout(std::time::Duration::from_secs(app_config.request_timeout.into()));

        let node_result = node_process(
            node_ep,
            &sender,
            &mut height,
            app_config.num_parallel,
            stop_flag.as_ref(),
            app_config.max_behind,
            &mut node_hooks,
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
