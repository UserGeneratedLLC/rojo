use std::{
    borrow::Cow,
    io::{BufWriter, Write},
    mem::forget,
    path::{self, Path, PathBuf},
    process,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::Context;
use clap::Parser;
use fs_err::File;
use memofs::Vfs;
use rayon::prelude::*;
use rbx_dom_weak::{types::Ref, Ustr};
use serde::{Deserialize, Serialize};
use tokio::runtime::Runtime;

use crate::{
    serve_session::ServeSession,
    snapshot::{AppliedPatchSet, InstanceWithMeta, RojoTree},
};

use super::resolve_path;

const ABSOLUTE_PATH_FAILED_ERR: &str = "Failed to turn relative path into absolute path!";

/// Representation of a node in the generated sourcemap tree.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SourcemapNode<'a> {
    name: &'a str,
    class_name: Ustr,

    #[serde(
        default,
        skip_serializing_if = "Vec::is_empty",
        serialize_with = "crate::path_serializer::serialize_vec_absolute"
    )]
    file_paths: Vec<Cow<'a, Path>>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    children: Vec<SourcemapNode<'a>>,
}

/// Generates a sourcemap file from the Rojo project.
#[derive(Debug, Parser)]
pub struct SourcemapCommand {
    /// Path to the project to use for the sourcemap. Defaults to the current
    /// directory.
    #[clap(default_value = "")]
    pub project: PathBuf,

    /// Where to output the sourcemap. Omit this to use stdout instead of
    /// writing to a file.
    ///
    /// Should end in .json5.
    #[clap(long, short)]
    pub output: Option<PathBuf>,

    /// If non-script files should be included or not. Defaults to false.
    #[clap(long)]
    pub include_non_scripts: bool,

    /// Whether to automatically recreate a snapshot when any input files change.
    #[clap(long)]
    pub watch: bool,

    /// Whether the sourcemap should use absolute paths instead of relative paths.
    #[clap(long)]
    pub absolute: bool,
}

impl SourcemapCommand {
    pub fn run(self) -> anyhow::Result<()> {
        let project_path = resolve_path(&self.project);

        log::trace!("Constructing in-memory filesystem");
        let vfs = Vfs::new_default();
        vfs.set_watch_enabled(self.watch);

        let session = ServeSession::new(vfs, project_path, None)?;
        let mut cursor = session.message_queue().cursor();

        let filter = if self.include_non_scripts {
            filter_nothing
        } else {
            filter_non_scripts
        };

        // Pre-build a rayon threadpool with a low number of threads to avoid
        // dynamic creation overhead on systems with a high number of cpus.
        rayon::ThreadPoolBuilder::new()
            .num_threads(num_cpus::get().min(6))
            .build_global()
            .ok();

        write_sourcemap(
            &session,
            self.output.as_deref(),
            filter,
            self.absolute,
            false,
        )?;

        if self.watch {
            let rt = Runtime::new().unwrap();

            loop {
                let receiver = session.message_queue().subscribe(cursor);
                let (new_cursor, patch_set) = rt.block_on(receiver).unwrap();
                cursor = new_cursor;

                if patch_set_affects_sourcemap(&session, &patch_set, filter) {
                    write_sourcemap(
                        &session,
                        self.output.as_deref(),
                        filter,
                        self.absolute,
                        false,
                    )?;
                }
            }
        }

        // Avoid dropping ServeSession: it's potentially VERY expensive to drop
        // and we're about to exit anyways.
        forget(session);

        Ok(())
    }
}

pub(crate) fn filter_nothing(_instance: &InstanceWithMeta) -> bool {
    true
}

fn filter_non_scripts(instance: &InstanceWithMeta) -> bool {
    matches!(
        instance.class_name().as_str(),
        "Script" | "LocalScript" | "ModuleScript"
    )
}

fn patch_set_affects_sourcemap(
    session: &ServeSession,
    patch_set: &[AppliedPatchSet],
    filter: fn(&InstanceWithMeta) -> bool,
) -> bool {
    let tree = session.tree();

    // A sourcemap has probably changed when:
    patch_set.par_iter().any(|set| {
        // 1. An instance was removed, in which case it will no
        // longer exist in the tree and we cant check the filter
        !set.removed.is_empty()
            // 2. A newly added instance passes the filter
            || set.added.iter().any(|referent| {
                let instance = tree
                    .get_instance(*referent)
                    .expect("instance did not exist when updating sourcemap");
                filter(&instance)
            })
            // 3. An existing instance has its class name, name,
            // or file paths changed, and passes the filter
            || set.updated.iter().any(|updated| {
                let changed = updated.changed_class_name.is_some()
                    || updated.changed_name.is_some()
                    || updated.changed_metadata.is_some();
                if changed {
                    let instance = tree
                        .get_instance(updated.id)
                        .expect("instance did not exist when updating sourcemap");
                    filter(&instance)
                } else {
                    false
                }
            })
    })
}

