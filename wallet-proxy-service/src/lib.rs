/// Configuration of the Wallet Proxy service
pub mod configuration;
pub mod logging;
/// Monitoring API router (health, metrics, etc.)
mod monitoring;
/// The Wallet Proxy service, including all endpoints
pub mod service;
/// REST API router
pub mod rest;
