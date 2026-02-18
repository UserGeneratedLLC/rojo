use std::{
    collections::HashMap,
    io::Cursor,
    mem::forget,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::Arc,
};

use anyhow::Context;
use clap::Parser;
use memofs::Vfs;
use rbx_dom_weak::{types::Ref, types::Variant, InstanceBuilder, WeakDom};

use crate::{
    serve_session::ServeSession,
    syncback::syncback_loop,
    web::{
        interface::{ServerExitReason, SyncbackPayload},
        LiveServer,
    },
};

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

        let (ip, port) = {
            let vfs = Vfs::new_oneshot();
            let session = ServeSession::new_oneshot(vfs, project_path.clone())?;
            let ip = self
                .address
                .or_else(|| session.serve_address())
                .unwrap_or(DEFAULT_BIND_ADDRESS.into());
            let port = self
                .port
                .or_else(|| session.project_port())
                .unwrap_or(DEFAULT_PORT);
            forget(session);
            (ip, port)
        };

        let addr: SocketAddr = (ip, port).into();
        let host = if ip.is_loopback() {
            "localhost".to_owned()
        } else {
            ip.to_string()
        };

        loop {
            let (vfs, critical_errors) = Vfs::new_default_with_errors();
            let session =
                Arc::new(ServeSession::new(vfs, project_path.clone(), Some(critical_errors))?);
            let server = LiveServer::new(session);

            log::info!("Listening: http://{}:{}", host, port);

            match server.start(addr) {
                ServerExitReason::SyncbackRequested(payload) => {
                    log::info!("Live syncback requested, running...");
                    run_live_syncback(&project_path, payload)?;
                    log::info!("Syncback complete, restarting serve...");
                    continue;
                }
            }
        }
    }
}

fn run_live_syncback(project_path: &Path, payload: SyncbackPayload) -> anyhow::Result<()> {
    let new_dom = build_dom_from_chunks(payload)?;

    let vfs = Vfs::new_oneshot();
    let session_old = ServeSession::new_oneshot(vfs, project_path.to_path_buf())?;

    let mut dom_old = session_old.tree();

    let syncback_timer = std::time::Instant::now();
    log::info!("Beginning live syncback (clean mode)...");

    let result = syncback_loop(
        session_old.vfs(),
        &mut dom_old,
        new_dom,
        session_old.root_project(),
        false, // incremental = false → clean mode
    )?;

    log::debug!(
        "Syncback finished in {:.02}s",
        syncback_timer.elapsed().as_secs_f32()
    );

    let base_path = session_old.root_project().folder_location();
    drop(dom_old);

    log::info!("Writing to the file system...");
    result
        .fs_snapshot
        .write_to_vfs_parallel(base_path, session_old.vfs())?;

    log::info!(
        "Finished live syncback: wrote {} files/folders, removed {}.",
        result.fs_snapshot.added_paths().len(),
        result.fs_snapshot.removed_paths().len()
    );

    refresh_git_index(base_path);

    forget(session_old);

    Ok(())
}

fn build_dom_from_chunks(payload: SyncbackPayload) -> anyhow::Result<WeakDom> {
    let mut dom = WeakDom::new(InstanceBuilder::new("DataModel"));
    let root_ref = dom.root_ref();

    let mut global_ref_map: HashMap<Ref, Ref> = HashMap::new();

    for chunk in &payload.services {
        let chunk_dom = rbx_binary::from_reader(Cursor::new(&chunk.data))
            .with_context(|| format!("Failed to parse rbxm for service {}", chunk.class_name))?;

        let service_ref = dom.insert(root_ref, InstanceBuilder::new(&chunk.class_name));

        for &child_ref in chunk_dom.root().children() {
            deep_clone_into(&chunk_dom, &mut dom, child_ref, service_ref, &mut global_ref_map);
        }
    }

    fixup_ref_properties(&mut dom, &global_ref_map);

    Ok(dom)
}

fn deep_clone_into(
    source: &WeakDom,
    target: &mut WeakDom,
    source_ref: Ref,
    target_parent: Ref,
    ref_map: &mut HashMap<Ref, Ref>,
) {
    let inst = source.get_by_ref(source_ref).unwrap();
    let mut builder = InstanceBuilder::new(inst.class.as_str()).with_name(inst.name.as_str());

    for (key, value) in &inst.properties {
        builder = builder.with_property(key.as_str(), value.clone());
    }

    let new_ref = target.insert(target_parent, builder);
    ref_map.insert(source_ref, new_ref);

    for &child_ref in inst.children() {
        deep_clone_into(source, target, child_ref, new_ref, ref_map);
    }
}

fn fixup_ref_properties(dom: &mut WeakDom, ref_map: &HashMap<Ref, Ref>) {
    let all_refs: Vec<Ref> = ref_map.values().copied().collect();
    for inst_ref in all_refs {
        let props_to_fix: Vec<(String, Ref)> = {
            let inst = dom.get_by_ref(inst_ref).unwrap();
            inst.properties
                .iter()
                .filter_map(|(key, value)| {
                    if let Variant::Ref(r) = value {
                        if let Some(&mapped) = ref_map.get(r) {
                            Some((key.to_string(), mapped))
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                })
                .collect()
        };

        if !props_to_fix.is_empty() {
            let inst = dom.get_by_ref_mut(inst_ref).unwrap();
            for (key, new_ref) in props_to_fix {
                inst.properties.insert(key.into(), Variant::Ref(new_ref));
            }
        }
    }
}

fn refresh_git_index(project_dir: &Path) {
    let mut check_dir = Some(project_dir);
    let mut is_git_repo = false;
    while let Some(dir) = check_dir {
        if dir.join(".git").exists() {
            is_git_repo = true;
            break;
        }
        check_dir = dir.parent();
    }

    if is_git_repo {
        log::info!("Refreshing git index...");
        match Command::new("git")
            .args(["update-index", "--refresh", "-q"])
            .current_dir(project_dir)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
        {
            Ok(_) => log::info!("Git index refreshed."),
            Err(e) => log::warn!("Failed to run git update-index --refresh: {}", e),
        }
    }
}
