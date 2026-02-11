//! Contains logic for generating new file names for Instances based on their
//! middleware.

use std::borrow::Cow;
use std::collections::HashSet;

use anyhow::Context;
use rbx_dom_weak::Instance;

use crate::{snapshot::InstanceWithMeta, snapshot_middleware::Middleware};

/// Generates a filesystem name for an instance.
/// Returns `(filename, needs_meta_name, dedup_key)`.
///
/// - `filename`: The full filesystem name (including extension for file middleware).
/// - `needs_meta_name`: `true` when the filesystem name differs from the instance
///   name (meaning a `name` field must be written in metadata).
/// - `dedup_key`: The bare slug (without extension) that callers must insert into
///   `taken_names`. This is the name-level identifier used for collision detection.
///   Callers should **always** use `dedup_key` (not `filename`) when accumulating
///   into `taken_names`.
///
/// If `old_inst` exists, its existing path is preserved (incremental mode).
/// For new instances, names with forbidden chars are slugified and deduplicated
/// against `taken_names`.
pub fn name_for_inst<'a>(
    middleware: Middleware,
    new_inst: &'a Instance,
    old_inst: Option<InstanceWithMeta<'a>>,
    taken_names: &HashSet<String>,
) -> anyhow::Result<(Cow<'a, str>, bool, String)> {
    if let Some(old_inst) = old_inst {
        if let Some(source) = old_inst.metadata().relevant_paths.first() {
            let name = source
                .file_name()
                .and_then(|s| s.to_str())
                .context("sources on the file system should be valid unicode and not be stubs")?;
            // Derive dedup_key by stripping the middleware extension from the
            // filename. For Dir middleware the extension is empty so this is a
            // no-op. This keeps extension logic inside this function.
            let dedup_key = strip_middleware_extension(name, middleware);
            Ok((Cow::Borrowed(name), false, dedup_key))
        } else {
            anyhow::bail!(
                "members of 'old' trees should have an instigating source. Somehow, {} did not.",
                old_inst.name(),
            );
        }
    } else {
        // Determine base name: slugify if the raw name isn't filesystem-safe
        let needs_slugify = name_needs_slugify(&new_inst.name);
        let base = if needs_slugify {
            slugify_name(&new_inst.name)
        } else {
            new_inst.name.clone()
        };

        match middleware {
            Middleware::Dir
            | Middleware::CsvDir
            | Middleware::ServerScriptDir
            | Middleware::ClientScriptDir
            | Middleware::ModuleScriptDir
            | Middleware::PluginScriptDir
            | Middleware::LocalScriptDir
            | Middleware::LegacyScriptDir => {
                let deduped = deduplicate_name(&base, taken_names);
                let needs_meta = needs_slugify || deduped != base;
                let dedup_key = deduped.clone();
                Ok((Cow::Owned(deduped), needs_meta, dedup_key))
            }
            _ => {
                let extension = extension_for_middleware(middleware);
                let deduped = deduplicate_name(&base, taken_names);
                let needs_meta = needs_slugify || deduped != base;
                let dedup_key = deduped.clone();
                Ok((
                    Cow::Owned(format!("{deduped}.{extension}")),
                    needs_meta,
                    dedup_key,
                ))
            }
        }
    }
}

/// Strips the middleware extension from a filename to recover the bare slug.
/// Used by `name_for_inst` to derive the dedup key for old instances, and by
/// callers that need to seed `taken_names` from filesystem paths.
pub fn strip_middleware_extension(filename: &str, middleware: Middleware) -> String {
    // Dir middleware has no extension — filename IS the slug
    match middleware {
        Middleware::Dir
        | Middleware::CsvDir
        | Middleware::ServerScriptDir
        | Middleware::ClientScriptDir
        | Middleware::ModuleScriptDir
        | Middleware::PluginScriptDir
        | Middleware::LocalScriptDir
        | Middleware::LegacyScriptDir => filename.to_string(),
        _ => {
            let ext = extension_for_middleware(middleware);
            let suffix = format!(".{ext}");
            filename
                .strip_suffix(&suffix)
                .unwrap_or(filename)
                .to_string()
        }
    }
}

/// Returns the extension a provided piece of middleware is supposed to use.
pub fn extension_for_middleware(middleware: Middleware) -> &'static str {
    match middleware {
        Middleware::Csv => "csv",
        Middleware::JsonModel => "model.json5",
        Middleware::Json => "json5",
        Middleware::ServerScript => "server.luau",
        Middleware::ClientScript => "client.luau",
        Middleware::ModuleScript => "luau",
        Middleware::PluginScript => "plugin.luau",
        Middleware::LocalScript => "local.luau",
        Middleware::LegacyScript => "legacy.luau",
        Middleware::Project => "project.json5",
        Middleware::Rbxm => "rbxm",
        Middleware::Rbxmx => "rbxmx",
        Middleware::Toml => "toml",
        Middleware::Text => "txt",
        Middleware::Yaml => "yml",

        // These are manually specified and not `_` to guard against future
        // middleware additions missing this function.
        Middleware::Ignore => unimplemented!("syncback does not work on Ignore middleware"),
        Middleware::Dir
        | Middleware::CsvDir
        | Middleware::ServerScriptDir
        | Middleware::ClientScriptDir
        | Middleware::ModuleScriptDir
        | Middleware::PluginScriptDir
        | Middleware::LocalScriptDir
        | Middleware::LegacyScriptDir => {
            unimplemented!("directory middleware requires special treatment")
        }
    }
}

