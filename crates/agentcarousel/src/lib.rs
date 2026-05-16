//! Evaluate agents and skills from YAML/TOML fixtures, run cases with mocks or live
//! backends, persist runs to SQLite, and export evidence for reporting or registry upload.
//!
//! # Audience
//!
//! - **CLI users** — run the [`agentcarousel`](crate::cli::Cli) or `agc` binary for
//!   `validate`, `test`, `eval`, `report`, `bundle` (pack / verify / pull), `publish`, `export`, and `trust-check`.
//! - **Library embedders** — use [`runner`] to execute fixtures programmatically,
//!   [`fixtures`] to load them, [`evaluators`] for scoring, [`reporters`] for output, and
//!   [`core`] for shared types ([`Run`], [`Case`], [`FixtureFile`], …).
//!
//! # Crate layout
//!
//! | Module | Role |
//! |--------|------|
//! | [`cli`] | Clap-based CLI; [`cli::run`] is the process entrypoint. |
//! | [`core`] | Serializable models, errors, and judge provider helpers. |
//! | [`runner`] | Async execution: [`runner::run_fixtures`], [`runner::run_eval`]. |
//! | [`evaluators`] | Rules, golden, process, and LLM judge evaluators. |
//! | [`fixtures`] | Load and validate fixtures; [`fixtures::MockEngine`] for stubbed tool/LLM responses. |
//! | [`reporters`] | Terminal, JSON, JUnit, history persistence, and run diffs. |
//!
//! # Quick start (CLI)
//!
//! ```text
//! agentcarousel validate path/to/fixture.yaml
//! agentcarousel test path/to/fixture.yaml --offline true
//! ```
//!
//! Install and full options are described in the repository README and on
//! [docs.rs](https://docs.rs/agentcarousel) for this crate version.
//!
//! # Library quick start
//!
//! Typical flow: load fixtures with [`fixtures::load_fixture`], build [`runner::RunnerConfig`]
//! or [`runner::EvalConfig`], then call [`runner::run_fixtures`] or [`runner::run_eval`]
//! inside a [`tokio`] runtime. See [`runner`] for configuration fields.
//!
//! [`Run`]: crate::core::Run
//! [`Case`]: crate::core::Case
//! [`FixtureFile`]: crate::core::FixtureFile
//! [`tokio`]: https://docs.rs/tokio

pub mod cli;
pub mod core;
pub mod evaluators;
pub mod fixtures;
pub mod reporters;
pub mod runner;

pub use cli::*;
pub use core::*;
pub use evaluators::*;
pub use fixtures::*;
pub use reporters::*;
pub use runner::*;

extern crate self as agentcarousel_cli;
extern crate self as agentcarousel_core;
extern crate self as agentcarousel_evaluators;
extern crate self as agentcarousel_fixtures;
extern crate self as agentcarousel_reporters;
extern crate self as agentcarousel_runner;
