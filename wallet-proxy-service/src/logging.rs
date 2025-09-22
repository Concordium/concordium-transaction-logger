use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, filter, fmt};

pub fn init(log_level: filter::LevelFilter) -> anyhow::Result<()> {
    let filter = if std::env::var("RUST_LOG").is_ok() {
        // If RUST_LOG env is defined we fall back to the default behavior of the env
        // filter.
        EnvFilter::builder().from_env_lossy()
    } else {
        // If RUST_LOG env is not defined, set the --log-level only for this project and
        // leave dependencies filter to info level.
        let pkg_name = env!("CARGO_PKG_NAME").replace('-', "_");
        let crate_name = env!("CARGO_CRATE_NAME");
        format!("info,{pkg_name}={0},{crate_name}={0}", log_level).parse()?
    };
    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(filter)
        .init();

    Ok(())
}
