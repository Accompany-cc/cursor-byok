---
name: release
description: Prepare and verify personal-fork desktop releases without updater signing.
---

# Desktop release

Release only through `.github/workflows/release.yml` in `Accompany-cc/cursor-byok`, authorized by repository owner `Accompany-cc`. This is a personal-use fork based on leookun and the leehainuo community baseline; see both READMEs for PR provenance and MIT attribution.

- Require explicit publication authorization before committing, pushing tags, running release workflows, or publishing/editing a Release. Preparing code is not publication authority.
- Never replace an existing version, tag or release asset. Preserve the historical 1.0.2 test exe; 1.0.3 is candidate source and is not Windows-runtime validated.
- Keep desktop versions identical in `apps/desktop/package.json`, both root versions in `package-lock.json`, `src-tauri/Cargo.toml`, `tauri.conf.json`, and the desktop entry in `Cargo.lock`. Leave the independent server version unchanged.
- Write `.github/release-notes/MAJOR.MINOR.PATCH.md`; run `node .github/scripts/check-release.mjs`, script tests, Rust checks and frontend checks. Do not claim platform runtime validation from compilation alone.
- Automatic update checks, downloads and installs are disabled in frontend and Rust entry points; updater plugin registration and capability are removed. `createUpdaterArtifacts` is false. The workflow consumes no updater signing secrets and emits no updater manifests or signatures. Adding secrets alone does not enable updates. Re-enabling requires a separately reviewed signing, runtime and release-pipeline change with fork-owned keys and tests.
- Never print or commit private keys, tokens, `.tauri/`, local configuration, databases or captures containing API keys or conversations. Do not change git configuration.
- Release tags must match app versions and belong to `origin/main`; workflow checks owner-triggered tag/manual events, runs CI and builds ordinary packages on all platforms before publishing the draft. PRs do not receive release permissions or signing secrets. Existing published versions are rejected.
- After authorized publication, inspect release assets: macOS arm64/x64 DMGs, Windows NSIS and portable ZIP (executable plus both license notices), Linux AppImage/DEB/RPM, LICENSE and THIRD-PARTY-LICENSES.md. No `latest.json`, `portable-latest.json` or `.sig` files should be present, including stale draft assets.
- Product name, application identifier and configuration paths remain unchanged. Warn that installation may overwrite a prior installation and reuse its settings; back up first.
- Updater signatures, Windows Authenticode, and Apple notarization are different. Current Windows builds are not Authenticode-signed and macOS builds are ad-hoc signed, not notarized. Report actual build/test scope and blockers.
