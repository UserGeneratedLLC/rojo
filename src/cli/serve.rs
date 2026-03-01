use std::{
    collections::HashMap,
    io::Cursor,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::{Path, PathBuf},
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

        let (first_vfs, first_errors) = Vfs::new_default_with_errors();
        let first_session = Arc::new(ServeSession::new(
            first_vfs,
            project_path.clone(),
            Some(first_errors),
        )?);

        let project = first_session.root_project();
        let ip = self
            .address
            .or(project.serve_address)
            .unwrap_or(DEFAULT_BIND_ADDRESS.into());
        let port = self.port.or(project.serve_port).unwrap_or(DEFAULT_PORT);

        let addr: SocketAddr = (ip, port).into();
        let host = if ip.is_loopback() {
            "localhost".to_owned()
        } else {
            ip.to_string()
        };

        let mut session = first_session;
        loop {
            let server = LiveServer::new(session);

            log::info!("Listening: http://{}:{}", host, port);

            match server.start(addr) {
                ServerExitReason::SyncbackRequested(payload) => {
                    log::info!("Live syncback requested, running...");
                    match run_live_syncback(&project_path, payload) {
                        Ok(()) => log::info!("Syncback complete, restarting serve..."),
                        Err(err) => {
                            log::error!("Live syncback failed: {err:#}. Restarting serve...")
                        }
                    }
                    let (vfs, critical_errors) = Vfs::new_default_with_errors();
                    session = Arc::new(ServeSession::new(
                        vfs,
                        project_path.clone(),
                        Some(critical_errors),
                    )?);
                    continue;
                }
            }
        }
    }
}

fn run_live_syncback(project_path: &Path, payload: SyncbackPayload) -> anyhow::Result<()> {
    let new_dom = build_dom_from_chunks(payload)?;

    let vfs = Vfs::new_oneshot();
    let session_old = ServeSession::new_oneshot(vfs, project_path)?;

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
    let git_cache = crate::git::GitIndexCache::new(base_path);
    result
        .fs_snapshot
        .write_to_vfs_parallel(base_path, session_old.vfs(), git_cache.as_ref())?;

    log::info!(
        "Finished live syncback: wrote {} files/folders, removed {}.",
        result.fs_snapshot.added_paths().len(),
        result.fs_snapshot.removed_paths().len()
    );

    crate::git::refresh_git_index(base_path);

    drop(session_old);

    Ok(())
}

fn build_dom_from_chunks(payload: SyncbackPayload) -> anyhow::Result<WeakDom> {
    use crate::syncback::VISIBLE_SERVICES;

    let mut dom = WeakDom::new(InstanceBuilder::new("DataModel"));
    let root_ref = dom.root_ref();
    let mut global_ref_map: HashMap<Ref, Ref> = HashMap::new();
    let mut created_services: std::collections::HashSet<String> = std::collections::HashSet::new();

    let cloned_children: Vec<Ref> = if !payload.data.is_empty() {
        let chunk_dom = rbx_binary::from_reader(Cursor::new(&payload.data))
            .context("Failed to parse rbxm data blob")?;

        let mut cloned = Vec::new();
        for &child_ref in chunk_dom.root().children() {
            deep_clone_into(
                &chunk_dom,
                &mut dom,
                child_ref,
                root_ref,
                &mut global_ref_map,
            );
            let new_ref = *global_ref_map.get(&child_ref).unwrap();
            cloned.push(new_ref);
        }
        cloned
    } else {
        Vec::new()
    };

    let mut cursor = 0usize;
    let mut all_carriers: Vec<Ref> = Vec::new();
    let mut deferred_refs: Vec<(Ref, Vec<(String, Ref)>)> = Vec::new();

    for chunk in &payload.services {
        let mut builder = InstanceBuilder::new(&chunk.class_name);
        for (key, value) in &chunk.properties {
            if !crate::syncback::should_property_serialize(&chunk.class_name, key) {
                continue;
            }
            builder = builder.with_property(key.as_str(), value.clone());
        }
        let service_ref = dom.insert(root_ref, builder);
        created_services.insert(chunk.class_name.clone());

        let child_count = chunk.child_count as usize;
        let ref_count = chunk.ref_target_count as usize;
        let total = child_count + ref_count;
        let end = cursor + total;
        anyhow::ensure!(
            end <= cloned_children.len(),
            "Service '{}' claims {} children + {} carriers but only {} instances remain in blob",
            chunk.class_name,
            child_count,
            ref_count,
            cloned_children.len().saturating_sub(cursor)
        );
        let service_range: Vec<Ref> = cloned_children[cursor..end].to_vec();

        for &child_ref in &service_range[..child_count.min(service_range.len())] {
            dom.transfer_within(child_ref, service_ref);
        }

        let carrier_start = child_count;
        let carrier_end = service_range.len();
        let carriers = &service_range[carrier_start..carrier_end];

        let mut ref_entries: Vec<(String, Ref)> = Vec::new();
        for (prop_name, &idx) in &chunk.refs {
            if idx == 0 || (idx as usize) > carriers.len() {
                continue;
            }
            ref_entries.push((prop_name.clone(), carriers[idx as usize - 1]));
        }
        if !ref_entries.is_empty() {
            deferred_refs.push((service_ref, ref_entries));
        }

        all_carriers.extend_from_slice(carriers);
        cursor = end;
    }

    anyhow::ensure!(
        cursor == cloned_children.len(),
        "Service chunks account for {} instances but rbxm blob contains {}",
        cursor,
        cloned_children.len()
    );

    for &service_name in VISIBLE_SERVICES {
        if !created_services.contains(service_name) {
            dom.insert(root_ref, InstanceBuilder::new(service_name));
        }
    }

    fixup_ref_properties(&mut dom, &global_ref_map);

    for (service_ref, ref_entries) in deferred_refs {
        for (prop_name, carrier_ref) in ref_entries {
            if let Some(Variant::Ref(actual_target)) = dom
                .get_by_ref(carrier_ref)
                .and_then(|inst| inst.properties.get(&rbx_dom_weak::ustr("Value")).cloned())
            {
                let service = dom.get_by_ref_mut(service_ref).unwrap();
                service
                    .properties
                    .insert(prop_name.as_str().into(), Variant::Ref(actual_target));
            }
        }
    }

    for carrier_ref in all_carriers {
        dom.destroy(carrier_ref);
    }

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
