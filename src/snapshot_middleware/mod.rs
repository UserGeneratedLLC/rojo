//! Defines the semantics that Rojo uses to turn entries on the filesystem into
//! Roblox instances using the instance snapshot subsystem.
//!
//! These modules define how files turn into instances.

#![allow(dead_code)]

mod csv;
mod dir;
mod json;
mod json_model;
mod lua;
mod meta_file;
mod project;
mod rbxm;
mod rbxmx;
mod toml;
mod txt;
mod util;
mod yaml;

use std::{
    path::{Path, PathBuf},
    sync::OnceLock,
};

use anyhow::Context;
use memofs::{IoResultExt, Vfs};
use serde::{Deserialize, Serialize};

use crate::{
    glob::Glob,
    syncback::{dedup_suffix::strip_dedup_suffix, SyncbackReturn, SyncbackSnapshot},
};
use crate::{
    snapshot::{InstanceContext, InstanceSnapshot, SyncRule},
    syncback::validate_file_name,
};

use self::{
    csv::{snapshot_csv, snapshot_csv_init, syncback_csv, syncback_csv_init},
    dir::{snapshot_dir, syncback_dir},
    json::snapshot_json,
    json_model::{snapshot_json_model, syncback_json_model},
    lua::{snapshot_lua, snapshot_lua_init, syncback_lua, syncback_lua_init},
    project::{snapshot_project, syncback_project},
    rbxm::{snapshot_rbxm, syncback_rbxm},
    rbxmx::{snapshot_rbxmx, syncback_rbxmx},
    toml::snapshot_toml,
    txt::{snapshot_txt, syncback_txt},
    yaml::snapshot_yaml,
};

pub use self::{lua::ScriptType, project::snapshot_project_node, util::PathExt};

/// Returns an `InstanceSnapshot` for the provided path.
/// This will inspect the path and find the appropriate middleware for it,
/// taking user-written rules into account. Then, it will attempt to convert
/// the path into an InstanceSnapshot using that middleware.
#[profiling::function]
pub fn snapshot_from_vfs(
    context: &InstanceContext,
    vfs: &Vfs,
    path: &Path,
) -> anyhow::Result<Option<InstanceSnapshot>> {
    let meta = match vfs.metadata(path).with_not_found()? {
        Some(meta) => meta,
        None => return Ok(None),
    };

    if meta.is_dir() {
        let (middleware, dir_name, init_path) = get_dir_middleware(vfs, path)?;
        // The directory name is used as-is from the filesystem.
        // If a different instance name is desired, it comes from the
        // `name` field in init.meta.json / init.meta.json5 (applied later
        // by DirectoryMetadata::apply_name).
        // TODO: Support user defined init paths
        // If and when we do, make sure to go support it in
        // `Project::set_file_name`, as right now it special-cases
        // `default.project.json5` as an `init` path.
        if context.sync_scripts_only
            && !middleware.is_script()
            && middleware != Middleware::Dir
            && middleware != Middleware::Project
        {
            return Ok(None);
        }
        match middleware {
            Middleware::Dir => middleware.snapshot(context, vfs, path, dir_name),
            _ => middleware.snapshot(context, vfs, &init_path, dir_name),
        }
    } else {
        let file_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .with_context(|| format!("file name of {} is invalid", path.display()))?;

        // TODO: Is this even necessary anymore?
        match file_name {
            // Modern extensions
            "init.server.luau" | "init.client.luau" | "init.local.luau" | "init.legacy.luau"
            | "init.plugin.luau" | "init.luau" | "init.csv" |
            // Legacy extensions (for backwards compatibility)
            "init.server.lua" | "init.client.lua" | "init.lua" => return Ok(None),
            _ => {}
        }

        snapshot_from_path(context, vfs, path)
    }
}

