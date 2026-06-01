use crate::common::{BackMatter, Link, Metadata, Property, ResponsibleParty};
use serde::{Deserialize, Serialize};

/// Top-level OSCAL Component Definition document.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct ComponentDefinitionDocument {
    pub component_definition: ComponentDefinition,
}

/// Describes how one or more software components implement compliance controls.
///
/// In the AgentCarousel context a skill's fixture suite is represented as a
/// [`DefinedComponent`] of `type = "validation"`, with each compliance tag
/// appearing as an [`ImplementedRequirement`].
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct ComponentDefinition {
    pub uuid: String,
    pub metadata: Metadata,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub components: Vec<DefinedComponent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub back_matter: Option<BackMatter>,
}

/// A single software component and its control implementations.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct DefinedComponent {
    pub uuid: String,
    #[serde(rename = "type")]
    pub component_type: String,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub props: Vec<Property>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub links: Vec<Link>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub responsible_roles: Vec<ResponsibleParty>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub control_implementations: Vec<ControlImplementation>,
}

/// A set of implemented requirements referencing a specific framework catalog.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct ControlImplementation {
    pub uuid: String,
    pub source: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub props: Vec<Property>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub implemented_requirements: Vec<ImplementedRequirement>,
}

/// States that a component satisfies (or partially satisfies) a specific control.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct ImplementedRequirement {
    pub uuid: String,
    pub control_id: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub props: Vec<Property>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub links: Vec<Link>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub statements: Vec<ControlStatement>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct ControlStatement {
    pub statement_id: String,
    pub uuid: String,
    pub description: String,
}
