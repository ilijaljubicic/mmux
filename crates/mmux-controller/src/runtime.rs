use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;

use mmux_node::ProfileRegistry;

use crate::{run_mcp_http_server, ControllerPolicy, ResolvedNodeWirePolicy};

pub(crate) trait ControllerRuntime {
    fn run(self) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send>>;
}

pub(crate) struct LocalRuntimeConfig {
    pub(crate) bind: SocketAddr,
    pub(crate) profiles: ProfileRegistry,
    pub(crate) policy: ControllerPolicy,
    pub(crate) mcp_token: Option<String>,
    pub(crate) wire_auth: ResolvedNodeWirePolicy,
    pub(crate) enable_local_node: bool,
}

pub(crate) struct LocalRuntime {
    config: LocalRuntimeConfig,
}

impl LocalRuntime {
    pub(crate) fn new(config: LocalRuntimeConfig) -> Self {
        Self { config }
    }
}

impl ControllerRuntime for LocalRuntime {
    fn run(self) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send>> {
        Box::pin(async move {
            let config = self.config;
            run_mcp_http_server(
                config.bind,
                config.profiles,
                config.policy,
                config.mcp_token,
                config.wire_auth,
                config.enable_local_node,
            )
            .await
        })
    }
}
