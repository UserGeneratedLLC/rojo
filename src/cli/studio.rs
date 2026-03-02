use std::path::PathBuf;
#[cfg(windows)]
use std::process::Command;

use anyhow::Context;
use clap::Parser;
use memofs::Vfs;

use crate::project::Project;
use crate::roblox_api;

use super::resolve_path;

/// Open a Rojo project in Roblox Studio.
#[derive(Debug, Parser)]
pub struct StudioCommand {
    /// Path to the project. Defaults to the current directory.
    #[clap(default_value = ".")]
    pub project: PathBuf,
}

impl StudioCommand {
    pub fn run(self, global: super::GlobalOptions) -> anyhow::Result<()> {
        let vfs = Vfs::new_oneshot();

        let base_path = resolve_path(&self.project);
        let project = Project::load_fuzzy(&vfs, &base_path)?
            .context("A project file is required to run 'atlas studio'")?;

        let serve_place_ids = project
            .serve_place_ids
            .as_ref()
            .context("No servePlaceIds in project file. Add servePlaceIds to your project file.")?;

        let place_id = serve_place_ids
            .iter()
            .min()
            .copied()
            .context("servePlaceIds is empty in project file")?;

        let auth = roblox_api::try_resolve_auth(global.opencloud.as_deref());
        let universe_id = match auth {
            Some(a) => roblox_api::get_universe_id(place_id, &a)?,
            None => anyhow::bail!("No Roblox auth cookie found. Please log into Roblox Studio."),
        };

        let url = format!(
            "roblox-studio:1+launchmode:edit+task:EditPlace+placeId:{}+universeId:{}",
            place_id, universe_id
        );

        #[cfg(windows)]
        Command::new("cmd")
            .args(["/c", "start", "", &url])
            .spawn()
            .context("Failed to launch Roblox Studio")?;

        #[cfg(not(windows))]
        opener::open(&url).context("Failed to open Roblox Studio")?;

        Ok(())
    }
}
