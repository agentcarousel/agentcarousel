use clap::{Parser, Subcommand};
use flate2::write::GzEncoder;
use flate2::Compression;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use tar::Builder;

use super::config::ResolvedConfig;
use super::exit_codes::ExitCode;
use super::registry_client::{resolve_registry_url, RegistryClient};
use super::GlobalOptions;

/// Pack, verify, or pull fixture bundles (manifest + tarball).
#[derive(Debug, Parser)]
pub struct BundleArgs {
    #[command(subcommand)]
    command: BundleCommand,
}

#[derive(Debug, Subcommand)]
enum BundleCommand {
    /// Update manifest sha256s and write a .tar.gz (default name from dir).
    Pack {
        /// Directory containing bundle.manifest.json (default: current directory).
        #[arg(value_name = "DIR", default_value = ".")]
        dir: PathBuf,
        #[arg(short = 'o', long)]
        out: Option<PathBuf>,
    },
    /// Verify hashes vs files (bundle dir, path to bundle.manifest.json, or .tar.gz).
    Verify { path: Option<PathBuf> },
    /// Download bundle manifest and artifacts from a registry (`GET /v1/bundles/{id}/manifest` + `/file?path=...`).
    Pull {
        /// Registry bundle id (e.g. `cmmc-assessor-1.0.0`), matching `publish --dry-run` `registry_bundle_id`.
        #[arg(value_name = "REGISTRY_BUNDLE_ID")]
        registry_bundle_id: String,
        /// Registry API URL (alias: --registry-url). Falls back to config/env.
        #[arg(long = "url", alias = "registry-url")]
        url: Option<String>,
        /// Output directory (default: `pulled-bundles/<id>` with `/` replaced by `-`).
        #[arg(short = 'o', long)]
        out: Option<PathBuf>,
        /// Run `bundle verify` on the output directory after download.
        #[arg(long, default_value_t = false)]
        verify: bool,
    },
}

pub fn run_bundle(args: BundleArgs, config: &ResolvedConfig, _globals: &GlobalOptions) -> i32 {
    match args.command {
        BundleCommand::Pack { dir, out } => match pack_bundle(&dir, out.as_deref()) {
            Ok(path) => {
                println!("created {}", path.display());
                ExitCode::Ok.as_i32()
            }
            Err(err) => {
                eprintln!("error: {err}");
                ExitCode::RuntimeError.as_i32()
            }
        },
        BundleCommand::Verify { path } => match verify_bundle(path.as_deref()) {
            Ok(resolved) => {
                println!("bundle verify: OK ({})", resolved.display());
                ExitCode::Ok.as_i32()
            }
            Err(err) => {
                eprintln!("error: {err}");
                ExitCode::RuntimeError.as_i32()
            }
        },
        BundleCommand::Pull {
            registry_bundle_id,
            url,
            out,
            verify,
        } => match pull_bundle(
            &registry_bundle_id,
            url.as_deref(),
            out.as_deref(),
            verify,
            config,
        ) {
            Ok(dir) => {
                println!("bundle pull: wrote {}", dir.display());
                ExitCode::Ok.as_i32()
            }
            Err(err) => {
                eprintln!("error: {err}");
                ExitCode::RuntimeError.as_i32()
            }
        },
    }
}

fn pull_bundle(
    registry_bundle_id: &str,
    url: Option<&str>,
    out: Option<&Path>,
    verify: bool,
    config: &ResolvedConfig,
) -> Result<PathBuf, String> {
    let id = registry_bundle_id.trim();
    if id.is_empty() {
        return Err("registry bundle id is required".to_string());
    }
    let endpoint = resolve_registry_url(url, config)?;
    let token = std::env::var("AGENTCAROUSEL_API_TOKEN")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_default();
    let client = RegistryClient::new(&endpoint, &token)?;
    let manifest = client.get_bundle_manifest(id)?;

    let out_dir = out.map(Path::to_path_buf).unwrap_or_else(|| {
        let safe = id.replace('/', "-");
        PathBuf::from("pulled-bundles").join(safe)
    });
    fs::create_dir_all(&out_dir).map_err(|err| err.to_string())?;

    let manifest_path = out_dir.join("bundle.manifest.json");
    let rendered = serde_json::to_string_pretty(&manifest).map_err(|err| err.to_string())?;
    fs::write(&manifest_path, rendered.as_bytes()).map_err(|err| err.to_string())?;

    pull_manifest_entries(&client, id, &manifest, "fixtures", &out_dir)?;
    pull_manifest_entries(&client, id, &manifest, "mocks", &out_dir)?;

    if verify {
        verify_bundle(Some(&out_dir))?;
    }
    Ok(out_dir)
}

