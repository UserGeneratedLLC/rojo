//! Defines Rojo's CLI through clap types.

mod build;
mod clone;
mod completions;
mod cursor;
mod doc;
mod fmt_project;
mod init;
mod plugin;
mod serve;
mod sourcemap;
mod studio;
mod syncback;
mod upload;

use std::{
    borrow::Cow,
    env,
    path::{Path, PathBuf},
    str::FromStr,
};

use clap::Parser;
use thiserror::Error;

pub use self::build::BuildCommand;
pub use self::clone::CloneCommand;
pub use self::completions::CompletionsCommand;
pub use self::cursor::CursorCommand;
pub use self::doc::DocCommand;
pub use self::fmt_project::FmtProjectCommand;
pub use self::init::{InitCommand, InitKind};
pub use self::plugin::{PluginCommand, PluginSubcommand};
pub use self::serve::ServeCommand;
pub use self::sourcemap::SourcemapCommand;
pub use self::studio::StudioCommand;
pub use self::syncback::SyncbackCommand;
pub use self::upload::UploadCommand;

/// Command line options that Rojo accepts, defined using the clap crate.
#[derive(Debug, Parser)]
#[clap(name = "Atlas", version, about)]
pub struct Options {
    #[clap(flatten)]
    pub global: GlobalOptions,

    /// Subcommand to run in this invocation.
    #[clap(subcommand)]
    pub subcommand: Subcommand,
}

impl Options {
    pub fn run(self) -> anyhow::Result<()> {
        match self.subcommand {
            Subcommand::Clone(subcommand) => subcommand.run(self.global),
            Subcommand::Completions(subcommand) => subcommand.run(),
            Subcommand::Init(subcommand) => subcommand.run(),
            Subcommand::Serve(subcommand) => subcommand.run(),
            Subcommand::Build(subcommand) => subcommand.run(),
            Subcommand::Upload(subcommand) => subcommand.run(),
            Subcommand::Sourcemap(subcommand) => subcommand.run(),
            Subcommand::FmtProject(subcommand) => subcommand.run(),
            Subcommand::Cursor(subcommand) => subcommand.run(),
            Subcommand::Doc(subcommand) => subcommand.run(),
            Subcommand::Plugin(subcommand) => subcommand.run(),
            Subcommand::Studio(subcommand) => subcommand.run(),
            Subcommand::Syncback(subcommand) | Subcommand::Pull(subcommand) => {
                subcommand.run(self.global)
            }
        }
    }
}

#[derive(Debug, Parser)]
pub struct GlobalOptions {
    /// Sets verbosity level. Can be specified multiple times.
    #[clap(long("verbose"), short, global(true), action = clap::ArgAction::Count)]
    pub verbosity: u8,

    /// Set color behavior. Valid values are auto, always, and never.
    #[clap(long("color"), global(true), default_value("auto"))]
    pub color: ColorChoice,
}

#[derive(Debug, Clone, Copy)]
pub enum ColorChoice {
    Auto,
    Always,
    Never,
}

impl FromStr for ColorChoice {
    type Err = ColorChoiceParseError;

    fn from_str(source: &str) -> Result<Self, Self::Err> {
        match source {
            "auto" => Ok(ColorChoice::Auto),
            "always" => Ok(ColorChoice::Always),
            "never" => Ok(ColorChoice::Never),
            _ => Err(ColorChoiceParseError {
                attempted: source.to_owned(),
            }),
        }
    }
}

impl From<ColorChoice> for termcolor::ColorChoice {
    fn from(value: ColorChoice) -> Self {
        match value {
            ColorChoice::Auto => termcolor::ColorChoice::Auto,
            ColorChoice::Always => termcolor::ColorChoice::Always,
            ColorChoice::Never => termcolor::ColorChoice::Never,
        }
    }
}

#[derive(Debug, Error)]
#[error("Invalid color choice '{attempted}'. Valid values are: auto, always, never")]
pub struct ColorChoiceParseError {
    attempted: String,
}

#[derive(Debug, Parser)]
pub enum Subcommand {
    Clone(CloneCommand),
    Completions(CompletionsCommand),
    Init(InitCommand),
    Serve(ServeCommand),
    Build(BuildCommand),
    Upload(UploadCommand),
    Sourcemap(SourcemapCommand),
    FmtProject(FmtProjectCommand),
    Cursor(CursorCommand),
    Doc(DocCommand),
    Plugin(PluginCommand),
    Studio(StudioCommand),
    Syncback(SyncbackCommand),
    /// Alias for `syncback`.
    #[clap(hide = true)]
    Pull(SyncbackCommand),
}

impl Subcommand {
    pub fn project_path(&self) -> Option<&Path> {
        match self {
            Subcommand::Clone(cmd) => cmd.path.as_deref(),
            Subcommand::Serve(cmd) => Some(&cmd.project),
            Subcommand::Build(cmd) => Some(&cmd.project),
            Subcommand::Upload(cmd) => Some(&cmd.project),
            Subcommand::Sourcemap(cmd) => Some(&cmd.project),
            Subcommand::FmtProject(cmd) => Some(&cmd.project),
            Subcommand::Studio(cmd) => Some(&cmd.project),
            Subcommand::Syncback(cmd) | Subcommand::Pull(cmd) => Some(&cmd.project),
            _ => None,
        }
    }

    pub fn command_name(&self) -> &'static str {
        match self {
            Subcommand::Clone(_) => "clone",
            Subcommand::Completions(_) => "completions",
            Subcommand::Init(_) => "init",
            Subcommand::Serve(_) => "serve",
            Subcommand::Build(_) => "build",
            Subcommand::Upload(_) => "upload",
            Subcommand::Sourcemap(_) => "sourcemap",
            Subcommand::FmtProject(_) => "fmt-project",
            Subcommand::Cursor(_) => "cursor",
            Subcommand::Doc(_) => "doc",
            Subcommand::Plugin(_) => "plugin",
            Subcommand::Studio(_) => "studio",
            Subcommand::Syncback(_) => "syncback",
            Subcommand::Pull(_) => "pull",
        }
    }
}

pub fn resolve_path(path: &Path) -> Cow<'_, Path> {
    if path.is_absolute() {
        Cow::Borrowed(path)
    } else {
        Cow::Owned(env::current_dir().unwrap().join(path))
    }
}

/// Resolves a project path (which may point to a file) to its parent directory.
pub fn resolve_project_dir(project_path: &Path) -> PathBuf {
    let resolved = resolve_path(project_path);
    let resolved = resolved.as_ref();

    if resolved.is_file() {
        resolved
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| resolved.to_path_buf())
    } else if resolved.as_os_str().is_empty() {
        env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
    } else {
        resolved.to_path_buf()
    }
}
