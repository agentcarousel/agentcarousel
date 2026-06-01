use crate::common::{BackMatter, Link, Metadata, Property};
use serde::{Deserialize, Serialize};

/// Top-level OSCAL Catalog — an authoritative collection of controls for a framework.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct CatalogDocument {
    pub catalog: Catalog,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Catalog {
    pub uuid: String,
    pub metadata: Metadata,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub groups: Vec<Group>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub controls: Vec<Control>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub back_matter: Option<BackMatter>,
}

impl Catalog {
    /// Returns every control in the catalog, flattening groups and nested enhancements.
    pub fn all_controls(&self) -> Vec<&Control> {
        let mut out = Vec::new();
        for g in &self.groups {
            g.collect_controls(&mut out);
        }
        for c in &self.controls {
            c.collect_all(&mut out);
        }
        out
    }
}

/// A logical grouping of controls (e.g. "Access Control" family in NIST 800-53).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Group {
    pub id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub props: Vec<Property>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub links: Vec<Link>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parts: Vec<Part>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub groups: Vec<Group>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub controls: Vec<Control>,
}

impl Group {
    fn collect_controls<'a>(&'a self, out: &mut Vec<&'a Control>) {
        for c in &self.controls {
            c.collect_all(out);
        }
        for g in &self.groups {
            g.collect_controls(out);
        }
    }
}

/// A single compliance control.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Control {
    pub id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub props: Vec<Property>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub links: Vec<Link>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parts: Vec<Part>,
    /// Control enhancements — nested controls within a parent control.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub controls: Vec<Control>,
}

impl Control {
    fn collect_all<'a>(&'a self, out: &mut Vec<&'a Control>) {
        out.push(self);
        for c in &self.controls {
            c.collect_all(out);
        }
    }

    /// Returns the statement text from the control's `statement` part, if present.
    pub fn statement(&self) -> Option<&str> {
        self.parts
            .iter()
            .find(|p| p.name == "statement")
            .and_then(|p| p.prose.as_deref())
    }
}

/// A structured prose block within a control (statement, guidance, assessment, etc.).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Part {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prose: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parts: Vec<Part>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub props: Vec<Property>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub links: Vec<Link>,
}

/// Source for loading an OSCAL catalog.
pub enum CatalogSource<'a> {
    /// One of the community catalogs embedded in the `catalogs` module.
    Embedded(&'a str),
    /// Raw JSON string.
    Json(&'a str),
}

/// Parse a [`Catalog`] from a [`CatalogSource`].
///
/// Returns an error string if parsing fails.
pub fn load_catalog(source: CatalogSource<'_>) -> Result<Catalog, String> {
    let json = match source {
        CatalogSource::Embedded(name) => crate::catalogs::embedded_catalog_json(name)
            .ok_or_else(|| format!("no embedded catalog named '{name}'"))?,
        CatalogSource::Json(s) => s,
    };
    let doc: CatalogDocument =
        serde_json::from_str(json).map_err(|e| format!("OSCAL catalog parse error: {e}"))?;
    Ok(doc.catalog)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_catalogs_parse() {
        for name in crate::catalogs::EMBEDDED_CATALOG_NAMES {
            let result = load_catalog(CatalogSource::Embedded(name));
            assert!(
                result.is_ok(),
                "failed to parse embedded catalog '{name}': {:?}",
                result.err()
            );
            let catalog = result.unwrap();
            assert!(
                !catalog.all_controls().is_empty(),
                "catalog '{name}' has no controls"
            );
        }
    }
}
