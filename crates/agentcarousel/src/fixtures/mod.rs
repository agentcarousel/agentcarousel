//! Fixture **I/O** and **validation**: load YAML/TOML into [`crate::FixtureFile`], validate
//! against the bundled JSON Schema, and resolve tool/LLM responses via [`MockEngine`].

mod loader;
mod mock;
mod schema;

pub use loader::{load_fixture, load_fixture_value, FixtureLoadError, FixtureSource};
pub use mock::{MockEngine, MockError, MockStub};
pub use schema::{validate_fixture_value, SchemaLocation, SchemaValidationIssue};
