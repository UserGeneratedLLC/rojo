use std::collections::HashSet;
use std::str::FromStr;
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
static ATLAS_PROJECT_MDC: &str = include_str!("../../.cursor/rules/atlas-project.mdc");

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

        println!("Creating new {:?} project '{}'", self.kind, project_name);

        write_template_files(
            &base_path,
            self.kind,
            project_name,
            self.placeid,
            self.skip_git,
            &HashSet::new(),
        )?;

        setup_git_and_rules(&base_path, self.skip_git, self.skip_rules)?;

        println!("Created project successfully.");

        if !self.skip_cd {
            std::env::set_current_dir(&base_path)
                .with_context(|| format!("Failed to cd into {}", base_path.display()))?;
        }

        Ok(())
    }
}

/// Write template files from the baked-in template to `base_path`.
///
/// `exclude_files` is a set of template filenames to skip (e.g. `"default.project.json5"`).
pub fn write_template_files(
    base_path: &Path,
    kind: InitKind,
    project_name: &str,
    place_id: Option<u64>,
    skip_git: bool,
    exclude_files: &HashSet<&str>,
) -> anyhow::Result<()> {
    let template = kind.template();
    let project_params = ProjectParams {
        name: project_name.to_owned(),
        place_id,
    };

    let vfs = Vfs::new(template);
    vfs.set_watch_enabled(false);

    let mut queue = VecDeque::with_capacity(8);
    for entry in vfs.read_dir("")? {
        queue.push_back(entry?.path().to_path_buf())
    }

    while let Some(mut path) = queue.pop_front() {
        let metadata = vfs.metadata(&path)?;
        if metadata.is_dir() {
            fs_err::create_dir_all(base_path.join(&path))?;
            for entry in vfs.read_dir(&path)? {
                queue.push_back(entry?.path().to_path_buf());
            }
        } else {
            if let Some(file_name) = path.file_name().and_then(OsStr::to_str) {
                if exclude_files.contains(file_name) {
                    continue;
                }
            }

            let content = vfs.read_to_string_lf_normalized(&path)?;
            if let Some(file_name) = path.file_name().and_then(OsStr::to_str) {
                if file_name == GIT_IGNORE_PLACEHOLDER {
                    if skip_git {
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

    let rules_dir = base_path.join(".cursor/rules");
    fs::create_dir_all(&rules_dir)?;
    write_if_not_exists(&rules_dir.join("atlas-project.mdc"), ATLAS_PROJECT_MDC)?;

    Ok(())
}

/// Initialize git repository, make initial commit, and optionally add agent submodules.
pub fn setup_git_and_rules(
    base_path: &Path,
    skip_git: bool,
    skip_rules: bool,
) -> anyhow::Result<()> {
    let did_git_init = if !skip_git && crate::git::git_repo_root(base_path).is_none() {
        log::debug!("Initializing Git repository...");
        crate::git::git_init_repo(base_path).context("Failed to initialize git repository")?;
        true
    } else {
        !skip_git
    };

    if did_git_init {
        crate::git::git_add_all_and_commit(base_path, "Initial commit");
    }

    if !skip_rules && did_git_init {
        log::debug!("Adding agent submodules...");

        let submodules: &[(&str, &str)] = &[
            (
                "https://github.com/UserGeneratedLLC/agent-rules.git",
                ".cursor/rules/shared",
            ),
            (
                "https://github.com/UserGeneratedLLC/agent-commands.git",
                ".cursor/commands/shared",
            ),
            (
                "https://github.com/UserGeneratedLLC/agent-skills.git",
                ".cursor/skills/shared",
            ),
            (
                "https://github.com/UserGeneratedLLC/agent-docs.git",
                ".cursor/docs/shared",
            ),
        ];

        let clone_results: Vec<anyhow::Result<()>> = std::thread::scope(|s| {
            let handles: Vec<_> = submodules
                .iter()
                .map(|(url, path)| {
                    let target = base_path.join(path);
                    s.spawn(move || crate::git::git_clone_shallow(url, &target))
                })
                .collect();
            handles
                .into_iter()
                .map(|h| {
                    h.join()
                        .unwrap_or_else(|_| Err(anyhow::anyhow!("thread panicked")))
                })
                .collect()
        });

        let mut any_failed = false;
        for ((url, path), clone_res) in submodules.iter().zip(clone_results) {
            if let Err(e) = clone_res {
                log::warn!("Failed to clone {path}: {e}");
                any_failed = true;
                continue;
            }
            if let Err(e) = crate::git::git_submodule_add(base_path, url, path) {
                log::warn!("Failed to register submodule {path}: {e}");
                any_failed = true;
            }
        }

        if let Err(e) = crate::git::git_config_set(base_path, "submodule.recurse", "true") {
            log::warn!("Failed to set submodule.recurse: {e}");
            any_failed = true;
        }

        if !any_failed {
            println!("Added agent submodules successfully.");
        }

        crate::git::git_add_all_and_commit(base_path, "Add agent submodules");
    }

    Ok(())
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
pub fn write_if_not_exists(path: &Path, contents: &str) -> Result<(), anyhow::Error> {
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
