# Release Checklist

Canonical reference for cutting an agentcarousel release. Run every step in order.

## 1. Bump version

Edit `crates/agentcarousel/Cargo.toml` — update `version = "x.y.z"`.

Run `cargo build` once so `Cargo.lock` is updated, then commit both files.

## 2. Update CHANGELOG.md

Add a section for the new version at the top of `CHANGELOG.md`. Include a summary of user-facing changes since the last release.

## 3. Refresh bundle manifest hashes

```bash
for bundle_dir in fixtures/bundles/*/; do
  agentcarousel bundle pack "${bundle_dir%/}"
done
```

Commit any updated `bundle.manifest.json` files. The release gate in `releasing.yml` enforces this — it will fail if any hash is stale.

## 4. Update the Homebrew formula

```bash
bash packaging/update-homebrew.sh vX.Y.Z
```

This fetches the source tarball for the tag, computes its SHA256, and patches `packaging/homebrew/agentcarousel.rb`. Commit the updated formula.

## 5. Tag and push

```bash
git tag vX.Y.Z
git push origin main vX.Y.Z
```

Pushing the tag triggers `releasing.yml`, which:
- Runs the release gate (fmt, clippy, tests, manifest hash check)
- Builds binaries for all five targets
- Attaches `install.sh` and `SHA256SUMS` to the GitHub release
- Triggers `packaging.yml` to publish to crates.io and update the Homebrew tap

## 6. Verify release assets

After CI completes, open the GitHub release and confirm:

- [ ] All five target archives are present (`x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`, `x86_64-apple-darwin`, `aarch64-apple-darwin`, `x86_64-pc-windows-msvc`)
- [ ] `SHA256SUMS` is present and covers all archives and `install.sh`
- [ ] `install.sh` is attached
- [ ] crates.io shows the new version
- [ ] `brew update && brew upgrade agentcarousel` installs the new version

## 7. Post-release

Update the Homebrew badge in `README.md` if the tap URL changed, and close any milestone or release-tracking issues.
