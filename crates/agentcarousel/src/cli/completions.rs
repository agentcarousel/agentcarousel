use clap::{CommandFactory, Parser};
use clap_complete::{generate, Shell};
use std::io;

#[derive(Debug, Parser)]
pub struct CompletionsArgs {
    /// Shell to generate completions for.
    shell: Shell,
}

pub fn run_completions(args: CompletionsArgs) -> i32 {
    let mut cmd = super::Cli::command();
    generate(args.shell, &mut cmd, "agc", &mut io::stdout());
    0
}
