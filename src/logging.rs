use std::{
    io::{self, IsTerminal, Write},
    path::Path,
};

use tracing_subscriber::{
    fmt::{self, time::UtcTime},
    layer::SubscriberExt,
    util::SubscriberInitExt,
    EnvFilter, Layer,
};

use crate::cli::ColorChoice;

pub struct LogGuard {
    _file_guard: Option<tracing_appender::non_blocking::WorkerGuard>,
}

pub fn init_logging(
    verbosity: u8,
    color: ColorChoice,
    project_dir: Option<&Path>,
    file_log_level: Option<tracing::level_filters::LevelFilter>,
    command_name: &str,
) -> LogGuard {
    tracing_log::LogTracer::init().expect("Failed to set log tracer");

    let console_filter = match verbosity {
        0 => "info",
        1 => "info,librojo=debug",
        2 => "info,librojo=trace",
        _ => "trace",
    };

    let console_env_filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(console_filter));

    let use_ansi = match color {
        ColorChoice::Always => true,
        ColorChoice::Never => false,
        ColorChoice::Auto => io::stderr().is_terminal(),
    };

    let console_layer = fmt::layer()
        .with_writer(io::stderr)
        .with_ansi(use_ansi)
        .without_time()
        .with_target(false)
        .with_thread_names(false)
        .with_level(true)
        .with_filter(console_env_filter);

    let mut file_guard: Option<tracing_appender::non_blocking::WorkerGuard> = None;

    let file_layer = if let (Some(dir), Some(level)) = (project_dir, file_log_level) {
        let log_dir = dir.join(".atlas").join("logs");

        match std::fs::create_dir_all(&log_dir) {
            Ok(()) => {
                compress_old_logs(&log_dir, command_name);

                let file_appender = tracing_appender::rolling::Builder::new()
                    .rotation(tracing_appender::rolling::Rotation::DAILY)
                    .filename_prefix(command_name)
                    .filename_suffix("log")
                    .build(&log_dir)
                    .expect("Failed to create rolling file appender");

                let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);
                file_guard = Some(guard);

                let file_filter = EnvFilter::new(level.to_string());

                let layer = fmt::layer()
                    .with_writer(non_blocking)
                    .with_ansi(false)
                    .with_timer(UtcTime::rfc_3339())
                    .with_target(true)
                    .with_thread_names(true)
                    .with_level(true)
                    .with_filter(file_filter);

                Some(layer)
            }
            Err(e) => {
                eprintln!(
                    "Warning: could not create log directory {}: {e}",
                    log_dir.display()
                );
                None
            }
        }
    } else {
        None
    };

    tracing_subscriber::registry()
        .with(console_layer)
        .with(file_layer)
        .init();

    LogGuard {
        _file_guard: file_guard,
    }
}

fn compress_old_logs(log_dir: &Path, command_name: &str) {
    let today = {
        let now = std::time::SystemTime::now();
        let since_epoch = now
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        let days = since_epoch.as_secs() / 86400;
        days
    };

    let entries = match std::fs::read_dir(log_dir) {
        Ok(entries) => entries,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let file_name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_owned(),
            None => continue,
        };

        if file_name.ends_with(".log.gz") {
            if let Ok(meta) = entry.metadata() {
                if let Ok(modified) = meta.modified() {
                    let age_days = modified
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| today.saturating_sub(d.as_secs() / 86400))
                        .unwrap_or(0);
                    if age_days > 7 {
                        let _ = std::fs::remove_file(&path);
                    }
                }
            }
            continue;
        }

        if !file_name.ends_with(".log") {
            continue;
        }

        if !file_name.starts_with(command_name) {
            continue;
        }

        if let Ok(meta) = entry.metadata() {
            if let Ok(modified) = meta.modified() {
                let file_days = modified
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs() / 86400)
                    .unwrap_or(today);
                if file_days >= today {
                    continue;
                }
            }
        }

        let gz_path = path.with_extension("log.gz");
        if let Ok(input) = std::fs::read(&path) {
            let gz_file = match std::fs::File::create(&gz_path) {
                Ok(f) => f,
                Err(_) => continue,
            };
            let mut encoder =
                flate2::write::GzEncoder::new(gz_file, flate2::Compression::default());
            if encoder.write_all(&input).is_ok() && encoder.finish().is_ok() {
                let _ = std::fs::remove_file(&path);
            } else {
                let _ = std::fs::remove_file(&gz_path);
            }
        }
    }
}

