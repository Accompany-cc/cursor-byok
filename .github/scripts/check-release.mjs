import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

const root = new URL("../../", import.meta.url);
const read = (path) => readFileSync(new URL(path, root), "utf8");
const json = (path) => JSON.parse(read(path));
const desktop = json("apps/desktop/package.json");
const lock = json("apps/desktop/package-lock.json");
const tauri = json("apps/desktop/src-tauri/tauri.conf.json");
const version = desktop.version;

assert.match(version, /^\d+\.\d+\.\d+$/, "desktop version must be stable SemVer");
assert.equal(
  read(`.github/release-notes/${version}.md`).split(/\r?\n/, 1)[0],
  `# Cursor BYOK v${version}`,
  "release notes must exist and match the desktop version",
);
for (const [name, actual] of Object.entries({
  "package-lock.json": lock.version,
  "package-lock.json root package": lock.packages[""].version,
  "tauri.conf.json": tauri.version,
  "src-tauri/Cargo.toml": read("apps/desktop/src-tauri/Cargo.toml").match(/^version = "([^"]+)"/m)?.[1],
  "Cargo.lock desktop package": read("Cargo.lock").match(/name = "cursor-byok-desktop"\nversion = "([^"]+)"/)?.[1],
})) {
  assert.equal(actual, version, `${name} version must match ${version}`);
}

const repository = desktop.repository.url
  .replace(/^git\+https:\/\/github\.com\//, "")
  .replace(/\.git$/, "");
assert.match(repository, /^[\w.-]+\/[\w.-]+$/, "invalid project repository");
const releases = `https://github.com/${repository}/releases/latest/download`;
assert.equal(repository, "Accompany-cc/cursor-byok");
assert.equal(tauri.bundle.createUpdaterArtifacts, false);
assert.deepEqual(tauri.plugins.updater.endpoints, []);
assert.equal(tauri.plugins.updater.pubkey, "");
const portable = read("apps/desktop/src-tauri/src/update/mod.rs");
assert.ok(portable.includes(`"${releases}/portable-latest.json"`));
assert.ok(portable.includes("const AUTOMATIC_UPDATES_ENABLED: bool = false;"));
assert.equal((portable.match(/if !AUTOMATIC_UPDATES_ENABLED/g) ?? []).length, 2, "both Rust update commands must be gated");
const lifecycle = read("apps/desktop/src/shared/native/appLifecycle.ts");
assert.ok(lifecycle.includes("export const AUTOMATIC_UPDATES_ENABLED = false;"));
assert.equal((lifecycle.match(/if \(!AUTOMATIC_UPDATES_ENABLED\)/g) ?? []).length, 2, "frontend checks and installation must be gated");
assert.ok(!json("apps/desktop/src-tauri/capabilities/default.json").permissions.includes("updater:default"));
assert.ok(!read("apps/desktop/src-tauri/src/desktop.rs").includes(".plugin(tauri_plugin_updater::"));
const workflow = read(".github/workflows/release.yml");
assert.ok(workflow.includes("uploadUpdaterJson: false"));
assert.ok(!workflow.includes("secrets.TAURI_SIGNING_PRIVATE_KEY"));
assert.ok(!workflow.includes("pull_request_target:"));
for (const name of ["LICENSE", "THIRD-PARTY-LICENSES.md"]) {
  assert.equal(tauri.bundle.resources[`../../../${name}`], `licenses/${name}`);
  assert.ok(read(name).includes("MIT License"), `${name} must retain the MIT license`);
}
for (const name of ["agent_v1.proto", "aiserver_v1.proto"]) {
  assert.ok(read(`protocols/cursor/${name}`).length > 0, `${name} is required`);
}

const args = process.argv.slice(2);
if (args.length) {
  assert.equal(args.length, 2, "usage: check-release.mjs [--tag vMAJOR.MINOR.PATCH]");
  assert.equal(args[0], "--tag");
  assert.equal(args[1], `v${version}`, "release tag must match the desktop version");
}
console.log(`Release configuration verified: ${repository} v${version} (${fileURLToPath(root)})`);
