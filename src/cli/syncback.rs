use std::{
    io::{self, BufReader, Write as _},
    mem::forget,
    path::{Path, PathBuf},
    time::Instant,
};

use anyhow::{bail, Context};
use clap::Parser;
use fs_err::File;
use memofs::Vfs;
use rbx_dom_weak::{InstanceBuilder, WeakDom};
use reqwest::header::{CACHE_CONTROL, COOKIE, PRAGMA, USER_AGENT};
use tempfile::NamedTempFile;
use termcolor::{BufferWriter, Color, ColorChoice, ColorSpec, WriteColor};

use crate::{
    path_serializer::display_absolute,
    project::Project,
    serve_session::ServeSession,
    syncback::{syncback_loop, FsSnapshot},
};

use super::{resolve_path, sourcemap::write_sourcemap_from_syncback, GlobalOptions};

const UNKNOWN_INPUT_KIND_ERR: &str = "Could not detect what kind of file was inputted. \
                                       Expected input file to end in .rbxl, .rbxlx, .rbxm, or .rbxmx.";

/// Performs 'syncback' for the provided project, using the `input` file
/// given.
///
/// Syncback exists to convert Roblox files into a Rojo project automatically.
/// It uses the project.json5 file provided to traverse the Roblox file passed as
/// to serialize Instances to the file system in a format that Rojo understands.
///
/// To ease programmatic use, this command pipes all normal output to stderr.
#[derive(Debug, Parser)]
pub struct SyncbackCommand {
    /// Path to the project to sync back to.
    #[clap(default_value = "default.project.json5")]
    pub project: PathBuf,

    /// Path to the Roblox file to pull Instances from.
    #[clap(long, short = 'f', default_value = "Project.rbxl")]
    pub input: PathBuf,

    /// Download the place file from Roblox with the specified place ID,
    /// ignoring any existing input file.
    #[clap(long, short = 'd')]
    pub download: Option<u64>,

    /// If provided, a list all of the files and directories that will be
    /// added or removed is emitted into stdout.
    #[clap(long, short = 'l')]
    pub list: bool,

    /// If provided, syncback will not actually write anything to the file
    /// system. The command will otherwise run normally.
    #[clap(long)]
    pub dry_run: bool,

    /// If provided, prompts before writing to the file system.
    /// By default, syncback runs non-interactively.
    #[clap(long, short = 'i')]
    pub interactive: bool,

    /// If provided, syncback will preserve existing file structure and middleware
    /// formats when possible. Without this flag (default), syncback creates a fresh
    /// project layout that exactly matches the input file, removing any orphaned files.
    #[clap(long, short = 'n')]
    pub incremental: bool,

    /// Generate a sourcemap.json file after syncback.
    #[clap(long)]
    pub sourcemap: bool,

    /// Base directory for resolving relative paths (project, input).
    /// Defaults to the current working directory.
    #[clap(long, hide = true, default_value = ".")]
    pub working_dir: PathBuf,
}

