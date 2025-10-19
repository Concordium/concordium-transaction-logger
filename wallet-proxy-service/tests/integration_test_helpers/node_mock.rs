use crate::integration_test_helpers::run_server::{ServerHandle, ServerStartup};
use mocktail::server::MockServer;
use parking_lot::Mutex;

use mocktail::mock_builder::{Then, When};
use std::sync::{Arc, OnceLock};
use tracing::info;

static NODE_MOCK: OnceLock<NodeMock> = OnceLock::new();

pub fn mock(handle: &ServerHandle) -> NodeMock {
    let node_mock = Clone::clone(NODE_MOCK.get().expect("node mock"));

    assert_eq!(
        node_mock.base_url(),
        handle.properties().node_url,
        "node url"
    );

    node_mock
}

pub async fn init_mock(_startup: &ServerStartup) -> NodeMock {
    let server = MockServer::new_grpc("node");
    server.start().await.unwrap();

    info!("started node mock: {}", server.base_url().unwrap());

    let node_mock = NodeMock {
        server: Arc::new(Mutex::new(server)),
    };

    NODE_MOCK
        .set(node_mock.clone())
        .map_err(|_| ())
        .expect("node mock already set");

    node_mock
}

#[derive(Clone)]
pub struct NodeMock {
    server: Arc<Mutex<MockServer>>,
}

impl NodeMock {
    pub fn base_url(&self) -> String {
        self.server.lock().base_url().unwrap().as_str().to_owned()
    }

    pub fn mock<F>(&self, f: F)
    where
        F: FnOnce(When, Then),
    {
        self.server.lock().mock(f)
    }
}
