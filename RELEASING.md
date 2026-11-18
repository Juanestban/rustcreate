# Releasing a version

Releases are built and published by GitHub Actions (`.github/workflows/release.yml`) when you
push a `vX.Y.Z` tag. Installed copies detect them and offer to update
([ADR 0002](docs/adr/0002-updates-from-github-releases.md)). No repository secrets are needed:
the workflow only uses the token GitHub Actions provides automatically.

## Steps

1. Bump the version in `Cargo.toml` (`[workspace.package] version`), following
   [SemVer](https://semver.org/): patch for fixes, minor for new features.
2. Update `Cargo.lock` and check that everything passes:
   ```
   cargo test --workspace
   cargo clippy --workspace --all-targets
   ```
3. Commit the version (for example `chore: version 0.2.0`) and push it.
4. Create the tag and push it:
   ```
   git tag v0.2.0
   git push origin v0.2.0
   ```
5. In the GitHub **Actions** tab, the *Release* workflow checks that the tag matches
   `Cargo.toml`, runs the tests, packages and publishes the release with notes generated from
   the commits.

## What it publishes

| File | Purpose |
|---|---|
| `Rustcreate-<version>-x64.msi` | Per-user installer (no administrator). The installed app downloads this to update. |
| `Rustcreate-<version>-x64-portable.exe` | Standalone executable. The portable app downloads this to update. |
| `SHA256SUMS.txt` | SHA-256 sums the app verifies before installing. |

**Do not rename these files**: the updater finds them by name.

## Testing the packaging locally (Windows)

```
dotnet tool restore
.\scripts\package.ps1      # -> target\dist
```

## If something goes wrong

- **The tag does not match the version:** delete the tag (`git tag -d v0.2.0` and
  `git push origin :refs/tags/v0.2.0`), fix `Cargo.toml` and create it again.
- **A broken release:** publish a new version with the fix. Deleting a release does not
  "downgrade" anyone who already installed it.
