//! Human-readable and machine-readable **output**: terminal tables, JSON, JUnit, persisted
//! history (SQLite), and [`diff_runs`] / [`print_diff`] for comparing two runs.

mod diff;
mod history;
mod json;
mod junit;
mod terminal;

pub use diff::{diff_runs, print_diff};
pub use history::{fetch_run, list_runs, persist_run, HistoryError, RunListing};
pub use json::print_json;
pub use junit::print_junit;
pub use terminal::{print_terminal, print_terminal_summary};