/// Single source of truth for init-file resolution priority.
/// Project files are checked first, then init files in this order.
/// Used by both `get_dir_middleware` (live) and `prefetch_project_files` (cache).
///
/// NOTE: The project file entries must match `DEFAULT_PROJECT_NAMES` in
/// `project.rs`. The `init_file_priority_includes_all_project_names` test
/// guards against drift.
pub static INIT_FILE_PRIORITY: &[(Middleware, &str)] = &[
    (Middleware::Project, "default.project.json5"),
    (Middleware::Project, "default.project.json"),
    (Middleware::ModuleScriptDir, "init.luau"),
    (Middleware::ServerScriptDir, "init.server.luau"),
    (Middleware::ClientScriptDir, "init.client.luau"),
    (Middleware::PluginScriptDir, "init.plugin.luau"),
    (Middleware::LocalScriptDir, "init.local.luau"),
    (Middleware::LegacyScriptDir, "init.legacy.luau"),
    (Middleware::CsvDir, "init.csv"),
    // Legacy extensions (for backwards compatibility)
    // init.server.lua → Script with RunContext.Legacy (old emitLegacyScripts behavior)
    // init.client.lua → LocalScript (old emitLegacyScripts behavior)
    (Middleware::ModuleScriptDir, "init.lua"),
    (Middleware::LegacyScriptDir, "init.server.lua"),
    (Middleware::LocalScriptDir, "init.client.lua"),
];

/// Gets the appropriate middleware for a directory by checking for `init`
/// files. This uses an intrinsic priority list and for compatibility,
/// that order should be left unchanged.
///
/// Returns the middleware, the name of the directory, and the path to
/// the init location.
fn get_dir_middleware<'path>(
    vfs: &Vfs,
    dir_path: &'path Path,
) -> anyhow::Result<(Middleware, &'path str, PathBuf)> {
    let dir_name = dir_path
        .file_name()
        .expect("Could not extract directory name")
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("File name was not valid UTF-8: {}", dir_path.display()))?;
    let dir_name = strip_dedup_suffix(dir_name);

    if let Some(cached) = vfs.prefetch_dir_init(dir_path) {
        return match cached {
            Some((init_name, init_path)) => {
                let middleware = INIT_FILE_PRIORITY
                    .iter()
                    .find(|(_, name)| *name == init_name)
                    .map(|(m, _)| *m)
                    .unwrap_or(Middleware::Dir);
                Ok((middleware, dir_name, init_path))
            }
            None => Ok((Middleware::Dir, dir_name, dir_path.to_path_buf())),
        };
    }

    for &(middleware, name) in INIT_FILE_PRIORITY {
        let test_path = dir_path.join(name);
        if vfs.metadata(&test_path).with_not_found()?.is_some() {
            return Ok((middleware, dir_name, test_path));
        }
    }

    Ok((Middleware::Dir, dir_name, dir_path.to_path_buf()))
}

/// Gets a snapshot for a path given an InstanceContext and Vfs, taking
/// user specified sync rules into account.
fn snapshot_from_path(
    context: &InstanceContext,
    vfs: &Vfs,
    path: &Path,
) -> anyhow::Result<Option<InstanceSnapshot>> {
    // File names are used as-is from the filesystem. If a different instance
    // name is needed (e.g. for names with special chars), it comes from the
    // `name` field in adjacent .meta.json / .model.json files.
    if let Some(rule) = context.get_user_sync_rule(path) {
        if context.sync_scripts_only
            && !rule.middleware.is_script()
            && rule.middleware != Middleware::Project
        {
            return Ok(None);
        }
        let name = rule.file_name_for_path(path)?;
        return rule.middleware.snapshot(context, vfs, path, name);
    } else {
        for rule in default_sync_rules() {
            if rule.matches(path) {
                if context.sync_scripts_only
                    && !rule.middleware.is_script()
                    && rule.middleware != Middleware::Project
                {
                    return Ok(None);
                }
                let name = rule.file_name_for_path(path)?;
                return rule.middleware.snapshot(context, vfs, path, name);
            }
        }
    }
    Ok(None)
}