impl SyncbackCommand {
    pub fn run(&self, global: GlobalOptions) -> anyhow::Result<()> {
        let base = resolve_path(&self.working_dir);
        let path_old = if self.project.is_absolute() {
            self.project.clone()
        } else {
            base.join(&self.project)
        };

        // Determine if we need to download the input file
        let resolved_input = if self.input.is_absolute() {
            self.input.clone()
        } else {
            base.join(&self.input)
        };
        let _temp_file: Option<NamedTempFile>;
        // Track if we should delete the input file after successful syncback
        // (when using default Project.rbxl and file exists locally)
        let delete_input_after_syncback: Option<PathBuf>;

        // Logic:
        // - If --download=PLACEID: always download that specific place
        // - If input file exists: use it
        // - If input file doesn't exist: auto-download using servePlaceIds
        let path_new = match &self.download {
            Some(place_id) => {
                // --download=PLACEID: always download this specific place
                log::info!("Downloading place {}...", place_id);
                let download_timer = Instant::now();
                let temp = download_place(*place_id)?;
                log::info!(
                    "Downloaded in {:.02}s",
                    download_timer.elapsed().as_secs_f32()
                );
                let temp_path = temp.path().to_path_buf();
                _temp_file = Some(temp);
                delete_input_after_syncback = None;
                temp_path
            }
            None if resolved_input.exists() => {
                // No --download flag, input file exists: use it
                _temp_file = None;
                // If using default input path, mark for deletion after success
                delete_input_after_syncback = if self.input.as_os_str() == "Project.rbxl" {
                    Some(resolved_input.clone())
                } else {
                    None
                };
                resolved_input
            }
            None => {
                // No --download flag, input file doesn't exist: auto-download
                let place_id = get_place_id_from_project(&path_old)?;
                log::info!(
                    "Input file '{}' not found, downloading place {}...",
                    resolved_input.display(),
                    place_id
                );
                let download_timer = Instant::now();
                let temp = download_place(place_id)?;
                log::info!(
                    "Downloaded in {:.02}s",
                    download_timer.elapsed().as_secs_f32()
                );
                let temp_path = temp.path().to_path_buf();
                _temp_file = Some(temp);
                delete_input_after_syncback = None;
                temp_path
            }
        };

        let input_kind = FileKind::from_path(&path_new).context(UNKNOWN_INPUT_KIND_ERR)?;
        let dom_start_timer = Instant::now();
        let dom_new = read_dom(&path_new, input_kind)?;
        log::debug!(
            "Finished opening file in {:0.02}s",
            dom_start_timer.elapsed().as_secs_f32()
        );

        // Use oneshot Vfs for syncback - file watching isn't needed and
        // watcher errors shouldn't terminate the process
        let vfs = Vfs::new_oneshot();

        let project_start_timer = Instant::now();
        let session_old = ServeSession::new_oneshot(vfs, path_old.clone())?;
        log::debug!(
            "Finished opening project in {:0.02}s",
            project_start_timer.elapsed().as_secs_f32()
        );

        let mut dom_old = session_old.tree();

        log::debug!("Old root: {}", dom_old.inner().root().class);
        log::debug!("New root: {}", dom_new.root().class);

        if log::log_enabled!(log::Level::Trace) {
            log::trace!("Children of old root:");
            for child in dom_old.inner().root().children() {
                let inst = dom_old.get_instance(*child).unwrap();
                log::trace!("{} (class: {})", inst.name(), inst.class_name());
            }
            log::trace!("Children of new root:");
            for child in dom_new.root().children() {
                let inst = dom_new.get_by_ref(*child).unwrap();
                log::trace!("{} (class: {})", inst.name, inst.class);
            }
        }

        let syncback_timer = Instant::now();
        if self.incremental {
            log::info!("Beginning incremental syncback...");
        } else {
            log::info!("Beginning syncback (clean mode)...");
        }
        let result = syncback_loop(
            session_old.vfs(),
            &mut dom_old,
            dom_new,
            session_old.root_project(),
            self.incremental,
        )?;
        log::debug!(
            "Syncback finished in {:.02}s!",
            syncback_timer.elapsed().as_secs_f32()
        );

        let base_path = session_old.root_project().folder_location();
        if self.list {
            list_files(&result.fs_snapshot, global.color.into(), base_path)?;
        }

        drop(dom_old);

        if !self.dry_run {
            if self.interactive {
                eprintln!(
                    "Would write {} files/folders and remove {} files/folders.",
                    result.fs_snapshot.added_paths().len(),
                    result.fs_snapshot.removed_paths().len()
                );
                eprint!("Is this okay? (Y/N): ");
                io::stderr().flush()?;
                let mut line = String::with_capacity(1);
                io::stdin().read_line(&mut line)?;
                line = line.trim().to_lowercase();
                if line != "y" {
                    eprintln!("Aborting due to user input!");
                    return Ok(());
                }
            }

            log::info!("Writing to the file system...");

            if self.sourcemap {
                let sourcemap_path = base_path.join("sourcemap.json");

                // Run file writes and sourcemap generation in parallel.
                // The sourcemap is built entirely from in-memory data (WeakDom + path map),
                // so it doesn't need to wait for files to be on disk.
                let (write_result, sourcemap_result) = std::thread::scope(|s| {
                    let write_handle = s.spawn(|| {
                        result
                            .fs_snapshot
                            .write_to_vfs_parallel(base_path, session_old.vfs())
                    });

                    let sourcemap_handle = s.spawn(|| {
                        write_sourcemap_from_syncback(
                            &result.new_tree,
                            &result.instance_paths,
                            base_path,
                            &sourcemap_path,
                        )
                    });

                    (
                        write_handle.join().unwrap(),
                        sourcemap_handle.join().unwrap(),
                    )
                });

                write_result?;

                match sourcemap_result {
                    Ok(()) => log::info!("Generated sourcemap at {}", sourcemap_path.display()),
                    Err(e) => log::warn!("Could not generate sourcemap: {}", e),
                }
            } else {
                result
                    .fs_snapshot
                    .write_to_vfs_parallel(base_path, session_old.vfs())?;
            }

            log::info!(
                "Finished syncback: wrote {} files/folders, removed {}.",
                result.fs_snapshot.added_paths().len(),
                result.fs_snapshot.removed_paths().len()
            );

            crate::git::refresh_git_index(base_path);

            // Delete input file if using default Project.rbxl location
            if let Some(input_path) = &delete_input_after_syncback {
                match std::fs::remove_file(input_path) {
                    Ok(()) => log::info!("Deleted input file: {}", input_path.display()),
                    Err(e) => log::warn!(
                        "Failed to delete input file {}: {}",
                        input_path.display(),
                        e
                    ),
                }
            }
        } else {
            log::info!(
                "Would write {} files/folders and remove {} files/folders.",
                result.fs_snapshot.added_paths().len(),
                result.fs_snapshot.removed_paths().len()
            );
            log::info!("Aborting before writing to file system due to `--dry-run`");
        }

        // It is potentially prohibitively expensive to drop a ServeSession,
        // and the program is about to exit anyway so we're just going to forget
        // about it.
        forget(session_old);

        // Temp file is automatically cleaned up when _temp_file is dropped

        Ok(())
    }
}

