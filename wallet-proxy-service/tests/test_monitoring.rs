//! Test HTTP monitoring endpoints

use reqwest::StatusCode;
use serde_json::value;
use std::net::TcpStream;
use std::time::Instant;
use std::{sync::Once, thread, time::Duration};
use tracing::info;
use tracing_subscriber::filter;
use wallet_proxy::configuration::Cli;
use wallet_proxy::{logging, service};

fn config() -> Cli {
    Cli {
        database_url: "db".to_string(),
        node: "http://node".parse().unwrap(),
        min_connections: 1,
        max_connections: 2,
        statement_timeout_secs: 10,
        listen: REST_HOST_PORT.parse().unwrap(),
        monitoring_listen: MONITORING_HOST_PORT.parse().unwrap(),
        log_level: filter::LevelFilter::INFO,
    }
}

struct Stubs {
    config: Cli,
}

fn init_stubs() -> Stubs {
    let config = config();

    Stubs { config }
}

const REST_HOST_PORT: &str = "0.0.0.0:18000";
const MONITORING_HOST_PORT: &str = "0.0.0.0:18003";

static START_SERVER_ONCE: Once = Once::new();

fn start_server() {
    START_SERVER_ONCE.call_once(start_server_impl);
}

fn start_server_impl() {
    logging::init(filter::LevelFilter::INFO).unwrap();

    // Create runtime that persists between tests
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .worker_threads(2)
        .build()
        .expect("tokio runtime");

    let stubs = init_stubs();

    // Start runtime and server in new thread
    thread::spawn(move || runtime.block_on(run_server(stubs)));

    // Wait for server to start
    let start = Instant::now();
    while TcpStream::connect(MONITORING_HOST_PORT).is_err() {
        if start.elapsed() > Duration::from_secs(60) {
            panic!("server did not start");
        }

        thread::sleep(Duration::from_millis(500));
    }
}

async fn run_server(stubs: Stubs) {
    info!("starting server for test");
    service::run_service(stubs.config)
        .await
        .expect("running server")
}

async fn create_client() -> reqwest::Client {
    reqwest::Client::new()
}

/// Test scraping metrics
#[tokio::test]
async fn test_prometheus_metrics_scrape() {
    start_server();

    let client = reqwest::Client::new();
    let status = client
        .get(format!("http://{}/metrics", MONITORING_HOST_PORT))
        .send()
        .await
        .unwrap()
        .status();

    assert_eq!(status, StatusCode::OK);
}

/// Test healthcheck endpoint
#[tokio::test]
async fn test_healthcheck() {
    start_server();

    let client = reqwest::Client::new();
    let resp = client
        .get(format!("http://{}/health", MONITORING_HOST_PORT))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let _body: value::Value = resp.json().await.unwrap();
}
