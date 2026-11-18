# ADR 0002: Updates from GitHub Releases

- Status: accepted
- Date: 2026-11-18
- Adapted from archi-games ADR 0002.

## Context

The app should tell users about new versions and update itself. Updating means downloading and
running code, so it needs clear guarantees.

## Decision

1. **One source:** the releases of `github.com/Juanestban/rustcreate`, over HTTPS
   (`api.github.com/.../releases/latest` and its asset downloads). No other server, no
   configurable URL, no API keys or secrets.
2. **Never silently:** the app only *checks* for a new version, at startup and every 5 hours
   (can be turned off in Settings). A non-modal banner offers **Update** or **Later**; *Later*
   hides it until the next check. It downloads and installs only when the user clicks
   **Update**, and first asks to save unsaved documents.
3. **Integrity:** every release publishes `SHA256SUMS.txt`. The download is verified against it
   before anything runs; on a mismatch it is deleted and the user is told. A release without an
   installer or without sums is not offered.
4. **No administrator:** the installed copy updates with the per-user MSI
   (`msiexec /i ... /passive`); the portable copy replaces itself (renames the running `.exe`,
   puts the new one in place and starts it).
5. **Releases are published by CI** (`.github/workflows/release.yml`) from a `vX.Y.Z` tag that
   matches the `Cargo.toml` version. Nothing is uploaded by hand.

## Consequences

- The sums live in the same release as the binaries: they protect against corrupt or truncated
  downloads, not against someone who controls the GitHub account. Code signing would, and is
  postponed; until then the GitHub account must use two-factor authentication.
- Without signing, Windows SmartScreen may warn when running the new version.
- Unauthenticated GitHub API allows 60 requests per hour per IP; one check every 5 hours is far
  below that.
