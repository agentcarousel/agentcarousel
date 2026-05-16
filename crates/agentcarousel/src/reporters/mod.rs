//! Human-readable and machine-readable **output**: terminal tables, JSON, JUnit, persisted
//! history (SQLite), and [`diff_runs`] / [`print_diff`] for comparing two runs.

mod diff;
mod history;
mod junit;
mod terminal;

pub use diff::{diff_runs, print_diff};
pub use history::{fetch_run, list_full_runs, list_runs, persist_run, HistoryError, RunListing};
pub use junit::print_junit;
pub use terminal::{print_terminal, print_terminal_summary};

pub fn print_json(run: &agentcarousel_core::Run) {
    let payload = serde_json::to_string_pretty(run).unwrap_or_else(|_| "{}".to_string());
    println!("{payload}");
}
