import test from "node:test";
import assert from "node:assert/strict";
import { createSnifferDocuments } from "./sniffer-documents.js";

test("Sniffer accepts current iframe media while rejecting old document subtrees", () => {
  const documents = createSnifferDocuments();
  documents.commit({ tabId: 1, frameId: 0, documentId: "main" });
  documents.commit({ tabId: 1, frameId: 2, parentFrameId: 0, documentId: "iframe" });
  documents.commit({ tabId: 1, frameId: 3, parentFrameId: 2, documentId: "nested" });
  assert.equal(documents.matches({ tabId: 1, frameId: 2, documentId: "iframe" }), true);
  documents.commit({ tabId: 1, frameId: 0, documentId: "main" }); // SPA does not replace live iframes.
  assert.equal(documents.matches({ tabId: 1, frameId: 3, documentId: "nested" }), true);
  documents.commit({ tabId: 1, frameId: 2, parentFrameId: 0, documentId: "new-iframe" });
  assert.equal(documents.matches({ tabId: 1, frameId: 2, documentId: "iframe" }), false);
  assert.equal(documents.matches({ tabId: 1, frameId: 3, documentId: "nested" }), false);
  assert.equal(documents.matches({ tabId: 1, frameId: 2, documentId: "new-iframe" }), true);
  documents.commit({ tabId: 1, frameId: 0, documentId: "new-main" });
  assert.equal(documents.matches({ tabId: 1, frameId: 2, documentId: "new-iframe" }), false);
});