fn recurse_create_node<'a>(
    tree: &'a RojoTree,
    referent: Ref,
    project_dir: &Path,
    filter: fn(&InstanceWithMeta) -> bool,
    use_absolute_paths: bool,
) -> Option<SourcemapNode<'a>> {
    let instance = tree.get_instance(referent).expect("instance did not exist");

    let children: Vec<_> = instance
        .children()
        .par_iter()
        .filter_map(|&child_id| {
            recurse_create_node(tree, child_id, project_dir, filter, use_absolute_paths)
        })
        .collect();

    // If this object has no children and doesn't pass the filter, it doesn't
    // contain any information we're looking for.
    if children.is_empty() && !filter(&instance) {
        return None;
    }

    let file_paths = instance
        .metadata()
        .relevant_paths
        .iter()
        // Not all paths listed as relevant are guaranteed to exist.
        .filter(|path| path.is_file())
        .map(|path| path.as_path());

    let mut output_file_paths: Vec<Cow<'a, Path>> =
        Vec::with_capacity(instance.metadata().relevant_paths.len());

    // Canonicalize project_dir once to normalize Windows \\?\ prefixes
    let canonical_project_dir =
        std::fs::canonicalize(project_dir).unwrap_or_else(|_| project_dir.to_path_buf());

    for val in file_paths {
        if use_absolute_paths {
            let abs_path = path::absolute(val).expect(ABSOLUTE_PATH_FAILED_ERR);
            output_file_paths.push(Cow::Owned(abs_path));
        } else {
            let canonical_val = std::fs::canonicalize(val).unwrap_or_else(|_| val.to_path_buf());
            output_file_paths.push(Cow::Owned(
                pathdiff::diff_paths(&canonical_val, &canonical_project_dir)
                    .expect("Failed to compute relative path from project dir"),
            ));
        }
    }

    Some(SourcemapNode {
        name: instance.name(),
        class_name: instance.class_name(),
        file_paths: output_file_paths,
        children,
    })
}

pub(crate) fn write_sourcemap(
    session: &ServeSession,
    output: Option<&Path>,
    filter: fn(&InstanceWithMeta) -> bool,
    use_absolute_paths: bool,
    quiet: bool,
) -> anyhow::Result<()> {
    let tree = session.tree();

    let root_node = recurse_create_node(
        &tree,
        tree.get_root_id(),
        session.root_dir(),
        filter,
        use_absolute_paths,
    );

    if let Some(output_path) = output {
        // Use standard JSON (not JSON5) for sourcemaps - required by external tools like LSPs
        let json_output = serde_json::to_string(&root_node)?;

        // Use atomic write (temp file + rename) to prevent file watchers from
        // reading partial files. Rename is atomic on all major filesystems when
        // source and destination are on the same filesystem.
        write_atomic(output_path, json_output.as_bytes())?;

        if !quiet {
            println!("Created sourcemap at {}", output_path.display());
        }
    } else {
        // Use standard JSON (not JSON5) for sourcemaps - required by external tools like LSPs
        let output = serde_json::to_string(&root_node)?;
        println!("{}", output);
    }

    Ok(())
}

/// Generates a sourcemap directly from a WeakDom and instance-to-path map,
/// without creating a ServeSession or re-reading the filesystem.
///
/// Used by syncback to build the sourcemap from in-memory data in parallel
/// with file writes.
pub(crate) fn write_sourcemap_from_syncback(
    dom: &rbx_dom_weak::WeakDom,
    instance_paths: &std::collections::HashMap<Ref, Vec<PathBuf>>,
    project_dir: &Path,
    output: &Path,
) -> anyhow::Result<()> {
    let canonical_project_dir =
        std::fs::canonicalize(project_dir).unwrap_or_else(|_| project_dir.to_path_buf());

    let root_node =
        recurse_create_node_from_dom(dom, dom.root_ref(), instance_paths, &canonical_project_dir);

    let json_output = serde_json::to_string(&root_node)?;
    write_atomic(output, json_output.as_bytes())?;

    Ok(())
}