/// A list of file names that are not valid on Windows.
const INVALID_WINDOWS_NAMES: [&str; 22] = [
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// A list of all characters that are outright forbidden to be included
/// in a file's name.
const FORBIDDEN_CHARS: [char; 9] = ['<', '>', ':', '"', '/', '|', '?', '*', '\\'];

/// Characters that must be slugified when converting instance names to
/// filesystem names. Extends FORBIDDEN_CHARS with:
/// - `~` (conflicts with dedup suffix `~1`, `~2`, etc.)
///
/// Note: spaces are NOT in this list. Middle spaces are valid in filenames.
/// Leading/trailing spaces are handled separately by boundary checks.
const SLUGIFY_CHARS: [char; 10] = ['<', '>', ':', '"', '/', '|', '?', '*', '\\', '~'];

/// Suffixes (case-insensitive) that, if an instance name ends with one,
/// would create a compound extension that tricks Rojo's sync rule matching.
///
/// Example: instance `foo.server` + extension `.luau` → `foo.server.luau`
/// which matches `*.server.luau` (ServerScript) instead of the intended
/// `*.luau` (ModuleScript).
const DANGEROUS_SUFFIXES: [&str; 8] = [
    ".server", ".client", ".plugin", ".local", ".legacy", ".meta", ".model", ".project",
];

/// Returns `true` if an instance name contains characters or patterns that
/// require slugification for safe filesystem use.
pub fn name_needs_slugify(name: &str) -> bool {
    if name.is_empty() {
        return true;
    }
    if name.starts_with(' ') || name.ends_with(' ') || name.ends_with('.') {
        return true;
    }
    for ch in name.chars() {
        if SLUGIFY_CHARS.contains(&ch) || ch.is_control() {
            return true;
        }
    }
    // Check for dangerous suffixes that would collide with Rojo's compound
    // extensions (e.g., ".server" + ".luau" → ".server.luau")
    if has_dangerous_suffix(name) {
        return true;
    }
    let name_lower = name.to_lowercase();
    for forbidden in INVALID_WINDOWS_NAMES {
        if name_lower == forbidden.to_lowercase() {
            return true;
        }
    }
    false
}

/// Returns `true` if the name ends with a dangerous suffix (case-insensitive).
fn has_dangerous_suffix(name: &str) -> bool {
    let lower = name.to_lowercase();
    DANGEROUS_SUFFIXES.iter().any(|s| lower.ends_with(s))
}

/// Slugifies a name by replacing forbidden filesystem characters with
/// underscores and ensuring the result is a valid file name.
///
/// Replaces OS-forbidden chars and `~` (dedup separator) with underscores.
/// Strips leading/trailing spaces. Neutralises dangerous suffixes
/// (`.server`, `.meta`, etc.) that would collide with Rojo's compound
/// extensions by replacing the offending dot with `_`.
///
/// This is a pure, stateless function. It does NOT handle collisions --
/// use `deduplicate_name()` after slugifying to resolve those.
pub fn slugify_name(name: &str) -> String {
    let mut result = String::with_capacity(name.len());

    for ch in name.chars() {
        if SLUGIFY_CHARS.contains(&ch) || ch.is_control() {
            result.push('_');
        } else {
            result.push(ch);
        }
    }

    // Strip leading spaces
    while result.starts_with(' ') {
        result.remove(0);
    }

    // Neutralise dangerous suffixes by replacing the last dot before
    // the suffix with an underscore. Loop because replacement could
    // reveal another dangerous suffix (e.g., "a.meta.server" → fix
    // ".server" → "a.meta_server" → still has ".meta" → fix that too).
    while has_dangerous_suffix(&result) {
        if let Some(pos) = result.rfind('.') {
            result.replace_range(pos..pos + 1, "_");
        } else {
            break;
        }
    }

    // Strip trailing spaces and dots (invalid on Windows) BEFORE the
    // reserved-name check so that inputs like "CON." or "CON " are
    // reduced to "CON" and then correctly caught below.
    while result.ends_with(' ') || result.ends_with('.') {
        result.pop();
    }

    // Handle Windows reserved names by appending an underscore
    let result_lower = result.to_lowercase();
    for forbidden in INVALID_WINDOWS_NAMES {
        if result_lower == forbidden.to_lowercase() {
            result.push('_');
            break;
        }
    }

    // If the result is empty or all underscores, use a fallback
    if result.is_empty() || result.chars().all(|c| c == '_') {
        result = "instance".to_string();
    }

    result
}

/// Appends ~1, ~2, etc. to avoid collisions. Returns the name as-is if
/// unclaimed.
///
/// Comparisons are **case-insensitive** because Windows and macOS have
/// case-insensitive filesystems. `taken_names` must contain **lowercased**
/// entries for this to work correctly.
pub fn deduplicate_name(base: &str, taken_names: &std::collections::HashSet<String>) -> String {
    let base_lower = base.to_lowercase();
    if !taken_names.contains(&base_lower) {
        return base.to_string();
    }
    for i in 1.. {
        let candidate = format!("{base}~{i}");
        if !taken_names.contains(&candidate.to_lowercase()) {
            return candidate;
        }
    }
    unreachable!()
}

/// Validates a provided file name to ensure it's allowed on the file system. An
/// error is returned if the name isn't allowed, indicating why.
/// This takes into account rules for Windows, MacOS, and Linux.
///
/// In practice however, these broadly overlap so the only unexpected behavior
/// is Windows, where there are 22 reserved names.
pub fn validate_file_name<S: AsRef<str>>(name: S) -> anyhow::Result<()> {
    let str = name.as_ref();

    if str.ends_with(' ') {
        anyhow::bail!("file names cannot end with a space")
    }
    if str.ends_with('.') {
        anyhow::bail!("file names cannot end with '.'")
    }

    for char in str.chars() {
        if FORBIDDEN_CHARS.contains(&char) {
            anyhow::bail!("file names cannot contain <, >, :, \", /, |, ?, *, or \\")
        } else if char.is_control() {
            anyhow::bail!("file names cannot contain control characters")
        }
    }

    for forbidden in INVALID_WINDOWS_NAMES {
        if str == forbidden {
            anyhow::bail!("files cannot be named {str}")
        }
    }

    Ok(())
}

/// Known script suffixes that appear between the base name and file extension.
/// For example, in `MyScript.server.luau`, `.server` is the suffix.
const KNOWN_SCRIPT_SUFFIXES: &[&str] = &[".server", ".client", ".plugin", ".local", ".legacy"];

/// Strips a known script suffix from a file stem.
///
/// For example, `"MyScript.server"` → `"MyScript"`, but `"MyScript"` → `"MyScript"`.
pub fn strip_script_suffix(stem: &str) -> &str {
    for suffix in KNOWN_SCRIPT_SUFFIXES {
        if let Some(base) = stem.strip_suffix(suffix) {
            return base;
        }
    }
    stem
}

/// Given a script file path like `parent/Foo_Bar.server.luau`,
/// returns the adjacent meta path `parent/Foo_Bar.meta.json5`.
///
/// Strips the file extension and any known script suffix (`.server`, `.client`,
/// etc.) to derive the base name, then appends `.meta.json5`.
pub fn adjacent_meta_path(script_path: &std::path::Path) -> std::path::PathBuf {
    let stem = script_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    let base = strip_script_suffix(stem);
    script_path.with_file_name(format!("{}.meta.json5", base))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rbx_dom_weak::{InstanceBuilder, WeakDom};

    // ── slugify_name ──────────────────────────────────────────────────

    #[test]
    fn slugify_clean_name_unchanged() {
        assert_eq!(slugify_name("Hello"), "Hello");
        assert_eq!(slugify_name("MyModule"), "MyModule");
        assert_eq!(slugify_name("foo_bar_baz"), "foo_bar_baz");
    }

    #[test]
    fn slugify_each_slugify_char() {
        for ch in SLUGIFY_CHARS {
            let input = format!("A{ch}B");
            let result = slugify_name(&input);
            assert_eq!(result, "A_B", "slugify char {ch:?} should become _");
        }
    }

    #[test]
    fn slugify_dots_preserved_when_safe() {
        // Dots are allowed when they don't form a dangerous suffix
        assert_eq!(slugify_name("v1.0"), "v1.0");
        assert_eq!(slugify_name("foo.bar"), "foo.bar");
        assert_eq!(slugify_name("hello.world"), "hello.world");
    }

    #[test]
    fn slugify_multiple_forbidden_chars() {
        assert_eq!(slugify_name("Hey/Bro"), "Hey_Bro");
        assert_eq!(slugify_name("Hey:Bro"), "Hey_Bro");
        assert_eq!(slugify_name("a<b>c:d"), "a_b_c_d");
        assert_eq!(slugify_name("What?Module"), "What_Module");
    }

    #[test]
    fn slugify_all_slugify_chars_string() {
        // Every char is slugified → all underscores → fallback to "instance"
        let input: String = SLUGIFY_CHARS.iter().collect();
        assert_eq!(slugify_name(&input), "instance");
    }

    #[test]
    fn slugify_windows_reserved_names() {
        assert_eq!(slugify_name("CON"), "CON_");
        assert_eq!(slugify_name("PRN"), "PRN_");
        assert_eq!(slugify_name("AUX"), "AUX_");
        assert_eq!(slugify_name("NUL"), "NUL_");
        assert_eq!(slugify_name("COM1"), "COM1_");
        assert_eq!(slugify_name("LPT9"), "LPT9_");
    }

    #[test]
    fn slugify_windows_reserved_case_insensitive() {
        assert_eq!(slugify_name("con"), "con_");
        assert_eq!(slugify_name("Con"), "Con_");
        assert_eq!(slugify_name("nul"), "nul_");
    }

    #[test]
    fn slugify_windows_reserved_with_trailing_dots_and_spaces() {
        // Trailing dots/spaces must be stripped BEFORE the reserved name
        // check, otherwise "CON." bypasses the check and becomes "CON".
        assert_eq!(slugify_name("CON."), "CON_");
        assert_eq!(slugify_name("CON "), "CON_");
        assert_eq!(slugify_name("CON.."), "CON_");
        assert_eq!(slugify_name("CON. "), "CON_");
        assert_eq!(slugify_name("PRN."), "PRN_");
        assert_eq!(slugify_name("AUX "), "AUX_");
        assert_eq!(slugify_name("nul."), "nul_");
        assert_eq!(slugify_name("com1."), "com1_");
        assert_eq!(slugify_name("LPT1 "), "LPT1_");
    }

    #[test]
    fn slugify_trailing_dot() {
        // Trailing dots are stripped (invalid on Windows)
        assert_eq!(slugify_name("hello."), "hello");
        assert_eq!(slugify_name("hello.."), "hello");
    }

    #[test]
    fn slugify_space() {
        // Middle spaces are preserved (valid in filenames)
        assert_eq!(slugify_name("hello world"), "hello world");
        // Trailing spaces are stripped
        assert_eq!(slugify_name("hello "), "hello");
        // Leading spaces are stripped
        assert_eq!(slugify_name(" hello"), "hello");
        // All spaces → empty after strip → fallback
        assert_eq!(slugify_name(" "), "instance");
        assert_eq!(slugify_name("  "), "instance");
    }

    #[test]
    fn slugify_trailing_dot_and_space_mixed() {
        // "hello. " → trailing " " stripped → "hello." → trailing "." stripped → "hello"
        assert_eq!(slugify_name("hello. "), "hello");
        // "hello ." → trailing "." stripped → "hello " → trailing " " stripped → "hello"
        assert_eq!(slugify_name("hello ."), "hello");
    }

    #[test]
    fn slugify_dangerous_suffixes() {
        // Suffixes that would collide with Rojo's compound extensions
        assert_eq!(slugify_name("foo.server"), "foo_server");
        assert_eq!(slugify_name("foo.client"), "foo_client");
        assert_eq!(slugify_name("foo.plugin"), "foo_plugin");
        assert_eq!(slugify_name("foo.local"), "foo_local");
        assert_eq!(slugify_name("foo.legacy"), "foo_legacy");
        assert_eq!(slugify_name("foo.meta"), "foo_meta");
        assert_eq!(slugify_name("foo.model"), "foo_model");
        assert_eq!(slugify_name("foo.project"), "foo_project");
    }

    #[test]
    fn slugify_dangerous_suffixes_case_insensitive() {
        assert_eq!(slugify_name("foo.Server"), "foo_Server");
        assert_eq!(slugify_name("foo.META"), "foo_META");
        assert_eq!(slugify_name("foo.Model"), "foo_Model");
    }

    #[test]
    fn slugify_nested_dangerous_suffixes() {
        // ".server" suffix is fixed; ".meta" in the middle is fine (not a suffix)
        assert_eq!(slugify_name("a.meta.server"), "a.meta_server");
        // Both are suffixes when stacked: fix ".server" → "a.meta_server" (no more suffix)
        // But if the name is just ".meta":
        assert_eq!(slugify_name("a.meta"), "a_meta");
    }

    #[test]
    fn slugify_empty_string() {
        assert_eq!(slugify_name(""), "instance");
    }

    #[test]
    fn slugify_single_forbidden_char() {
        // Single forbidden char → "_" → all underscores → fallback
        assert_eq!(slugify_name("/"), "instance");
        assert_eq!(slugify_name("*"), "instance");
    }

    #[test]
    fn slugify_tilde() {
        // ~ is slugified to prevent confusion with dedup suffix ~N
        assert_eq!(slugify_name("foo~1"), "foo_1");
        assert_eq!(slugify_name("~bar"), "_bar");
        assert_eq!(slugify_name("a~b~c"), "a_b_c");
    }

    #[test]
    fn slugify_unicode_preserved() {
        assert_eq!(slugify_name("日本語"), "日本語");
        assert_eq!(slugify_name("café"), "café");
        assert_eq!(slugify_name("émoji🎮"), "émoji🎮");
    }

    #[test]
    fn slugify_mixed_valid_and_forbidden() {
        assert_eq!(slugify_name("src/main:test"), "src_main_test");
        assert_eq!(slugify_name("file<1>"), "file_1_");
        // Dot preserved when safe, forbidden chars replaced
        assert_eq!(slugify_name("v1.0/release"), "v1.0_release");
    }

    // ── deduplicate_name ──────────────────────────────────────────────
    //
    // Contract: taken_names must contain LOWERCASED entries.

    #[test]
    fn dedup_no_collision() {
        let taken = HashSet::new();
        assert_eq!(deduplicate_name("Foo", &taken), "Foo");
    }

    #[test]
    fn dedup_single_collision() {
        let taken: HashSet<String> = ["foo".to_string()].into_iter().collect();
        assert_eq!(deduplicate_name("Foo", &taken), "Foo~1");
    }

    #[test]
    fn dedup_multiple_collisions() {
        let taken: HashSet<String> = ["foo", "foo~1", "foo~2"]
            .into_iter()
            .map(String::from)
            .collect();
        assert_eq!(deduplicate_name("Foo", &taken), "Foo~3");
    }

    #[test]
    fn dedup_skips_taken_suffix() {
        let taken: HashSet<String> = ["foo", "foo~1"].into_iter().map(String::from).collect();
        assert_eq!(deduplicate_name("Foo", &taken), "Foo~2");
    }

    #[test]
    fn dedup_gap_in_suffixes() {
        let taken: HashSet<String> = ["foo", "foo~2"].into_iter().map(String::from).collect();
        assert_eq!(deduplicate_name("Foo", &taken), "Foo~1");
    }

    #[test]
    fn dedup_natural_vs_slug_collision() {
        let taken: HashSet<String> = ["hey_bro".to_string()].into_iter().collect();
        assert_eq!(deduplicate_name("Hey_Bro", &taken), "Hey_Bro~1");
    }

    #[test]
    fn dedup_empty_taken_set() {
        let taken = HashSet::new();
        assert_eq!(deduplicate_name("anything", &taken), "anything");
    }

    #[test]
    fn dedup_case_insensitive() {
        // "foo" is taken, "Foo" should collide (case-insensitive filesystem)
        let taken: HashSet<String> = ["foo".to_string()].into_iter().collect();
        assert_eq!(deduplicate_name("Foo", &taken), "Foo~1");
        assert_eq!(deduplicate_name("FOO", &taken), "FOO~1");
        assert_eq!(deduplicate_name("foo", &taken), "foo~1");
    }

    #[test]
    fn dedup_case_only_difference() {
        // Two instances: "MyScript" and "myscript" - second must get ~1
        let taken: HashSet<String> = ["myscript".to_string()].into_iter().collect();
        assert_eq!(deduplicate_name("MyScript", &taken), "MyScript~1");
    }

    // ── name_needs_slugify ──────────────────────────────────────────────

    #[test]
    fn needs_slugify_clean_names() {
        assert!(!name_needs_slugify("Hello"));
        assert!(!name_needs_slugify("my_module"));
        assert!(!name_needs_slugify("test-123"));
    }

    #[test]
    fn needs_slugify_os_forbidden() {
        assert!(name_needs_slugify("a/b"));
        assert!(name_needs_slugify("a:b"));
        assert!(name_needs_slugify("a*b"));
    }

    #[test]
    fn needs_slugify_tilde() {
        assert!(name_needs_slugify("foo~1"));
        assert!(name_needs_slugify("~bar"));
    }

    #[test]
    fn needs_slugify_safe_dots_allowed() {
        // Dots in names that don't form dangerous suffixes are fine
        assert!(!name_needs_slugify("v1.0"));
        assert!(!name_needs_slugify("foo.bar"));
    }

    #[test]
    fn needs_slugify_dangerous_suffix() {
        assert!(name_needs_slugify("foo.server"));
        assert!(name_needs_slugify("foo.Server"));
        assert!(name_needs_slugify("foo.meta"));
        assert!(name_needs_slugify("foo.model"));
        assert!(name_needs_slugify("foo.project"));
        assert!(name_needs_slugify("foo.client"));
    }

    #[test]
    fn needs_slugify_windows_reserved() {
        assert!(name_needs_slugify("CON"));
        assert!(name_needs_slugify("con"));
        assert!(name_needs_slugify("NUL"));
    }

    #[test]
    fn needs_slugify_leading_or_trailing_space() {
        assert!(name_needs_slugify("hello ")); // trailing
        assert!(name_needs_slugify(" hello")); // leading
        assert!(name_needs_slugify("hello.")); // trailing dot
        assert!(!name_needs_slugify("hello world")); // middle space is fine
    }

    #[test]
    fn needs_slugify_empty() {
        assert!(name_needs_slugify(""));
    }

    #[test]
    fn needs_slugify_control_chars() {
        assert!(name_needs_slugify("a\x00b"));
        assert!(name_needs_slugify("tab\there"));
    }

    // ── validate_file_name ────────────────────────────────────────────

    #[test]
    fn validate_clean_names() {
        assert!(validate_file_name("Hello").is_ok());
        assert!(validate_file_name("my_file").is_ok());
        assert!(validate_file_name("test-123").is_ok());
        assert!(validate_file_name("日本語").is_ok());
    }

    #[test]
    fn validate_rejects_forbidden_chars() {
        for ch in FORBIDDEN_CHARS {
            let name = format!("a{ch}b");
            assert!(validate_file_name(&name).is_err(), "should reject {ch:?}");
        }
    }

    #[test]
    fn validate_rejects_trailing_dot() {
        assert!(validate_file_name("hello.").is_err());
    }

    #[test]
    fn validate_rejects_trailing_space() {
        assert!(validate_file_name("hello ").is_err());
    }

    #[test]
    fn validate_rejects_windows_reserved() {
        assert!(validate_file_name("CON").is_err());
        assert!(validate_file_name("NUL").is_err());
        assert!(validate_file_name("COM1").is_err());
    }

    #[test]
    fn validate_rejects_control_chars() {
        assert!(validate_file_name("hello\x00world").is_err());
        assert!(validate_file_name("tab\there").is_err());
    }

    #[test]
    fn validate_slugified_names_always_pass() {
        // Any output of slugify_name should pass validation
        let nasty_inputs = [
            "CON",
            "hello/world",
            "a:b:c",
            "test?",
            "file*glob",
            "trailing.",
            "trailing ",
            "",
            "<>:\"/\\|?*",
            "COM1",
            "LPT9",
            "foo~1",
            "foo.server",
            "bar.meta",
            "v1.0~beta",
            " leading space",
            "trailing space ",
            "hello world",
            "a.meta.server",
        ];
        for input in nasty_inputs {
            let slug = slugify_name(input);
            assert!(
                validate_file_name(&slug).is_ok(),
                "slugify_name({input:?}) = {slug:?} should pass validation"
            );
        }
    }

    // ── name_for_inst (new instances, no old_inst) ────────────────────

    /// Helper: create a WeakDom with a child instance of given name and class,
    /// return the dom and the child's Ref.
    fn make_inst(name: &str, class: &str) -> WeakDom {
        let builder = InstanceBuilder::new("DataModel")
            .with_child(InstanceBuilder::new(class).with_name(name));
        WeakDom::new(builder)
    }

    #[test]
    fn name_for_inst_clean_name_file_middleware() {
        let dom = make_inst("MyModule", "ModuleScript");
        let child_ref = dom.root().children()[0];
        let child = dom.get_by_ref(child_ref).unwrap();
        let taken = HashSet::new();

        let (filename, needs_meta, _dk) =
            name_for_inst(Middleware::ModuleScript, child, None, &taken).unwrap();
        assert_eq!(filename.as_ref(), "MyModule.luau");
        assert!(!needs_meta);
    }

    #[test]
    fn name_for_inst_clean_name_dir_middleware() {
        let dom = make_inst("MyFolder", "Folder");
        let child_ref = dom.root().children()[0];
        let child = dom.get_by_ref(child_ref).unwrap();
        let taken = HashSet::new();

        let (filename, needs_meta, _dk) =
            name_for_inst(Middleware::Dir, child, None, &taken).unwrap();
        assert_eq!(filename.as_ref(), "MyFolder");
        assert!(!needs_meta);
    }

    #[test]
    fn name_for_inst_forbidden_chars_slugified() {
        let dom = make_inst("Hey/Bro", "ModuleScript");
        let child_ref = dom.root().children()[0];
        let child = dom.get_by_ref(child_ref).unwrap();
        let taken = HashSet::new();

        let (filename, needs_meta, _dk) =
            name_for_inst(Middleware::ModuleScript, child, None, &taken).unwrap();
        assert_eq!(filename.as_ref(), "Hey_Bro.luau");
        assert!(needs_meta, "slug differs from real name, needs meta");
    }

    #[test]
    fn name_for_inst_forbidden_chars_dir_middleware() {
        let dom = make_inst("Hey:Bro", "Folder");
        let child_ref = dom.root().children()[0];
        let child = dom.get_by_ref(child_ref).unwrap();
        let taken = HashSet::new();

        let (filename, needs_meta, _dk) =
            name_for_inst(Middleware::Dir, child, None, &taken).unwrap();
        assert_eq!(filename.as_ref(), "Hey_Bro");
        assert!(needs_meta);
    }

    #[test]
    fn name_for_inst_dedup_collision() {
        let dom = make_inst("Foo", "ModuleScript");
        let child_ref = dom.root().children()[0];
        let child = dom.get_by_ref(child_ref).unwrap();
        let taken: HashSet<String> = ["foo".to_string()].into_iter().collect();

        let (filename, needs_meta, _dk) =
            name_for_inst(Middleware::ModuleScript, child, None, &taken).unwrap();
        assert_eq!(filename.as_ref(), "Foo~1.luau");
        assert!(needs_meta, "deduped name differs from original");
    }

    #[test]
    fn name_for_inst_dedup_dir_collision() {
        let dom = make_inst("Stuff", "Folder");
        let child_ref = dom.root().children()[0];
        let child = dom.get_by_ref(child_ref).unwrap();
        let taken: HashSet<String> = ["stuff".to_string()].into_iter().collect();

        let (filename, needs_meta, _dk) =
            name_for_inst(Middleware::Dir, child, None, &taken).unwrap();
        assert_eq!(filename.as_ref(), "Stuff~1");
        assert!(needs_meta);
    }

    #[test]
    fn name_for_inst_slug_plus_dedup() {
        // Name with forbidden chars AND collision with existing slug
        let dom = make_inst("Hey/Bro", "ModuleScript");
        let child_ref = dom.root().children()[0];
        let child = dom.get_by_ref(child_ref).unwrap();
        let taken: HashSet<String> = ["hey_bro".to_string()].into_iter().collect();

        let (filename, needs_meta, _dk) =
            name_for_inst(Middleware::ModuleScript, child, None, &taken).unwrap();
        assert_eq!(filename.as_ref(), "Hey_Bro~1.luau");
        assert!(needs_meta);
    }

    #[test]
    fn name_for_inst_server_script_extension() {
        let dom = make_inst("Main", "Script");
        let child_ref = dom.root().children()[0];
        let child = dom.get_by_ref(child_ref).unwrap();
        let taken = HashSet::new();

        let (filename, needs_meta, _dk) =
            name_for_inst(Middleware::ServerScript, child, None, &taken).unwrap();
        assert_eq!(filename.as_ref(), "Main.server.luau");
        assert!(!needs_meta);
    }

    #[test]
    fn name_for_inst_client_script_extension() {
        let dom = make_inst("Client", "LocalScript");
        let child_ref = dom.root().children()[0];
        let child = dom.get_by_ref(child_ref).unwrap();
        let taken = HashSet::new();

        let (filename, needs_meta, _dk) =
            name_for_inst(Middleware::ClientScript, child, None, &taken).unwrap();
        assert_eq!(filename.as_ref(), "Client.client.luau");
        assert!(!needs_meta);
    }

    #[test]
    fn name_for_inst_text_extension() {
        let dom = make_inst("Readme", "StringValue");
        let child_ref = dom.root().children()[0];
        let child = dom.get_by_ref(child_ref).unwrap();
        let taken = HashSet::new();

        let (filename, needs_meta, _dk) =
            name_for_inst(Middleware::Text, child, None, &taken).unwrap();
        assert_eq!(filename.as_ref(), "Readme.txt");
        assert!(!needs_meta);
    }

    // ══════════════════════════════════════════════════════════════════
    //  Comprehensive slugified-name + dedup integration tests
    //
    //  These simulate the full name_for_inst pipeline: slugify decision,
    //  dedup against taken_names, filename construction, and needs_meta
    //  flag correctness. Every edge case, ordering, and middleware
    //  combination the two-way sync could encounter.
    // ══════════════════════════════════════════════════════════════════

    /// Helper: run name_for_inst with a given name, middleware, and taken set.
    /// Returns (filename_string, needs_meta).
    fn nfi(name: &str, mw: Middleware, taken: &[&str]) -> (String, bool) {
        let dom = make_inst(name, "ModuleScript");
        let child_ref = dom.root().children()[0];
        let child = dom.get_by_ref(child_ref).unwrap();
        let taken_set: HashSet<String> = taken.iter().map(|s| s.to_string()).collect();
        let (filename, needs_meta, _dk) = name_for_inst(mw, child, None, &taken_set).unwrap();
        (filename.into_owned(), needs_meta)
    }

    // ── Dangerous suffix slugification through name_for_inst ─────────

    #[test]
    fn nfi_dangerous_suffix_server_luau() {
        let (f, m) = nfi("foo.server", Middleware::ModuleScript, &[]);
        assert_eq!(f, "foo_server.luau");
        assert!(m, "dangerous suffix must trigger needs_meta");
    }

    #[test]
    fn nfi_dangerous_suffix_client_luau() {
        let (f, m) = nfi("bar.client", Middleware::ClientScript, &[]);
        assert_eq!(f, "bar_client.client.luau");
        assert!(m);
    }

    #[test]
    fn nfi_dangerous_suffix_meta_dir() {
        let (f, m) = nfi("Config.meta", Middleware::Dir, &[]);
        assert_eq!(f, "Config_meta");
        assert!(m);
    }

    #[test]
    fn nfi_dangerous_suffix_model_json_model() {
        let (f, m) = nfi("Part.model", Middleware::JsonModel, &[]);
        assert_eq!(f, "Part_model.model.json5");
        assert!(m);
    }

    #[test]
    fn nfi_dangerous_suffix_project() {
        let (f, m) = nfi("Game.project", Middleware::ModuleScript, &[]);
        assert_eq!(f, "Game_project.luau");
        assert!(m);
    }

    #[test]
    #[allow(non_snake_case)]
    fn nfi_dangerous_suffix_case_insensitive_SERVER() {
        let (f, m) = nfi("test.SERVER", Middleware::ModuleScript, &[]);
        assert_eq!(f, "test_SERVER.luau");
        assert!(m);
    }

    #[test]
    #[allow(non_snake_case)]
    fn nfi_dangerous_suffix_case_insensitive_Meta() {
        let (f, m) = nfi("stuff.Meta", Middleware::Dir, &[]);
        assert_eq!(f, "stuff_Meta");
        assert!(m);
    }

    // ── Safe dots pass through ───────────────────────────────────────

    #[test]
    fn nfi_safe_dot_v1_0() {
        let (f, m) = nfi("v1.0", Middleware::ModuleScript, &[]);
        assert_eq!(f, "v1.0.luau");
        assert!(!m);
    }

    #[test]
    fn nfi_safe_dot_hello_world() {
        let (f, m) = nfi("hello.world", Middleware::ModuleScript, &[]);
        assert_eq!(f, "hello.world.luau");
        assert!(!m);
    }

    #[test]
    fn nfi_safe_dot_dir() {
        let (f, m) = nfi("Release.1", Middleware::Dir, &[]);
        assert_eq!(f, "Release.1");
        assert!(!m);
    }

    // ── Spaces in names ──────────────────────────────────────────────

    #[test]
    fn nfi_middle_space_preserved() {
        let (f, m) = nfi("My Module", Middleware::ModuleScript, &[]);
        assert_eq!(f, "My Module.luau");
        assert!(!m, "middle space is valid, no meta needed");
    }

    #[test]
    fn nfi_leading_space_slugified() {
        let (f, m) = nfi(" Leading", Middleware::ModuleScript, &[]);
        assert_eq!(f, "Leading.luau");
        assert!(m, "leading space stripped → slug differs");
    }

    #[test]
    fn nfi_trailing_space_slugified() {
        let (f, m) = nfi("Trailing ", Middleware::ModuleScript, &[]);
        assert_eq!(f, "Trailing.luau");
        assert!(m);
    }

    // ── Tilde in names ───────────────────────────────────────────────

    #[test]
    fn nfi_tilde_in_name_slugified() {
        let (f, m) = nfi("Data~1", Middleware::ModuleScript, &[]);
        assert_eq!(f, "Data_1.luau");
        assert!(m);
    }

    #[test]
    fn nfi_tilde_in_dir_name() {
        let (f, m) = nfi("Cache~old", Middleware::Dir, &[]);
        assert_eq!(f, "Cache_old");
        assert!(m);
    }

    // ── OS-forbidden chars ───────────────────────────────────────────

    #[test]
    fn nfi_slash_in_name() {
        let (f, m) = nfi("src/main", Middleware::ModuleScript, &[]);
        assert_eq!(f, "src_main.luau");
        assert!(m);
    }

    #[test]
    fn nfi_colon_in_name() {
        let (f, m) = nfi("Drive:C", Middleware::Dir, &[]);
        assert_eq!(f, "Drive_C");
        assert!(m);
    }

    #[test]
    fn nfi_question_mark() {
        let (f, m) = nfi("What?", Middleware::ServerScript, &[]);
        assert_eq!(f, "What_.server.luau");
        assert!(m);
    }

    #[test]
    fn nfi_asterisk() {
        let (f, m) = nfi("glob*", Middleware::ModuleScript, &[]);
        assert_eq!(f, "glob_.luau");
        assert!(m);
    }

    #[test]
    fn nfi_angle_brackets() {
        let (f, m) = nfi("<init>", Middleware::ModuleScript, &[]);
        assert_eq!(f, "_init_.luau");
        assert!(m);
    }

    #[test]
    fn nfi_pipe() {
        let (f, m) = nfi("A|B", Middleware::Dir, &[]);
        assert_eq!(f, "A_B");
        assert!(m);
    }

    #[test]
    fn nfi_backslash() {
        let (f, m) = nfi("path\\to", Middleware::ModuleScript, &[]);
        assert_eq!(f, "path_to.luau");
        assert!(m);
    }

    #[test]
    fn nfi_double_quote() {
        let (f, m) = nfi("say\"hi\"", Middleware::ModuleScript, &[]);
        assert_eq!(f, "say_hi_.luau");
        assert!(m);
    }

    // ── Windows reserved names ───────────────────────────────────────

    #[test]
    fn nfi_con() {
        let (f, m) = nfi("CON", Middleware::ModuleScript, &[]);
        assert_eq!(f, "CON_.luau");
        assert!(m);
    }

    #[test]
    fn nfi_nul_dir() {
        let (f, m) = nfi("NUL", Middleware::Dir, &[]);
        assert_eq!(f, "NUL_");
        assert!(m);
    }

    #[test]
    fn nfi_com1_csv() {
        let (f, m) = nfi("COM1", Middleware::Csv, &[]);
        assert_eq!(f, "COM1_.csv");
        assert!(m);
    }

    // ── Empty / fallback names ───────────────────────────────────────

    #[test]
    fn nfi_empty_name() {
        let (f, m) = nfi("", Middleware::ModuleScript, &[]);
        assert_eq!(f, "instance.luau");
        assert!(m);
    }

    #[test]
    fn nfi_all_forbidden() {
        let (f, m) = nfi("<>:\"/\\|?*", Middleware::Dir, &[]);
        assert_eq!(f, "instance");
        assert!(m);
    }

    // ── Dedup with slugified names (collision scenarios) ─────────────

    // NOTE: deduplicate_name compares the bare slug against taken_names.
    // In real syncback (dir.rs), taken_names stores full filenames, so file-
    // level dedup only works for directory middleware where filename == slug.
    // For file middleware, the duplicate-name pre-filter handles collisions.
    // These tests use bare slugs in taken_names to verify the dedup logic.

    #[test]
    fn nfi_slug_collides_with_existing_dir() {
        // "Hey/Bro" slugifies to "Hey_Bro", which is already taken as a dir name
        let (f, m) = nfi("Hey/Bro", Middleware::Dir, &["hey_bro"]);
        assert_eq!(f, "Hey_Bro~1");
        assert!(m);
    }

    #[test]
    fn nfi_slug_collides_with_natural_dir() {
        // Dir "Hey_Bro" exists naturally, then "Hey:Bro" slugifies to same
        let (f, m) = nfi("Hey:Bro", Middleware::Dir, &["hey_bro"]);
        assert_eq!(f, "Hey_Bro~1");
        assert!(m);
    }

    #[test]
    fn nfi_two_slugs_collide_dir() {
        // Both "A/B" and "A:B" slugify to "A_B". First taken, second gets ~1
        let (f, m) = nfi("A:B", Middleware::Dir, &["a_b"]);
        assert_eq!(f, "A_B~1");
        assert!(m);
    }

    #[test]
    fn nfi_three_way_collision_dir() {
        let (f, _) = nfi("X/Y", Middleware::Dir, &["x_y", "x_y~1"]);
        assert_eq!(f, "X_Y~2");
    }

    #[test]
    fn nfi_clean_name_dedup_still_needs_meta_dir() {
        // Clean name "Foo", but "foo" is already taken → dedup adds ~1 → needs meta
        let (f, m) = nfi("Foo", Middleware::Dir, &["foo"]);
        assert_eq!(f, "Foo~1");
        assert!(
            m,
            "dedup suffix means filesystem name differs from instance name"
        );
    }

    #[test]
    fn nfi_case_collision_dir() {
        let (f, m) = nfi("Assets", Middleware::Dir, &["assets"]);
        assert_eq!(f, "Assets~1");
        assert!(m);
    }

    #[test]
    fn nfi_slug_collision_file_middleware() {
        // For file middleware, dedup compares bare slug against taken.
        // Using bare names to verify the dedup logic works.
        let (f, m) = nfi("Hey/Bro", Middleware::ModuleScript, &["hey_bro"]);
        assert_eq!(f, "Hey_Bro~1.luau");
        assert!(m);
    }

    // ── Ordering simulation: multiple siblings processed sequentially ─

    /// Simulates processing N siblings in order, accumulating taken_names.
    /// Uses Dir middleware for all to test dedup correctly (dir filenames
    /// ARE the bare slug, matching what deduplicate_name compares against).
    fn process_siblings_dir(siblings: &[&str]) -> Vec<(String, String, bool)> {
        let mut taken: HashSet<String> = HashSet::new();
        let mut results = Vec::new();
        for &name in siblings {
            let dom = make_inst(name, "Folder");
            let child_ref = dom.root().children()[0];
            let child = dom.get_by_ref(child_ref).unwrap();
            let (filename, needs_meta, dedup_key) =
                name_for_inst(Middleware::Dir, child, None, &taken).unwrap();
            taken.insert(dedup_key.to_lowercase());
            results.push((name.to_string(), filename.into_owned(), needs_meta));
        }
        results
    }

    #[test]
    fn ordering_two_identical_names() {
        let r = process_siblings_dir(&["Foo", "Foo"]);
        assert_eq!(r[0].1, "Foo");
        assert!(!r[0].2);
        assert_eq!(r[1].1, "Foo~1");
        assert!(r[1].2);
    }

    #[test]
    fn ordering_three_identical_names() {
        let r = process_siblings_dir(&["X", "X", "X"]);
        assert_eq!(r[0].1, "X");
        assert_eq!(r[1].1, "X~1");
        assert_eq!(r[2].1, "X~2");
    }

    #[test]
    fn ordering_natural_then_slugified_collision() {
        // "Hey_Bro" goes first (natural), then "Hey/Bro" slugifies to same
        let r = process_siblings_dir(&["Hey_Bro", "Hey/Bro"]);
        assert_eq!(r[0].1, "Hey_Bro");
        assert!(!r[0].2);
        assert_eq!(r[1].1, "Hey_Bro~1");
        assert!(r[1].2);
    }

    #[test]
    fn ordering_slugified_then_natural_collision() {
        // "Hey/Bro" goes first (slugified to "Hey_Bro"), then natural "Hey_Bro"
        let r = process_siblings_dir(&["Hey/Bro", "Hey_Bro"]);
        assert_eq!(r[0].1, "Hey_Bro");
        assert!(r[0].2);
        assert_eq!(r[1].1, "Hey_Bro~1");
        assert!(r[1].2, "dedup suffix → needs meta even for natural name");
    }

    #[test]
    fn ordering_case_variants() {
        let r = process_siblings_dir(&["Script", "script", "SCRIPT"]);
        assert_eq!(r[0].1, "Script");
        assert!(!r[0].2);
        assert_eq!(r[1].1, "script~1");
        assert!(r[1].2);
        assert_eq!(r[2].1, "SCRIPT~2");
        assert!(r[2].2);
    }

    #[test]
    fn ordering_mixed_forbidden_chars_same_slug() {
        // All three slugify to "A_B"
        let r = process_siblings_dir(&["A/B", "A:B", "A*B"]);
        assert_eq!(r[0].1, "A_B");
        assert_eq!(r[1].1, "A_B~1");
        assert_eq!(r[2].1, "A_B~2");
        assert!(r[0].2);
        assert!(r[1].2);
        assert!(r[2].2);
    }

    #[test]
    fn ordering_dangerous_suffix_then_safe() {
        // "test.server" slugifies to "test_server", "test" is clean
        let r = process_siblings_dir(&["test.server", "test"]);
        assert_eq!(r[0].1, "test_server");
        assert!(r[0].2);
        assert_eq!(r[1].1, "test");
        assert!(!r[1].2);
    }

    #[test]
    fn ordering_windows_reserved_then_normal() {
        // "CON" slugifies to "CON_", then natural "CON_" collides
        let r = process_siblings_dir(&["CON", "CON_"]);
        assert_eq!(r[0].1, "CON_");
        assert!(r[0].2);
        assert_eq!(r[1].1, "CON_~1");
        assert!(r[1].2);
    }

    #[test]
    fn ordering_five_siblings_complex() {
        let r = process_siblings_dir(&[
            "Utils",  // first claim
            "utils",  // case collision with Utils
            "Utils",  // exact collision (already taken + ~1 taken)
            "UTILS",  // case collision
            "Uti/ls", // different slug "Uti_ls", no collision
        ]);
        assert_eq!(r[0].1, "Utils");
        assert!(!r[0].2);
        assert_eq!(r[1].1, "utils~1");
        assert!(r[1].2);
        assert_eq!(r[2].1, "Utils~2");
        assert!(r[2].2);
        assert_eq!(r[3].1, "UTILS~3");
        assert!(r[3].2);
        assert_eq!(r[4].1, "Uti_ls");
        assert!(r[4].2);
    }

    #[test]
    fn ordering_six_way_slug_pileup() {
        // All six slugify to "X_Y" as dirs
        let r = process_siblings_dir(&["X/Y", "X:Y", "X*Y", "X?Y", "X<Y", "X>Y"]);
        assert_eq!(r[0].1, "X_Y");
        assert_eq!(r[1].1, "X_Y~1");
        assert_eq!(r[2].1, "X_Y~2");
        assert_eq!(r[3].1, "X_Y~3");
        assert_eq!(r[4].1, "X_Y~4");
        assert_eq!(r[5].1, "X_Y~5");
    }

    #[test]
    fn ordering_tilde_in_name_vs_dedup_suffix() {
        // "Foo~1" (tilde slugified to "Foo_1") vs dedup suffix "Foo~1"
        // The slugified "Foo~1" → "Foo_1" which is different from dedup "Foo~1"
        let r = process_siblings_dir(&["Foo", "Foo", "Foo~1"]);
        assert_eq!(r[0].1, "Foo");
        assert_eq!(r[1].1, "Foo~1"); // dedup suffix
        assert_eq!(r[2].1, "Foo_1"); // tilde was slugified to _
        assert!(r[2].2);
    }

    #[test]
    fn ordering_spaces_and_collisions() {
        let r = process_siblings_dir(&[
            " Leading", // leading space stripped → "Leading"
            "Leading",  // collides with stripped version
            "Leading ", // trailing space stripped → "Leading" → collides
        ]);
        assert_eq!(r[0].1, "Leading");
        assert!(r[0].2, "leading space was stripped");
        assert_eq!(r[1].1, "Leading~1");
        assert!(r[1].2);
        assert_eq!(r[2].1, "Leading~2");
        assert!(r[2].2);
    }

    // ── Every middleware type with slugified name ─────────────────────

    #[test]
    fn nfi_every_middleware_with_slash() {
        let cases: &[(&str, Middleware, &str)] = &[
            ("a/b", Middleware::ModuleScript, "a_b.luau"),
            ("a/b", Middleware::ServerScript, "a_b.server.luau"),
            ("a/b", Middleware::ClientScript, "a_b.client.luau"),
            ("a/b", Middleware::PluginScript, "a_b.plugin.luau"),
            ("a/b", Middleware::LocalScript, "a_b.local.luau"),
            ("a/b", Middleware::LegacyScript, "a_b.legacy.luau"),
            ("a/b", Middleware::Csv, "a_b.csv"),
            ("a/b", Middleware::JsonModel, "a_b.model.json5"),
            ("a/b", Middleware::Text, "a_b.txt"),
            ("a/b", Middleware::Rbxm, "a_b.rbxm"),
            ("a/b", Middleware::Rbxmx, "a_b.rbxmx"),
            ("a/b", Middleware::Toml, "a_b.toml"),
            ("a/b", Middleware::Yaml, "a_b.yml"),
            ("a/b", Middleware::Dir, "a_b"),
            ("a/b", Middleware::ServerScriptDir, "a_b"),
            ("a/b", Middleware::ClientScriptDir, "a_b"),
            ("a/b", Middleware::ModuleScriptDir, "a_b"),
        ];
        for &(name, mw, expected) in cases {
            let (f, m) = nfi(name, mw, &[]);
            assert_eq!(f, expected, "name={name:?} mw={mw:?}");
            assert!(m, "name={name:?} mw={mw:?} should need meta");
        }
    }

    // ── Stress: validate_file_name accepts every slugify output ──────

    #[test]
    fn slugify_output_always_valid_filesystem_name() {
        let long_name = "A".repeat(255);
        let corpus = [
            "",
            " ",
            "  ",
            ".",
            "..",
            "...",
            ". .",
            " . ",
            "/",
            "\\",
            "*",
            "?",
            "<",
            ">",
            "|",
            ":",
            "\"",
            "~",
            "~1",
            "~99",
            "CON",
            "PRN",
            "AUX",
            "NUL",
            "COM1",
            "COM9",
            "LPT1",
            "LPT9",
            "con",
            "prn",
            "aux",
            "nul",
            "com1",
            "foo.server",
            "foo.client",
            "foo.plugin",
            "foo.local",
            "foo.legacy",
            "foo.meta",
            "foo.model",
            "foo.project",
            "foo.Server",
            "foo.CLIENT",
            "foo.Meta",
            "a.meta.server",
            "x.model.project.client",
            "hello world",
            " hello",
            "hello ",
            " hello ",
            "日本語",
            "café",
            "émoji🎮",
            "v1.0",
            "foo.bar",
            "a.b.c.d",
            "<>:\"/\\|?*~. ",
            long_name.as_str(),
            "\x00",
            "\t",
            "\n",
            "\r",
        ];
        for input in corpus {
            let slug = slugify_name(input);
            assert!(
                validate_file_name(&slug).is_ok(),
                "slugify_name({input:?}) = {slug:?} failed validate_file_name"
            );
            assert!(
                !slug.is_empty(),
                "slugify_name({input:?}) should never be empty"
            );
        }
    }

    #[test]
    fn name_needs_slugify_implies_slug_differs() {
        // If name_needs_slugify returns true, slugify_name MUST return
        // something different from the input (otherwise we'd set needs_meta
        // but the name and slug would be the same — pointless meta field).
        let corpus = [
            "CON",
            "a/b",
            "foo~1",
            "foo.server",
            " leading",
            "trailing ",
            "hello.",
            "",
            "\t",
            "COM1",
        ];
        for input in corpus {
            if name_needs_slugify(input) {
                let slug = slugify_name(input);
                assert_ne!(
                    slug, input,
                    "name_needs_slugify({input:?})=true but slugify produced identical output"
                );
            }
        }
    }

    #[test]
    fn name_needs_slugify_false_means_slug_is_identity() {
        // If name_needs_slugify returns false, slugify_name should return
        // the input unchanged (no unnecessary transformation).
        let corpus = [
            "Hello",
            "my_module",
            "test-123",
            "v1.0",
            "foo.bar",
            "hello world",
            "日本語",
            "café",
        ];
        for input in corpus {
            assert!(
                !name_needs_slugify(input),
                "{input:?} should not need slugify"
            );
            assert_eq!(
                slugify_name(input),
                input,
                "slugify_name should be identity for {input:?}"
            );
        }
    }

    // ══════════════════════════════════════════════════════════════════
    //  DISK-SEEDED TAKEN NAMES TESTS
    //
    //  These simulate the fix where taken_names is pre-seeded from
    //  existing directory contents. Before the fix, taken_names was
    //  empty and new instances could overwrite existing files.
    //
    //  Every test in this section would have FAILED before the fix.
    // ══════════════════════════════════════════════════════════════════

    /// Simulates dir.rs behavior AFTER fix #3: seed taken_names from
    /// "on-disk" entries, then process new children sequentially.
    fn process_new_children_with_disk_seed(
        disk_entries: &[&str],
        new_children: &[(&str, Middleware)],
    ) -> Vec<(String, String, bool)> {
        // Seed from bare slugs (simulates tree-based seeding)
        let mut taken: HashSet<String> = disk_entries.iter().map(|e| e.to_lowercase()).collect();
        let mut results = Vec::new();
        for &(name, mw) in new_children {
            let dom = make_inst(name, "ModuleScript");
            let child_ref = dom.root().children()[0];
            let child = dom.get_by_ref(child_ref).unwrap();
            let (filename, needs_meta, dedup_key) = name_for_inst(mw, child, None, &taken).unwrap();
            taken.insert(dedup_key.to_lowercase());
            results.push((name.to_string(), filename.into_owned(), needs_meta));
        }
        results
    }

    #[test]
    fn disk_seed_prevents_overwrite_of_existing_dir() {
        // "Utils" directory exists on disk. New instance "Utils" (dir)
        // must get "Utils~1".
        let r = process_new_children_with_disk_seed(&["utils"], &[("Utils", Middleware::Dir)]);
        assert_eq!(r[0].1, "Utils~1");
        assert!(r[0].2);
    }

    #[test]
    fn disk_seed_prevents_overwrite_of_existing_dir_case_insensitive() {
        // "SCRIPTS" directory on disk, new instance "Scripts" (dir) must dedup.
        let r = process_new_children_with_disk_seed(&["scripts"], &[("Scripts", Middleware::Dir)]);
        assert_eq!(r[0].1, "Scripts~1");
        assert!(r[0].2);
    }

    #[test]
    fn disk_seed_slug_collision_with_existing_dir() {
        // "Hey_Bro" directory on disk. New instance "Hey/Bro" slugifies
        // to "Hey_Bro" — must dedup to "Hey_Bro~1".
        let r = process_new_children_with_disk_seed(&["hey_bro"], &[("Hey/Bro", Middleware::Dir)]);
        assert_eq!(r[0].1, "Hey_Bro~1");
        assert!(r[0].2);
    }

    #[test]
    fn disk_seed_multiple_existing_dirs() {
        // Directory has: Utils/, Config/, Shared/
        // New dir children: Utils (collision!), Config (collision!), NewThing (clean)
        let r = process_new_children_with_disk_seed(
            &["utils", "config", "shared"],
            &[
                ("Utils", Middleware::Dir),
                ("Config", Middleware::Dir),
                ("NewThing", Middleware::Dir),
            ],
        );
        assert_eq!(r[0].1, "Utils~1", "Utils collides with utils/ on disk");
        assert!(r[0].2);
        assert_eq!(r[1].1, "Config~1", "Config collides with config/ on disk");
        assert!(r[1].2);
        assert_eq!(r[2].1, "NewThing", "NewThing is clean");
        assert!(!r[2].2);
    }

    #[test]
    fn disk_seed_existing_dir_plus_meta_and_files() {
        // Directory contains: Stuff/ (dir), Stuff.meta.json5, Other.luau
        // New "Stuff" dir must dedup against "stuff" (the dir).
        let r = process_new_children_with_disk_seed(
            &["stuff", "stuff.meta.json5", "other.luau"],
            &[("Stuff", Middleware::Dir)],
        );
        assert_eq!(r[0].1, "Stuff~1");
        assert!(r[0].2);
    }

    #[test]
    fn disk_seed_file_middleware_collides_with_bare_slug() {
        // taken_names now uses bare slugs (dedup_keys), so file-format
        // dedup works correctly. "mymodule" in taken matches slug "MyModule".
        let r = process_new_children_with_disk_seed(
            &["mymodule"],
            &[("MyModule", Middleware::ModuleScript)],
        );
        assert_eq!(r[0].1, "MyModule~1.luau");
        assert!(r[0].2, "dedup suffix means needs meta");
    }

    #[test]
    fn disk_seed_file_middleware_sibling_slug_collision_deduped() {
        // Two file-middleware children whose names slugify to the same base.
        // Since taken_names accumulates dedup_keys (bare slugs), the second
        // child correctly detects the collision and gets ~1.
        let r = process_new_children_with_disk_seed(
            &[],
            &[
                ("A/B", Middleware::ModuleScript),
                ("A:B", Middleware::ModuleScript),
            ],
        );
        assert_eq!(r[0].1, "A_B.luau");
        assert!(r[0].2);
        assert_eq!(r[1].1, "A_B~1.luau");
        assert!(r[1].2);
    }

    #[test]
    fn disk_seed_gitkeep_doesnt_interfere() {
        // .gitkeep files are common in empty dirs. New "gitkeep" instance
        // shouldn't collide (hidden files start with dot, regular names don't).
        // But ".gitkeep" lowered is ".gitkeep" — if we have a bizarre instance
        // named ".gitkeep", it won't be slugified (no forbidden chars) but would
        // collide. Testing that the seeding doesn't break normal flow.
        let r = process_new_children_with_disk_seed(
            &[".gitkeep"],
            &[("MyModule", Middleware::ModuleScript)],
        );
        assert_eq!(r[0].1, "MyModule.luau");
        assert!(!r[0].2, "no collision with .gitkeep");
    }

    #[test]
    fn disk_seed_init_file_doesnt_prevent_new_siblings() {
        // Parent dir has "init.luau". New children should work fine.
        let r = process_new_children_with_disk_seed(
            &["init.luau"],
            &[
                ("Child1", Middleware::ModuleScript),
                ("Child2", Middleware::ServerScript),
            ],
        );
        assert_eq!(r[0].1, "Child1.luau");
        assert!(!r[0].2);
        assert_eq!(r[1].1, "Child2.server.luau");
        assert!(!r[1].2);
    }

    #[test]
    fn disk_seed_dedup_suffix_dir_already_on_disk() {
        // Pathological: "Foo" AND "Foo~1" directories already on disk.
        // New "Foo" dir must skip both and land on "Foo~2".
        let r = process_new_children_with_disk_seed(&["foo", "foo~1"], &[("Foo", Middleware::Dir)]);
        assert_eq!(r[0].1, "Foo~2");
        assert!(r[0].2);
    }

    #[test]
    fn disk_seed_dedup_chain_on_disk() {
        // "A_B", "A_B~1", "A_B~2" already on disk. New "A/B" (slug "A_B")
        // must land on "A_B~3".
        let r = process_new_children_with_disk_seed(
            &["a_b", "a_b~1", "a_b~2"],
            &[("A/B", Middleware::Dir)],
        );
        assert_eq!(r[0].1, "A_B~3");
        assert!(r[0].2);
    }

    // ══════════════════════════════════════════════════════════════════
    //  SIBLING DEDUP TESTS (project.rs fix #2 simulation)
    //
    //  These simulate the fix where siblings under the same parent
    //  share a single taken_names set. Before the fix, each sibling
    //  got an independent empty HashSet and could collide.
    //
    //  Every test in this section would have FAILED before the fix.
    // ══════════════════════════════════════════════════════════════════

    /// Simulates project.rs behavior AFTER fix #2: shared taken_names
    /// across siblings, seeded from disk, accumulated per-child.
    fn process_project_siblings(
        disk_entries: &[&str],
        siblings: &[(&str, Middleware)],
    ) -> Vec<(String, String, bool)> {
        let mut taken: HashSet<String> = disk_entries.iter().map(|e| e.to_lowercase()).collect();
        let mut results = Vec::new();
        for &(name, mw) in siblings {
            let dom = make_inst(name, "ModuleScript");
            let child_ref = dom.root().children()[0];
            let child = dom.get_by_ref(child_ref).unwrap();
            let (filename, needs_meta, dedup_key) = name_for_inst(mw, child, None, &taken).unwrap();
            taken.insert(dedup_key.to_lowercase());
            results.push((name.to_string(), filename.into_owned(), needs_meta));
        }
        results
    }

    #[test]
    fn project_siblings_same_slug_different_names() {
        // Two new project children: "A/B" and "A:B" both slug to "A_B".
        // Before fix: both got "A_B" path (collision!).
        // After fix: second gets "A_B~1".
        let r =
            process_project_siblings(&[], &[("A/B", Middleware::Dir), ("A:B", Middleware::Dir)]);
        assert_eq!(r[0].1, "A_B");
        assert!(r[0].2);
        assert_eq!(r[1].1, "A_B~1");
        assert!(r[1].2);
    }

    #[test]
    fn project_siblings_natural_vs_slug_collision_dir() {
        // "Hey_Bro" (natural) and "Hey/Bro" (slug "Hey_Bro") as dir children.
        // Dir middleware: bare slug IS the filename → dedup works.
        let r = process_project_siblings(
            &[],
            &[("Hey_Bro", Middleware::Dir), ("Hey/Bro", Middleware::Dir)],
        );
        assert_eq!(r[0].1, "Hey_Bro");
        assert!(!r[0].2);
        assert_eq!(r[1].1, "Hey_Bro~1");
        assert!(r[1].2);
    }

    #[test]
    fn project_siblings_three_way_slug_collision() {
        // Three siblings all slug to "X_Y" — must get X_Y, X_Y~1, X_Y~2.
        let r = process_project_siblings(
            &[],
            &[
                ("X/Y", Middleware::Dir),
                ("X:Y", Middleware::Dir),
                ("X*Y", Middleware::Dir),
            ],
        );
        assert_eq!(r[0].1, "X_Y");
        assert_eq!(r[1].1, "X_Y~1");
        assert_eq!(r[2].1, "X_Y~2");
    }

    #[test]
    fn project_siblings_disk_plus_siblings_collision_dir() {
        // "Helper" directory already on disk. Two new dir siblings: "Helper" x2.
        // First → "Helper~1" (disk has "helper").
        // Second → "Helper~2" (both disk and first sibling taken).
        let r = process_project_siblings(
            &["helper"],
            &[("Helper", Middleware::Dir), ("Helper", Middleware::Dir)],
        );
        assert_eq!(r[0].1, "Helper~1");
        assert!(r[0].2);
        assert_eq!(r[1].1, "Helper~2");
        assert!(r[1].2);
    }

    #[test]
    fn project_siblings_case_collision_chain() {
        // Case variants: "Test", "test", "TEST", "tEsT" — all collide case-insensitively.
        let r = process_project_siblings(
            &[],
            &[
                ("Test", Middleware::Dir),
                ("test", Middleware::Dir),
                ("TEST", Middleware::Dir),
                ("tEsT", Middleware::Dir),
            ],
        );
        assert_eq!(r[0].1, "Test");
        assert!(!r[0].2);
        assert_eq!(r[1].1, "test~1");
        assert!(r[1].2);
        assert_eq!(r[2].1, "TEST~2");
        assert!(r[2].2);
        assert_eq!(r[3].1, "tEsT~3");
        assert!(r[3].2);
    }

    // ══════════════════════════════════════════════════════════════════
    //  NIGHTMARE EDGE CASES
    //
    //  Tests that combine multiple transformation layers in ways that
    //  should make your eyes water. Each one targets a specific
    //  interaction between slugify, dedup, dangerous suffixes, case
    //  folding, Windows reserved names, and unicode.
    // ══════════════════════════════════════════════════════════════════

    #[test]
    fn nightmare_con_slash_prn_sibling_collision() {
        // "CON" → "CON_", "PRN" → "PRN_" — different slugs, no collision.
        // But "CON/" → "CON_" collides with "CON" → "CON_".
        let r = process_siblings_dir(&["CON", "CON/"]);
        assert_eq!(r[0].1, "CON_");
        assert!(r[0].2);
        assert_eq!(r[1].1, "CON_~1");
        assert!(r[1].2);
    }

    #[test]
    fn nightmare_dangerous_suffix_cascade_collision() {
        // "foo.server" → "foo_server", "foo/server" → "foo_server" — collision!
        let r = process_siblings_dir(&["foo.server", "foo/server"]);
        assert_eq!(r[0].1, "foo_server");
        assert!(r[0].2);
        assert_eq!(r[1].1, "foo_server~1");
        assert!(r[1].2);
    }

    #[test]
    fn nightmare_tilde_looks_like_dedup_but_gets_slugified() {
        // "Foo~1" has tilde slugified to "Foo_1". Then real "Foo" gets
        // dedup "Foo~1". These should NOT collide because tilde-in-name
        // becomes underscore, but dedup adds actual tilde.
        let r = process_siblings_dir(&["Foo~1", "Foo", "Foo"]);
        assert_eq!(r[0].1, "Foo_1", "tilde slugified");
        assert_eq!(r[1].1, "Foo", "clean, no collision");
        assert_eq!(r[2].1, "Foo~1", "dedup suffix, different from Foo_1");
    }

    #[test]
    fn nightmare_dedup_suffix_avoids_slugified_tilde_name() {
        // Process "Foo", then "Foo" again (dedup to "Foo~1"), then "Foo~1"
        // (tilde slugified to "Foo_1"). The dedup "Foo~1" and slug "Foo_1"
        // should coexist.
        let r = process_siblings_dir(&["Foo", "Foo", "Foo~1"]);
        assert_eq!(r[0].1, "Foo");
        assert_eq!(r[1].1, "Foo~1");
        assert_eq!(r[2].1, "Foo_1");
        // All three have distinct lowercased names
        let names: HashSet<String> = r.iter().map(|(_, f, _)| f.to_lowercase()).collect();
        assert_eq!(names.len(), 3, "all three filenames must be distinct");
    }

    #[test]
    fn nightmare_unicode_plus_forbidden_chars() {
        // "カフェ/Bar" → "カフェ_Bar", "カフェ:Bar" → "カフェ_Bar" — collision
        let r = process_siblings_dir(&["カフェ/Bar", "カフェ:Bar"]);
        assert_eq!(r[0].1, "カフェ_Bar");
        assert_eq!(r[1].1, "カフェ_Bar~1");
    }

    #[test]
    fn nightmare_emoji_name_preserved() {
        // Emoji names are not forbidden chars — they pass through.
        let r = process_siblings_dir(&["🎮 Games", "🎮 Games"]);
        assert_eq!(r[0].1, "🎮 Games");
        assert!(!r[0].2, "emoji name is clean");
        assert_eq!(r[1].1, "🎮 Games~1");
        assert!(r[1].2);
    }

    #[test]
    fn nightmare_windows_reserved_then_slug_that_matches() {
        // "CON" → "CON_", then natural "CON_" collides.
        // Then "con" → "con_" collides case-insensitively with "CON_".
        let r = process_siblings_dir(&["CON", "CON_", "con"]);
        assert_eq!(r[0].1, "CON_");
        assert!(r[0].2);
        assert_eq!(r[1].1, "CON_~1");
        assert!(r[1].2, "natural CON_ collides with reserved CON's slug");
        assert_eq!(r[2].1, "con_~2");
        assert!(r[2].2);
    }

    #[test]
    fn nightmare_trailing_dot_chain() {
        // "test." → strip trailing dot → "test", "test" → "test" collision!
        let r = process_siblings_dir(&["test.", "test"]);
        assert_eq!(r[0].1, "test");
        assert!(r[0].2, "trailing dot stripped → slug differs");
        assert_eq!(r[1].1, "test~1");
        assert!(r[1].2);
    }

    #[test]
    fn nightmare_leading_space_chain() {
        // " A" → "A", "A" → "A" collision, "  A" → "A" triple collision
        let r = process_siblings_dir(&[" A", "A", "  A"]);
        assert_eq!(r[0].1, "A");
        assert!(r[0].2, "leading space stripped");
        assert_eq!(r[1].1, "A~1");
        assert!(r[1].2);
        assert_eq!(r[2].1, "A~2");
        assert!(r[2].2);
    }

    #[test]
    fn nightmare_all_reserved_names_siblings() {
        // Every Windows reserved name as a sibling — all get "_" suffix,
        // and they should NOT collide with each other since the base names differ.
        let reserved = [
            "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7",
            "COM8", "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
        ];
        let r = process_siblings_dir(&reserved);
        let names: HashSet<String> = r.iter().map(|(_, f, _)| f.to_lowercase()).collect();
        assert_eq!(
            names.len(),
            reserved.len(),
            "every reserved name should produce a unique slug"
        );
        for (_, filename, needs_meta) in &r {
            assert!(needs_meta, "{filename} should need meta");
            assert!(
                validate_file_name(filename).is_ok(),
                "{filename} should be valid"
            );
        }
    }

    #[test]
    fn nightmare_dangerous_suffix_every_variant() {
        // All 8 dangerous suffixes as sibling names, each unique slug.
        let siblings: Vec<String> = DANGEROUS_SUFFIXES
            .iter()
            .map(|s| format!("foo{s}"))
            .collect();
        let r = process_siblings_dir(&siblings.iter().map(|s| s.as_str()).collect::<Vec<_>>());
        let filenames: HashSet<String> = r.iter().map(|(_, f, _)| f.to_lowercase()).collect();
        assert_eq!(
            filenames.len(),
            DANGEROUS_SUFFIXES.len(),
            "each dangerous suffix variant should produce a unique filename"
        );
    }

    #[test]
    fn nightmare_mixed_middleware_dir_then_file() {
        // "Shared" as Dir (filename "Shared") then ModuleScript.
        // Dir filename "Shared" → taken gets "shared".
        // ModuleScript dedup checks "shared" → MATCH → dedup to "Shared~1".
        // Then final filename becomes "Shared~1.luau".
        let r = process_new_children_with_disk_seed(
            &[],
            &[
                ("Shared", Middleware::Dir),
                ("Shared", Middleware::ModuleScript),
            ],
        );
        assert_eq!(r[0].1, "Shared");
        assert!(!r[0].2);
        // File middleware bare slug "Shared" matches dir entry "shared" in taken.
        assert_eq!(r[1].1, "Shared~1.luau");
        assert!(r[1].2);
    }

    #[test]
    fn nightmare_mixed_middleware_file_then_dir() {
        // Reverse order: ModuleScript first, Dir second.
        // ModuleScript "Shared" → dedup_key "shared" → taken gets "shared"
        // Dir "Shared" → slug "Shared" → dedup checks "shared" → MATCH
        // (dedup_key-based taken catches this correctly) → "Shared~1"
        let r = process_new_children_with_disk_seed(
            &[],
            &[
                ("Shared", Middleware::ModuleScript),
                ("Shared", Middleware::Dir),
            ],
        );
        assert_eq!(r[0].1, "Shared.luau");
        assert!(!r[0].2);
        // Dir slug "Shared" collides with dedup_key "shared" from the ModuleScript
        assert_eq!(r[1].1, "Shared~1");
        assert!(r[1].2, "dedup suffix means different from instance name");
    }

    #[test]
    fn nightmare_10_way_slug_pileup_with_disk_seed() {
        // "A_B" already on disk. Then 9 new instances all slugifying to "A_B".
        let new_children: Vec<(&str, Middleware)> = vec![
            ("A/B", Middleware::Dir),
            ("A:B", Middleware::Dir),
            ("A*B", Middleware::Dir),
            ("A?B", Middleware::Dir),
            ("A<B", Middleware::Dir),
            ("A>B", Middleware::Dir),
            ("A|B", Middleware::Dir),
            ("A\\B", Middleware::Dir),
            ("A\"B", Middleware::Dir),
        ];
        let r = process_new_children_with_disk_seed(&["a_b"], &new_children);
        for (i, (_, filename, needs_meta)) in r.iter().enumerate() {
            let expected = if i == 0 {
                "A_B~1".to_string()
            } else {
                format!("A_B~{}", i + 1)
            };
            assert_eq!(filename, &expected, "child {i}");
            assert!(needs_meta, "child {i} needs meta");
        }
    }

    #[test]
    fn nightmare_empty_name_siblings() {
        // Multiple instances with empty names. All slugify to "instance".
        let r = process_siblings_dir(&["", "", ""]);
        assert_eq!(r[0].1, "instance");
        assert_eq!(r[1].1, "instance~1");
        assert_eq!(r[2].1, "instance~2");
    }

    #[test]
    fn nightmare_all_forbidden_chars_name_siblings() {
        // Names that are entirely forbidden chars → all become "instance".
        let r = process_siblings_dir(&["<>", ":/", "?*", "|\\", "\"~"]);
        assert_eq!(r[0].1, "instance");
        assert_eq!(r[1].1, "instance~1");
        assert_eq!(r[2].1, "instance~2");
        assert_eq!(r[3].1, "instance~3");
        assert_eq!(r[4].1, "instance~4");
    }

    #[test]
    fn nightmare_control_chars_with_normal_text() {
        // "Hello\x00World" → "Hello_World", "Hello/World" → "Hello_World"
        let r = process_siblings_dir(&["Hello\x00World", "Hello/World"]);
        assert_eq!(r[0].1, "Hello_World");
        assert_eq!(r[1].1, "Hello_World~1");
    }

    #[test]
    fn nightmare_long_dedup_chain_gap_filling() {
        // "Foo", "Foo~1", "Foo~3" on disk (gap at ~2).
        // New "Foo" should fill the gap and get "Foo~2".
        let r = process_new_children_with_disk_seed(
            &["foo", "foo~1", "foo~3"],
            &[("Foo", Middleware::Dir)],
        );
        assert_eq!(r[0].1, "Foo~2", "should fill gap at ~2");
    }

    #[test]
    fn nightmare_meta_suffix_collision_with_dangerous() {
        // "init.meta" → dangerous suffix ".meta" → "init_meta"
        // "init/meta" → slug "init_meta" → collision!
        let r = process_siblings_dir(&["init.meta", "init/meta"]);
        assert_eq!(r[0].1, "init_meta");
        assert_eq!(r[1].1, "init_meta~1");
    }

    #[test]
    fn nightmare_project_suffix_collision_chain() {
        // "a.project" → "a_project", "a/project" → "a_project", "a_project" (natural)
        let r = process_siblings_dir(&["a.project", "a/project", "a_project"]);
        assert_eq!(r[0].1, "a_project");
        assert!(r[0].2);
        assert_eq!(r[1].1, "a_project~1");
        assert!(r[1].2);
        assert_eq!(r[2].1, "a_project~2");
        assert!(r[2].2, "natural name collides with slugified siblings");
    }

    #[test]
    fn nightmare_100_identical_names() {
        // 100 instances all named "Script" — dedup must handle gracefully.
        let names: Vec<&str> = vec!["Script"; 100];
        let r = process_siblings_dir(&names);
        assert_eq!(r[0].1, "Script");
        assert!(!r[0].2);
        for (i, entry) in r.iter().enumerate().skip(1) {
            assert_eq!(entry.1, format!("Script~{i}"));
            assert!(entry.2);
        }
        // All filenames must be unique
        let unique: HashSet<String> = r.iter().map(|(_, f, _)| f.to_lowercase()).collect();
        assert_eq!(unique.len(), 100);
    }

    #[test]
    fn nightmare_mixed_clean_and_slug_interleaved() {
        // Interleaved clean and forbidden-char names that produce same slug.
        // Tests that accumulation order doesn't matter.
        let r = process_siblings_dir(&[
            "A_B", // clean, claims "A_B"
            "C_D", // clean, claims "C_D"
            "A/B", // slug "A_B" → collision → "A_B~1"
            "E_F", // clean, claims "E_F"
            "C:D", // slug "C_D" → collision → "C_D~1"
            "A:B", // slug "A_B" → collision with A_B and A_B~1 → "A_B~2"
        ]);
        assert_eq!(r[0].1, "A_B");
        assert!(!r[0].2);
        assert_eq!(r[1].1, "C_D");
        assert!(!r[1].2);
        assert_eq!(r[2].1, "A_B~1");
        assert!(r[2].2);
        assert_eq!(r[3].1, "E_F");
        assert!(!r[3].2);
        assert_eq!(r[4].1, "C_D~1");
        assert!(r[4].2);
        assert_eq!(r[5].1, "A_B~2");
        assert!(r[5].2);
    }

    // ══════════════════════════════════════════════════════════════════
    //  IDEMPOTENCY AND INVARIANT TESTS
    //
    //  These verify structural invariants that must always hold.
    // ══════════════════════════════════════════════════════════════════

    #[test]
    fn invariant_no_two_siblings_share_filesystem_name() {
        // Process a massive batch of problematic names and verify
        // every resulting filename is unique (case-insensitive).
        let names = [
            "Foo",
            "foo",
            "FOO",
            "Foo",
            "Foo",
            "A/B",
            "A:B",
            "A*B",
            "A_B",
            "CON",
            "CON_",
            "con",
            "Con",
            "test.server",
            "test_server",
            "test/server",
            "",
            "",
            "",
            " X",
            "X",
            "X ",
            "Hello\x00",
            "Hello_",
            "日本語",
            "日本語",
        ];
        let r = process_siblings_dir(&names);
        let lowered: Vec<String> = r.iter().map(|(_, f, _)| f.to_lowercase()).collect();
        let unique: HashSet<&String> = lowered.iter().collect();
        assert_eq!(
            unique.len(),
            lowered.len(),
            "every sibling must have a unique filename. Duplicates found in: {:?}",
            lowered
        );
    }

    #[test]
    fn invariant_every_filename_passes_validation() {
        // Every filename produced by the pipeline must pass validate_file_name.
        let names = [
            "CON",
            "PRN",
            "NUL",
            "COM1",
            "LPT9",
            "foo.server",
            "bar.meta",
            "baz.model",
            "A/B",
            "C:D",
            "E*F",
            "G?H",
            "<init>",
            "a|b",
            "x\\y",
            "q\"r",
            "",
            " ",
            ".",
            "..",
            "hello.",
            "hello ",
            " hello",
            "~1",
            "~~",
            "a~b~c",
        ];
        let r = process_siblings_dir(&names);
        for (original, filename, _) in &r {
            assert!(
                validate_file_name(filename).is_ok(),
                "filename {filename:?} (from {original:?}) must be valid"
            );
        }
    }

    #[test]
    fn invariant_needs_meta_iff_name_differs_from_slug() {
        // For new instances: needs_meta is true IFF the filesystem name
        // differs from the instance name. This is the contract.
        let cases = [
            "Clean",
            "Hello World",
            "test-123",
            "A/B",
            "CON",
            "foo.server",
            " X",
            "X ",
            "",
            "Foo~1",
            "hello.",
        ];
        for &name in &cases {
            let dom = make_inst(name, "Folder");
            let child_ref = dom.root().children()[0];
            let child = dom.get_by_ref(child_ref).unwrap();
            let taken = HashSet::new();
            let (filename, needs_meta, _dk) =
                name_for_inst(Middleware::Dir, child, None, &taken).unwrap();
            if needs_meta {
                assert_ne!(
                    filename.as_ref(),
                    name,
                    "needs_meta=true but filename matches name for {name:?}"
                );
            } else {
                assert_eq!(
                    filename.as_ref(),
                    name,
                    "needs_meta=false but filename differs from name for {name:?}"
                );
            }
        }
    }

    // ── strip_script_suffix ──────────────────────────────────────────

    #[test]
    fn strip_suffix_server() {
        assert_eq!(strip_script_suffix("MyScript.server"), "MyScript");
    }

    #[test]
    fn strip_suffix_client() {
        assert_eq!(strip_script_suffix("MyScript.client"), "MyScript");
    }

    #[test]
    fn strip_suffix_plugin() {
        assert_eq!(strip_script_suffix("MyScript.plugin"), "MyScript");
    }

    #[test]
    fn strip_suffix_local() {
        assert_eq!(strip_script_suffix("MyScript.local"), "MyScript");
    }

    #[test]
    fn strip_suffix_legacy() {
        assert_eq!(strip_script_suffix("MyScript.legacy"), "MyScript");
    }

    #[test]
    fn strip_suffix_none() {
        assert_eq!(strip_script_suffix("MyModule"), "MyModule");
    }

    #[test]
    fn strip_suffix_dots_in_name() {
        assert_eq!(strip_script_suffix("v1.0.server"), "v1.0");
    }

    // ── adjacent_meta_path ───────────────────────────────────────────

    #[test]
    fn adjacent_meta_server_script() {
        let path = std::path::Path::new("src/Key_Script.server.luau");
        let meta = adjacent_meta_path(path);
        assert_eq!(meta, std::path::PathBuf::from("src/Key_Script.meta.json5"));
    }

    #[test]
    fn adjacent_meta_module_script() {
        let path = std::path::Path::new("src/Helper.luau");
        let meta = adjacent_meta_path(path);
        assert_eq!(meta, std::path::PathBuf::from("src/Helper.meta.json5"));
    }

    #[test]
    fn adjacent_meta_client_script() {
        let path = std::path::Path::new("src/Gui.client.luau");
        let meta = adjacent_meta_path(path);
        assert_eq!(meta, std::path::PathBuf::from("src/Gui.meta.json5"));
    }

    #[test]
    fn adjacent_meta_legacy_lua() {
        let path = std::path::Path::new("src/Old.server.lua");
        let meta = adjacent_meta_path(path);
        assert_eq!(meta, std::path::PathBuf::from("src/Old.meta.json5"));
    }

    // ── tilde dedup end-to-end (unit level) ──────────────────────────

    #[test]
    fn tilde_dedup_collision_roundtrip() {
        // Two instances "A/B" and "A_B" both slugify to "A_B".
        // First gets "A_B", second gets "A_B~1".
        let mut taken = HashSet::new();

        let dom1 = make_inst("A/B", "ModuleScript");
        let child1_ref = dom1.root().children()[0];
        let child1 = dom1.get_by_ref(child1_ref).unwrap();
        let (name1, meta1, dk1) =
            name_for_inst(Middleware::ModuleScript, child1, None, &taken).unwrap();
        taken.insert(dk1.to_lowercase());

        let dom2 = make_inst("A_B", "ModuleScript");
        let child2_ref = dom2.root().children()[0];
        let child2 = dom2.get_by_ref(child2_ref).unwrap();
        let (name2, meta2, dk2) =
            name_for_inst(Middleware::ModuleScript, child2, None, &taken).unwrap();
        taken.insert(dk2.to_lowercase());

        assert_eq!(name1.as_ref(), "A_B.luau");
        assert!(meta1, "A/B was slugified, needs meta name");
        assert_eq!(name2.as_ref(), "A_B~1.luau");
        assert!(meta2, "A_B was deduped to ~1, needs meta name");
    }

    #[test]
    fn tilde_in_filename_not_parsed_as_dedup_marker() {
        // A file named "Foo~1" should produce instance name "Foo~1",
        // NOT be interpreted as "Foo" with dedup suffix ~1.
        // This is verified by the fact that name_needs_slugify("Foo~1")
        // returns true (tilde is in SLUGIFY_CHARS), so a file named
        // "Foo~1.luau" can only exist if it was written by the dedup
        // system. Forward sync reads the filename as-is.
        assert!(
            name_needs_slugify("Foo~1"),
            "tilde should trigger slugification"
        );
    }

    // ── stress: large batch with collisions ──────────────────────────

    #[test]
    fn stress_100_instances_deterministic() {
        // Create 100 instances where groups of 5 share the same slug.
        // Run dedup twice and verify the results are identical.
        let forbidden_chars = ['/', ':', '?', '|', '<', '>', '"', '*', '\\'];
        let mut names = Vec::new();
        for i in 0..100 {
            let group = i / 5;
            let variant = i % 5;
            // Each group of 5 uses a different forbidden char so they all
            // slugify to "Group_XX" but have different real names.
            let ch = forbidden_chars[variant % forbidden_chars.len()];
            names.push(format!("Group{ch}{:02}", group));
        }
        // Sort names alphabetically to simulate deterministic processing
        names.sort();

        let run = |names: &[String]| -> Vec<(String, bool, String)> {
            let mut taken = HashSet::new();
            let mut results = Vec::new();
            for name in names {
                let dom = make_inst(name, "ModuleScript");
                let child_ref = dom.root().children()[0];
                let child = dom.get_by_ref(child_ref).unwrap();
                let (filename, needs_meta, dk) =
                    name_for_inst(Middleware::ModuleScript, child, None, &taken).unwrap();
                taken.insert(dk.to_lowercase());
                results.push((filename.into_owned(), needs_meta, dk));
            }
            results
        };

        let run1 = run(&names);
        let run2 = run(&names);
        assert_eq!(
            run1, run2,
            "dedup results must be deterministic across runs"
        );

        // Verify no duplicate filenames
        let filenames: HashSet<String> = run1.iter().map(|(f, _, _)| f.to_lowercase()).collect();
        assert_eq!(
            filenames.len(),
            100,
            "all 100 instances should have unique filenames"
        );
    }

    // ── idempotency: second pass produces same results ───────────────

    #[test]
    fn idempotency_old_inst_preserves_path() {
        // When an old instance exists with a path, name_for_inst should
        // return the same path regardless of taken_names.
        let dom = make_inst("Hey/Bro", "ModuleScript");
        let child_ref = dom.root().children()[0];
        let child = dom.get_by_ref(child_ref).unwrap();

        // First pass: new instance
        let mut taken = HashSet::new();
        let (name1, meta1, dk1) =
            name_for_inst(Middleware::ModuleScript, child, None, &taken).unwrap();
        taken.insert(dk1.to_lowercase());

        assert_eq!(name1.as_ref(), "Hey_Bro.luau");
        assert!(meta1);

        // On the old-inst path the existing filename is preserved,
        // so a second syncback (incremental) should not change the name.
        // We can't easily test the full old-inst path here since it
        // requires InstanceWithMeta, but we verify the dedup_key is
        // stable: calling strip_middleware_extension on the result gives
        // the same dedup_key.
        let dk_from_filename = strip_middleware_extension(&name1, Middleware::ModuleScript);
        assert_eq!(dk_from_filename.to_lowercase(), dk1.to_lowercase());
    }

    #[test]
    fn syncback_forbidden_chars_produce_slugified_filenames() {
        // Verify that instances with forbidden chars produce correct
        // (filename, needs_meta, dedup_key) via name_for_inst
        let mut dom = rbx_dom_weak::WeakDom::new(rbx_dom_weak::InstanceBuilder::new("Folder"));
        let root_ref = dom.root_ref();
        let child = dom.insert(
            root_ref,
            rbx_dom_weak::InstanceBuilder::new("ModuleScript").with_name("Hey/Bro"),
        );
        let inst = dom.get_by_ref(child).unwrap();
        let taken = std::collections::HashSet::new();
        let (filename, needs_meta, dedup_key) = name_for_inst(
            crate::snapshot_middleware::Middleware::ModuleScript,
            inst,
            None,
            &taken,
        )
        .unwrap();
        assert_eq!(filename.as_ref(), "Hey_Bro.luau");
        assert!(needs_meta);
        assert_eq!(dedup_key, "Hey_Bro");
    }

    #[test]
    fn syncback_collision_deduplicates() {
        let mut dom = rbx_dom_weak::WeakDom::new(rbx_dom_weak::InstanceBuilder::new("Folder"));
        let root_ref = dom.root_ref();
        let child1 = dom.insert(
            root_ref,
            rbx_dom_weak::InstanceBuilder::new("ModuleScript").with_name("A/B"),
        );
        let child2 = dom.insert(
            root_ref,
            rbx_dom_weak::InstanceBuilder::new("ModuleScript").with_name("A:B"),
        );
        let inst1 = dom.get_by_ref(child1).unwrap();
        let inst2 = dom.get_by_ref(child2).unwrap();

        let mut taken = std::collections::HashSet::new();
        let (f1, m1, dk1) = name_for_inst(
            crate::snapshot_middleware::Middleware::ModuleScript,
            inst1,
            None,
            &taken,
        )
        .unwrap();
        taken.insert(dk1.to_lowercase());
        let (f2, m2, dk2) = name_for_inst(
            crate::snapshot_middleware::Middleware::ModuleScript,
            inst2,
            None,
            &taken,
        )
        .unwrap();

        assert_eq!(f1.as_ref(), "A_B.luau");
        assert!(m1);
        assert_eq!(f2.as_ref(), "A_B~1.luau");
        assert!(m2);
        assert_ne!(dk1.to_lowercase(), dk2.to_lowercase());
    }

    #[test]
    fn syncback_idempotency_same_input_same_output() {
        // Run the same set of instances through name_for_inst twice
        // and verify identical output (determinism).
        let names = vec!["Alpha", "Beta/Gamma", "Beta:Gamma", "Delta", "CON"];
        let mw = crate::snapshot_middleware::Middleware::ModuleScript;

        let mut dom = rbx_dom_weak::WeakDom::new(rbx_dom_weak::InstanceBuilder::new("Folder"));
        let root = dom.root_ref();
        let refs: Vec<_> = names
            .iter()
            .map(|n| {
                dom.insert(
                    root,
                    rbx_dom_weak::InstanceBuilder::new("ModuleScript").with_name(*n),
                )
            })
            .collect();

        let run = |dom: &rbx_dom_weak::WeakDom,
                   refs: &[rbx_dom_weak::types::Ref]|
         -> Vec<(String, bool, String)> {
            let mut taken = std::collections::HashSet::new();
            refs.iter()
                .map(|r| {
                    let inst = dom.get_by_ref(*r).unwrap();
                    let (f, m, dk) = name_for_inst(mw, inst, None, &taken).unwrap();
                    taken.insert(dk.to_lowercase());
                    (f.into_owned(), m, dk)
                })
                .collect()
        };

        let result1 = run(&dom, &refs);
        let result2 = run(&dom, &refs);
        assert_eq!(
            result1, result2,
            "Two runs of name_for_inst must produce identical output"
        );
    }
}
