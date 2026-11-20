use clap::{Parser, Subcommand};
use flate2::write::GzEncoder;
use flate2::Compression;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fs;
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use tar::Builder;

use crate::core::hex_util::hex_lower;

use super::config::ResolvedConfig;
use super::exit_codes::ExitCode;
use super::output::{JsonError, JsonOutput};
use super::registry_client::{resolve_registry_url, RegistryClient};
use super::GlobalOptions;

/// Package fixture suites for sharing, verify their integrity, or pull them from a registry.
///
/// Bundles let you distribute a versioned, signed set of fixtures to teammates or CI. Use `pack` to create a bundle archive, `verify` to check its integrity, and `pull` to download a bundle from a remote registry.
#[derive(Debug, Parser)]
#[command(
    after_help = "Examples:\n  agc bundle pack fixtures/customer-support          # create a versioned bundle archive\n  agc bundle verify fixtures/customer-support        # verify bundle integrity\n  agc bundle pull customer-support-1.0.0 --url https://registry.example.com"
)]
pub struct BundleArgs {
    /// Config file path (default: agentcarousel.toml in the current directory).
    #[arg(long)]
    pub config: Option<PathBuf>,
    #[command(subcommand)]
    command: BundleCommand,
}

#[derive(Debug, Subcommand)]
enum BundleCommand {
    /// Update manifest sha256s and write a .tar.gz to bundle_packs/ (or -o path).
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
        /// Registry bundle id (e.g. `customer-support-1.0.0`), matching `publish --dry-run` `registry_bundle_id`.
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

pub fn run_bundle(args: BundleArgs, config: &ResolvedConfig, globals: &GlobalOptions) -> i32 {
    match args.command {
        BundleCommand::Pack { dir, out } => match pack_bundle(&dir, out.as_deref()) {
            Ok(path) => {
                if globals.json {
                    JsonOutput::ok(
                        "bundle pack",
                        serde_json::json!({ "path": path.display().to_string() }),
                    )
                    .print();
                } else {
                    println!("created {}", path.display());
                }
                ExitCode::Ok.as_i32()
            }
            Err(err) => {
                if globals.json {
                    JsonOutput::err("bundle pack", JsonError::new("runtime_error", err)).print();
                } else {
                    eprintln!("error: {err}");
                }
                ExitCode::RuntimeError.as_i32()
            }
        },
        BundleCommand::Verify { path } => match verify_bundle(path.as_deref()) {
            Ok(resolved) => {
                if globals.json {
                    JsonOutput::ok(
                        "bundle verify",
                        serde_json::json!({ "path": resolved.display().to_string(), "ok": true }),
                    )
                    .print();
                } else {
                    println!("bundle verify: OK ({})", resolved.display());
                }
                ExitCode::Ok.as_i32()
            }
            Err(err) => {
                if globals.json {
                    JsonOutput::err("bundle verify", JsonError::new("verification_failed", err))
                        .print();
                } else {
                    eprintln!("error: {err}");
                }
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
                if globals.json {
                    JsonOutput::ok(
                        "bundle pull",
                        serde_json::json!({ "path": dir.display().to_string() }),
                    )
                    .print();
                } else {
                    println!("bundle pull: wrote {}", dir.display());
                }
                ExitCode::Ok.as_i32()
            }
            Err(err) => {
                if globals.json {
                    JsonOutput::err("bundle pull", JsonError::new("runtime_error", err)).print();
                } else {
                    eprintln!("error: {err}");
                }
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
        let dest = resolve_bundle_artifact_path(bundle_root, path_value)?;
        ensure_no_symlinks_below_root(bundle_root, &dest)?;
        let bytes = client.get_bundle_file(registry_bundle_id, path_value)?;
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

/// Resolve a manifest `path` strictly inside `bundle_root`.
///
/// Manifest paths come from untrusted places (registry manifests, downloaded
/// archives), so only plain relative components are accepted. `..`, absolute
/// paths, Windows prefixes and backslash separators are rejected instead of
/// being normalized, so a manifest can never name a file outside the bundle.
fn resolve_bundle_artifact_path(bundle_root: &Path, relative: &str) -> Result<PathBuf, String> {
    if relative.contains('\\') || relative.contains('\0') {
        return Err(format!(
            "manifest path `{relative}` must use `/` separators and no NUL bytes"
        ));
    }
    let mut out = bundle_root.to_path_buf();
    let mut named = false;
    for component in Path::new(relative).components() {
        match component {
            Component::Normal(part) => {
                out.push(part);
                named = true;
            }
            Component::CurDir => {}
            Component::ParentDir => {
                return Err(format!(
                    "manifest path `{relative}` must not contain `..` (paths stay inside the bundle)"
                ));
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(format!(
                    "manifest path `{relative}` must be relative (no root or prefix components)"
                ));
            }
        }
    }
    if !named {
        return Err(format!("manifest path `{relative}` does not name a file"));
    }
    Ok(out)
}

/// Refuse to write through a symlink that already exists between `bundle_root`
/// and `dest` (for example one checked into a cloned repository).
fn ensure_no_symlinks_below_root(bundle_root: &Path, dest: &Path) -> Result<(), String> {
    let Ok(relative) = dest.strip_prefix(bundle_root) else {
        return Err(format!(
            "refusing to write {}: outside bundle root {}",
            dest.display(),
            bundle_root.display()
        ));
    };
    let mut current = bundle_root.to_path_buf();
    for component in relative.components() {
        current.push(component);
        if let Ok(meta) = fs::symlink_metadata(&current) {
            if meta.file_type().is_symlink() {
                return Err(format!(
                    "refusing to write through symlink {}",
                    current.display()
                ));
            }
        }
    }
    Ok(())
}

fn pack_bundle(dir: &Path, out: Option<&Path>) -> Result<PathBuf, String> {
    let dir = dir.canonicalize().map_err(|err| err.to_string())?;
    let manifest_path = dir.join("bundle.manifest.json");
    if manifest_path.exists() {
        update_manifest_hashes(&manifest_path, &dir)?;
    } else {
        return Err("bundle.manifest.json not found".to_string());
    }

    let out_path = out.map(|path| path.to_path_buf()).unwrap_or_else(|| {
        let dir_name = dir
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("bundle");
        let bundle_packs = PathBuf::from("bundle_packs");
        fs::create_dir_all(&bundle_packs).ok();
        bundle_packs.join(format!("{dir_name}.tar.gz"))
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
            // Hardened extraction: regular files and directories only, nothing
            // outside the temp dir; the temp dir is removed on every return path.
            let tmp = tempfile::tempdir().map_err(|err| err.to_string())?;
            super::safe_extract::extract_tar_gz(path, tmp.path()).map_err(|err| err.to_string())?;
            let manifest_path = tmp.path().join("bundle.manifest.json");
            verify_manifest(&manifest_path, tmp.path())?;
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
        let file_path = resolve_bundle_artifact_path(root, path_value)?;
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
    let mut listed = BTreeSet::new();
    verify_entries(&manifest, "fixtures", root, &mut listed)?;
    verify_entries(&manifest, "mocks", root, &mut listed)?;
    if listed.is_empty() {
        return Err(
            "bundle.manifest.json lists no fixtures or mocks with hashes; nothing to verify"
                .to_string(),
        );
    }
    let unlisted = unlisted_files(root, &listed);
    if !unlisted.is_empty() {
        let shown: Vec<String> = unlisted
            .iter()
            .take(10)
            .map(|path| path.display().to_string())
            .collect();
        let more = unlisted.len().saturating_sub(shown.len());
        eprintln!(
            "warning: {} file(s) in the bundle are not covered by bundle.manifest.json and were not verified: {}{}",
            unlisted.len(),
            shown.join(", "),
            if more > 0 { format!(" (+{more} more)") } else { String::new() }
        );
    }
    Ok(())
}

fn verify_entries(
    manifest: &Value,
    field: &str,
    root: &Path,
    listed: &mut BTreeSet<PathBuf>,
) -> Result<(), String> {
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
        let file_path = resolve_bundle_artifact_path(root, path_value)?;
        let actual = sha256_file(&file_path)?;
        if actual != expected {
            return Err(format!(
                "{field} hash mismatch for {path_value}: expected {expected}, got {actual}"
            ));
        }
        if let Ok(relative) = file_path.strip_prefix(root) {
            listed.insert(relative.to_path_buf());
        }
    }
    Ok(())
}

/// Regular files under `root` that the manifest does not cover (ignoring the
/// manifest itself, signatures and dotfiles). Reported, not fatal: `bundle pack`
/// archives the whole directory, so packs can legitimately carry extra files.
fn unlisted_files(root: &Path, listed: &BTreeSet<PathBuf>) -> Vec<PathBuf> {
    walkdir::WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .filter_map(|entry| entry.path().strip_prefix(root).ok().map(Path::to_path_buf))
        .filter(|relative| {
            let name = relative
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default();
            name != "bundle.manifest.json"
                && !name.ends_with(".minisig")
                && !relative
                    .components()
                    .any(|c| c.as_os_str().to_string_lossy().starts_with('.'))
        })
        .filter(|relative| !listed.contains(relative))
        .collect()
}

fn sha256_file(path: &Path) -> Result<String, String> {
    let contents = fs::read(path).map_err(|err| err.to_string())?;
    let mut hasher = Sha256::new();
    hasher.update(contents);
    Ok(hex_lower(hasher.finalize().as_ref()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_paths_resolve_inside_root() {
        let root = Path::new("pulled-bundles/demo");
        assert_eq!(
            resolve_bundle_artifact_path(root, "cases.yaml").unwrap(),
            root.join("cases.yaml")
        );
        assert_eq!(
            resolve_bundle_artifact_path(root, "./golden/x.txt").unwrap(),
            root.join("golden").join("x.txt")
        );
    }

    #[test]
    fn manifest_paths_that_escape_are_rejected() {
        let root = Path::new("pulled-bundles/demo");
        for bad in [
            "../x",
            "a/../../b",
            "../../.git/hooks/pre-commit",
            "/etc/passwd",
            "C:\\x",
            "..\\..\\x",
            "",
            ".",
            "a\0b",
        ] {
            assert!(
                resolve_bundle_artifact_path(root, bad).is_err(),
                "expected `{bad}` to be rejected"
            );
        }
    }

    #[test]
    fn verify_rejects_manifest_path_traversal() {
        let dir = tempfile::tempdir().unwrap();
        let bundle = dir.path().join("bundle");
        fs::create_dir_all(&bundle).unwrap();
        fs::write(dir.path().join("outside.txt"), b"secret").unwrap();
        fs::write(
            bundle.join("bundle.manifest.json"),
            serde_json::json!({
                "fixtures": [{ "path": "../outside.txt", "sha256": "00" }]
            })
            .to_string(),
        )
        .unwrap();
        let err = verify_bundle(Some(&bundle)).unwrap_err();
        assert!(err.contains("must not contain `..`"), "{err}");
    }

    fn write_bundle(dir: &Path) {
        fs::create_dir_all(dir).unwrap();
        fs::write(dir.join("cases.yaml"), b"cases: []\n").unwrap();
        fs::write(
            dir.join("bundle.manifest.json"),
            serde_json::json!({ "fixtures": [{ "path": "cases.yaml", "sha256": "" }] }).to_string(),
        )
        .unwrap();
        update_manifest_hashes(&dir.join("bundle.manifest.json"), dir).unwrap();
    }

    #[test]
    fn packed_bundle_round_trips_through_verify() {
        let dir = tempfile::tempdir().unwrap();
        let bundle = dir.path().join("bundle");
        write_bundle(&bundle);
        let out = dir.path().join("bundle.tar.gz");
        pack_bundle(&bundle, Some(&out)).unwrap();
        verify_bundle(Some(&out)).unwrap();
        verify_bundle(Some(&bundle)).unwrap();
    }

    #[test]
    fn manifest_without_entries_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("bundle.manifest.json"), b"{}").unwrap();
        let err = verify_bundle(Some(dir.path())).unwrap_err();
        assert!(err.contains("nothing to verify"), "{err}");
    }

    #[cfg(unix)]
    #[test]
    fn archive_with_symlink_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let bundle = dir.path().join("bundle");
        write_bundle(&bundle);
        let out = dir.path().join("evil.tar.gz");
        let file = fs::File::create(&out).unwrap();
        let mut tar = Builder::new(GzEncoder::new(file, Compression::default()));
        tar.append_path_with_name(bundle.join("bundle.manifest.json"), "bundle.manifest.json")
            .unwrap();
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(tar::EntryType::Symlink);
        header.set_size(0);
        tar.append_link(&mut header, "cases.yaml", "/etc/hosts")
            .unwrap();
        tar.into_inner().unwrap().finish().unwrap();
        let err = verify_bundle(Some(&out)).unwrap_err();
        assert!(err.contains("non-regular entry"), "{err}");
    }

    #[cfg(unix)]
    #[test]
    fn pull_refuses_to_write_through_existing_symlink() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("bundle");
        let target = dir.path().join("elsewhere");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&target).unwrap();
        std::os::unix::fs::symlink(&target, root.join("fixtures")).unwrap();
        let dest = resolve_bundle_artifact_path(&root, "fixtures/cases.yaml").unwrap();
        let err = ensure_no_symlinks_below_root(&root, &dest).unwrap_err();
        assert!(err.contains("symlink"), "{err}");
        let ok = resolve_bundle_artifact_path(&root, "cases.yaml").unwrap();
        ensure_no_symlinks_below_root(&root, &ok).unwrap();
    }
}
