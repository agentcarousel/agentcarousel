use clap::{CommandFactory, ValueEnum};
use clap_complete::env::{Bash, EnvCompleter, Fish, Zsh};
use std::io;

#[derive(Debug, Clone, ValueEnum)]
pub enum Shell {
    Bash,
    Zsh,
    Fish,
}

#[derive(Debug, clap::Parser)]
pub struct CompletionsArgs {
    /// Shell to generate completions for.
    shell: Shell,
}

pub fn run_completions(args: CompletionsArgs) -> i32 {
    let mut out = io::stdout();
    let completer: &dyn EnvCompleter = match args.shell {
        Shell::Bash => &Bash,
        Shell::Zsh => &Zsh,
        Shell::Fish => &Fish,
    };
    let cmd = super::Cli::command();
    let name = cmd.get_name().to_owned();
    match completer.write_registration("COMPLETE", &name, "agc", "agc", &mut out) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}
