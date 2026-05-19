use clap::Parser;
use std::path::PathBuf;

use crate::reporters::dashboard::serve;

use super::exit_codes::ExitCode;
use super::output::{JsonError, JsonOutput};
use super::GlobalOptions;

#[derive(Debug, Parser)]
#[command(
    after_help = "Examples:\n  agc dashboard                        # http://localhost:7421\n  agc dashboard --port 8080            # custom port\n  agc dashboard --db path/to/history.db\n\nExit codes:\n  0  server exited cleanly\n  4  runtime error (port in use, IO error)\n\nNote: agc dashboard is only available in the full binary variant.\n  Install: curl -fsSL https://install.agentcarousel.com | sh -s -- --feature dashboard\n  Upgrade: agc update --feature dashboard"
)]
pub struct DashboardArgs {
    /// Port to listen on (default: 7421).
    #[arg(long, default_value_t = 7421)]
    port: u16,

    /// Path to a custom history database (overrides AGENTCAROUSEL_HISTORY_DB).
    #[arg(long, value_name = "PATH")]
    db: Option<PathBuf>,

    /// Serve HTML/CSS/JS from disk instead of embedded bytes (for development).
    #[arg(long, hide = true)]
    dev: bool,
}

pub fn run_dashboard(args: DashboardArgs, globals: &GlobalOptions) -> i32 {
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_io()
        .enable_time()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            if globals.json {
                JsonOutput::err("dashboard", JsonError::new("runtime_error", e.to_string()))
                    .print();
            } else {
                eprintln!("error: failed to start runtime: {e}");
            }
            return ExitCode::RuntimeError.as_i32();
        }
    };

    // Set the DB env var if --db was supplied.
    if let Some(ref db) = args.db {
        std::env::set_var("AGENTCAROUSEL_HISTORY_DB", db);
    }

    let port = args.port;
    let dev = args.dev;

    runtime.block_on(async move {
        if let Err(e) = serve(port, args.db, dev).await {
            eprintln!("error: {e}");
        }
    });

    ExitCode::Ok.as_i32()
}
