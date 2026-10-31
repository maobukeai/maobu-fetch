// Read-only audit probes. Observation=true can mean a defect was reproduced.
// All cookies/tokens are synthetic; all browser profiles and evidence stay in TEMP.
import fs from "node:fs/promises";
import path from "node:path";
import os from "node:os";
import assert from "node:assert/strict";
import { runLocalProbes } from "./audit/extension-local-probes.mjs";
import { runBrowserProbes } from "./audit/extension-browser-probes.mjs";
import { runBackgroundProbes } from "./audit/extension-background-probes.mjs";

const root = new URL("../", import.meta.url);
const auditDir = await fs.mkdtemp(path.join(os.tmpdir(), "maobu-extension-audit-"));
const result = {
  generatedAt: new Date().toISOString(),
  scope: "Isolated source/API probes; no real account, native desktop transfer or public-site download",
  local: await runLocalProbes(root),
  background: await runBackgroundProbes(),
  browsers: process.argv.includes("--skip-browsers") ? [] : await runBrowserProbes(root, auditDir),
};
const evidence = path.join(auditDir, "evidence.json");
await fs.writeFile(evidence, JSON.stringify(result, null, 2), "utf8");
console.log(JSON.stringify({ evidence, ...result }, null, 2));
if (result.browsers.some((browser) => browser.status !== "executed")) process.exitCode = 1;
assert.equal(result.local.cookieScope.unrelatedPageCookieForwarded, false);
assert.equal(result.local.sizeThreshold.tenByteArchiveEligibleWith100MbMinimum, false);
assert.equal(result.local.lanDiskMessage.unrelatedPageMessageForwarded, false);
assert.equal(result.local.snifferNavigation.oldMediaRemainsAfterNavigation, false);
assert.equal(result.background.lanDiskTaskAcceptedWithInterceptionDisabled, false);
assert.equal(result.background.explicitOneMbThresholdRewrittenToZero, false);
assert.equal(result.background.magnet401Result.fallback, true);
for (const browser of result.browsers.filter((item) => item.status === "executed")) {
  assert.equal(browser.observations.pageCookieForwardedToOtherHost, false);
  assert.equal(browser.observations.privateHeaderUsesNormalStore, false);
  assert.equal(browser.observations.privateHeaderUsesPrivateStore, true);
  assert.ok(browser.observations.protocol.every((item) => item.signatureValid && item.extensionOriginPresent));
  assert.ok(Object.values(browser.observations.lanDiskApproval).every((value) => value === true));
}
