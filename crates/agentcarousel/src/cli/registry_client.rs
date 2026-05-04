use reqwest::blocking::{multipart, Client};
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use reqwest::Url;
use serde_json::Value;
use std::path::Path;
use std::time::Duration;

use super::config::ResolvedConfig;

pub struct RegistryClient {
    base_url: String,
    token: String,
    http: Client,
}

impl RegistryClient {
    pub fn new(base_url: &str, token: &str) -> Result<Self, String> {
        let http = Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .map_err(|err| format!("failed to construct HTTP client: {err}"))?;
        Ok(Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            token: token.to_string(),
            http,
        })
    }

    pub fn push_bundle_manifest(&self, manifest: &Value) -> Result<Value, String> {
        let url = format!("{}/v1/bundles", self.base_url);
        let res = self
            .http
            .post(url)
            .header(AUTHORIZATION, format!("Bearer {}", self.token))
            .header(CONTENT_TYPE, "application/json")
            .json(manifest)
            .send()
            .map_err(|err| format!("request failed: {err}"))?;
        parse_json_response("bundle push", res)
    }

    pub fn submit_run_evidence(
        &self,
        registry_bundle_id: &str,
        evidence_path: &Path,
    ) -> Result<Value, String> {
        let url = format!("{}/v1/runs", self.base_url);
        let form = multipart::Form::new()
            .text("registry_bundle_id", registry_bundle_id.to_string())
            .file("evidence", evidence_path)
            .map_err(|err| format!("failed to attach evidence file: {err}"))?;
        let res = self
            .http
            .post(url)
            .header(AUTHORIZATION, format!("Bearer {}", self.token))
            .multipart(form)
            .send()
            .map_err(|err| format!("request failed: {err}"))?;
        parse_json_response("submit run", res)
    }

    pub fn get_trust_state(&self, bundle_id: &str) -> Result<Value, String> {
        let encoded_bundle_id = encode_registry_bundle_id(bundle_id);
        let url = format!(
            "{}/v1/bundles/{}/trust-state",
            self.base_url, encoded_bundle_id
        );
        let req = self.http.get(url).header(CONTENT_TYPE, "application/json");
        let req = if self.token.trim().is_empty() {
            req
        } else {
            req.header(AUTHORIZATION, format!("Bearer {}", self.token))
        };
        let res = req.send().map_err(|err| format!("request failed: {err}"))?;
        parse_json_response("trust check", res)
    }

    /// Fetch `bundle.manifest.json` for a registry bundle id (same id used by `publish` dry-run).
    pub fn get_bundle_manifest(&self, registry_bundle_id: &str) -> Result<Value, String> {
        let encoded = encode_registry_bundle_id(registry_bundle_id);
        let url = format!("{}/v1/bundles/{}/manifest", self.base_url, encoded);
        let req = self.http.get(url).header(CONTENT_TYPE, "application/json");
        let req = if self.token.trim().is_empty() {
            req
        } else {
            req.header(AUTHORIZATION, format!("Bearer {}", self.token))
        };
        let res = req.send().map_err(|err| format!("request failed: {err}"))?;
        parse_json_response("bundle manifest", res)
    }

    /// Fetch a single artifact listed in the manifest (`fixtures` / `mocks` `path` field).
    pub fn get_bundle_file(&self, registry_bundle_id: &str, path: &str) -> Result<Vec<u8>, String> {
        let encoded = encode_registry_bundle_id(registry_bundle_id);
        let mut url = Url::parse(&format!("{}/v1/bundles/{}/file", self.base_url, encoded))
            .map_err(|err| format!("invalid registry URL: {err}"))?;
        url.query_pairs_mut().append_pair("path", path);
        let req = self
            .http
            .get(url)
            .header(CONTENT_TYPE, "application/octet-stream");
        let req = if self.token.trim().is_empty() {
            req
        } else {
            req.header(AUTHORIZATION, format!("Bearer {}", self.token))
        };
        let res = req.send().map_err(|err| format!("request failed: {err}"))?;
        let status = res.status();
        if status.is_success() {
            return res
                .bytes()
                .map(|b| b.to_vec())
                .map_err(|err| err.to_string());
        }
        let body = res.text().unwrap_or_default();
        let guidance = match status.as_u16() {
            401 => "Unauthorized: check AGENTCAROUSEL_API_TOKEN.",
            404 => "Not found: verify registry bundle id, path, and endpoint URL.",
            _ => "Registry request failed.",
        };
        if body.trim().is_empty() {
            Err(format!("bundle file fetch failed ({status}): {guidance}"))
        } else {
            Err(format!(
                "bundle file fetch failed ({status}): {guidance} body={body}"
            ))
        }
    }
}

fn encode_registry_bundle_id(bundle_id: &str) -> String {
    bundle_id.replace('/', "%2F")
}

/// Resolve registry base URL from CLI flag, config, or environment (same rules as `publish`).
pub fn resolve_registry_url(
    registry_url: Option<&str>,
    config: &ResolvedConfig,
) -> Result<String, String> {
    if let Some(url) = registry_url {
        let trimmed = url.trim();
        if !trimmed.is_empty() {
            return Ok(trimmed.to_string());
        }
    }
    if let Some(url) = &config.msp.registry_endpoint {
        let trimmed = url.trim();
        if !trimmed.is_empty() {
            return Ok(trimmed.to_string());
        }
    }
    for key in ["REGISTRY_API_BASE_URL", "REGISTRY_URL"] {
        if let Ok(value) = std::env::var(key) {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                return Ok(trimmed.to_string());
            }
        }
    }
    Err("registry URL is required: pass --url/--registry-url, set msp.registry_endpoint in config, or export REGISTRY_API_BASE_URL".to_string())
}

fn parse_json_response(step: &str, res: reqwest::blocking::Response) -> Result<Value, String> {
    let status = res.status();
    let body = res.text().unwrap_or_default();
    if status.is_success() {
        return serde_json::from_str::<Value>(&body)
            .map_err(|err| format!("{step} succeeded but response was not JSON: {err}"));
    }
    if status.as_u16() == 409 {
        let parsed = serde_json::from_str::<Value>(&body).unwrap_or(Value::Null);
        let payload = if parsed.is_null() {
            serde_json::json!({
                "duplicate": true,
                "status": "already_uploaded",
                "message": "run was already submitted; no new trust-state mutation"
            })
        } else {
            serde_json::json!({
                "duplicate": true,
                "status": "already_uploaded",
                "registry_response": parsed
            })
        };
        return Ok(payload);
    }

    let guidance = match status.as_u16() {
        401 => "Unauthorized: check AGENTCAROUSEL_API_TOKEN and Authorization Bearer value.",
        404 => "Not found: verify registry bundle id and endpoint URL.",
        415 => "Unsupported media type: registry expects multipart/form-data with evidence file.",
        _ => "Registry request failed.",
    };
    if body.trim().is_empty() {
        Err(format!("{step} failed ({status}): {guidance}"))
    } else {
        Err(format!("{step} failed ({status}): {guidance} body={body}"))
    }
}
