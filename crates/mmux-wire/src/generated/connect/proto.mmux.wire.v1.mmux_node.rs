///Shorthand for `OwnedView<RegisterNodeRequestView<'static>>`.
pub type OwnedRegisterNodeRequestView = ::buffa::view::OwnedView<
    crate::proto::mmux::wire::v1::__buffa::view::RegisterNodeRequestView<'static>,
>;
///Shorthand for `OwnedView<RegisterNodeResponseView<'static>>`.
pub type OwnedRegisterNodeResponseView = ::buffa::view::OwnedView<
    crate::proto::mmux::wire::v1::__buffa::view::RegisterNodeResponseView<'static>,
>;
///Shorthand for `OwnedView<PullCommandsRequestView<'static>>`.
pub type OwnedPullCommandsRequestView = ::buffa::view::OwnedView<
    crate::proto::mmux::wire::v1::__buffa::view::PullCommandsRequestView<'static>,
>;
///Shorthand for `OwnedView<PullCommandsResponseView<'static>>`.
pub type OwnedPullCommandsResponseView = ::buffa::view::OwnedView<
    crate::proto::mmux::wire::v1::__buffa::view::PullCommandsResponseView<'static>,
>;
///Shorthand for `OwnedView<SubmitCommandResultRequestView<'static>>`.
pub type OwnedSubmitCommandResultRequestView = ::buffa::view::OwnedView<
    crate::proto::mmux::wire::v1::__buffa::view::SubmitCommandResultRequestView<'static>,
>;
///Shorthand for `OwnedView<SubmitCommandResultResponseView<'static>>`.
pub type OwnedSubmitCommandResultResponseView = ::buffa::view::OwnedView<
    crate::proto::mmux::wire::v1::__buffa::view::SubmitCommandResultResponseView<'static>,
>;
///Shorthand for `OwnedView<HeartbeatRequestView<'static>>`.
pub type OwnedHeartbeatRequestView = ::buffa::view::OwnedView<
    crate::proto::mmux::wire::v1::__buffa::view::HeartbeatRequestView<'static>,
>;
///Shorthand for `OwnedView<HeartbeatResponseView<'static>>`.
pub type OwnedHeartbeatResponseView = ::buffa::view::OwnedView<
    crate::proto::mmux::wire::v1::__buffa::view::HeartbeatResponseView<'static>,
>;
impl ::connectrpc::Encodable<crate::proto::mmux::wire::v1::RegisterNodeResponse>
for crate::proto::mmux::wire::v1::__buffa::view::RegisterNodeResponseView<'_> {
    fn encode(
        &self,
        codec: ::connectrpc::CodecFormat,
    ) -> ::std::result::Result<::buffa::bytes::Bytes, ::connectrpc::ConnectError> {
        ::connectrpc::__codegen::encode_view_body(self, codec)
    }
}
impl ::connectrpc::Encodable<crate::proto::mmux::wire::v1::RegisterNodeResponse>
for ::buffa::view::OwnedView<
    crate::proto::mmux::wire::v1::__buffa::view::RegisterNodeResponseView<'static>,
> {
    fn encode(
        &self,
        codec: ::connectrpc::CodecFormat,
    ) -> ::std::result::Result<::buffa::bytes::Bytes, ::connectrpc::ConnectError> {
        ::connectrpc::__codegen::encode_view_body(&**self, codec)
    }
}
impl ::connectrpc::Encodable<crate::proto::mmux::wire::v1::PullCommandsResponse>
for crate::proto::mmux::wire::v1::__buffa::view::PullCommandsResponseView<'_> {
    fn encode(
        &self,
        codec: ::connectrpc::CodecFormat,
    ) -> ::std::result::Result<::buffa::bytes::Bytes, ::connectrpc::ConnectError> {
        ::connectrpc::__codegen::encode_view_body(self, codec)
    }
}
impl ::connectrpc::Encodable<crate::proto::mmux::wire::v1::PullCommandsResponse>
for ::buffa::view::OwnedView<
    crate::proto::mmux::wire::v1::__buffa::view::PullCommandsResponseView<'static>,
