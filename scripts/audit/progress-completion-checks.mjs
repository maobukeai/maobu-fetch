import assert from 'node:assert/strict';
import path from 'node:path';

export async function loadCompletion({ page, base }, count = 1) {
  await page.mouse.move(0, 0);
  await page.goto(base + '?completion=1&known=1&count=' + count);
  await page.waitForFunction(() => window.audit?.initialCalled);
  await page.waitForSelector('.download-progress-window');
}
async function complete(page, id = 'progress-0') {
  await page.evaluate(id => window.audit.complete(id), id);
  await page.waitForSelector('.progress-window-completed-summary');
}
export async function waitCompletionDetails(page) {
  await page.waitForFunction(() => {
    const state = window.audit.snapshot;
    const rows = Math.max(1, Math.min(Math.max(state.total_count, state.completed_count ?? 0), state.expanded ? 6 : 3));
    const tabs = state.total_count && state.completed_count ? 24 : 0;
    const completedOnly = state.total_count === 0 && (state.completed_count ?? 0) > 0;
    return state.revealed && document.getElementById('root').getBoundingClientRect().height === 82 + rows * 36 + tabs - (completedOnly ? 24 : 0);
  });
}
const count = page => page.evaluate(() => window.audit.snapshot.completed_count);
const held = (page, value) => page.waitForFunction(value => window.audit.interacting === value, {}, value);
async function outside(page) {
  await page.click('#audit-external-focus');
  await held(page, false);
}

export async function checkCompletionLifetime(context) {
  const { page, output, record } = context;
  await loadCompletion(context);
  await complete(page);
  assert.equal(await page.evaluate(() => window.audit.nativeVisible), true);
  assert.equal(await count(page), 1);
  assert.equal(await page.$('.progress-window-completed-summary [role="progressbar"]'), null);
  assert.equal(await page.$eval('#root', el => el.getBoundingClientRect().height), 28);
  await page.$('.download-progress-window').then(el => el.screenshot({ path: path.join(output, 'completed-compact.png'), omitBackground: true }));
  await page.evaluate(() => window.audit.advance(179999));
  assert.equal(await count(page), 1);
  await page.evaluate(() => window.audit.advance(1));
  await page.waitForFunction(() => !window.audit.nativeVisible);
  assert.equal(await count(page), 0);
  record('Last completion becomes an actionable compact strip; simulated backend expires at 180000ms without an active task');

  await loadCompletion(context);
  await page.hover('.progress-window-summary');
  await held(page, true);
  await complete(page);
  await page.hover('.progress-window-completed-reveal');
  await held(page, true);
  await page.evaluate(() => window.audit.advance(900000));
  assert.equal(await count(page), 1);
  await page.focus('.progress-window-completed-summary .progress-window-completed-open');
  await page.mouse.move(0, 0);
  await page.evaluate(() => window.audit.advance(900000));
  assert.equal(await count(page), 1);
  await outside(page);
  await page.evaluate(() => window.audit.advance(180000));
  await page.waitForFunction(() => !window.audit.nativeVisible);
  record('Hover already held before the last completion and keyboard focus keep its retention paused; leaving resumes the simulated remaining lifetime');

  await loadCompletion(context);
  await complete(page);
  await page.hover('.progress-window-completed-reveal');
  await held(page, true);
  await page.evaluate(() => window.dispatchEvent(new Event('blur')));
  await page.waitForFunction(() => document.querySelector('.download-progress-window').dataset.revealed === 'false');
  await page.evaluate(() => window.audit.advance(900000));
  assert.equal(await count(page), 1);
  assert.equal(await page.evaluate(() => window.audit.interacting), true);
  await page.mouse.move(0, 0);
  await held(page, false);
  await page.evaluate(() => window.audit.advance(180000));
  await page.waitForFunction(() => !window.audit.nativeVisible);
  record('DOM window blur collapses details but preserves a real pointer hold; pointer leave releases the retention hold');

  await page.mouse.move(0, 0);
  await page.goto(context.base + '?completion=1&count=1&race=1');
  await page.waitForFunction(() => window.audit?.initialCalled);
  await complete(page);
  await new Promise(resolve => setTimeout(resolve, 220));
  assert.equal(await count(page), 1);
  assert.ok(await page.$('.progress-window-completed-summary'));
  record('A late initial active snapshot cannot replace the newer completion event');
}

