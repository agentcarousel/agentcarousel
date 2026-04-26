# agentcarousel

Evaluate agent behavior and skills with reproducible fixtures, scored checks, and exportable evidence.

<sup>Built in Rust.</sup>

## Quick start (no API keys)

After [install](#installation), from a clone of this repository:

```bash
agentcarousel validate fixtures/skills/example-skill.yaml
agentcarousel test fixtures/skills/example-skill.yaml --offline true --filter-tags smoke
```

Both commands should succeed. To see validation catch broken fixtures:

```bash
agentcarousel validate fixtures/examples/invalid-skill.yaml
```

Full walkthrough: [docs/getting-started.md](docs/getting-started.md).

## Reading paths


| You want... | Start here |
| --- | --- |
| Install and first commands | [docs/getting-started.md](docs/getting-started.md) |
| Fixture and run model | [docs/concepts.md](docs/concepts.md) |
| CI behavior on pushes | [docs/ci-smoke.md](docs/ci-smoke.md) |
| Propose a fixture | [docs/contributing-fixtures.md](docs/contributing-fixtures.md), [CONTRIBUTING.md](CONTRIBUTING.md) |


## Installation

### Install Crate

```bash
cargo install agentcarousel
```

### Install script

```bash
curl -fsSL "http://install.agentcarousel.com" | sh
```

If you prefer to pin the source, use the raw installer and choose a release tag:

```bash
curl -fsSL "https://raw.githubusercontent.com/agentcarousel/agentcarousel/v0.2.0/install.sh" | sh
```

`install.sh` picks the archive for your architecture, verifies checksums, and installs the binary to `${AGENTCAROUSEL_INSTALL_DIR:-$HOME/.local/bin}`.

On Windows, download the `.zip` from the [Releases](https://github.com/agentcarousel/agentcarousel/releases) page.

## `agc` alias

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

Report vulnerabilities through the contact process described in [SECURITY.md](SECURITY.md).

---

## Contributing

Contributions to **fixtures** or docs may be accepted. Feature work on `agentcarousel` is coordinated out-of-band. See [CONTRIBUTING.md](CONTRIBUTING.md).