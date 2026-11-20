//! Extraction of untrusted `.tar.gz` archives (evidence packs, bundle archives).

use flate2::read::GzDecoder;
use std::fs;
use std::path::Path;

/// Why an archive could not be extracted.
#[derive(Debug)]
pub(crate) enum ExtractError {
    /// The archive itself is unacceptable (links, special files, escaping paths).
    Rejected(String),
    /// The archive could not be read or written (I/O, not gzip/tar).
    Io(String),
}

impl std::fmt::Display for ExtractError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExtractError::Rejected(msg) | ExtractError::Io(msg) => f.write_str(msg),
        }
    }
}

/// Extract only regular files and directories; `unpack_in` refuses paths that
/// escape `dest`, and symlinks, hardlinks and special files are rejected outright,
/// so nothing read later from `dest` can point outside it.
pub(crate) fn extract_tar_gz(archive: &Path, dest: &Path) -> Result<(), ExtractError> {
    let file = fs::File::open(archive).map_err(|e| ExtractError::Io(e.to_string()))?;
    let mut tar = tar::Archive::new(GzDecoder::new(file));
    let entries = tar
        .entries()
        .map_err(|e| ExtractError::Io(format!("not a readable .tar.gz: {e}")))?;
    for entry in entries {
        let mut entry =
            entry.map_err(|e| ExtractError::Io(format!("not a readable .tar.gz: {e}")))?;
        let kind = entry.header().entry_type();
        if !(kind.is_file() || kind.is_dir()) {
            let name = entry
                .path()
                .map(|p| p.display().to_string())
                .unwrap_or_default();
            return Err(ExtractError::Rejected(format!(
                "archive contains a non-regular entry: {name}"
            )));
        }
        let ok = entry
            .unpack_in(dest)
            .map_err(|e| ExtractError::Io(format!("extract failed: {e}")))?;
        if !ok {
            return Err(ExtractError::Rejected(
                "archive contains a path outside the extraction directory".to_string(),
            ));
        }
    }
    Ok(())
}
