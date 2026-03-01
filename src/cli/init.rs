use std::num::NonZero;
use std::str::FromStr;
use std::sync::atomic::AtomicBool;
use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
};
use std::{
    ffi::OsStr,
    io::{self, Write},
};

use anyhow::{bail, format_err, Context};
use clap::Parser;
use fs_err as fs;
use fs_err::OpenOptions;
use memofs::{InMemoryFs, Vfs, VfsSnapshot};

use super::resolve_path;

const GIT_IGNORE_PLACEHOLDER: &str = "gitignore.txt";

static TEMPLATE_BINCODE: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/templates.bincode"));

/// Initializes a new Rojo project.
///
/// By default, this will attempt to initialize a 'git' repository in the
/// project directory. To avoid this, pass `--skip-git`.
#[derive(Debug, Parser)]
pub struct InitCommand {
    /// Path to the place to create the project. Defaults to the current directory.
    #[clap(long, default_value = ".")]
    pub path: PathBuf,

    /// The kind of project to create, 'place', 'plugin', or 'model'.
    #[clap(long, default_value = "place")]
    pub kind: InitKind,

    /// Skips the initialization of a git repository.
    #[clap(long)]
    pub skip_git: bool,

    /// Place ID to use for servePlaceIds.
    pub placeid: Option<u64>,

    /// Skip cloning cursor rules into .cursor directory.
    #[clap(long)]
    pub skip_rules: bool,

    /// Skip changing the working directory into the project after creation.
    #[clap(skip)]
    pub skip_cd: bool,
}

impl InitCommand {
    pub fn run(self) -> anyhow::Result<()> {
        let template = self.kind.template();

        let base_path = resolve_path(&self.path);

        if base_path.exists() {
            let is_empty = base_path.read_dir()?.next().is_none();
            if !is_empty {
                bail!(
                    "Directory '{}' is not empty. Please use an empty directory.",
                    base_path.display()
                );
            }
        }

        fs::create_dir_all(&base_path)?;

        let canonical = fs::canonicalize(&base_path)?;
        let project_name = canonical
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("new-project");

        let project_params = ProjectParams {
            name: project_name.to_owned(),
            place_id: self.placeid,
        };

        println!(
            "Creating new {:?} project '{}'",
            self.kind, project_params.name
        );

        let vfs = Vfs::new(template);
        vfs.set_watch_enabled(false);

        let mut queue = VecDeque::with_capacity(8);
        for entry in vfs.read_dir("")? {
            queue.push_back(entry?.path().to_path_buf())
        }

        while let Some(mut path) = queue.pop_front() {
            let metadata = vfs.metadata(&path)?;
            if metadata.is_dir() {
                fs_err::create_dir(base_path.join(&path))?;
                for entry in vfs.read_dir(&path)? {
                    queue.push_back(entry?.path().to_path_buf());
                }
            } else {
                let content = vfs.read_to_string_lf_normalized(&path)?;
                if let Some(file_stem) = path.file_name().and_then(OsStr::to_str) {
                    if file_stem == GIT_IGNORE_PLACEHOLDER {
                        if self.skip_git {
                            continue;
                        } else {
                            path.set_file_name(".gitignore");
                        }
                    }
                }
                write_if_not_exists(
                    &base_path.join(&path),
                    &project_params.render_template(&content),
                )?;
            }
        }

        let did_git_init = if !self.skip_git && should_git_init(&base_path) {
            log::debug!("Initializing Git repository...");

            let mut repo = gix::init(&base_path).context("Failed to initialize git repository")?;

            {
                let mut config = repo.config_snapshot_mut();
                let _ = config.set_raw_value(&gix::config::tree::Core::AUTO_CRLF, "false");
                let _ = config.set_raw_value(&gix::config::tree::Core::EOL, "lf");
                let _ = config.set_raw_value(&gix::config::tree::Core::SAFE_CRLF, "false");
                let _ = config.commit_auto_rollback();
            }

            true
        } else {
            !self.skip_git
        };

        if !self.skip_rules {
            log::debug!("Cloning cursor rules...");

            let cursor_dir = base_path.join(".cursor");
            let result = (|| -> anyhow::Result<()> {
                let prep = gix::prepare_clone(
                    "https://github.com/jrmelsha/cursor-rules.git",
                    &cursor_dir,
                )?;
                let mut prep = prep.with_shallow(gix::remote::fetch::Shallow::DepthAtRemote(
                    NonZero::new(1).unwrap(),
                ));
                let (mut checkout, _) =
                    prep.fetch_then_checkout(gix::progress::Discard, &AtomicBool::new(false))?;
                let (_repo, _) =
                    checkout.main_worktree(gix::progress::Discard, &AtomicBool::new(false))?;
                Ok(())
            })();

            match result {
                Ok(()) => {
                    let git_dir = cursor_dir.join(".git");
                    if git_dir.exists() {
                        let _ = fs::remove_dir_all(&git_dir);
                    }
                    println!("Cloned cursor rules successfully.");
                }
                Err(_) => {
                    log::debug!("Failed to clone cursor rules, skipping.");
                }
            }
        }

        if did_git_init {
            crate::git::git_add_all_and_commit(&base_path, "Initial commit");
        }

        println!("Created project successfully.");

        if !self.skip_cd {
            std::env::set_current_dir(&base_path)
                .with_context(|| format!("Failed to cd into {}", base_path.display()))?;
        }

        Ok(())
    }
}

