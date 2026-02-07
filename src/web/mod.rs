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
use std::sync::Arc;

use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::Request;
use hyper_util::rt::TokioIo;
use tokio::net::TcpListener;
use tokio::runtime::Runtime;

use crate::serve_session::ServeSession;

pub struct LiveServer {
    serve_session: Arc<ServeSession>,
}

impl LiveServer {
    pub fn new(serve_session: Arc<ServeSession>) -> Self {
        LiveServer { serve_session }
    }

    pub fn start(self, address: SocketAddr) {
        let serve_session = Arc::clone(&self.serve_session);

        let rt = Runtime::new().unwrap();
        rt.block_on(async move {
            let listener = TcpListener::bind(address).await.unwrap();

            loop {
                let (stream, _) = listener.accept().await.unwrap();
                let io = TokioIo::new(stream);
                let serve_session = Arc::clone(&serve_session);

                tokio::spawn(async move {
                    let service = service_fn(move |req: Request<Incoming>| {
                        let serve_session = Arc::clone(&serve_session);

                        async move {
                            if req.uri().path().starts_with("/api") {
                                Ok::<_, Infallible>(api::call(serve_session, req).await)
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
        });
    }
}
