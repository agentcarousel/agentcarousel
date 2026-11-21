#!/bin/sh
# Install agentcarousel from GitHub releases.
#
# Usage:
#   curl -fsSL https://install.agentcarousel.com | sh
#
# The release binary includes the web dashboard (`agentcarousel dashboard`).
#
# Options:
#   --feature <name>   Accepted for 0.9.x scripts and ignored: there is one binary now.
#
# Environment:
#   AGENTCAROUSEL_VERSION       Install this version (e.g. 0.10.0 or v0.10.0) instead of the latest.
#   AGENTCAROUSEL_LIBC          Linux only: "musl" (default; static, runs on Alpine and old glibc)
#                               or "gnu".
#   AGENTCAROUSEL_INSTALL_DIR   Override installation directory (default: ~/.local/bin).
#   AGENTCAROUSEL_FEATURES      Ignored (see --feature).
#
# The download is verified against the release's SHA256SUMS. The install stops if the
# checksum file cannot be fetched, has no entry for the archive, or does not match.

set -eu

REPO="agentcarousel/agentcarousel"

fail() {
  echo "install.sh: $*" >&2
  exit 1
}

# ── Argument parsing ──────────────────────────────────────────────────────────

NOTE_FEATURE=0
[ -n "${AGENTCAROUSEL_FEATURES:-}" ] && NOTE_FEATURE=1

while [ $# -gt 0 ]; do
  case "$1" in
    --feature)
      shift
      [ $# -eq 0 ] && fail "--feature requires an argument"
      NOTE_FEATURE=1
      shift
      ;;
    --feature=*)
      NOTE_FEATURE=1
      shift
      ;;
    --)
      shift
      break
      ;;
    -*)
      fail "unknown option '$1'"
      ;;
    *)
      break
      ;;
  esac
done

if [ "${NOTE_FEATURE}" = "1" ]; then
  echo "Note: --feature / AGENTCAROUSEL_FEATURES is no longer needed; the release binary includes the dashboard." >&2
fi

# ── Resolve the release tag ───────────────────────────────────────────────────

if [ -n "${AGENTCAROUSEL_VERSION:-}" ]; then
  VERSION="${AGENTCAROUSEL_VERSION}"
  case "${VERSION}" in v*) ;; *) VERSION="v${VERSION}" ;; esac
else
  # The /releases/latest redirect needs no API token and is not rate limited like the API.
  LATEST_URL="$(curl -fsSL -o /dev/null -w '%{url_effective}' "https://github.com/${REPO}/releases/latest")" \
    || fail "could not resolve the latest release for ${REPO}"
  VERSION="${LATEST_URL##*/}"
fi

case "${VERSION}" in
  v[0-9]*) ;;
  *) fail "could not determine a release version (got '${VERSION}')" ;;
esac

# ── Detect platform ───────────────────────────────────────────────────────────

OS="$(uname -s)"
ARCH="$(uname -m)"

# Normalize Windows-style values when run under Git Bash / MSYS
case "${ARCH}" in
  AMD64) ARCH=x86_64 ;;
esac

LIBC="${AGENTCAROUSEL_LIBC:-musl}"
case "${LIBC}" in
  musl|gnu) ;;
  *) fail "AGENTCAROUSEL_LIBC must be 'musl' or 'gnu' (got '${LIBC}')" ;;
esac

triple=""
case "${OS}" in
  Linux)
    case "${ARCH}" in
      x86_64)          triple="x86_64-unknown-linux-${LIBC}" ;;
      aarch64|arm64)   triple="aarch64-unknown-linux-${LIBC}" ;;
      *) fail "unsupported Linux machine: ${ARCH}" ;;
    esac
    ;;
  Darwin)
    case "${ARCH}" in
      x86_64)          triple=x86_64-apple-darwin ;;
      arm64|aarch64)   triple=aarch64-apple-darwin ;;
      *) fail "unsupported Darwin machine: ${ARCH}" ;;
    esac
    ;;
  *)
    echo "install.sh: only Linux and macOS are supported by this installer (got OS=${OS})." >&2
    echo "For Windows, download agentcarousel-${VERSION}-x86_64-pc-windows-gnu.zip from:" >&2
    echo "  https://github.com/${REPO}/releases/tag/${VERSION}" >&2
    exit 1
    ;;
esac

# ── Download and verify ───────────────────────────────────────────────────────

BASE_URL="https://github.com/${REPO}/releases/download/${VERSION}"

TMP="${TMPDIR:-/tmp}/agentcarousel-install.$$"
mkdir -p "${TMP}"
trap 'rm -rf "${TMP}"' EXIT INT TERM

# The checksum file comes first: it is mandatory, and it also tells us which archive to fetch.
curl -fsSL --retry 3 --retry-delay 1 -o "${TMP}/SHA256SUMS" "${BASE_URL}/SHA256SUMS" \
  || fail "could not fetch ${BASE_URL}/SHA256SUMS; refusing to install an unverified binary"