/// Parses a Roblox log message forwarded from the plugin, extracting the real
/// log level and a cleaned message. Rojo plugin logs have `[Rojo-Trace] ` etc.
/// prefixes that encode the level; these are stripped and replaced with `[Rojo]`.
/// Game script output without a Rojo prefix falls back to the `MessageType` enum.
///
/// Returns `(log::Level, cleaned_message)`.
pub fn parse_roblox_log(message: &str, message_type: u64) -> (log::Level, String) {
    if let Some(rest) = message.strip_prefix("[Rojo-Trace] ") {
        (log::Level::Trace, format!("[Rojo] {}", rest))
    } else if let Some(rest) = message.strip_prefix("[Rojo-Debug] ") {
        (log::Level::Debug, format!("[Rojo] {}", rest))
    } else if let Some(rest) = message.strip_prefix("[Rojo-Info] ") {
        (log::Level::Info, format!("[Rojo] {}", rest))
    } else if let Some(rest) = message.strip_prefix("[Rojo-Warn] ") {
        (log::Level::Warn, format!("[Rojo] {}", rest))
    } else {
        let lvl = match message_type {
            2 => log::Level::Warn,
            3 => log::Level::Error,
            _ => log::Level::Info,
        };
        (lvl, message.to_string())
    }
}