fn recurse_create_node_from_dom<'a>(
    dom: &'a rbx_dom_weak::WeakDom,
    referent: Ref,
    instance_paths: &std::collections::HashMap<Ref, Vec<PathBuf>>,
    project_dir: &Path,
) -> Option<SourcemapNode<'a>> {
    let instance = dom.get_by_ref(referent)?;

    let children: Vec<_> = instance
        .children()
        .iter()
        .filter_map(|&child_ref| {
            recurse_create_node_from_dom(dom, child_ref, instance_paths, project_dir)
        })
        .collect();

    if children.is_empty() && instance.class.as_str() == "DataModel" {
        return None;
    }

    let file_paths: Vec<Cow<'a, Path>> = instance_paths
        .get(&referent)
        .map(|paths| {
            paths
                .iter()
                .filter_map(|p| {
                    let canonical = std::fs::canonicalize(p).unwrap_or_else(|_| p.clone());
                    pathdiff::diff_paths(&canonical, project_dir).map(Cow::Owned)
                })
                .collect()
        })
        .unwrap_or_default();

    Some(SourcemapNode {
        name: &instance.name,
        class_name: instance.class,
        file_paths,
        children,
    })
}

/// Writes data to a file atomically by writing to a temporary file first,
/// then renaming it to the target path. This ensures file watchers never
/// see partial file contents.
fn write_atomic(target: &Path, data: &[u8]) -> anyhow::Result<()> {
    // Generate a unique temp filename in the same directory as the target.
    // Using same directory ensures rename is atomic (same filesystem).
    let parent = target.parent().unwrap_or_else(|| Path::new("."));

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);

    let temp_name = format!(
        ".{}.{}.{}.tmp",
        target
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("sourcemap"),
        process::id(),
        timestamp
    );
    let temp_path = parent.join(&temp_name);

    // Write to temp file
    let mut file = BufWriter::new(
        File::create(&temp_path)
            .with_context(|| format!("Failed to create temp file: {}", temp_path.display()))?,
    );
    file.write_all(data)
        .with_context(|| format!("Failed to write temp file: {}", temp_path.display()))?;
    file.flush()?;

    // Ensure data is synced to disk before rename (important for crash safety)
    file.into_inner()?.sync_all()?;

    // Atomic rename to target path
    std::fs::rename(&temp_path, target).with_context(|| {
        format!(
            "Failed to rename {} to {}",
            temp_path.display(),
            target.display()
        )
    })?;

    Ok(())
}

#[cfg(test)]
mod test {
    use crate::cli::sourcemap::SourcemapNode;
    use crate::cli::SourcemapCommand;
    use insta::internals::Content;
    use std::path::Path;

    #[test]
    fn maps_relative_paths() {
        let sourcemap_dir = tempfile::tempdir().unwrap();
        let sourcemap_output = sourcemap_dir.path().join("sourcemap.json");
        let project_path = fs_err::canonicalize(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("test-projects")
                .join("relative_paths")
                .join("project"),
        )
        .unwrap();
        let sourcemap_command = SourcemapCommand {
            project: project_path,
            output: Some(sourcemap_output.clone()),
            include_non_scripts: false,
            watch: false,
            absolute: false,
        };
        assert!(sourcemap_command.run().is_ok());

        let raw_sourcemap_contents = fs_err::read_to_string(sourcemap_output.as_path()).unwrap();
        let sourcemap_contents =
            serde_json::from_str::<SourcemapNode>(&raw_sourcemap_contents).unwrap();
        insta::assert_json_snapshot!(sourcemap_contents);
    }

    #[test]
    fn maps_absolute_paths() {
        let sourcemap_dir = tempfile::tempdir().unwrap();
        let sourcemap_output = sourcemap_dir.path().join("sourcemap.json");
        let project_path = fs_err::canonicalize(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("test-projects")
                .join("relative_paths")
                .join("project"),
        )
        .unwrap();
        let sourcemap_command = SourcemapCommand {
            project: project_path,
            output: Some(sourcemap_output.clone()),
            include_non_scripts: false,
            watch: false,
            absolute: true,
        };
        assert!(sourcemap_command.run().is_ok());

        let raw_sourcemap_contents = fs_err::read_to_string(sourcemap_output.as_path()).unwrap();
        let sourcemap_contents =
            serde_json::from_str::<SourcemapNode>(&raw_sourcemap_contents).unwrap();
        insta::assert_json_snapshot!(sourcemap_contents, {
            ".**.filePaths" => insta::dynamic_redaction(|mut value, _path| {
                let mut paths_count = 0;

                match value {
                    Content::Seq(ref mut vec) => {
                        for path in vec.iter().map(|i| i.as_str().unwrap()) {
                            assert!(fs_err::canonicalize(path).is_ok(), "path was not valid");
                            assert!(Path::new(path).is_absolute(), "path was not absolute");

                            paths_count += 1;
                        }
                    }
                    _ => panic!("Expected filePaths to be a sequence"),
                }
                format!("[...{} path{} omitted...]", paths_count, if paths_count != 1 { "s" } else { "" } )
            })
        });
    }
}
