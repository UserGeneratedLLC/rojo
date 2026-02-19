//! Defines the Rojo web interface. This is what the Roblox Studio plugin
//! communicates with. Eventually, we'll make this API stable, produce better
//! documentation for it, and open it up for other consumers.

mod api;
mod assets;
pub mod interface;
mod ui;
mod util;

use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

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
}

impl LiveServer {
    pub fn new(serve_session: Arc<ServeSession>) -> Self {
        LiveServer {
            serve_session,
            syncback_signal: Arc::new(SyncbackSignal::new()),
        }
    }

    pub fn start(self, address: SocketAddr) -> ServerExitReason {
        let serve_session = Arc::clone(&self.serve_session);
        let syncback_signal = Arc::clone(&self.syncback_signal);

        let rt = Runtime::new().unwrap();
        let exit_reason = rt.block_on(async move {
            let listener = TcpListener::bind(address).await.unwrap();

            loop {
                tokio::select! {
                    result = listener.accept() => {
                        let (stream, _) = result.unwrap();
                        let io = TokioIo::new(stream);
                        let serve_session = Arc::clone(&serve_session);
                        let syncback_signal = Arc::clone(&syncback_signal);

                        tokio::spawn(async move {
                            let service = service_fn(move |req: Request<Incoming>| {
                                let serve_session = Arc::clone(&serve_session);
                                let syncback_signal = Arc::clone(&syncback_signal);

                                async move {
                                    if req.uri().path().starts_with("/api") {
                                        Ok::<_, Infallible>(
                                            api::call(serve_session, req, syncback_signal).await,
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
