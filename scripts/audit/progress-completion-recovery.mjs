import assert from 'node:assert/strict';

export async function checkCompletionMountRecovery({ page, base, record }) {
  await page.mouse.move(0, 0);
  // An old WebView was torn down before its release reached the native actor.
  // Recreate only the React document; the simulated actor starts held=true.
  await page.goto(base + '?completion=1&count=0&completed=1&held=1');
  await page.waitForSelector('.progress-window-completed-summary');
  await page.waitForFunction(() => window.audit.calls.some(call => call[0] === 'interacting' && call[1] === false), { timeout: 1500 });
  assert.equal(await page.evaluate(() => window.audit.interacting), false);
  assert.deepEqual(await page.evaluate(() => window.audit.calls.filter(call => call[0] === 'interacting').map(call => call[1])), [false]);
  await page.evaluate(() => window.audit.advance(179999));
  assert.equal(await page.evaluate(() => window.audit.snapshot.completed_count), 1);
  await page.evaluate(() => window.audit.advance(1));
  await page.waitForFunction(() => !window.audit.nativeVisible);
  record('Fresh WebView mount explicitly releases a stale backend interaction hold before simulated completion expiry');
}
