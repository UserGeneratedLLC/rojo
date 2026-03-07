use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{bail, Context};
use clap::Parser;
use fs_err as fs;

use crate::roblox_api;

use super::init::{setup_git_and_rules, write_if_not_exists, write_template_files, InitKind};
use super::syncback::SyncbackCommand;
use super::GlobalOptions;

/// Initializes a new Rojo project from one or more Roblox places and syncs them back.
///
/// With a single place ID, behaves identically to the original clone flow
/// (init + syncback into `default.project.json5` with `$path: "src"`).
///
/// With multiple place IDs (must belong to the same universe), creates a
/// multi-place project where each place gets its own `<name>.project.json5`
/// and `<name>/` directory.
#[derive(Debug, Parser)]
pub struct CloneCommand {
    /// One or more place IDs to download and sync back.
    #[clap(required = true, num_args = 1..)]
    pub placeids: Vec<u64>,

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

    /// Skip changing the working directory into the project after creation.
    #[clap(long)]
    pub skip_cd: bool,
}

impl CloneCommand {
    pub fn run(self, global: GlobalOptions) -> anyhow::Result<()> {
        if self.placeids.len() == 1 {
            return self.run_single(global);
        }
        self.run_multi(global)
    }

    fn run_single(self, global: GlobalOptions) -> anyhow::Result<()> {
        let place_id = self.placeids[0];

        let path = match self.path {
            Some(p) => p,
            None => resolve_folder_from_experience(place_id, global.opencloud.as_deref())?,
        };

        let skip_git = self.skip_git;

        let init = super::init::InitCommand {
            path: path.clone(),
            kind: self.kind,
            skip_git,
            placeid: Some(place_id),
            skip_rules: self.skip_rules,
            skip_cd: true,
        };

        init.run()?;

        let syncback = SyncbackCommand {
            project: PathBuf::from("default.project.json5"),
            input: PathBuf::from("Project.rbxl"),
            download: Some(place_id),
            list: false,
            dry_run: false,
            interactive: false,
            incremental: false,
            sourcemap: false,
            working_dir: path.clone(),
        };

        syncback.run(global)?;

        if !skip_git {
            crate::git::git_add_all_and_commit(&path, "syncback");
        }

        if !self.skip_cd {
            std::env::set_current_dir(&path)
                .with_context(|| format!("Failed to cd into {}", path.display()))?;
        }

        Ok(())
    }