export async function checkCompletionActions(context) {
  const { page, record } = context;
  await loadCompletion(context);
  await complete(page);
  await page.click('.progress-window-completed-summary .progress-window-completed-open');
  await page.waitForFunction(() => window.audit.calls.some(call => call[0] === 'open-completed' && call[1] === 'progress-0' && call[2] === 'file'));
  await page.click('.progress-window-completed-summary .progress-window-completed-folder');
  await page.waitForFunction(() => window.audit.calls.some(call => call[0] === 'open-completed' && call[1] === 'progress-0' && call[2] === 'folder'));
  assert.equal(await count(page), 1);
  record('Compact completed actions send exact task ID and file/folder target without exposing or accepting a path');

  await outside(page);
  await page.evaluate(() => { window.audit.advance(170000); window.audit.failOpen = true; });
  await page.click('.progress-window-completed-summary .progress-window-completed-open');
  await page.waitForSelector('[role="alert"]');
  assert.ok(!(await page.$eval('body', el => el.textContent)).includes('synthetic-private'));
  assert.equal(await count(page), 1);
  assert.equal(await page.evaluate(() => window.audit.completionRemaining[0][1]), 180000);
  await page.evaluate(() => window.dispatchEvent(new Event('blur')));
  await page.waitForFunction(() => document.querySelector('.download-progress-window').dataset.revealed === 'false');
  assert.ok(await page.$('.progress-window-completed-summary .progress-window-summary-alert'));
  assert.ok(!(await page.$eval('.progress-window-completed-reveal', el => el.title)).includes('synthetic-private'));
  await page.evaluate(() => { window.audit.failOpen = false; });
  await page.click('.progress-window-completed-summary .progress-window-completed-folder');
  await page.waitForFunction(() => !document.querySelector('[role="alert"]'));
  record('Open failure retains its completed entry, renews mocked retry lifetime, shows a safe error and permits folder retry');

  await loadCompletion(context);
  await complete(page);
  await page.evaluate(() => { window.audit.holdOpen = true; });
  await page.click('.progress-window-completed-summary .progress-window-completed-open');
  await page.waitForFunction(() => Boolean(window.audit.releaseOpen));
  await page.click('#audit-external-focus');
  await held(page, true);
  await page.evaluate(() => window.audit.advance(900000));
  assert.equal(await count(page), 1);
  await page.evaluate(() => { window.audit.holdOpen = false; window.audit.releaseOpen(); });
  await held(page, false);
  record('An opening action retains interaction hold after focus leaves; its completion releases the hold');
}

export async function checkCompletionBatches(context) {
  const { page, record } = context;
  await loadCompletion(context, 2);
  await page.evaluate(() => window.audit.complete('progress-0'));
  await page.hover('.progress-window-summary');
  await page.waitForSelector('[role="tab"][data-view="active"]');
  await waitCompletionDetails(page);
  assert.equal(await page.$eval('.progress-window-summary [role="progressbar"]', el => el.getAttribute('aria-valuenow')), '50');
  await page.focus('[role="tab"][data-view="active"]');
  await page.keyboard.press('End');
  await page.waitForSelector('[role="tab"][data-view="completed"][aria-selected="true"]');
  await page.waitForSelector('[data-completed-id="progress-0"]');
  await page.click('[data-completed-id="progress-0"] .progress-window-completed-folder');
  await page.waitForFunction(() => window.audit.calls.some(call => call[0] === 'open-completed' && call[1] === 'progress-0' && call[2] === 'folder'));
  await page.focus('[role="tab"][data-view="completed"]');
  await page.keyboard.press('Home');
  await page.waitForSelector('[data-task-id="progress-1"]');
  record('Active work keeps real aggregate progress and switches accessibly to the recently completed tab');

  await page.click('.progress-window-header .progress-window-icon:last-child');
  await page.waitForFunction(() => !window.audit.nativeVisible);
  assert.equal(await count(page), 0);
  await page.evaluate(() => window.audit.complete('progress-1'));
  await page.waitForFunction(() => window.audit.nativeVisible);
  await page.waitForSelector('.progress-window-completed-summary');
  assert.equal(await count(page), 1);
  assert.deepEqual(await page.evaluate(() => window.audit.snapshot.completed_tasks.map(task => task.id)), ['progress-1']);
  record('Manual close clears the completed batch; a later real completion can show a fresh strip');

  await page.evaluate(() => window.audit.addActive('next-task'));
  await page.waitForSelector('.progress-window-summary [role="progressbar"]');
  assert.equal(await page.$eval('.progress-window-summary [role="progressbar"]', el => el.getAttribute('aria-valuenow')), '25');
  await page.evaluate(() => window.audit.removeCompleted('progress-1'));
  await page.waitForFunction(() => window.audit.snapshot.completed_count === 0);
  assert.equal(await page.$('[data-completed-id="progress-1"]'), null);
  assert.equal(await page.evaluate(() => window.audit.nativeVisible), true);
  record('New active download replaces the completion summary; removing a recent entry never leaves its stale action');
}