/// Gets the first place ID from the project's servePlaceIds field.
fn get_place_id_from_project(project_path: &Path) -> anyhow::Result<u64> {
    // Use oneshot Vfs to avoid file watching issues
    let temp_vfs = Vfs::new_oneshot();
    let project =
        Project::load_fuzzy(&temp_vfs, project_path)?.context("Could not find project file")?;
    let serve_place_ids = project.serve_place_ids.as_ref().context(
        "No servePlaceIds in project file. Add servePlaceIds to your project or use --download=PLACEID",
    )?;
    // Get the smallest ID for deterministic behavior
    serve_place_ids
        .iter()
        .min()
        .copied()
        .context("servePlaceIds is empty in project file")
}

/// Downloads a place file from Roblox's asset delivery API.
///
/// Uses rbx_cookie to get the authentication cookie from the system.
fn download_place(place_id: u64) -> anyhow::Result<NamedTempFile> {
    let cookie = rbx_cookie::get_value()
        .context("Could not find Roblox authentication cookie. Please log into Roblox Studio.")?;

    let url = format!("https://assetdelivery.roblox.com/v1/asset/?id={}", place_id);

    let client = reqwest::blocking::Client::builder()
        .gzip(true)
        .brotli(true)
        .deflate(true)
        .build()?;

    let response = client
        .get(&url)
        .header(COOKIE, format!(".ROBLOSECURITY={}", cookie))
        .header(CACHE_CONTROL, "no-cache, no-store, must-revalidate")
        .header(PRAGMA, "no-cache")
        .header("Expires", "0")
        .header(USER_AGENT, "Rojo")
        .send()?;

    let status = response.status();
    if !status.is_success() {
        bail!(
            "Failed to download place {}: HTTP {} - {}",
            place_id,
            status,
            response.text().unwrap_or_default()
        );
    }

    // Create temp file with .rbxl extension
    let mut temp_file = tempfile::Builder::new()
        .prefix("rojo-syncback-")
        .suffix(".rbxl")
        .tempfile()
        .context("Failed to create temporary file")?;

    // Write response body to temp file
    let bytes = response.bytes()?;
    io::copy(&mut bytes.as_ref(), &mut temp_file)?;
    temp_file.flush()?;

    log::debug!(
        "Downloaded {} bytes to {}",
        bytes.len(),
        temp_file.path().display()
    );

    Ok(temp_file)
}