> {
    fn encode(
        &self,
        codec: ::connectrpc::CodecFormat,
    ) -> ::std::result::Result<::buffa::bytes::Bytes, ::connectrpc::ConnectError> {
        ::connectrpc::__codegen::encode_view_body(&**self, codec)
    }
}
impl ::connectrpc::Encodable<crate::proto::mmux::wire::v1::SubmitCommandResultResponse>
for crate::proto::mmux::wire::v1::__buffa::view::SubmitCommandResultResponseView<'_> {
    fn encode(
        &self,
        codec: ::connectrpc::CodecFormat,
    ) -> ::std::result::Result<::buffa::bytes::Bytes, ::connectrpc::ConnectError> {
        ::connectrpc::__codegen::encode_view_body(self, codec)
    }
}
impl ::connectrpc::Encodable<crate::proto::mmux::wire::v1::SubmitCommandResultResponse>
for ::buffa::view::OwnedView<
    crate::proto::mmux::wire::v1::__buffa::view::SubmitCommandResultResponseView<'static>,
> {
    fn encode(
        &self,
        codec: ::connectrpc::CodecFormat,
    ) -> ::std::result::Result<::buffa::bytes::Bytes, ::connectrpc::ConnectError> {
        ::connectrpc::__codegen::encode_view_body(&**self, codec)
    }
}
impl ::connectrpc::Encodable<crate::proto::mmux::wire::v1::HeartbeatResponse>
for crate::proto::mmux::wire::v1::__buffa::view::HeartbeatResponseView<'_> {
    fn encode(
        &self,
        codec: ::connectrpc::CodecFormat,
    ) -> ::std::result::Result<::buffa::bytes::Bytes, ::connectrpc::ConnectError> {
        ::connectrpc::__codegen::encode_view_body(self, codec)
    }
}
impl ::connectrpc::Encodable<crate::proto::mmux::wire::v1::HeartbeatResponse>
for ::buffa::view::OwnedView<
    crate::proto::mmux::wire::v1::__buffa::view::HeartbeatResponseView<'static>,
