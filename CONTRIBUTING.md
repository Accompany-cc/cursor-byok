# Contributing

Issues and pull requests are welcome. Keep each change focused, follow the existing directory structure, and describe the problem and validation results. Use Conventional Commits, such as `fix: correct model selection` or `docs: clarify setup`.

欢迎提交 Issue 和 Pull Request。每次改动保持聚焦，遵循现有目录结构，并说明解决的问题及验证结果。提交信息使用 Conventional Commits。

## Development

Install Rust stable, Node.js 22 or newer, and the [Tauri 2 prerequisites](https://v2.tauri.app/start/prerequisites/) for your platform.

```bash
npm --prefix apps/desktop ci
make dev-desktop
```

Before opening a pull request:

```bash
make check
```

## Dependency updates

Dependabot checks on Mondays at 09:00 Asia/Shanghai. Ordinary patch updates are grouped; ordinary minor and major updates get individual PRs. React, Babel, Tauri, and Protobuf packages are grouped by family at every version level, including major upgrades. SQLx, similar, json5, and auto-launch stay out of the general Rust patch group. Security updates remain enabled and are not included in version-update groups.

All updates require passing CI and maintainer review. Patch updates need checks of affected behavior; minor updates need runtime regression checks; major updates also need a migration review. React and React DOM must stay aligned, and Protobuf code generation and runtime versions must remain compatible. Tauri's npm and Cargo groups are separate PRs and must be checked together when their requirements change.

For sensitive changes, verify Cursor protocol encoding, database behavior, file diffs, preservation of Cursor settings, or desktop startup/relaunch/update behavior as applicable. Check Node.js requirements when upgrading build tools. A green CI run does not replace these checks, and dependency PRs are not automatically merged.

依赖更新按普通补丁、独立版本升级和关联依赖家族组织。所有更新都需要 CI 和维护者审查；涉及协议、数据库、文件差异、Cursor 配置或桌面生命周期时，需补充对应的运行验证。

## Releases

Desktop versions must match in `package.json`, `package-lock.json`, `src-tauri/Cargo.toml`, `tauri.conf.json`, and the desktop entry in `Cargo.lock`.

Write the release notes in `.github/release-notes/MAJOR.MINOR.PATCH.md`. The workflow appends the platform download guide before publication.

Run `node .github/scripts/check-release.mjs`, then validate the desktop build with `npm --prefix apps/desktop run tauri:build -- --debug --no-bundle`.

The maintainer publishes releases in `Accompany-cc/cursor-byok` by pushing a matching `vMAJOR.MINOR.PATCH` tag after CI passes on `main`. The workflow only accepts owner-triggered tag or manual runs, builds ordinary packages without updater secrets, and publishes only after every build succeeds. Ordinary commits and external PRs do not publish releases or receive signing secrets. To recover an unpublished release, run `Release desktop app` manually from `main` with its existing tag; published versions are rejected.

Automatic updates are deliberately disabled in the frontend, Rust commands, plugin registration, and bundle configuration. The release workflow does not consume updater secrets or generate/upload `latest.json`, `portable-latest.json`, or `.sig` files. `make build-desktop` builds ordinary packages without generating a local key. Enabling updates later requires a separate reviewed change with a new fork-owned signing key/public key pair, runtime gates, plugin registration, endpoints, signed manifests, and regression tests; merely adding secrets does not enable updates. Updater signing is separate from Apple notarization and Windows Authenticode signing.

本分支以个人自用为主，欢迎贡献，但不保证响应与合并时限。`1.0.3` 是集成候选源码，未完成真实 Windows 端到端验证；不要覆盖或重发历史 `1.0.2` 测试 exe。应用标识及配置目录不变，安装前需备份，可能覆盖旧安装。