/// Represents a possible 'transformer' used by Rojo to turn a file system
/// item into a Roblox Instance. Missing from this list is metadata.
/// This is deliberate, as metadata is not a snapshot middleware.
///
/// Directories cannot be used for sync rules so they're ignored by Serde.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Middleware {
    Csv,
    JsonModel,
    Json,
    ServerScript,
    ClientScript,
    ModuleScript,
    PluginScript,
    LocalScript,
    LegacyScript,
    Project,
    Rbxm,
    Rbxmx,
    Toml,
    Text,
    Yaml,
    Ignore,

    #[serde(skip_deserializing)]
    Dir,
    #[serde(skip_deserializing)]
    ServerScriptDir,
    #[serde(skip_deserializing)]
    ClientScriptDir,
    #[serde(skip_deserializing)]
    ModuleScriptDir,
    #[serde(skip_deserializing)]
    PluginScriptDir,
    #[serde(skip_deserializing)]
    LocalScriptDir,
    #[serde(skip_deserializing)]
    LegacyScriptDir,
    #[serde(skip_deserializing)]
    CsvDir,
}

impl Middleware {
    /// Creates a snapshot for the given path from the Middleware with
    /// the provided name.
    fn snapshot(
        &self,
        context: &InstanceContext,
        vfs: &Vfs,
        path: &Path,
        name: &str,
    ) -> anyhow::Result<Option<InstanceSnapshot>> {
        let mut output = match self {
            Self::Csv => snapshot_csv(context, vfs, path, name),
            Self::JsonModel => snapshot_json_model(context, vfs, path, name),
            Self::Json => snapshot_json(context, vfs, path, name),
            Self::ServerScript => snapshot_lua(context, vfs, path, name, ScriptType::Server),
            Self::ClientScript => snapshot_lua(context, vfs, path, name, ScriptType::Client),
            Self::ModuleScript => snapshot_lua(context, vfs, path, name, ScriptType::Module),
            Self::PluginScript => snapshot_lua(context, vfs, path, name, ScriptType::Plugin),
            Self::LocalScript => snapshot_lua(context, vfs, path, name, ScriptType::Local),
            Self::LegacyScript => snapshot_lua(context, vfs, path, name, ScriptType::Legacy),
            Self::Project => snapshot_project(context, vfs, path, name),
            Self::Rbxm => snapshot_rbxm(context, vfs, path, name),
            Self::Rbxmx => snapshot_rbxmx(context, vfs, path, name),
            Self::Toml => snapshot_toml(context, vfs, path, name),
            Self::Text => snapshot_txt(context, vfs, path, name),
            Self::Yaml => snapshot_yaml(context, vfs, path, name),
            Self::Ignore => Ok(None),

            Self::Dir => snapshot_dir(context, vfs, path, name),
            Self::ServerScriptDir => {
                snapshot_lua_init(context, vfs, path, name, ScriptType::Server)
            }
            Self::ClientScriptDir => {
                snapshot_lua_init(context, vfs, path, name, ScriptType::Client)
            }
            Self::ModuleScriptDir => {
                snapshot_lua_init(context, vfs, path, name, ScriptType::Module)
            }
            Self::PluginScriptDir => {
                snapshot_lua_init(context, vfs, path, name, ScriptType::Plugin)
            }
            Self::LocalScriptDir => snapshot_lua_init(context, vfs, path, name, ScriptType::Local),
            Self::LegacyScriptDir => {
                snapshot_lua_init(context, vfs, path, name, ScriptType::Legacy)
            }
            Self::CsvDir => snapshot_csv_init(context, vfs, path, name),
        };
        if let Ok(Some(ref mut snapshot)) = output {
            snapshot.metadata.middleware = Some(*self);
        }
        output
    }

