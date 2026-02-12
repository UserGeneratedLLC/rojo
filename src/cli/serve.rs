use std::{
    net::{IpAddr, Ipv4Addr},
    path::PathBuf,
    sync::Arc,
};

use clap::Parser;
use memofs::Vfs;

use crate::{serve_session::ServeSession, web::LiveServer};

use super::resolve_path;

const DEFAULT_BIND_ADDRESS: Ipv4Addr = Ipv4Addr::new(127, 0, 0, 1);
const DEFAULT_PORT: u16 = 34873;

/// Expose a Rojo project to the Rojo Studio plugin.
#[derive(Debug, Parser)]
pub struct ServeCommand {
    /// Path to the project to serve. Defaults to `default.project.json5`.
    #[clap(default_value = "default.project.json5")]
    pub project: PathBuf,

    /// The IP address to listen on. Defaults to `127.0.0.1`.
    #[clap(long)]
    pub address: Option<IpAddr>,

    /// The port to listen on. Defaults to the project's preference, or `34873` if
    /// it has none.
    #[clap(long)]
    pub port: Option<u16>,
}

impl ServeCommand {
    pub fn run(self) -> anyhow::Result<()> {
        let project_path = resolve_path(&self.project);

        let (vfs, critical_errors) = Vfs::new_default_with_errors();

        let session = Arc::new(ServeSession::new(vfs, project_path, Some(critical_errors))?);

        let ip = self
            .address
            .or_else(|| session.serve_address())
            .unwrap_or(DEFAULT_BIND_ADDRESS.into());

        let port = self
            .port
            .or_else(|| session.project_port())
            .unwrap_or(DEFAULT_PORT);

        let server = LiveServer::new(session);

        let host = if ip.is_loopback() {
            "localhost".to_owned()
        } else {
            ip.to_string()
        };
        log::info!("Listening: http://{}:{}", host, port);
        server.start((ip, port).into());

        Ok(())
    }
}
