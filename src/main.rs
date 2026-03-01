use std::{env, panic, process};

use backtrace::Backtrace;
use clap::Parser;

use librojo::cli::{resolve_project_dir, Options};
use librojo::logging;

fn main() {
    #[cfg(feature = "profile-with-tracy")]
    profiling::tracy_client::Client::start();

    panic::set_hook(Box::new(|panic_info| {
        let message = match panic_info.payload().downcast_ref::<&str>() {
            Some(&message) => message.to_string(),
            None => match panic_info.payload().downcast_ref::<String>() {
                Some(message) => message.clone(),
                None => "<no message>".to_string(),
            },
        };

        log::error!(
            "Rojo crashed! You are running Rojo {}.",
            env!("CARGO_PKG_VERSION")
        );
        log::error!("This is probably a Rojo bug.");
        log::error!("");
        log::error!(
            "Please consider filing an issue: {}/issues",
            env!("CARGO_PKG_REPOSITORY")
        );
        log::error!("");
        log::error!("Details: {}", message);

        if let Some(location) = panic_info.location() {
            log::error!("in file {} on line {}", location.file(), location.line());
        }

        let should_backtrace = env::var("RUST_BACKTRACE")
            .map(|var| var == "1")
            .unwrap_or(false);

        if should_backtrace {
            eprintln!("{:?}", Backtrace::new());
        } else {
            eprintln!(
                "note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace."
            );
        }

        process::exit(1);
    }));

    let options = Options::parse();

    let project_dir = options.subcommand.project_path().map(resolve_project_dir);

    let file_log_level = if env::var("ATLAS_NO_FILE_LOG").is_ok() {
        None
    } else {
        project_dir
            .as_deref()
            .and_then(logging::quick_read_file_log_level)
            .unwrap_or(Some(tracing::level_filters::LevelFilter::TRACE))
    };

    let command_name = format!("atlas-{}", options.subcommand.command_name());

    let _log_guard = logging::init_logging(
        options.global.verbosity,
        options.global.color,
        project_dir.as_deref(),
        file_log_level,
        &command_name,
    );

    if let Err(err) = options.run() {
        log::error!("{:?}", err);
        process::exit(1);
    }
}