    /// Runs the syncback mechanism for the provided middleware given a
    /// SyncbackSnapshot.
    pub fn syncback<'sync>(
        &self,
        snapshot: &SyncbackSnapshot<'sync>,
    ) -> anyhow::Result<SyncbackReturn<'sync>> {
        let file_name = snapshot.path.file_name().and_then(|s| s.to_str());
        if let Some(file_name) = file_name {
            validate_file_name(file_name).with_context(|| {
                format!("cannot create a file or directory with name {file_name}")
            })?;
        }
        match self {
            Middleware::Csv => syncback_csv(snapshot),
            Middleware::JsonModel => syncback_json_model(snapshot),
            Middleware::Json => anyhow::bail!("cannot syncback Json middleware"),
            // Projects are only generated from files that already exist on the
            // file system, so we don't need to pass a file name.
            Middleware::Project => syncback_project(snapshot),
            Middleware::ServerScript => syncback_lua(snapshot),
            Middleware::ClientScript => syncback_lua(snapshot),
            Middleware::ModuleScript => syncback_lua(snapshot),
            Middleware::PluginScript => syncback_lua(snapshot),
            Middleware::LocalScript => syncback_lua(snapshot),
            Middleware::LegacyScript => syncback_lua(snapshot),
            Middleware::Rbxm => syncback_rbxm(snapshot),
            Middleware::Rbxmx => syncback_rbxmx(snapshot),
            Middleware::Toml => anyhow::bail!("cannot syncback Toml middleware"),
            Middleware::Text => syncback_txt(snapshot),
            Middleware::Yaml => anyhow::bail!("cannot syncback Yaml middleware"),
            Middleware::Ignore => anyhow::bail!("cannot syncback Ignore middleware"),
            Middleware::Dir => syncback_dir(snapshot),
            Middleware::ServerScriptDir => syncback_lua_init(ScriptType::Server, snapshot),
            Middleware::ClientScriptDir => syncback_lua_init(ScriptType::Client, snapshot),
            Middleware::ModuleScriptDir => syncback_lua_init(ScriptType::Module, snapshot),
            Middleware::LocalScriptDir => syncback_lua_init(ScriptType::Local, snapshot),
            Middleware::LegacyScriptDir => syncback_lua_init(ScriptType::Legacy, snapshot),
            Middleware::PluginScriptDir => syncback_lua_init(ScriptType::Plugin, snapshot),
            Middleware::CsvDir => syncback_csv_init(snapshot),
        }
    }

    /// Returns whether this middleware produces a script instance
    /// (Script, LocalScript, or ModuleScript).
    #[inline]
    pub fn is_script(&self) -> bool {
        matches!(
            self,
            Self::ServerScript
                | Self::ClientScript
                | Self::ModuleScript
                | Self::PluginScript
                | Self::LocalScript
                | Self::LegacyScript
                | Self::ServerScriptDir
                | Self::ClientScriptDir
                | Self::ModuleScriptDir
                | Self::PluginScriptDir
                | Self::LocalScriptDir
                | Self::LegacyScriptDir
        )
    }

    /// Returns whether this particular middleware would become a directory.
    #[inline]
    pub fn is_dir(&self) -> bool {
        matches!(
            self,
            Middleware::Dir
                | Middleware::ServerScriptDir
                | Middleware::ClientScriptDir
                | Middleware::ModuleScriptDir
                | Middleware::PluginScriptDir
                | Middleware::LocalScriptDir
                | Middleware::LegacyScriptDir
                | Middleware::CsvDir
        )
    }

    /// Returns whether this particular middleware sets its own properties.
    /// This applies to things like `JsonModel` and `Project`, since they
    /// set properties without needing a meta.json5 file.
    ///
    /// It does not cover middleware like `ServerScript` or `Csv` because they
    /// need a meta.json5 file to set properties that aren't their designated
    /// 'special' properties.
    #[inline]
    pub fn handles_own_properties(&self) -> bool {
        matches!(
            self,
            Middleware::JsonModel | Middleware::Project | Middleware::Rbxm | Middleware::Rbxmx
        )
    }

    /// Attempts to return a middleware that should be used for the given path.
    ///
    /// Returns `Err` only if the Vfs cannot read information about the path.
    pub fn middleware_for_path(
        vfs: &Vfs,
        sync_rules: &[SyncRule],
        path: &Path,
    ) -> anyhow::Result<Option<Self>> {
        let meta = match vfs.metadata(path).with_not_found()? {
            Some(meta) => meta,
            None => return Ok(None),
        };

        if meta.is_dir() {
            let (middleware, _, _) = get_dir_middleware(vfs, path)?;
            Ok(Some(middleware))
        } else {
            for rule in sync_rules.iter().chain(default_sync_rules()) {
                if rule.matches(path) {
                    return Ok(Some(rule.middleware));
                }
            }
            Ok(None)
        }
    }
}

