import test from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, mkdir, readFile, writeFile, access } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { execFile } from "node:child_process";
import { promisify } from "node:util";
import { fileURLToPath } from "node:url";

test("Extension build is cwd-independent and never deploys into installed APPDATA", async () => {
  const temp = await mkdtemp(path.join(tmpdir(), "maobu-extension-build-test-"));
  const installed = path.join(temp, "app.lumaget.desktop", "extension");
  await mkdir(path.join(installed, "src"), { recursive: true });
  await writeFile(path.join(installed, "manifest.json"), "KEEP_INSTALLED_MANIFEST");
  await writeFile(path.join(installed, "src", "landisk-injected.js"), "KEEP_INSTALLED_SCRIPT");
  const extensionRoot = fileURLToPath(new URL("../", import.meta.url));
  await promisify(execFile)(process.execPath, [path.join(extensionRoot, "build.mjs")], {
    cwd: temp, env: { ...process.env, APPDATA: temp },
  });
  assert.equal(await readFile(path.join(installed, "manifest.json"), "utf8"), "KEEP_INSTALLED_MANIFEST");
  assert.equal(await readFile(path.join(installed, "src", "landisk-injected.js"), "utf8"), "KEEP_INSTALLED_SCRIPT");
  await access(path.join(extensionRoot, "dist", "src", "landisk-bridge.js"));
});
