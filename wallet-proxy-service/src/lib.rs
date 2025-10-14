/// Configuration of the Wallet Proxy service
pub mod configuration;
/// Initialization of logging
pub mod logging;
/// Monitoring API router (health, metrics, etc.)
mod monitoring;
/// REST API router
pub mod rest;
/// The Wallet Proxy service, including all endpoints
pub mod service;