/// A helper for easily defining a SyncRule. Arguments are passed literally
/// to this macro in the order `include`, `middleware`, `suffix`,
/// and `exclude`. Both `suffix` and `exclude` are optional.
///
/// All arguments except `middleware` are expected to be strings.
/// The `middleware` parameter is expected to be a variant of `Middleware`,
/// not including the enum name itself.
macro_rules! sync_rule {
    ($pattern:expr, $middleware:ident) => {
        SyncRule {
            middleware: Middleware::$middleware,
            include: Glob::new($pattern).unwrap(),
            exclude: None,
            suffix: None,
            base_path: PathBuf::new(),
        }
    };
    ($pattern:expr, $middleware:ident, $suffix:expr) => {
        SyncRule {
            middleware: Middleware::$middleware,
            include: Glob::new($pattern).unwrap(),
            exclude: None,
            suffix: Some($suffix.into()),
            base_path: PathBuf::new(),
        }
    };
    ($pattern:expr, $middleware:ident, $suffix:expr, $exclude:expr) => {
        SyncRule {
            middleware: Middleware::$middleware,
            include: Glob::new($pattern).unwrap(),
            exclude: Some(Glob::new($exclude).unwrap()),
            suffix: Some($suffix.into()),
            base_path: PathBuf::new(),
        }
    };
}

/// Defines the 'default' syncing rules that Rojo uses.
/// These do not broadly overlap, but the order matters for some in the case of
/// e.g. JSON models.
pub fn default_sync_rules() -> &'static [SyncRule] {
    static DEFAULT_SYNC_RULES: OnceLock<Vec<SyncRule>> = OnceLock::new();

    DEFAULT_SYNC_RULES.get_or_init(|| {
        vec![
            // Modern extensions (preferred)
            sync_rule!("*.server.luau", ServerScript, ".server.luau"),
            sync_rule!("*.client.luau", ClientScript, ".client.luau"),
            sync_rule!("*.plugin.luau", PluginScript, ".plugin.luau"),
            sync_rule!("*.legacy.luau", LegacyScript, ".legacy.luau"),
            sync_rule!("*.local.luau", LocalScript, ".local.luau"),
            sync_rule!("*.luau", ModuleScript),
            sync_rule!("*.project.json5", Project, ".project.json5"),
            sync_rule!("*.model.json5", JsonModel, ".model.json5"),
            sync_rule!("*.json5", Json, ".json5", "*.meta.json5"),
            // Legacy Lua extensions (for backwards compatibility)
            // .server.lua → Script with RunContext.Legacy (old emitLegacyScripts behavior)
            // .client.lua → LocalScript (old emitLegacyScripts behavior)
            // .plugin.lua → Script with RunContext.Plugin
            sync_rule!("*.server.lua", LegacyScript, ".server.lua"),
            sync_rule!("*.client.lua", LocalScript, ".client.lua"),
            sync_rule!("*.plugin.lua", PluginScript, ".plugin.lua"),
            sync_rule!("*.lua", ModuleScript),
            // Legacy JSON extensions (for backwards compatibility)
            sync_rule!("*.project.json", Project, ".project.json"),
            sync_rule!("*.model.json", JsonModel, ".model.json"),
            sync_rule!("*.json", Json, ".json", "*.meta.json"),
            // Other formats
            sync_rule!("*.toml", Toml),
            sync_rule!("*.csv", Csv),
            sync_rule!("*.txt", Text),
            sync_rule!("*.rbxmx", Rbxmx),
            sync_rule!("*.rbxm", Rbxm),
            sync_rule!("*.{yml,yaml}", Yaml),
        ]
    })
}

/// Returns whether a filesystem path is relevant in scripts-only mode.
///
/// Matches script files (`.luau`, `.lua`), meta files (`.meta.json5`,
/// `.meta.json`), and project files (`.project.json5`, `.project.json`).
pub fn is_script_relevant_path(path: &Path) -> bool {
    let name = match path.file_name().and_then(|n| n.to_str()) {
        Some(n) => n.to_lowercase(),
        None => return false,
    };
    name.ends_with(".luau")
        || name.ends_with(".lua")
        || name.ends_with(".meta.json5")
        || name.ends_with(".meta.json")
        || name.ends_with(".project.json5")
        || name.ends_with(".project.json")
}

#[cfg(test)]
mod test {
    use super::*;
    use std::collections::HashMap;