> {
    fn encode(
        &self,
        codec: ::connectrpc::CodecFormat,
    ) -> ::std::result::Result<::buffa::bytes::Bytes, ::connectrpc::ConnectError> {
        ::connectrpc::__codegen::encode_view_body(&**self, codec)
    }
}
/// Full service name for this service.
pub const MMUX_NODE_REGISTRY_SERVICE_SERVICE_NAME: &str = "mmux.wire.v1.MmuxNodeRegistryService";
/// Server trait for MmuxNodeRegistryService.
///
/// # Implementing handlers
///
/// Handlers receive requests as `OwnedFooView` (an alias for
/// `OwnedView<FooView<'static>>`), which gives zero-copy borrowed access
/// to fields (e.g. `request.name` is a `&str` into the decoded buffer).
/// The view can be held across `.await` points.
///
/// Implement methods with plain `async fn`; the returned future satisfies
/// the `Send` bound automatically. See the
/// [buffa user guide](https://github.com/anthropics/buffa/blob/main/docs/guide.md#ownedview-in-async-trait-implementations)
/// for zero-copy access patterns and when `to_owned_message()` is needed.
///
/// The `impl Encodable<Out>` return bound accepts the owned `Out`, the
/// generated `OutView<'_>` / `OwnedOutView`, or
/// [`MaybeBorrowed`](::connectrpc::MaybeBorrowed). View bodies are not
/// emitted for output types mapped via `extern_path` (the impl would be
/// an orphan); return owned for WKT/extern outputs.
#[allow(clippy::type_complexity)]
pub trait MmuxNodeRegistryService: Send + Sync + 'static {
    /// Handle the RegisterNode RPC.
    ///
    /// `'a` lets the response body borrow from `&self` (e.g. server-resident state).
    fn register_node<'a>(
        &'a self,
        ctx: ::connectrpc::RequestContext,
        request: OwnedRegisterNodeRequestView,
    ) -> impl ::std::future::Future<
        Output = ::connectrpc::ServiceResult<
            impl ::connectrpc::Encodable<
                crate::proto::mmux::wire::v1::RegisterNodeResponse,
            > + Send + use<'a, Self>,
        >,
    > + Send;
    /// Handle the PullCommands RPC.
    ///
    /// `'a` lets the response body borrow from `&self` (e.g. server-resident state).
    fn pull_commands<'a>(
        &'a self,
        ctx: ::connectrpc::RequestContext,
        request: OwnedPullCommandsRequestView,
    ) -> impl ::std::future::Future<
        Output = ::connectrpc::ServiceResult<
            impl ::connectrpc::Encodable<
                crate::proto::mmux::wire::v1::PullCommandsResponse,
            > + Send + use<'a, Self>,
        >,
    > + Send;
    /// Handle the SubmitCommandResult RPC.
    ///
    /// `'a` lets the response body borrow from `&self` (e.g. server-resident state).
    fn submit_command_result<'a>(
        &'a self,
        ctx: ::connectrpc::RequestContext,
        request: OwnedSubmitCommandResultRequestView,
    ) -> impl ::std::future::Future<
        Output = ::connectrpc::ServiceResult<
            impl ::connectrpc::Encodable<
                crate::proto::mmux::wire::v1::SubmitCommandResultResponse,
            > + Send + use<'a, Self>,
        >,
    > + Send;
    /// Handle the Heartbeat RPC.
    ///
    /// `'a` lets the response body borrow from `&self` (e.g. server-resident state).
    fn heartbeat<'a>(
        &'a self,
        ctx: ::connectrpc::RequestContext,
        request: OwnedHeartbeatRequestView,
    ) -> impl ::std::future::Future<
        Output = ::connectrpc::ServiceResult<
            impl ::connectrpc::Encodable<
                crate::proto::mmux::wire::v1::HeartbeatResponse,
            > + Send + use<'a, Self>,
        >,
    > + Send;
}
/// Extension trait for registering a service implementation with a Router.
///
/// This trait is automatically implemented for all types that implement the service trait.
///
/// # Example
///
/// ```rust,ignore
/// use std::sync::Arc;
///
/// let service = Arc::new(MyServiceImpl);
/// let router = service.register(Router::new());
/// ```
pub trait MmuxNodeRegistryServiceExt: MmuxNodeRegistryService {
    /// Register this service implementation with a Router.
    ///
    /// Takes ownership of the `Arc<Self>` and returns a new Router with
    /// this service's methods registered.
    fn register(
        self: ::std::sync::Arc<Self>,
        router: ::connectrpc::Router,
    ) -> ::connectrpc::Router;
}
impl<S: MmuxNodeRegistryService> MmuxNodeRegistryServiceExt for S {
    fn register(
        self: ::std::sync::Arc<Self>,
        router: ::connectrpc::Router,
    ) -> ::connectrpc::Router {
        router
            .route_view(
                MMUX_NODE_REGISTRY_SERVICE_SERVICE_NAME,
                "RegisterNode",
                {
                    let svc = ::std::sync::Arc::clone(&self);
                    ::connectrpc::view_handler_fn(move |ctx, req, format| {
                        let svc = ::std::sync::Arc::clone(&svc);
                        async move {
                            svc.register_node(ctx, req)
                                .await?
                                .encode::<
                                    crate::proto::mmux::wire::v1::RegisterNodeResponse,
                                >(format)
                        }
                    })
                },
            )
            .route_view(
                MMUX_NODE_REGISTRY_SERVICE_SERVICE_NAME,
                "PullCommands",
                {
                    let svc = ::std::sync::Arc::clone(&self);
                    ::connectrpc::view_handler_fn(move |ctx, req, format| {
                        let svc = ::std::sync::Arc::clone(&svc);
                        async move {
                            svc.pull_commands(ctx, req)
                                .await?
                                .encode::<
                                    crate::proto::mmux::wire::v1::PullCommandsResponse,
                                >(format)
                        }
                    })
                },
            )
            .route_view(
                MMUX_NODE_REGISTRY_SERVICE_SERVICE_NAME,
                "SubmitCommandResult",
                {
                    let svc = ::std::sync::Arc::clone(&self);
                    ::connectrpc::view_handler_fn(move |ctx, req, format| {
                        let svc = ::std::sync::Arc::clone(&svc);
                        async move {
                            svc.submit_command_result(ctx, req)
                                .await?
                                .encode::<
                                    crate::proto::mmux::wire::v1::SubmitCommandResultResponse,
                                >(format)
                        }
                    })
                },
            )
            .route_view(
                MMUX_NODE_REGISTRY_SERVICE_SERVICE_NAME,
                "Heartbeat",
                {
                    let svc = ::std::sync::Arc::clone(&self);
                    ::connectrpc::view_handler_fn(move |ctx, req, format| {
                        let svc = ::std::sync::Arc::clone(&svc);
                        async move {
                            svc.heartbeat(ctx, req)
                                .await?
                                .encode::<
                                    crate::proto::mmux::wire::v1::HeartbeatResponse,
                                >(format)
                        }
                    })
                },
            )
    }
}
/// Monomorphic dispatcher for `MmuxNodeRegistryService`.
///
/// Unlike `.register(Router)` which type-erases each method into an `Arc<dyn ErasedHandler>` stored in a `HashMap`, this struct dispatches via a compile-time `match` on method name: no vtable, no hash lookup.
///
/// # Example
///
/// ```rust,ignore
/// use connectrpc::ConnectRpcService;
///
/// let server = MmuxNodeRegistryServiceServer::new(MyImpl);
/// let service = ConnectRpcService::new(server);
/// // hand `service` to axum/hyper as a fallback_service
/// ```
pub struct MmuxNodeRegistryServiceServer<T> {
    inner: ::std::sync::Arc<T>,
}
impl<T: MmuxNodeRegistryService> MmuxNodeRegistryServiceServer<T> {
    /// Wrap a service implementation in a monomorphic dispatcher.
    pub fn new(service: T) -> Self {
        Self {
            inner: ::std::sync::Arc::new(service),
        }
    }
    /// Wrap an already-`Arc`'d service implementation.
    pub fn from_arc(inner: ::std::sync::Arc<T>) -> Self {
        Self { inner }
    }
}
impl<T> Clone for MmuxNodeRegistryServiceServer<T> {
    fn clone(&self) -> Self {
        Self {
            inner: ::std::sync::Arc::clone(&self.inner),
        }
    }
}
impl<T: MmuxNodeRegistryService> ::connectrpc::Dispatcher
for MmuxNodeRegistryServiceServer<T> {
    #[inline]
    fn lookup(
        &self,
        path: &str,
    ) -> Option<::connectrpc::dispatcher::codegen::MethodDescriptor> {
        let method = path.strip_prefix("mmux.wire.v1.MmuxNodeRegistryService/")?;
        match method {
            "RegisterNode" => {
                Some(::connectrpc::dispatcher::codegen::MethodDescriptor::unary(false))
            }
            "PullCommands" => {
                Some(::connectrpc::dispatcher::codegen::MethodDescriptor::unary(false))
            }
            "SubmitCommandResult" => {
                Some(::connectrpc::dispatcher::codegen::MethodDescriptor::unary(false))
            }
            "Heartbeat" => {
                Some(::connectrpc::dispatcher::codegen::MethodDescriptor::unary(false))
            }
            _ => None,
        }
    }
    fn call_unary(
        &self,
        path: &str,
        ctx: ::connectrpc::RequestContext,
        request: ::buffa::bytes::Bytes,
        format: ::connectrpc::CodecFormat,
    ) -> ::connectrpc::dispatcher::codegen::UnaryResult {
        let Some(method) = path.strip_prefix("mmux.wire.v1.MmuxNodeRegistryService/")
        else {
            return ::connectrpc::dispatcher::codegen::unimplemented_unary(path);
        };
        let _ = (&ctx, &request, &format);
        match method {
            "RegisterNode" => {
                let svc = ::std::sync::Arc::clone(&self.inner);
                Box::pin(async move {
                    let req = ::connectrpc::dispatcher::codegen::decode_request_view::<
                        crate::proto::mmux::wire::v1::__buffa::view::RegisterNodeRequestView,
                    >(request, format)?;
                    svc.register_node(ctx, req)
                        .await?
                        .encode::<
                            crate::proto::mmux::wire::v1::RegisterNodeResponse,
                        >(format)
                })
            }
            "PullCommands" => {
                let svc = ::std::sync::Arc::clone(&self.inner);
                Box::pin(async move {
                    let req = ::connectrpc::dispatcher::codegen::decode_request_view::<
                        crate::proto::mmux::wire::v1::__buffa::view::PullCommandsRequestView,
                    >(request, format)?;
                    svc.pull_commands(ctx, req)
                        .await?
                        .encode::<
                            crate::proto::mmux::wire::v1::PullCommandsResponse,
                        >(format)
                })
            }
            "SubmitCommandResult" => {
                let svc = ::std::sync::Arc::clone(&self.inner);
                Box::pin(async move {
                    let req = ::connectrpc::dispatcher::codegen::decode_request_view::<
                        crate::proto::mmux::wire::v1::__buffa::view::SubmitCommandResultRequestView,
                    >(request, format)?;
                    svc.submit_command_result(ctx, req)
                        .await?
                        .encode::<
                            crate::proto::mmux::wire::v1::SubmitCommandResultResponse,
                        >(format)
                })
            }
            "Heartbeat" => {
                let svc = ::std::sync::Arc::clone(&self.inner);
                Box::pin(async move {
                    let req = ::connectrpc::dispatcher::codegen::decode_request_view::<
                        crate::proto::mmux::wire::v1::__buffa::view::HeartbeatRequestView,
                    >(request, format)?;
                    svc.heartbeat(ctx, req)
                        .await?
                        .encode::<
                            crate::proto::mmux::wire::v1::HeartbeatResponse,
                        >(format)
                })
            }
            _ => ::connectrpc::dispatcher::codegen::unimplemented_unary(path),
        }
    }
    fn call_server_streaming(
        &self,
        path: &str,
        ctx: ::connectrpc::RequestContext,
        request: ::buffa::bytes::Bytes,
        format: ::connectrpc::CodecFormat,
    ) -> ::connectrpc::dispatcher::codegen::StreamingResult {
        let Some(method) = path.strip_prefix("mmux.wire.v1.MmuxNodeRegistryService/")
        else {
            return ::connectrpc::dispatcher::codegen::unimplemented_streaming(path);
        };
        let _ = (&ctx, &request, &format);
        match method {
            _ => ::connectrpc::dispatcher::codegen::unimplemented_streaming(path),
        }
    }
    fn call_client_streaming(
        &self,
        path: &str,
        ctx: ::connectrpc::RequestContext,
        requests: ::connectrpc::dispatcher::codegen::RequestStream,
        format: ::connectrpc::CodecFormat,
    ) -> ::connectrpc::dispatcher::codegen::UnaryResult {
        let Some(method) = path.strip_prefix("mmux.wire.v1.MmuxNodeRegistryService/")
        else {
            return ::connectrpc::dispatcher::codegen::unimplemented_unary(path);
        };
        let _ = (&ctx, &requests, &format);
        match method {
            _ => ::connectrpc::dispatcher::codegen::unimplemented_unary(path),
        }
    }
    fn call_bidi_streaming(
        &self,
        path: &str,
        ctx: ::connectrpc::RequestContext,
        requests: ::connectrpc::dispatcher::codegen::RequestStream,
        format: ::connectrpc::CodecFormat,
    ) -> ::connectrpc::dispatcher::codegen::StreamingResult {
        let Some(method) = path.strip_prefix("mmux.wire.v1.MmuxNodeRegistryService/")
        else {
            return ::connectrpc::dispatcher::codegen::unimplemented_streaming(path);
        };
        let _ = (&ctx, &requests, &format);
        match method {
            _ => ::connectrpc::dispatcher::codegen::unimplemented_streaming(path),
        }
    }
}
/// Client for this service.
///
/// Generic over `T: ClientTransport`. For **gRPC** (HTTP/2), use
/// `Http2Connection` — it has honest `poll_ready` and composes with
/// `tower::balance` for multi-connection load balancing. For **Connect
/// over HTTP/1.1** (or unknown protocol), use `HttpClient`.
///
/// # Example (gRPC / HTTP/2)
///
/// ```rust,ignore
/// use connectrpc::client::{Http2Connection, ClientConfig};
/// use connectrpc::Protocol;
///
/// let uri: http::Uri = "http://localhost:8080".parse()?;
/// let conn = Http2Connection::connect_plaintext(uri.clone()).await?.shared(1024);
/// let config = ClientConfig::new(uri).protocol(Protocol::Grpc);
///
/// let client = MmuxNodeRegistryServiceClient::new(conn, config);
/// let response = client.register_node(request).await?;
/// ```
///
/// # Example (Connect / HTTP/1.1 or ALPN)
///
/// ```rust,ignore
/// use connectrpc::client::{HttpClient, ClientConfig};
///
/// let http = HttpClient::plaintext();  // cleartext http:// only
/// let config = ClientConfig::new("http://localhost:8080".parse()?);
///
/// let client = MmuxNodeRegistryServiceClient::new(http, config);
/// let response = client.register_node(request).await?;
/// ```
///
/// # Working with the response
///
/// Unary calls return [`UnaryResponse<OwnedView<FooView>>`](::connectrpc::client::UnaryResponse).
/// The `OwnedView` derefs to the view, so field access is zero-copy:
///
/// ```rust,ignore
/// let resp = client.register_node(request).await?.into_view();
/// let name: &str = resp.name;  // borrow into the response buffer
/// ```
///
/// If you need the owned struct (e.g. to store or pass by value), use
/// [`into_owned()`](::connectrpc::client::UnaryResponse::into_owned):
///
/// ```rust,ignore
/// let owned = client.register_node(request).await?.into_owned();
/// ```
#[derive(Clone)]
pub struct MmuxNodeRegistryServiceClient<T> {
    transport: T,
    config: ::connectrpc::client::ClientConfig,
}
impl<T> MmuxNodeRegistryServiceClient<T>
where
    T: ::connectrpc::client::ClientTransport,
    <T::ResponseBody as ::http_body::Body>::Error: ::std::fmt::Display,
{
    /// Create a new client with the given transport and configuration.
    pub fn new(transport: T, config: ::connectrpc::client::ClientConfig) -> Self {
        Self { transport, config }
    }
    /// Get the client configuration.
    pub fn config(&self) -> &::connectrpc::client::ClientConfig {
        &self.config
    }
    /// Get a mutable reference to the client configuration.
    pub fn config_mut(&mut self) -> &mut ::connectrpc::client::ClientConfig {
        &mut self.config
    }
    /// Call the RegisterNode RPC. Sends a request to /mmux.wire.v1.MmuxNodeRegistryService/RegisterNode.
    pub async fn register_node(
        &self,
        request: crate::proto::mmux::wire::v1::RegisterNodeRequest,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::proto::mmux::wire::v1::__buffa::view::RegisterNodeResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        self.register_node_with_options(
                request,
                ::connectrpc::client::CallOptions::default(),
            )
            .await
    }
    /// Call the RegisterNode RPC with explicit per-call options. Options override [`ClientConfig`](::connectrpc::client::ClientConfig) defaults.
    pub async fn register_node_with_options(
        &self,
        request: crate::proto::mmux::wire::v1::RegisterNodeRequest,
        options: ::connectrpc::client::CallOptions,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::proto::mmux::wire::v1::__buffa::view::RegisterNodeResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        ::connectrpc::client::call_unary(
                &self.transport,
                &self.config,
                MMUX_NODE_REGISTRY_SERVICE_SERVICE_NAME,
                "RegisterNode",
                request,
                options,
            )
            .await
    }
    /// Call the PullCommands RPC. Sends a request to /mmux.wire.v1.MmuxNodeRegistryService/PullCommands.
    pub async fn pull_commands(
        &self,
        request: crate::proto::mmux::wire::v1::PullCommandsRequest,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::proto::mmux::wire::v1::__buffa::view::PullCommandsResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        self.pull_commands_with_options(
                request,
                ::connectrpc::client::CallOptions::default(),
            )
            .await
    }
    /// Call the PullCommands RPC with explicit per-call options. Options override [`ClientConfig`](::connectrpc::client::ClientConfig) defaults.
    pub async fn pull_commands_with_options(
        &self,
        request: crate::proto::mmux::wire::v1::PullCommandsRequest,
        options: ::connectrpc::client::CallOptions,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::proto::mmux::wire::v1::__buffa::view::PullCommandsResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        ::connectrpc::client::call_unary(
                &self.transport,
                &self.config,
                MMUX_NODE_REGISTRY_SERVICE_SERVICE_NAME,
                "PullCommands",
                request,
                options,
            )
            .await
    }
    /// Call the SubmitCommandResult RPC. Sends a request to /mmux.wire.v1.MmuxNodeRegistryService/SubmitCommandResult.
    pub async fn submit_command_result(
        &self,
        request: crate::proto::mmux::wire::v1::SubmitCommandResultRequest,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::proto::mmux::wire::v1::__buffa::view::SubmitCommandResultResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        self.submit_command_result_with_options(
                request,
                ::connectrpc::client::CallOptions::default(),
            )
            .await
    }
    /// Call the SubmitCommandResult RPC with explicit per-call options. Options override [`ClientConfig`](::connectrpc::client::ClientConfig) defaults.
    pub async fn submit_command_result_with_options(
        &self,
        request: crate::proto::mmux::wire::v1::SubmitCommandResultRequest,
        options: ::connectrpc::client::CallOptions,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::proto::mmux::wire::v1::__buffa::view::SubmitCommandResultResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        ::connectrpc::client::call_unary(
                &self.transport,
                &self.config,
                MMUX_NODE_REGISTRY_SERVICE_SERVICE_NAME,
                "SubmitCommandResult",
                request,
                options,
            )
            .await
    }
    /// Call the Heartbeat RPC. Sends a request to /mmux.wire.v1.MmuxNodeRegistryService/Heartbeat.
    pub async fn heartbeat(
        &self,
        request: crate::proto::mmux::wire::v1::HeartbeatRequest,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::proto::mmux::wire::v1::__buffa::view::HeartbeatResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        self.heartbeat_with_options(
                request,
                ::connectrpc::client::CallOptions::default(),
            )
            .await
    }
    /// Call the Heartbeat RPC with explicit per-call options. Options override [`ClientConfig`](::connectrpc::client::ClientConfig) defaults.
    pub async fn heartbeat_with_options(
        &self,
        request: crate::proto::mmux::wire::v1::HeartbeatRequest,
        options: ::connectrpc::client::CallOptions,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::proto::mmux::wire::v1::__buffa::view::HeartbeatResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        ::connectrpc::client::call_unary(
                &self.transport,
                &self.config,
                MMUX_NODE_REGISTRY_SERVICE_SERVICE_NAME,
                "Heartbeat",
                request,
                options,
            )
            .await
    }
}