fn pull_manifest_entries(
    client: &RegistryClient,
    registry_bundle_id: &str,
    manifest: &Value,
    field: &str,
    bundle_root: &Path,
) -> Result<(), String> {
    let Some(entries) = manifest.get(field).and_then(|value| value.as_array()) else {
        return Ok(());
    };
    for entry in entries {
        let Some(path_value) = entry.get("path").and_then(|value| value.as_str()) else {
            return Err(format!("{field} entry missing path"));
        };
        let bytes = client.get_bundle_file(registry_bundle_id, path_value)?;
        let dest = resolve_bundle_artifact_path(bundle_root, path_value)?;
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent).map_err(|err| err.to_string())?;
        }
        fs::write(&dest, bytes).map_err(|err| {
            format!(
                "failed to write {} (from manifest path {path_value}): {err}",
                dest.display()
            )
        })?;
    }
    Ok(())
}

/// Resolve a manifest `path` relative to the bundle directory (supports `..` like checked-in bundles).
fn resolve_bundle_artifact_path(bundle_root: &Path, relative: &str) -> Result<PathBuf, String> {
    let mut out = bundle_root.to_path_buf();
    for component in Path::new(relative).components() {
        match component {
            Component::Normal(part) => out.push(part),
            Component::ParentDir => {
                if !out.pop() {
                    return Err(format!(
                        "manifest path `{relative}` has too many `..` segments for output root {}",
                        bundle_root.display()
                    ));
                }
            }
            Component::CurDir => {}
            Component::RootDir | Component::Prefix(_) => {
                return Err(format!(
                    "manifest path `{relative}` must be relative (no root or prefix components)"
                ));
            }
        }
    }
    if out.as_os_str().is_empty() {
        return Err(format!(
            "manifest path `{relative}` resolved to an empty path"
        ));
    }
    Ok(out)
}

fn pack_bundle(dir: &Path, out: Option<&Path>) -> Result<PathBuf, String> {
    let dir = dir.canonicalize().map_err(|err| err.to_string())?;
    let manifest_path = dir.join("bundle.manifest.json");
    if manifest_path.exists() {
        // Update sha256 entries so bundle contents are self-consistent.
        update_manifest_hashes(&manifest_path, &dir)?;
    } else {
        return Err("bundle.manifest.json not found".to_string());
    }

    let out_path = out.map(|path| path.to_path_buf()).unwrap_or_else(|| {
        let dir_name = dir
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("bundle");
        PathBuf::from(format!("{dir_name}.tar.gz"))
    });

    let archive = fs::File::create(&out_path).map_err(|err| err.to_string())?;
    let encoder = GzEncoder::new(archive, Compression::default());
    let mut tar = Builder::new(encoder);
    tar.append_dir_all(".", &dir)
        .map_err(|err| err.to_string())?;
    tar.finish().map_err(|err| err.to_string())?;
    Ok(out_path)
}

fn is_gzip_tarball(path: &Path) -> bool {
    let s = path.to_string_lossy();
    s.ends_with(".tar.gz") || s.ends_with(".tgz")
}