fn read_dom(path: &Path, file_kind: FileKind) -> anyhow::Result<WeakDom> {
    let content = BufReader::new(File::open(path)?);
    match file_kind {
        FileKind::Rbxl => rbx_binary::from_reader(content).with_context(|| {
            format!(
                "Could not deserialize binary place file at {}",
                path.display()
            )
        }),
        FileKind::Rbxlx => rbx_xml::from_reader(content, xml_decode_config())
            .with_context(|| format!("Could not deserialize XML place file at {}", path.display())),
        FileKind::Rbxm => {
            let temp_tree = rbx_binary::from_reader(content).with_context(|| {
                format!(
                    "Could not deserialize binary place file at {}",
                    path.display()
                )
            })?;

            process_model_dom(temp_tree)
        }
        FileKind::Rbxmx => {
            let temp_tree =
                rbx_xml::from_reader(content, xml_decode_config()).with_context(|| {
                    format!("Could not deserialize XML model file at {}", path.display())
                })?;
            process_model_dom(temp_tree)
        }
    }
}

fn process_model_dom(dom: WeakDom) -> anyhow::Result<WeakDom> {
    let temp_children = dom.root().children();
    if temp_children.len() == 1 {
        let real_root = dom.get_by_ref(temp_children[0]).unwrap();
        let mut new_tree = WeakDom::new(InstanceBuilder::new(real_root.class));
        for (name, property) in &real_root.properties {
            new_tree
                .root_mut()
                .properties
                .insert(*name, property.to_owned());
        }

        let children = dom.clone_multiple_into_external(real_root.children(), &mut new_tree);
        for child in children {
            new_tree.transfer_within(child, new_tree.root_ref());
        }
        Ok(new_tree)
    } else {
        anyhow::bail!(
            "Rojo does not currently support models with more \
        than one Instance at the Root!"
        );
    }
}

fn xml_decode_config() -> rbx_xml::DecodeOptions<'static> {
    rbx_xml::DecodeOptions::new().property_behavior(rbx_xml::DecodePropertyBehavior::ReadUnknown)
}

/// The different kinds of input that Rojo can syncback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FileKind {
    /// An XML model file.
    Rbxmx,

    /// An XML place file.
    Rbxlx,

    /// A binary model file.
    Rbxm,

    /// A binary place file.
    Rbxl,
}

impl FileKind {
    fn from_path(output: &Path) -> Option<FileKind> {
        let extension = output.extension()?.to_str()?;

        match extension {
            "rbxlx" => Some(FileKind::Rbxlx),
            "rbxmx" => Some(FileKind::Rbxmx),
            "rbxl" => Some(FileKind::Rbxl),
            "rbxm" => Some(FileKind::Rbxm),
            _ => None,
        }
    }
}

fn list_files(snapshot: &FsSnapshot, color: ColorChoice, base_path: &Path) -> io::Result<()> {
    let no_color = ColorSpec::new();
    let mut add_color = ColorSpec::new();
    add_color.set_fg(Some(Color::Green));
    let mut remove_color = ColorSpec::new();
    remove_color.set_fg(Some(Color::Red));

    let writer = BufferWriter::stdout(color);
    let mut buffer = writer.buffer();

    let added = snapshot.added_paths();
    if !added.is_empty() {
        buffer.set_color(&add_color)?;
        for path in added {
            writeln!(
                &mut buffer,
                "Writing {}",
                display_absolute(path.strip_prefix(base_path).unwrap_or(path))
            )?;
        }
    }
    let removed = snapshot.removed_paths();
    if !removed.is_empty() {
        buffer.set_color(&remove_color)?;
        for path in removed {
            writeln!(
                &mut buffer,
                "Removing {}",
                display_absolute(path.strip_prefix(base_path).unwrap_or(path))
            )?;
        }
    }
    buffer.set_color(&no_color)?;

    writer.print(&buffer)
}
