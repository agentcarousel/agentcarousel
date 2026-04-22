# agentcarousel

> **My agent skill is better with Claude than Gemini and I have *proof***

Evaluate AI agents and skills the way regulators would audit them with reproducible runs, scored rubrics, and signed evidence.

<sup>Compiled and built in Rust</sup>

## Installation

```bash
curl -fsSL "https://install.agentcarousel.com" | sh
```

If you prefer to pin the source, use the raw installer and choose a release tag:

```bash
curl -fsSL "https://raw.githubusercontent.com/agentcarousel/agentcarousel/v0.1.0/install.sh" | sh
```

`install.sh` picks the archive for your architecture, verifies checksums, and installs the binary to `${AGENTCAROUSEL_INSTALL_DIR:-$HOME/.local/bin}`.

On Windows, download the `.zip` from the [Releases](https://github.com/agentcarousel/agentcarousel/releases) page.

## `agc` shorthand

After install, the CLI is callable as either `agentcarousel` or the shorthand `agc`.

```bash
agentcarousel --help
agc --help
```

## Supported platforms

- Linux x86_64 / ARM (aarch64)
- macOS Intel / Apple Silicon
- Windows x86_64

---

## Security

Report vulnerabilities through the contact process described in [SECURITY.md](SECURITY.md). Do **not** file public security issues with exploit details before maintainers acknowledge receipt.

---

## Contributing

The CLI is developed in a different workspace. Contributions to **fixtures** or docs that ship here may be accepted. Feature work on the core product is coordinated out-of-band. See [CONTRIBUTING.md](CONTRIBUTING.md).
