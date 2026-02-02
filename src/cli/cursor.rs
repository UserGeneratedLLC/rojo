use std::path::PathBuf;
use std::process::{Command, Stdio};

use anyhow::Context;
use clap::Parser;

use super::resolve_path;

/// Open the project directory in Cursor IDE.
#[derive(Debug, Parser)]
pub struct CursorCommand {
    /// Path to open. Defaults to the current directory.
    #[clap(default_value = ".")]
    pub path: PathBuf,
}

impl CursorCommand {
    pub fn run(self) -> anyhow::Result<()> {
        let path = resolve_path(&self.path);

        // Use cmd /c to run cursor through the shell (cursor is a shell command, not an exe)
        #[cfg(windows)]
        Command::new("cmd")
            .args(["/c", "cursor", &path.to_string_lossy()])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .context("Failed to launch Cursor IDE. Is 'cursor' in your PATH?")?;

        #[cfg(not(windows))]
        Command::new("cursor")
            .arg(&*path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .context("Failed to launch Cursor IDE. Is 'cursor' in your PATH?")?;

        Ok(())
    }
}
