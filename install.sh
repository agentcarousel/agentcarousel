#!/bin/sh
# Install agentcarousel from GitHub (latest tag).
# Usage:
#   curl -fsSL https://install.agentcarousel.com | sh

set -eu

REPO="agentcarousel/agentcarousel"
VERSION="$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" | sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' | head -n1)"

if [ -z "${VERSION}" ]; then
  echo "install.sh: could not resolve latest release for ${REPO}" >&2
  exit 1
fi

OS="$(uname -s)"
ARCH="$(uname -m)"

# Normalize Windows-style values when run under Git Bash / MSYS
case "${ARCH}" in
  AMD64) ARCH=x86_64 ;;
esac

triple=""
case "${OS}" in
  Linux)
    case "${ARCH}" in
      x86_64) triple=x86_64-unknown-linux-gnu ;;
      aarch64) triple=aarch64-unknown-linux-gnu ;;
      arm64) triple=aarch64-unknown-linux-gnu ;;
      *) echo "unsupported Linux machine: ${ARCH}" >&2; exit 1 ;;
    esac
    ;;
  Darwin)
    case "${ARCH}" in
      x86_64) triple=x86_64-apple-darwin ;;
      arm64) triple=aarch64-apple-darwin ;;
      aarch64) triple=aarch64-apple-darwin ;;
      *) echo "unsupported Darwin machine: ${ARCH}" >&2; exit 1 ;;
    esac
    ;;
  *)
    echo "install.sh: only Linux and macOS are supported by this installer (got OS=${OS})." >&2
    echo "For Windows, download the .zip for your architecture from:" >&2
    echo "  https://github.com/${REPO}/releases/tag/${VERSION}" >&2
    exit 1
    ;;
esac

ASSET="agentcarousel-${VERSION}-${triple}.tar.gz"
URL="https://github.com/${REPO}/releases/download/${VERSION}/${ASSET}"
SUMS_URL="https://github.com/${REPO}/releases/download/${VERSION}/SHA256SUMS"

TMP="${TMPDIR:-/tmp}/agentcarousel-install.$$"
mkdir -p "${TMP}"
trap 'rm -rf "${TMP}"' EXIT INT TERM

echo "Downloading ${URL} ..."
curl -fL --retry 3 --retry-delay 1 -o "${TMP}/${ASSET}" "${URL}"

if curl -fsSL -o "${TMP}/SHA256SUMS" "${SUMS_URL}" 2>/dev/null; then
  (
    cd "${TMP}"
    if command -v sha256sum >/dev/null 2>&1; then
      grep -F "${ASSET}" SHA256SUMS | sha256sum -c -
    elif command -v shasum >/dev/null 2>&1; then
      want="$(grep -F "${ASSET}" SHA256SUMS | awk '{print $1}')"
      got="$(shasum -a 256 "${ASSET}" | awk '{print $1}')"
      if [ "${want}" != "${got}" ]; then
        echo "checksum mismatch for ${ASSET}" >&2
        exit 1
      fi
    fi
  )
fi

INSTALL_DIR="${AGENTCAROUSEL_INSTALL_DIR:-${HOME}/.local/bin}"
mkdir -p "${INSTALL_DIR}"

tar -xzf "${TMP}/${ASSET}" -C "${TMP}"
# Expect single binary named agentcarousel at archive root
if [ ! -f "${TMP}/agentcarousel" ]; then
  echo "archive did not contain ./agentcarousel at root" >&2
  exit 1
fi

chmod +x "${TMP}/agentcarousel"
mv -f "${TMP}/agentcarousel" "${INSTALL_DIR}/agentcarousel"

echo "Installed to ${INSTALL_DIR}/agentcarousel"
echo "Ensure ${INSTALL_DIR} is on your PATH."

setup_alias() {
  if [ ! -r /dev/tty ]; then
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

  shell_name="$(basename "${SHELL:-}")"
  case "${shell_name}" in
    bash) rc_file="${HOME}/.bashrc" ;;
    zsh) rc_file="${HOME}/.zshrc" ;;
    *)
      echo "Skipping alias setup (unsupported shell: ${shell_name:-unknown})."
      return
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
