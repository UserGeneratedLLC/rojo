use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{bail, Context};
use clap::Parser;
use reqwest::blocking::Client;
use serde::Deserialize;

use super::init::{InitCommand, InitKind};
use super::syncback::SyncbackCommand;
use super::GlobalOptions;

/// Initializes a new Rojo project from a Roblox place and syncs it back.
///
/// Fetches the experience name from Roblox to auto-generate the project
/// folder. Equivalent to `rojo init` + `rojo syncback --download`.
#[derive(Debug, Parser)]
pub struct CloneCommand {
    /// The place ID to download and sync back.
    pub placeid: u64,

    /// Path to create the project in. If omitted, a directory is
    /// auto-generated from the experience name.
    #[clap(long)]
    pub path: Option<PathBuf>,

    /// The kind of project to create, 'place', 'plugin', or 'model'.
    #[clap(long, default_value = "place")]
    pub kind: InitKind,

    /// Skips the initialization of a git repository.
    #[clap(long)]
    pub skip_git: bool,

    /// Skip cloning cursor rules into .cursor directory.
    #[clap(long)]
    pub skip_rules: bool,
}

impl CloneCommand {
    pub fn run(self, global: GlobalOptions) -> anyhow::Result<()> {
        let path = match self.path {
            Some(p) => p,
            None => {
                let name = fetch_experience_name(self.placeid)?;
                let folder = sanitize_name(&name);
                if folder.is_empty() {
                    bail!(
                        "Could not derive a folder name from experience '{}'. \
                         Use --path to specify one manually.",
                        name
                    );
                }
                if Path::new(&folder).exists() {
                    bail!(
                        "Directory '{}' already exists. \
                         Remove it or use --path to specify a different location.",
                        folder
                    );
                }
                println!("Using folder: {folder}");
                PathBuf::from(folder)
            }
        };

        let skip_git = self.skip_git;

        let init = InitCommand {
            path: path.clone(),
            kind: self.kind,
            skip_git,
            placeid: Some(self.placeid),
            skip_rules: self.skip_rules,
        };

        init.run()?;

        let syncback = SyncbackCommand {
            project: PathBuf::from("default.project.json5"),
            input: PathBuf::from("Project.rbxl"),
            download: Some(self.placeid),
            list: false,
            dry_run: false,
            interactive: false,
            incremental: false,
            working_dir: path.clone(),
        };

        syncback.run(global)?;

        // Commit syncback result
        if !skip_git {
            let _ = Command::new("git")
                .args(["add", "."])
                .current_dir(&path)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();

            let _ = Command::new("git")
                .args(["commit", "--no-verify", "-m", "syncback"])
                .current_dir(&path)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }

        Ok(())
    }
}

#[derive(Debug, Deserialize)]
struct UniverseResponse {
    #[serde(rename = "universeId")]
    universe_id: u64,
}

#[derive(Debug, Deserialize)]
struct GamesResponse {
    data: Vec<GameData>,
}

#[derive(Debug, Deserialize)]
struct GameData {
    name: String,
}

/// Resolves a place ID to its experience name via the Roblox API.
fn fetch_experience_name(place_id: u64) -> anyhow::Result<String> {
    let client = Client::new();

    // Place ID -> Universe ID
    let universe_url = format!("https://apis.roblox.com/universes/v1/places/{place_id}/universe");
    let universe: UniverseResponse = client
        .get(&universe_url)
        .send()
        .context("Failed to reach Roblox universe API")?
        .json()
        .context("Failed to parse universe response")?;

    // Universe ID -> Game info
    let games_url = format!(
        "https://games.roblox.com/v1/games?universeIds={}",
        universe.universe_id
    );
    let games: GamesResponse = client
        .get(&games_url)
        .send()
        .context("Failed to reach Roblox games API")?
        .json()
        .context("Failed to parse games response")?;

    let game = games
        .data
        .into_iter()
        .next()
        .context("No game data returned from Roblox")?;

    println!("Experience: {}", game.name);

    Ok(game.name)
}

/// Sanitize a Roblox experience name into a valid folder name.
///
/// 1. Strip `[...]` and `(...)` sections
/// 2. Keep only ASCII alphanumeric and spaces
/// 3. Collapse whitespace, lowercase, join with hyphens
fn sanitize_name(name: &str) -> String {
    // Remove [...] and (...) sections (handles nesting)
    let mut cleaned = String::with_capacity(name.len());
    let mut bracket_depth: u32 = 0;
    let mut paren_depth: u32 = 0;

    for ch in name.chars() {
        match ch {
            '[' => bracket_depth += 1,
            ']' if bracket_depth > 0 => bracket_depth -= 1,
            '(' => paren_depth += 1,
            ')' if paren_depth > 0 => paren_depth -= 1,
            _ if bracket_depth == 0 && paren_depth == 0 => cleaned.push(ch),
            _ => {}
        }
    }

    // Keep only ASCII alphanumeric + spaces, then collapse/join
    cleaned
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == ' '))
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join("-")
        .to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_typical_name() {
        assert_eq!(
            sanitize_name("Escape Tsunami For Brainrots!"),
            "escape-tsunami-for-brainrots"
        );
    }

    #[test]
    fn sanitize_brackets_and_parens() {
        assert_eq!(sanitize_name("My Game (UPDATE) [v2.0]"), "my-game");
    }

    #[test]
    fn sanitize_emojis() {
        assert_eq!(sanitize_name("Cool Game 🌊🏃⚡"), "cool-game");
    }

    #[test]
    fn sanitize_complex() {
        assert_eq!(
            sanitize_name("some crazy game! (UPDATE) [LOL] 🎮"),
            "some-crazy-game"
        );
    }

    #[test]
    fn sanitize_only_special_chars() {
        assert_eq!(sanitize_name("🎮🌊⚡"), "");
    }
}