    use memofs::{InMemoryFs, VfsSnapshot};

    #[test]
    fn init_file_priority_includes_all_project_names() {
        use crate::project::DEFAULT_PROJECT_NAMES;
        let priority_names: Vec<&str> = INIT_FILE_PRIORITY
            .iter()
            .filter(|(m, _)| *m == Middleware::Project)
            .map(|(_, name)| *name)
            .collect();
        for project_name in DEFAULT_PROJECT_NAMES {
            assert!(
                priority_names.contains(&project_name),
                "DEFAULT_PROJECT_NAMES entry {project_name:?} missing from INIT_FILE_PRIORITY"
            );
        }
    }

    #[test]
    fn is_script_covers_all_script_types() {
        assert!(Middleware::ServerScript.is_script());
        assert!(Middleware::ClientScript.is_script());
        assert!(Middleware::ModuleScript.is_script());
        assert!(Middleware::PluginScript.is_script());
        assert!(Middleware::LocalScript.is_script());
        assert!(Middleware::LegacyScript.is_script());
        assert!(Middleware::ServerScriptDir.is_script());
        assert!(Middleware::ClientScriptDir.is_script());
        assert!(Middleware::ModuleScriptDir.is_script());
        assert!(Middleware::PluginScriptDir.is_script());
        assert!(Middleware::LocalScriptDir.is_script());
        assert!(Middleware::LegacyScriptDir.is_script());
    }

    #[test]
    fn is_script_excludes_non_scripts() {
        assert!(!Middleware::Dir.is_script());
        assert!(!Middleware::Project.is_script());
        assert!(!Middleware::Csv.is_script());
        assert!(!Middleware::JsonModel.is_script());
        assert!(!Middleware::Json.is_script());
        assert!(!Middleware::Rbxm.is_script());
        assert!(!Middleware::Rbxmx.is_script());
        assert!(!Middleware::Toml.is_script());
        assert!(!Middleware::Text.is_script());
        assert!(!Middleware::Yaml.is_script());
        assert!(!Middleware::Ignore.is_script());
        assert!(!Middleware::CsvDir.is_script());
    }

    #[test]
    fn scripts_only_skips_non_script_file() {
        let mut imfs = InMemoryFs::new();
        imfs.load_snapshot(
            "/project",
            VfsSnapshot::dir(HashMap::from([
                ("data.json5", VfsSnapshot::file("{}")),
                ("script.server.luau", VfsSnapshot::file("print('hi')")),
            ])),
        )
        .unwrap();

        let vfs = Vfs::new(imfs);
        let mut context = InstanceContext::new();
        context.sync_scripts_only = true;

        let json_result =
            snapshot_from_vfs(&context, &vfs, Path::new("/project/data.json5")).unwrap();
        assert!(json_result.is_none());

        let script_result =
            snapshot_from_vfs(&context, &vfs, Path::new("/project/script.server.luau")).unwrap();
        assert!(script_result.is_some());
        assert_eq!(script_result.unwrap().class_name.as_str(), "Script");
    }

    #[test]
    fn scripts_only_preserves_directories() {
        let mut imfs = InMemoryFs::new();
        imfs.load_snapshot(
            "/project",
            VfsSnapshot::dir(HashMap::from([(
                "models",
                VfsSnapshot::dir(HashMap::from([(
                    "part.rbxm",
                    VfsSnapshot::file(b"\x00".as_ref()),
                )])),
            )])),
        )
        .unwrap();

        let vfs = Vfs::new(imfs);
        let mut context = InstanceContext::new();
        context.sync_scripts_only = true;

        let result = snapshot_from_vfs(&context, &vfs, Path::new("/project/models")).unwrap();
        assert!(result.is_some());
        let snapshot = result.unwrap();
        assert_eq!(snapshot.class_name.as_str(), "Folder");
        assert!(snapshot.children.is_empty());
    }