/// Lightweight pre-read of a project file to extract the `fileLogLevel` config.
/// Accepts either a project file path or a directory (searches for default project).
/// Returns `None` if no project found or the field is absent (caller defaults
/// to trace). Returns `Some(None)` when `"none"` (disabled).
pub fn quick_read_file_log_level(
    path: &Path,
) -> Option<Option<tracing::level_filters::LevelFilter>> {
    use tracing::level_filters::LevelFilter;

    let project_file = if path.is_file() {
        path.to_path_buf()
    } else {
        let json5_path = path.join("default.project.json5");
        if json5_path.is_file() {
            json5_path
        } else {
            let json_path = path.join("default.project.json");
            if json_path.is_file() {
                json_path
            } else {
                return None;
            }
        }
    };

    let contents = std::fs::read_to_string(&project_file).ok()?;
    let val: serde_json::Value = json5::from_str(&contents).ok()?;
    let level_str = val.get("fileLogLevel")?.as_str()?;

    Some(match level_str.to_lowercase().as_str() {
        "none" | "off" => None,
        "error" => Some(LevelFilter::ERROR),
        "warn" => Some(LevelFilter::WARN),
        "info" => Some(LevelFilter::INFO),
        "debug" => Some(LevelFilter::DEBUG),
        "trace" => Some(LevelFilter::TRACE),
        _ => Some(LevelFilter::TRACE),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use tracing::level_filters::LevelFilter;

    #[test]
    fn parse_roblox_log_rojo_trace() {
        let (level, msg) = parse_roblox_log("[Rojo-Trace] snapshot started", 0);
        assert_eq!(level, log::Level::Trace);
        assert_eq!(msg, "[Rojo] snapshot started");
    }

    #[test]
    fn parse_roblox_log_rojo_debug() {
        let (level, msg) = parse_roblox_log("[Rojo-Debug] loading project", 0);
        assert_eq!(level, log::Level::Debug);
        assert_eq!(msg, "[Rojo] loading project");
    }

    #[test]
    fn parse_roblox_log_rojo_info() {
        let (level, msg) = parse_roblox_log("[Rojo-Info] connected to server", 0);
        assert_eq!(level, log::Level::Info);
        assert_eq!(msg, "[Rojo] connected to server");
    }

    #[test]
    fn parse_roblox_log_rojo_warn() {
        let (level, msg) = parse_roblox_log("[Rojo-Warn] something fishy", 0);
        assert_eq!(level, log::Level::Warn);
        assert_eq!(msg, "[Rojo] something fishy");
    }

    #[test]
    fn parse_roblox_log_game_print() {
        let (level, msg) = parse_roblox_log("hello from game", 0);
        assert_eq!(level, log::Level::Info);
        assert_eq!(msg, "hello from game");
    }

    #[test]
    fn parse_roblox_log_game_warn() {
        let (level, msg) = parse_roblox_log("something wrong", 2);
        assert_eq!(level, log::Level::Warn);
        assert_eq!(msg, "something wrong");
    }

    #[test]
    fn parse_roblox_log_game_error() {
        let (level, msg) = parse_roblox_log("crash!", 3);
        assert_eq!(level, log::Level::Error);
        assert_eq!(msg, "crash!");
    }

    #[test]
    fn parse_roblox_log_message_info_type() {
        let (level, msg) = parse_roblox_log("info message", 1);
        assert_eq!(level, log::Level::Info);
        assert_eq!(msg, "info message");
    }

    #[test]
    fn parse_roblox_log_unknown_message_type() {
        let (level, msg) = parse_roblox_log("weird type", 99);
        assert_eq!(level, log::Level::Info);
        assert_eq!(msg, "weird type");
    }

    #[test]
    fn parse_roblox_log_partial_prefix_not_stripped() {
        let (level, msg) = parse_roblox_log("[Rojo-Trace]no space", 0);
        assert_eq!(level, log::Level::Info);
        assert_eq!(msg, "[Rojo-Trace]no space");
    }

    #[test]
    fn parse_roblox_log_empty_message_after_prefix() {
        let (level, msg) = parse_roblox_log("[Rojo-Info] ", 0);
        assert_eq!(level, log::Level::Info);
        assert_eq!(msg, "[Rojo] ");
    }

    #[test]
    fn quick_read_file_log_level_trace() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("default.project.json5"),
            r#"{ "name": "Test", "tree": {}, "fileLogLevel": "trace" }"#,
        )
        .unwrap();
        let result = quick_read_file_log_level(dir.path());
        assert_eq!(result, Some(Some(LevelFilter::TRACE)));
    }

    #[test]
    fn quick_read_file_log_level_none() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("default.project.json5"),
            r#"{ "name": "Test", "tree": {}, "fileLogLevel": "none" }"#,
        )
        .unwrap();
        let result = quick_read_file_log_level(dir.path());
        assert_eq!(result, Some(None));
    }

    #[test]
    fn quick_read_file_log_level_off() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("default.project.json5"),
            r#"{ "name": "Test", "tree": {}, "fileLogLevel": "off" }"#,
        )
        .unwrap();
        let result = quick_read_file_log_level(dir.path());
        assert_eq!(result, Some(None));
    }

    #[test]
    fn quick_read_file_log_level_warn() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("default.project.json5"),
            r#"{ "name": "Test", "tree": {}, "fileLogLevel": "warn" }"#,
        )
        .unwrap();
        let result = quick_read_file_log_level(dir.path());
        assert_eq!(result, Some(Some(LevelFilter::WARN)));
    }

    #[test]
    fn quick_read_file_log_level_case_insensitive() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("default.project.json5"),
            r#"{ "name": "Test", "tree": {}, "fileLogLevel": "DEBUG" }"#,
        )
        .unwrap();
        let result = quick_read_file_log_level(dir.path());
        assert_eq!(result, Some(Some(LevelFilter::DEBUG)));
    }

    #[test]
    fn quick_read_file_log_level_absent() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("default.project.json5"),
            r#"{ "name": "Test", "tree": {} }"#,
        )
        .unwrap();
        let result = quick_read_file_log_level(dir.path());
        assert_eq!(result, None);
    }

    #[test]
    fn quick_read_file_log_level_no_project() {
        let dir = tempfile::tempdir().unwrap();
        let result = quick_read_file_log_level(dir.path());
        assert_eq!(result, None);
    }

    #[test]
    fn quick_read_file_log_level_legacy_json() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("default.project.json"),
            r#"{ "name": "Test", "tree": {}, "fileLogLevel": "error" }"#,
        )
        .unwrap();
        let result = quick_read_file_log_level(dir.path());
        assert_eq!(result, Some(Some(LevelFilter::ERROR)));
    }

    #[test]
    fn quick_read_file_log_level_json5_preferred_over_json() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("default.project.json5"),
            r#"{ "name": "Test", "tree": {}, "fileLogLevel": "info" }"#,
        )
        .unwrap();
        std::fs::write(
            dir.path().join("default.project.json"),
            r#"{ "name": "Test", "tree": {}, "fileLogLevel": "error" }"#,
        )
        .unwrap();
        let result = quick_read_file_log_level(dir.path());
        assert_eq!(result, Some(Some(LevelFilter::INFO)));
    }

    #[test]
    fn quick_read_file_log_level_direct_file_path() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("custom.project.json5");
        std::fs::write(
            &file_path,
            r#"{ "name": "Test", "tree": {}, "fileLogLevel": "debug" }"#,
        )
        .unwrap();
        let result = quick_read_file_log_level(&file_path);
        assert_eq!(result, Some(Some(LevelFilter::DEBUG)));
    }

    #[test]
    fn quick_read_file_log_level_unknown_defaults_to_trace() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("default.project.json5"),
            r#"{ "name": "Test", "tree": {}, "fileLogLevel": "banana" }"#,
        )
        .unwrap();
        let result = quick_read_file_log_level(dir.path());
        assert_eq!(result, Some(Some(LevelFilter::TRACE)));
    }

    #[test]
    fn compress_old_logs_compresses_old_files() {
        let dir = tempfile::tempdir().unwrap();
        let log_dir = dir.path();

        let old_file = log_dir.join("atlas-serve.2020-01-01.log");
        std::fs::write(&old_file, "old log content").unwrap();

        let mtime =
            std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(86400 * 18262);
        filetime::set_file_mtime(&old_file, filetime::FileTime::from_system_time(mtime))
            .unwrap_or_default();

        compress_old_logs(log_dir, "atlas-serve");

        assert!(!old_file.exists(), "original .log file should be deleted");
        let gz_file = log_dir.join("atlas-serve.2020-01-01.log.gz");
        assert!(gz_file.exists(), ".log.gz file should be created");

        let gz_data = std::fs::read(&gz_file).unwrap();
        let mut decoder = flate2::read::GzDecoder::new(&gz_data[..]);
        let mut decompressed = String::new();
        decoder.read_to_string(&mut decompressed).unwrap();
        assert_eq!(decompressed, "old log content");
    }

    #[test]
    fn compress_old_logs_skips_other_commands() {
        let dir = tempfile::tempdir().unwrap();
        let log_dir = dir.path();

        let other_file = log_dir.join("atlas-build.2020-01-01.log");
        std::fs::write(&other_file, "build log").unwrap();

        let mtime =
            std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(86400 * 18262);
        filetime::set_file_mtime(&other_file, filetime::FileTime::from_system_time(mtime))
            .unwrap_or_default();

        compress_old_logs(log_dir, "atlas-serve");

        assert!(
            other_file.exists(),
            "other command's log should NOT be compressed"
        );
    }

    #[test]
    fn compress_old_logs_skips_today() {
        let dir = tempfile::tempdir().unwrap();
        let log_dir = dir.path();

        let today_file = log_dir.join("atlas-serve.today.log");
        std::fs::write(&today_file, "today's log").unwrap();

        compress_old_logs(log_dir, "atlas-serve");

        assert!(today_file.exists(), "today's log should NOT be compressed");
    }
}
