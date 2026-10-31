import test from "node:test";
import assert from "node:assert/strict";
import { createLanDiskApproval } from "./landisk-approval.js";

function setup() {
  const listeners = {};
  const closed = [];
  let count = 0;
  const chrome = {
    runtime: { getURL: (path) => `chrome-extension://test/${path}` },
    windows: {
      create: async () => ({ id: 3, tabs: [{ id: 10 }] }), remove: async (id) => closed.push(id),
      onRemoved: { addListener: (fn) => { listeners.close = fn; } },
    },
    tabs: { onRemoved: { addListener: (fn) => { listeners.remove = fn; } }, onUpdated: { addListener: (fn) => { listeners.update = fn; } } },
  };
  return { controller: createLanDiskApproval({ chrome, timeoutMs: 1000, randomId: () => `request-${++count}` }), listeners, closed };
}
const request = { fileName: "files.zip", origin: "http://localhost", fileCount: 1, files: ["report.txt"] };
const source = { tab: { id: 1 }, documentId: "source", frameId: 0 };
const dialog = { tab: { id: 10 }, url: "chrome-extension://test/src/landisk-approval.html#request-1" };

test("Only the bound extension popup can approve; webpage messages and replay cannot", async () => {
  const { controller } = setup();
  const result = controller.ask(request, source);
  const message = { type: "landisk-approval-choice", id: "request-1", accepted: true };
  assert.equal((await controller.message(message, { tab: { id: 1 }, url: "http://localhost" })).ok, false);
  assert.equal((await controller.message(message, { ...dialog, tab: { id: 99 } })).ok, false);
  const preview = await controller.message({ ...message, type: "landisk-approval-details" }, dialog);
  assert.deepEqual(preview.result.files, ["report.txt"]);
  assert.equal((await controller.message(message, dialog)).ok, true);
  assert.equal(await result, true);
  assert.equal((await controller.message(message, dialog)).ok, false);
});

test("Closing popup, navigating source, and duplicate requests safely fall back", async () => {
  const { controller, listeners } = setup();
  const first = controller.ask(request, source);
  await assert.rejects(controller.ask(request, source), /当前打包确认/);
  await new Promise((resolve) => setTimeout(resolve, 0));
  listeners.close(3);
  assert.equal(await first, false);
  const next = controller.ask(request, source);
  listeners.update(1, { url: "http://localhost/other" });
  assert.equal(await next, false);
});
