use std::path::PathBuf;
#[cfg(windows)]
use std::process::Command;

use anyhow::Context;
use clap::Parser;
use memofs::Vfs;
use serde::Deserialize;

use crate::project::Project;

use super::resolve_path;

#[derive(Deserialize)]
struct UniverseResponse {
    #[serde(rename = "universeId")]
    universe_id: u64,
}

/// Open a Rojo project in Roblox Studio.
#[derive(Debug, Parser)]
pub struct StudioCommand {
    /// Path to the project. Defaults to the current directory.
    #[clap(default_value = ".")]
    pub project: PathBuf,
}

impl StudioCommand {
    pub fn run(self) -> anyhow::Result<()> {
        let vfs = Vfs::new_oneshot();

        let base_path = resolve_path(&self.project);
        let project = Project::load_fuzzy(&vfs, &base_path)?
            .context("A project file is required to run 'rojo studio'")?;

        let serve_place_ids = project
            .serve_place_ids
            .as_ref()
            .context("No servePlaceIds in project file. Add servePlaceIds to your project file.")?;

        let place_id = serve_place_ids
            .iter()
            .min()
            .copied()
            .context("servePlaceIds is empty in project file")?;

        let universe_id = get_universe_id(place_id)?;

        let url = format!(
            "roblox-studio:1+launchmode:edit+task:EditPlace+placeId:{}+universeId:{}",
            place_id, universe_id
        );

        // Use cmd /c start to fully detach the process on Windows
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

fn get_universe_id(place_id: u64) -> anyhow::Result<u64> {
    let url = format!(
        "https://apis.roblox.com/universes/v1/places/{}/universe",
        place_id
    );

    let response: UniverseResponse = reqwest::blocking::get(&url)
        .context("Failed to fetch universe ID from Roblox API")?
        .json()
        .context("Failed to parse universe ID response")?;

    Ok(response.universe_id)
}
