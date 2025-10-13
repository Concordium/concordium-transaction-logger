pub mod node_mock;
pub mod rest;
pub mod server;
pub mod fixtures;

#[derive(Debug)]
pub struct ServerProperties {
    rest_url: String,
    monitoring_url: String,
    node_url: String,
}

// todo ar integrate into one helper call and one server type
// todo ar use sdk client trait instead of proto mock