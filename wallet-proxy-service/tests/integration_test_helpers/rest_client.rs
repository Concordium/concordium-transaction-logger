use crate::integration_test_helpers::run_server::ServerHandle;
use reqwest::RequestBuilder;
use std::fmt::Display;
use std::sync::LazyLock;

static REQWEST_CLIENT: LazyLock<reqwest::Client> = LazyLock::new(|| reqwest::Client::new());

pub fn rest_client(handle: &ServerHandle) -> RestClient {
    RestClient {
        client: Default::default(),
        base_url: handle.properties().rest_url.clone(),
    }
}

pub fn monitoring_client(handle: &ServerHandle) -> RestClient {
    RestClient {
        client: Default::default(),
        base_url: handle.properties().monitoring_url.clone(),
    }
}

#[derive(Debug, Clone)]
pub struct RestClient {
    client: reqwest::Client,
    base_url: String,
}

impl RestClient {
    pub fn get(&self, path: impl Display) -> RequestBuilder {
        self.client.get(format!("{}/{}", self.base_url, path))
    }
}
