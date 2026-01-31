use std::path::PathBuf;

use anyhow::Context;
use clap::Parser;
use memofs::Vfs;

use crate::project::Project;

use super::resolve_path;

/// Reformat a Rojo project using the standard JSON formatting rules.
#[derive(Debug, Parser)]
pub struct FmtProjectCommand {
    /// Path to the project to format. Defaults to the current directory.
    #[clap(default_value = "")]
    pub project: PathBuf,
}

impl FmtProjectCommand {
    pub fn run(self) -> anyhow::Result<()> {
        // Use oneshot Vfs - file watching isn't needed for formatting
        let vfs = Vfs::new_oneshot();

        let base_path = resolve_path(&self.project);
        let project = Project::load_fuzzy(&vfs, &base_path)?
            .context("A project file is required to run 'rojo fmt-project'")?;

        let serialized = String::from_utf8(
            crate::json::to_vec_pretty_sorted(&project)
                .context("could not re-encode project file as JSON5")?,
        )
        .context("JSON5 output was not valid UTF-8")?;

        fs_err::write(&project.file_location, serialized)
            .context("could not write back to project file")?;

        Ok(())
    }
}