    fn run_multi(self, global: GlobalOptions) -> anyhow::Result<()> {
        let auth = roblox_api::resolve_auth(global.opencloud.as_deref())?;

        // Validate all places belong to the same universe.
        println!("Validating place IDs...");
        let universe_ids: Vec<u64> = self
            .placeids
            .iter()
            .map(|id| {
                roblox_api::get_universe_id(*id, &auth)
                    .with_context(|| format!("Failed to resolve universe for place {id}"))
            })
            .collect::<anyhow::Result<Vec<_>>>()?;

        let first_universe = universe_ids[0];
        for (i, uid) in universe_ids.iter().enumerate() {
            if *uid != first_universe {
                bail!(
                    "Place {} belongs to universe {} but place {} belongs to universe {}. \
                     All place IDs must be from the same experience.",
                    self.placeids[i],
                    uid,
                    self.placeids[0],
                    first_universe,
                );
            }
        }

        // Resolve root folder.
        let path = match self.path {
            Some(p) => p,
            None => resolve_folder_from_experience(self.placeids[0], global.opencloud.as_deref())?,
        };

        if path.exists() {
            let is_empty = path.read_dir()?.next().is_none();
            if !is_empty {
                bail!(
                    "Directory '{}' is not empty. Please use an empty directory.",
                    path.display()
                );
            }
        }

        fs::create_dir_all(&path)?;

        // Fetch place names.
        println!("Fetching place names...");
        let place_names = roblox_api::fetch_place_names(&self.placeids, &auth)?;

        let places: Vec<PlaceEntry> = build_place_entries(&self.placeids, &place_names);

        println!("Cloning {} places into '{}':", places.len(), path.display());
        for p in &places {
            println!("  {} -> {}.project.json5", p.place_id, p.dir_name);
        }

        // Write shared template files (everything except default.project.json5).
        let canonical = fs::canonicalize(&path)?;
        let project_name = canonical
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("new-project");

        let exclude: HashSet<&str> = ["default.project.json5"].into();
        write_template_files(
            &path,
            self.kind,
            project_name,
            None,
            self.skip_git,
            &exclude,
        )?;

        // Generate a project file for each place.
        for entry in &places {
            let project_content = generate_project_json5(entry);
            let project_path = path.join(format!("{}.project.json5", entry.dir_name));
            write_if_not_exists(&project_path, &project_content)?;
        }

        // Git init + cursor rules.
        setup_git_and_rules(&path, self.skip_git, self.skip_rules)?;

        // Download all places in parallel.
        println!("Downloading {} places...", places.len());
        let download_timer = Instant::now();
        let auth_ref = &auth;
        let temp_files: Vec<(u64, anyhow::Result<tempfile::NamedTempFile>)> =
            std::thread::scope(|s| {
                let handles: Vec<_> = places
                    .iter()
                    .map(|entry| {
                        let pid = entry.place_id;
                        s.spawn(move || (pid, roblox_api::download_place(pid, auth_ref)))
                    })
                    .collect();
                handles
                    .into_iter()
                    .map(|h| {
                        h.join()
                            .unwrap_or_else(|_| (0, Err(anyhow::anyhow!("thread panicked"))))
                    })
                    .collect()
            });
        println!(
            "Downloaded in {:.02}s",
            download_timer.elapsed().as_secs_f32()
        );

        // Collect results, bail on any download failure.
        let mut downloaded: HashMap<u64, tempfile::NamedTempFile> =
            HashMap::with_capacity(places.len());
        for (pid, result) in temp_files {
            let temp = result.with_context(|| format!("Failed to download place {pid}"))?;
            downloaded.insert(pid, temp);
        }

        // Syncback each place sequentially.
        for entry in &places {
            println!(
                "Syncing back place {} ({})...",
                entry.dir_name, entry.place_id
            );

            let temp = downloaded.remove(&entry.place_id).unwrap();
            let input_path = temp.path().to_path_buf();

            let syncback = SyncbackCommand {
                project: PathBuf::from(format!("{}.project.json5", entry.dir_name)),
                input: input_path,
                download: None,
                list: false,
                dry_run: false,
                interactive: false,
                incremental: false,
                sourcemap: false,
                working_dir: path.clone(),
            };

            syncback.run(GlobalOptions {
                verbosity: global.verbosity,
                color: global.color,
                opencloud: global.opencloud.clone(),
            })?;
        }

        if !self.skip_git {
            crate::git::git_add_all_and_commit(&path, "syncback");
        }

        println!("Created multi-place project successfully.");

        if !self.skip_cd {
            std::env::set_current_dir(&path)
                .with_context(|| format!("Failed to cd into {}", path.display()))?;
        }

        Ok(())
    }
}

struct PlaceEntry {
    place_id: u64,
    dir_name: String,
}

fn build_place_entries(place_ids: &[u64], names: &HashMap<u64, String>) -> Vec<PlaceEntry> {
    let mut entries = Vec::with_capacity(place_ids.len());
    let mut taken: HashSet<String> = HashSet::new();

    for (i, &pid) in place_ids.iter().enumerate() {
        let fallback = || format!("place-{}", i + 1);

        let base = match names.get(&pid) {
            Some(api_name) => {
                let sanitized = sanitize_place_name(api_name);
                if sanitized.is_empty() {
                    fallback()
                } else {
                    sanitized
                }
            }
            None => fallback(),
        };

        let dir_name = if taken.contains(&base) {
            let mut n = 2u32;
            loop {
                let candidate = format!("{base}-{n}");
                if !taken.contains(&candidate) {
                    break candidate;
                }
                n += 1;
            }
        } else {
            base
        };

        taken.insert(dir_name.clone());
        entries.push(PlaceEntry {
            place_id: pid,
            dir_name,
        });
    }

    entries
}

fn generate_project_json5(entry: &PlaceEntry) -> String {
    format!(
        "{{\n\
         \tname: \"{name}\",\n\
         \tservePlaceIds: [{id}],\n\
         \tsyncScriptsOnly: true,\n\
         \tglobIgnorePaths: [\"{name}/ServerStorage/RBX_ANIMSAVES/**\", \"{name}/ServerStorage/MoonAnimator2Saves/**\"],\n\
         \tsyncbackRules: {{\n\
         \t\tignoreTrees: [\"ServerStorage/RBX_ANIMSAVES/**\", \"ServerStorage/MoonAnimator2Saves/**\"],\n\
         \t}},\n\
         \ttree: {{\n\
         \t\t$className: \"DataModel\",\n\
         \t\t$path: \"{name}\"\n\
         \t}}\n\
         }}\n",
        name = entry.dir_name,
        id = entry.place_id,
    )
}

