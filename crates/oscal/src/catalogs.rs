/// Names of all OSCAL catalogs embedded in this crate.
///
/// Official NIST catalogs sourced from usnistgov/oscal-content:
/// - `nist-800-53`  — SP 800-53 Rev 5.2.0 (all 20 control families)
/// - `nist-800-171` — SP 800-171 Rev 3 (CUI protection, 110 requirements; CMMC Level 2)
/// - `nist-800-172` — SP 800-172 Rev 3 (enhanced CUI requirements; CMMC Level 3)
///
/// Community-authored catalogs for AI governance and regulated industries:
/// - `eu-ai-act`, `nist-ai-rmf`, `iso-42001`, `hipaa`, `fda-samd`, `nist-800-207`
pub const EMBEDDED_CATALOG_NAMES: &[&str] = &[
    "eu-ai-act",
    "nist-ai-rmf",
    "iso-42001",
    "hipaa",
    "fda-samd",
    "nist-800-207",
    "nist-800-53",
    "nist-800-171",
    "nist-800-172",
];

/// Returns the raw JSON string for an embedded catalog by name, or `None` if unknown.
pub fn embedded_catalog_json(name: &str) -> Option<&'static str> {
    match name {
        "eu-ai-act" => Some(include_str!("../catalogs/eu-ai-act.json")),
        "nist-ai-rmf" => Some(include_str!("../catalogs/nist-ai-rmf.json")),
        "iso-42001" => Some(include_str!("../catalogs/iso-42001.json")),
        "hipaa" => Some(include_str!("../catalogs/hipaa.json")),
        "fda-samd" => Some(include_str!("../catalogs/fda-samd.json")),
        "nist-800-207" => Some(include_str!("../catalogs/nist-800-207.json")),
        // Official NIST catalogs (usnistgov/oscal-content)
        "nist-800-53" => Some(include_str!("../catalogs/nist-800-53.json")),
        "nist-800-171" => Some(include_str!("../catalogs/nist-800-171.json")),
        "nist-800-172" => Some(include_str!("../catalogs/nist-800-172.json")),
        _ => None,
    }
}
