use clap::Parser;

use tracing_subscriber::{layer::SubscriberExt as _, util::SubscriberInitExt as _};
use wallet_proxy::configuration::Cli;
use wallet_proxy::{logging, service};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    logging::init(cli.log_level)?;
    service::run_service(cli).await
}