/// Resolve a folder name from the experience name for a given place ID.
fn resolve_folder_from_experience(
    place_id: u64,
    opencloud_key: Option<&str>,
) -> anyhow::Result<PathBuf> {
    let auth = roblox_api::try_resolve_auth(opencloud_key);
    let name = match auth {
        Some(a) => roblox_api::fetch_experience_name(place_id, &a)?
            .context("Could not fetch experience name from Roblox API")?,
        None => bail!(
            "No --opencloud API key or Roblox cookie available to fetch experience name. \
             Use --path to specify a folder manually, or pass --opencloud <KEY>."
        ),
    };
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
    Ok(PathBuf::from(folder))
}

/// Sanitize a Roblox experience name into a valid folder name.
///
/// 1. Strip `[...]` and `(...)` sections
/// 2. Remove noise words (e.g. "testing")
/// 3. Keep only ASCII alphanumeric and spaces
/// 4. Collapse whitespace, lowercase, join with hyphens
fn sanitize_name(name: &str) -> String {
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

    cleaned
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == ' '))
        .collect::<String>()
        .split_whitespace()
        .filter(|w| !w.eq_ignore_ascii_case("testing"))
        .collect::<Vec<_>>()
        .join("-")
        .to_lowercase()
}

/// Sanitize a Roblox place name into a valid directory name.
///
/// Simpler than experience name sanitization: keeps alphanumeric + spaces,
/// collapses whitespace, lowercases, joins with hyphens.
fn sanitize_place_name(name: &str) -> String {
    name.split(|c: char| !(c.is_ascii_alphanumeric() || c == ' '))
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

    #[test]
    fn sanitize_removes_testing() {
        assert_eq!(sanitize_name("My Game Testing"), "my-game");
        assert_eq!(sanitize_name("TESTING My Game"), "my-game");
        assert_eq!(sanitize_name("My testing Game"), "my-game");
    }

    #[test]
    fn sanitize_place_name_simple() {
        assert_eq!(sanitize_place_name("Game"), "game");
        assert_eq!(sanitize_place_name("Lobby"), "lobby");
        assert_eq!(sanitize_place_name("Main Menu"), "main-menu");
    }

    #[test]
    fn sanitize_place_name_special_chars() {
        assert_eq!(sanitize_place_name("Game! 🎮"), "game");
        assert_eq!(sanitize_place_name("Test: Level 1"), "test-level-1");
    }

    #[test]
    fn build_entries_uses_api_names() {
        let mut names = HashMap::new();
        names.insert(111, "Game".to_string());
        names.insert(222, "Lobby".to_string());

        let entries = build_place_entries(&[111, 222], &names);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].dir_name, "game");
        assert_eq!(entries[0].place_id, 111);
        assert_eq!(entries[1].dir_name, "lobby");
        assert_eq!(entries[1].place_id, 222);
    }

    #[test]
    fn build_entries_dedup_collision() {
        let mut names = HashMap::new();
        names.insert(111, "Game".to_string());
        names.insert(222, "Game".to_string());

        let entries = build_place_entries(&[111, 222], &names);
        assert_eq!(entries[0].dir_name, "game");
        assert_eq!(entries[1].dir_name, "game-2");
    }

    #[test]
    fn build_entries_fallback_when_no_name() {
        let names = HashMap::new();

        let entries = build_place_entries(&[111, 222], &names);
        assert_eq!(entries[0].dir_name, "place-1");
        assert_eq!(entries[1].dir_name, "place-2");
    }

    #[test]
    fn generate_project_json5_format() {
        let entry = PlaceEntry {
            place_id: 12345,
            dir_name: "game".to_string(),
        };
        let content = generate_project_json5(&entry);
        assert!(content.contains("name: \"game\""));
        assert!(content.contains("servePlaceIds: [12345]"));
        assert!(content.contains("$path: \"game\""));
        assert!(content.contains("syncScriptsOnly: true"));
    }
}
