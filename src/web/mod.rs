//! Defines the Rojo web interface. This is what the Roblox Studio plugin
//! communicates with. Eventually, we'll make this API stable, produce better
//! documentation for it, and open it up for other consumers.

mod api;
mod assets;
pub mod interface;
pub mod mcp;
mod ui;
mod util;

use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::atomic::AtomicUsize;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::Request;
use hyper_util::rt::TokioIo;
use tokio::net::TcpListener;
use tokio::runtime::Runtime;
use tokio::sync::Notify;

use crate::serve_session::ServeSession;

use self::interface::{ServerExitReason, SyncbackPayload};

/// Shared signal for the syncback endpoint to deposit its payload and notify
/// the accept loop to shut down.
pub struct SyncbackSignal {
    payload: Mutex<Option<SyncbackPayload>>,
    notify: Notify,
}

impl SyncbackSignal {
    pub fn new() -> Self {
        Self {
            payload: Mutex::new(None),
            notify: Notify::new(),
        }
    }

    pub fn fire(&self, payload: SyncbackPayload) -> bool {
        let mut guard = self.payload.lock().unwrap_or_else(|e| e.into_inner());
        if guard.is_some() {
            return false;
        }
        *guard = Some(payload);
        self.notify.notify_one();
        true
    }

    pub fn take_payload(&self) -> Option<SyncbackPayload> {
        self.payload
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
    }
}

pub struct LiveServer {
    serve_session: Arc<ServeSession>,
    syncback_signal: Arc<SyncbackSignal>,
    mcp_state: Arc<mcp::McpSyncState>,
    active_api_connections: Arc<AtomicUsize>,
}

impl LiveServer {
    pub fn new(serve_session: Arc<ServeSession>) -> Self {
        LiveServer {
            serve_session,
            syncback_signal: Arc::new(SyncbackSignal::new()),
            mcp_state: Arc::new(mcp::McpSyncState::new()),
            active_api_connections: Arc::new(AtomicUsize::new(0)),
        }
    }

    pub fn start(self, address: SocketAddr) -> ServerExitReason {
        let serve_session = Arc::clone(&self.serve_session);
        let syncback_signal = Arc::clone(&self.syncback_signal);
        let mcp_state = Arc::clone(&self.mcp_state);
        let active_api_connections = Arc::clone(&self.active_api_connections);

        let rt = Runtime::new().unwrap();
        let exit_reason = rt.block_on(async move {
            let listener = {
                const MAX_BIND_ATTEMPTS: u32 = 5;
                const BASE_BACKOFF_MS: u64 = 200;
                let mut attempts = 0u32;
                loop {
                    attempts += 1;
                    match TcpListener::bind(address).await {
                        Ok(listener) => break listener,
                        Err(err)
                            if err.kind() == std::io::ErrorKind::AddrInUse
                                && attempts < MAX_BIND_ATTEMPTS =>
                        {
                            let delay = BASE_BACKOFF_MS * 2u64.pow(attempts - 1);
                            log::warn!(
                                "Port {} in use, retrying in {}ms (attempt {}/{})",
                                address.port(),
                                delay,
                                attempts,
                                MAX_BIND_ATTEMPTS
                            );
                            tokio::time::sleep(Duration::from_millis(delay)).await;
                        }
                        Err(err) => {
                            panic!(
                                "Failed to bind to {}: {} (after {} attempts)",
                                address, err, attempts
                            );
                        }
                    }
                }
            };

            loop {
                tokio::select! {
                    result = listener.accept() => {
                        let (stream, _) = result.unwrap();
                        let io = TokioIo::new(stream);
                        let serve_session = Arc::clone(&serve_session);
                        let syncback_signal = Arc::clone(&syncback_signal);
                        let mcp_state = Arc::clone(&mcp_state);
                        let active_api_connections = Arc::clone(&active_api_connections);

                        tokio::spawn(async move {
                            let service = service_fn(move |req: Request<Incoming>| {
                                let serve_session = Arc::clone(&serve_session);
                                let syncback_signal = Arc::clone(&syncback_signal);
                                let mcp_state = Arc::clone(&mcp_state);
                                let active_api_connections = Arc::clone(&active_api_connections);

                                async move {
                                    if req.uri().path().starts_with("/mcp") {
                                        Ok::<_, Infallible>(
                                            mcp::call(req, mcp_state, active_api_connections)
                                                .await,
                                        )
                                    } else if req.uri().path().starts_with("/api") {
                                        Ok::<_, Infallible>(
                                            api::call(
                                                serve_session,
                                                req,
                                                syncback_signal,
                                                mcp_state,
                                                active_api_connections,
                                            )
                                            .await,
                                        )
                                    } else {
                                        Ok::<_, Infallible>(ui::call(serve_session, req).await)
                                    }
                                }
                            });

                            if let Err(err) = http1::Builder::new()
                                .serve_connection(io, service)
                                .with_upgrades()
                                .await
                            {
                                log::error!("Error serving connection: {err}");
                            }
                        });
                    }
                    _ = syncback_signal.notify.notified() => {
                        break;
                    }
                }
            }

            let payload = syncback_signal
                .take_payload()
                .expect("Syncback signal fired but no payload was deposited");
            ServerExitReason::SyncbackRequested(payload)
        });

        exit_reason
    }
}
