#!/usr/bin/env bash
# Pack release archives, write SHA256SUMS, and lightweight release-dir checks.
# Used by .github/workflows/releasing.yml
set -euo pipefail

usage() {
  echo "usage: $0 pack <release-tag> <rust-triple> <path-to-binary> <out-dir>" >&2
  echo "       $0 checksums <release-assets-dir>" >&2
  echo "       $0 scan-release-dir <release-assets-dir>" >&2
}

cmd_pack() {
  local tag="$1"
  local target="$2"
  local bin_path="$3"
  local outdir="$4"
  local asset_base="agentcarousel-${tag}-${target}"
  local tmp
  local outdir_abs
  tmp="$(mktemp -d)"

  mkdir -p "${outdir}"
  # The archive step runs under cd "${tmp}"; a relative outdir would resolve inside tmp.
  outdir_abs="$(cd "${outdir}" && pwd)"
  if [[ ! -f "${bin_path}" ]]; then
    echo "publish-distribution: binary not found: ${bin_path}" >&2
    rm -rf "${tmp}"
    exit 1
  fi

  if [[ "${bin_path}" == *.exe ]]; then
    # Windows ships a single .zip (there is no -full alias: the updater never fetched one).
    cp "${bin_path}" "${tmp}/agentcarousel.exe"
    (cd "${tmp}" && zip -q -X "${outdir_abs}/${asset_base}.zip" agentcarousel.exe)
  else
    cp "${bin_path}" "${tmp}/agentcarousel"
    chmod +x "${tmp}/agentcarousel"
    (cd "${tmp}" && tar czf "${outdir_abs}/${asset_base}.tar.gz" agentcarousel)
    # There is one binary per target and it includes the dashboard. 0.9.x `agc update
    # --feature dashboard` requests the -full name, so publish an identical alias. Remove
    # it in a later minor release, with a CHANGELOG notice.
    cp "${outdir_abs}/${asset_base}.tar.gz" "${outdir_abs}/${asset_base}-full.tar.gz"
  fi
  rm -rf "${tmp}"
}

cmd_checksums() {
  local dir="$1"
  if [[ ! -d "${dir}" ]]; then
    echo "publish-distribution: not a directory: ${dir}" >&2
    exit 1
  fi
  (
    cd "${dir}" || exit 1
    rm -f SHA256SUMS
    shopt -s nullglob
    files=( *)
    if [[ ${#files[@]} -eq 0 ]]; then
      echo "publish-distribution: no files in ${dir}" >&2
      exit 1
    fi
    if command -v sha256sum >/dev/null 2>&1; then
      sha256sum "${files[@]}" | LC_ALL=C sort -k2 > SHA256SUMS
    else
      for f in "${files[@]}"; do
        shasum -a 256 "${f}"
      done | LC_ALL=C sort -k2 > SHA256SUMS
    fi
  )
}

cmd_scan_release_dir() {
  local dir="$1"
  if [[ ! -d "${dir}" ]]; then
    echo "publish-distribution: not a directory: ${dir}" >&2
    exit 1
  fi
  if [[ ! -s "${dir}/SHA256SUMS" ]]; then
    echo "publish-distribution: missing or empty SHA256SUMS in ${dir}" >&2
    exit 1
  fi
  echo "Release assets in ${dir}:"
  ls -la "${dir}"
}

main() {
  local cmd="${1:-}"
  shift || true
  case "${cmd}" in
    pack) cmd_pack "$@" ;;
    checksums) cmd_checksums "$@" ;;
    scan-release-dir) cmd_scan_release_dir "$@" ;;
    *)
      usage
      exit 1
      ;;
  esac
}

main "$@"
