# Security policy

## Supported versions

| Version | Supported |
|---------|-----------|
| 0.10.x  | Yes |
| < 0.10  | No. 0.9.x and earlier have known vulnerabilities that are fixed in 0.10.0; see the [CHANGELOG](CHANGELOG.md). |

Security fixes are released on the latest minor release line. Check the [Releases](https://github.com/agentcarousel/agentcarousel/releases) page for the current version, and upgrade with `agentcarousel update`, your package manager, or the install script.

## Reporting a vulnerability

**Please do not** open a public issue, pull request, or discussion for an undisclosed vulnerability.

Report it privately through GitHub: [Report a vulnerability](https://github.com/agentcarousel/agentcarousel/security/advisories/new) (the repository's **Security** tab → **Report a vulnerability**).

Please include:

- the affected version (`agentcarousel --version`) and platform
- steps to reproduce, or a proof of concept
- the impact as you understand it

We will acknowledge the report, keep you informed while we work on a fix, and publish a GitHub Security Advisory when the fixed release is out, crediting you unless you prefer otherwise.

## Scope

This repository covers the `agentcarousel` CLI and library, the release archives and checksums, `install.sh`, and the GitHub Actions workflows that build and publish them. Release archives are listed with their SHA-256 digests in `SHA256SUMS` on each release.