    #[test]
    fn scripts_only_allows_project_files() {
        let mut imfs = InMemoryFs::new();
        imfs.load_snapshot(
            "/nested.project.json5",
            VfsSnapshot::file(r#"{"name": "Nested", "tree": {"$className": "Folder"}}"#),
        )
        .unwrap();

        let vfs = Vfs::new(imfs);
        let mut context = InstanceContext::new();
        context.sync_scripts_only = true;

        let result = snapshot_from_vfs(&context, &vfs, Path::new("/nested.project.json5")).unwrap();
        assert!(result.is_some());
    }

    #[test]
    fn normal_mode_includes_all_files() {
        let mut imfs = InMemoryFs::new();
        imfs.load_snapshot(
            "/project",
            VfsSnapshot::dir(HashMap::from([
                ("data.json5", VfsSnapshot::file("{}")),
                ("script.server.luau", VfsSnapshot::file("print('hi')")),
            ])),
        )
        .unwrap();

        let vfs = Vfs::new(imfs);
        let context = InstanceContext::new();

        let json_result =
            snapshot_from_vfs(&context, &vfs, Path::new("/project/data.json5")).unwrap();
        assert!(json_result.is_some());

        let script_result =
            snapshot_from_vfs(&context, &vfs, Path::new("/project/script.server.luau")).unwrap();
        assert!(script_result.is_some());
    }

    #[test]
    fn is_script_relevant_path_accepts_scripts_and_meta() {
        assert!(is_script_relevant_path(Path::new("/src/main.luau")));
        assert!(is_script_relevant_path(Path::new("/src/init.server.luau")));
        assert!(is_script_relevant_path(Path::new("/src/old.lua")));
        assert!(is_script_relevant_path(Path::new("/src/old.server.lua")));
        assert!(is_script_relevant_path(Path::new("/src/file.meta.json5")));
        assert!(is_script_relevant_path(Path::new("/src/file.meta.json")));
        assert!(is_script_relevant_path(Path::new("/nested.project.json5")));
        assert!(is_script_relevant_path(Path::new("/nested.project.json")));
    }

    #[test]
    fn is_script_relevant_path_rejects_non_scripts() {
        assert!(!is_script_relevant_path(Path::new("/src/data.json5")));
        assert!(!is_script_relevant_path(Path::new("/src/model.rbxm")));
        assert!(!is_script_relevant_path(Path::new("/src/notes.txt")));
        assert!(!is_script_relevant_path(Path::new("/src/table.csv")));
        assert!(!is_script_relevant_path(Path::new("/src/config.toml")));
        assert!(!is_script_relevant_path(Path::new("/src/data.yaml")));
        assert!(!is_script_relevant_path(Path::new("/src/model.rbxmx")));
        assert!(!is_script_relevant_path(Path::new("/src")));
        assert!(!is_script_relevant_path(Path::new("/src/no_ext")));
    }

    #[test]
    fn scripts_only_filters_csv_dir() {
        let mut imfs = InMemoryFs::new();
        imfs.load_snapshot(
            "/project",
            VfsSnapshot::dir(HashMap::from([(
                "localization",
                VfsSnapshot::dir(HashMap::from([(
                    "init.csv",
                    VfsSnapshot::file("Key,Source,Example\nkey1,Hello,Hello"),
                )])),
            )])),
        )
        .unwrap();

        let vfs = Vfs::new(imfs);
        let mut context = InstanceContext::new();
        context.sync_scripts_only = true;

        let result = snapshot_from_vfs(&context, &vfs, Path::new("/project/localization")).unwrap();
        assert!(
            result.is_none(),
            "CsvDir should be filtered in scripts-only mode"
        );
    }

    #[test]
    fn scripts_only_preserves_script_dir() {
        let mut imfs = InMemoryFs::new();
        imfs.load_snapshot(
            "/project",
            VfsSnapshot::dir(HashMap::from([(
                "MyScript",
                VfsSnapshot::dir(HashMap::from([(
                    "init.server.luau",
                    VfsSnapshot::file("print('hello')"),
                )])),
            )])),
        )
        .unwrap();

        let vfs = Vfs::new(imfs);
        let mut context = InstanceContext::new();
        context.sync_scripts_only = true;

        let result = snapshot_from_vfs(&context, &vfs, Path::new("/project/MyScript")).unwrap();
        assert!(
            result.is_some(),
            "Script dir should pass through in scripts-only mode"
        );
        assert_eq!(result.unwrap().class_name.as_str(), "Script");
    }
}