# From 0.10.1 the unsuffixed archive is the one binary, dashboard included, and "-full" is an
# identical copy. 0.10.0 and older ship a slim unsuffixed archive and the dashboard build as
# "-full". Prefer "-full" when the release lists it, so every version gets the dashboard.
ASSET="agentcarousel-${VERSION}-${triple}.tar.gz"
if grep -qF " agentcarousel-${VERSION}-${triple}-full.tar.gz" "${TMP}/SHA256SUMS"; then
  ASSET="agentcarousel-${VERSION}-${triple}-full.tar.gz"
fi

want="$(awk -v f="${ASSET}" '$2 == f || $2 == "*" f { print $1; exit }' "${TMP}/SHA256SUMS")"
[ -n "${want}" ] || fail "no checksum entry for ${ASSET} in SHA256SUMS; refusing to install"

echo "Installing agentcarousel ${VERSION} (${triple})..."
echo "Downloading ${BASE_URL}/${ASSET} ..."
curl -fL --retry 3 --retry-delay 1 -o "${TMP}/${ASSET}" "${BASE_URL}/${ASSET}" \
  || fail "download failed: ${BASE_URL}/${ASSET}"

if command -v sha256sum >/dev/null 2>&1; then
  got="$(sha256sum "${TMP}/${ASSET}" | awk '{print $1}')"
elif command -v shasum >/dev/null 2>&1; then
  got="$(shasum -a 256 "${TMP}/${ASSET}" | awk '{print $1}')"
else
  fail "neither sha256sum nor shasum is available; cannot verify the download"
fi

if [ "${want}" != "${got}" ]; then
  fail "checksum mismatch for ${ASSET} (expected ${want}, got ${got})"
fi
echo "Checksum verified."

# ── Install ───────────────────────────────────────────────────────────────────

INSTALL_DIR="${AGENTCAROUSEL_INSTALL_DIR:-${HOME}/.local/bin}"
if ! mkdir -p "${INSTALL_DIR}"; then
  echo "install.sh: could not create install directory: ${INSTALL_DIR}" >&2
  echo "Typical fix if ~/.local was created by root:" >&2
  echo '  sudo chown -R "$(whoami)" "${HOME}/.local"' >&2
  echo "Or pick a directory you own, then:" >&2
  echo '  mkdir -p "${HOME}/bin"' >&2
  echo '  AGENTCAROUSEL_INSTALL_DIR="${HOME}/bin" curl -fsSL https://install.agentcarousel.com | sh' >&2
  exit 1
fi

tar -xzf "${TMP}/${ASSET}" -C "${TMP}"
if [ ! -f "${TMP}/agentcarousel" ]; then
  echo "archive did not contain ./agentcarousel at root" >&2
  exit 1
fi

chmod +x "${TMP}/agentcarousel"
mv -f "${TMP}/agentcarousel" "${INSTALL_DIR}/agentcarousel"

echo "Installed agentcarousel ${VERSION} to ${INSTALL_DIR}/agentcarousel"
echo "Ensure ${INSTALL_DIR} is on your PATH."

echo "Dashboard included — run: agentcarousel dashboard"

# ── Optional agc alias ────────────────────────────────────────────────────────

setup_alias() {
  # `-r /dev/tty` is true even without a controlling terminal (CI, containers), where
  # opening it fails; test that it can really be opened.
  if ! ( : < /dev/tty ) 2>/dev/null; then
    echo "Skipping alias setup (non-interactive install)."
    return
  fi

  printf "Add agc alias for agentcarousel? (y/N) " > /dev/tty
  read -r reply < /dev/tty || return
  case "${reply}" in
    y|Y|yes|YES|Yes)
      ;;
    *)
      echo "Skipping alias setup."
      return
      ;;
  esac

  shell_base="$(basename "${SHELL:-}")"
  case "${shell_base}" in
    bash) rc_file="${HOME}/.bashrc" ;;
    zsh)  rc_file="${HOME}/.zshrc" ;;
    *)
      case "$(uname -s)" in
        Darwin)
          rc_file="${HOME}/.zshrc"
          echo "Note: installer ran as sh; adding alias to ${rc_file} (macOS default is zsh)." >&2
          ;;
        *)
          if [ -f "${HOME}/.bashrc" ]; then
            rc_file="${HOME}/.bashrc"
          elif [ -f "${HOME}/.zshrc" ]; then
            rc_file="${HOME}/.zshrc"
          else
            rc_file="${HOME}/.bashrc"
            echo "Note: installer ran as sh; adding alias to ${rc_file}. If you use zsh, move the line to ~/.zshrc." >&2
          fi
          ;;
      esac
      ;;
  esac

  touch "${rc_file}"
  if grep -F "alias agc=" "${rc_file}" >/dev/null 2>&1; then
    echo "Alias agc already configured in ${rc_file}."
    return
  fi

  printf '\n# agentcarousel alias\nalias agc='\''agentcarousel'\''\n' >> "${rc_file}"
  echo "Added agc alias to ${rc_file}. Restart your shell or run: source ${rc_file}"
}

setup_alias
