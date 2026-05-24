//! Human-readable and machine-readable **output**: terminal tables, JSON, and persisted
//! history (SQLite).

#[cfg(feature = "dashboard")]
pub mod dashboard;
mod history;
mod terminal;

pub use history::{
    fetch_run, find_previous_run, find_tagged_run, list_full_runs, list_runs, persist_run, tag_run,
    HistoryError, RunListing,
};
pub use terminal::{print_audit, print_terminal, print_terminal_summary};

pub fn print_json(run: &agentcarousel_core::Run) {
    let payload = serde_json::to_string_pretty(run).unwrap_or_else(|_| "{}".to_string());
    println!("{payload}");
}
