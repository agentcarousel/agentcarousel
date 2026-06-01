//! Serde types for OSCAL (Open Security Controls Assessment Language).
//!
//! Covers three models needed for compliance attestation:
//! - [`catalog`] — authoritative control definitions for a framework
//! - [`component`] — how a software component (skill) implements controls
//! - [`assessment_results`] — findings from evaluating controls against evidence
//!
//! Community OSCAL catalogs for AI governance and security frameworks are
//! embedded in the [`catalogs`] module and loaded via [`CatalogSource::Embedded`].

pub mod assessment_results;
pub mod catalog;
pub mod catalogs;
pub mod common;
pub mod component;

pub use assessment_results::{
    AssessmentLog, AssessmentResults, AssessmentRisk, Finding, LogEntry, Observation,
    RiskRemediation,
};
pub use catalog::{Catalog, CatalogSource, Control, Group};
pub use component::ComponentDefinition;