/// Returns the bundle root or archive path that was verified (for user feedback).
fn verify_bundle(path: Option<&Path>) -> Result<PathBuf, String> {
    let path = path.unwrap_or_else(|| Path::new("."));
    if path.is_file() {
        let file_name = path.file_name().and_then(|name| name.to_str());
        if file_name == Some("bundle.manifest.json") {
            let root = path
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
                .unwrap_or(Path::new("."));
            verify_manifest(path, root)?;
            return Ok(root.to_path_buf());
        }
        if is_gzip_tarball(path) {
            let tmp_dir =
                std::env::temp_dir().join(format!("agentcarousel-bundle-{}", std::process::id()));
            if tmp_dir.exists() {
                fs::remove_dir_all(&tmp_dir).map_err(|err| err.to_string())?;
            }
            fs::create_dir_all(&tmp_dir).map_err(|err| err.to_string())?;
            let archive = fs::File::open(path).map_err(|err| err.to_string())?;
            let decoder = flate2::read::GzDecoder::new(archive);
            let mut tar = tar::Archive::new(decoder);
            tar.unpack(&tmp_dir).map_err(|err| err.to_string())?;
            let manifest_path = tmp_dir.join("bundle.manifest.json");
            verify_manifest(&manifest_path, &tmp_dir)?;
            fs::remove_dir_all(&tmp_dir).ok();
            return Ok(path.to_path_buf());
        }
        return Err(format!(
            "expected a bundle directory, bundle.manifest.json, or a .tar.gz archive; got {}",
            path.display()
        ));
    }
    let manifest_path = path.join("bundle.manifest.json");
    verify_manifest(&manifest_path, path)?;
    Ok(path.to_path_buf())
}

fn update_manifest_hashes(manifest_path: &Path, root: &Path) -> Result<(), String> {
    let contents = fs::read_to_string(manifest_path).map_err(|err| err.to_string())?;
    let mut manifest: Value = serde_json::from_str(&contents).map_err(|err| err.to_string())?;
    update_entries(&mut manifest, "fixtures", root)?;
    update_entries(&mut manifest, "mocks", root)?;
    let rendered = serde_json::to_string_pretty(&manifest).map_err(|err| err.to_string())?;
    let mut file = fs::File::create(manifest_path).map_err(|err| err.to_string())?;
    file.write_all(rendered.as_bytes())
        .map_err(|err| err.to_string())?;
    Ok(())
}

fn update_entries(manifest: &mut Value, field: &str, root: &Path) -> Result<(), String> {
    let Some(entries) = manifest
        .get_mut(field)
        .and_then(|value| value.as_array_mut())
    else {
        return Ok(());
    };
    for entry in entries {
        let Some(path_value) = entry.get("path").and_then(|value| value.as_str()) else {
            return Err(format!("{field} entry missing path"));
        };
        let file_path = root.join(path_value);
        let hash = sha256_file(&file_path)?;
        entry["sha256"] = Value::String(hash);
    }
    Ok(())
}

fn verify_manifest(manifest_path: &Path, root: &Path) -> Result<(), String> {
    if !manifest_path.exists() {
        return Err("bundle.manifest.json not found".to_string());
    }
    let contents = fs::read_to_string(manifest_path).map_err(|err| err.to_string())?;
    let manifest: Value = serde_json::from_str(&contents).map_err(|err| err.to_string())?;
    verify_entries(&manifest, "fixtures", root)?;
    verify_entries(&manifest, "mocks", root)?;
    Ok(())
}

fn verify_entries(manifest: &Value, field: &str, root: &Path) -> Result<(), String> {
    let Some(entries) = manifest.get(field).and_then(|value| value.as_array()) else {
        return Ok(());
    };
    for entry in entries {
        let Some(path_value) = entry.get("path").and_then(|value| value.as_str()) else {
            return Err(format!("{field} entry missing path"));
        };
        let Some(expected) = entry.get("sha256").and_then(|value| value.as_str()) else {
            return Err(format!("{field} entry missing sha256 for {path_value}"));
        };
        let file_path = root.join(path_value);
        let actual = sha256_file(&file_path)?;
        if actual != expected {
            return Err(format!(
                "{field} hash mismatch for {path_value}: expected {expected}, got {actual}"
            ));
        }
    }
    Ok(())
}

fn sha256_file(path: &Path) -> Result<String, String> {
    let contents = fs::read(path).map_err(|err| err.to_string())?;
    let mut hasher = Sha256::new();
    hasher.update(contents);
    Ok(format!("{:x}", hasher.finalize()))
}
