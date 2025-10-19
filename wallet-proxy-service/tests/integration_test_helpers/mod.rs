pub mod fixtures;
pub mod node_mock;
pub mod rest_client;
pub mod run_server;

#[derive(Debug)]
pub struct ServerProperties {
    rest_url: String,
    monitoring_url: String,
    node_url: String,
}

// todo ar integrate into one helper call and one server type

