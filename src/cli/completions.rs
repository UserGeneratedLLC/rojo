use std::io;

use clap::Parser;
use clap_complete::{generate, Shell};

use super::Options;

#[derive(Debug, Parser)]
pub struct CompletionsCommand {
    /// Shell to generate completions for.
    #[clap(value_enum)]
    pub shell: Shell,
}

impl CompletionsCommand {
    pub fn run(self) -> anyhow::Result<()> {
        let mut cmd = <Options as clap::CommandFactory>::command();
        generate(self.shell, &mut cmd, "atlas", &mut io::stdout());
        Ok(())
    }
}