/// Tells whether we should initialize a Git repository inside the given path.
///
/// Returns true if the path is not already inside a Git repository.
fn should_git_init(path: &Path) -> bool {
    gix::discover(path).is_err()
}

/// The templates we support for initializing a Rojo project.
#[derive(Debug, Clone, Copy)]
pub enum InitKind {
    /// A place that contains a baseplate.
    Place,

    /// An empty model, suitable for a library.
    Model,

    /// An empty plugin.
    Plugin,
}

impl InitKind {
    fn template(&self) -> InMemoryFs {
        let template_path = match self {
            Self::Place => "place",
            Self::Model => "model",
            Self::Plugin => "plugin",
        };

        let (snapshot, _): (VfsSnapshot, usize) =
            bincode::serde::decode_from_slice(TEMPLATE_BINCODE, bincode::config::standard())
                .expect("Rojo's templates were not properly packed into Rojo's binary");

        if let VfsSnapshot::Dir { mut children } = snapshot {
            if let Some(template) = children.remove(template_path) {
                let mut fs = InMemoryFs::new();
                fs.load_snapshot("", template)
                    .expect("loading a template in memory should never fail");
                fs
            } else {
                panic!("template for project type {:?} is missing", self)
            }
        } else {
            panic!("Rojo's templates were packed as a file instead of a directory")
        }
    }
}

impl FromStr for InitKind {
    type Err = anyhow::Error;

    fn from_str(source: &str) -> Result<Self, Self::Err> {
        match source {
            "place" => Ok(InitKind::Place),
            "model" => Ok(InitKind::Model),
            "plugin" => Ok(InitKind::Plugin),
            _ => Err(format_err!(
                "Invalid init kind '{}'. Valid kinds are: place, model, plugin",
                source
            )),
        }
    }
}

struct ProjectParams {
    name: String,
    place_id: Option<u64>,
}

impl ProjectParams {
    fn render_template(&self, template: &str) -> String {
        let place_id_str = self
            .place_id
            .map(|id| id.to_string())
            .unwrap_or_else(|| "0".to_string());

        template
            .replace("{project_name}", &self.name)
            .replace("{rojo_version}", env!("CARGO_PKG_VERSION"))
            .replace("{place_id}", &place_id_str)
    }
}

/// Write a file if it does not exist yet, otherwise, leave it alone.
fn write_if_not_exists(path: &Path, contents: &str) -> Result<(), anyhow::Error> {
    let file_res = OpenOptions::new().write(true).create_new(true).open(path);

    let mut file = match file_res {
        Ok(file) => file,
        Err(err) => {
            return match err.kind() {
                io::ErrorKind::AlreadyExists => return Ok(()),
                _ => Err(err.into()),
            }
        }
    };

    file.write_all(contents.as_bytes())?;

    Ok(())
}
