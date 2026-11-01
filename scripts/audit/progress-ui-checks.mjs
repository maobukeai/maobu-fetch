import assert from "node:assert/strict";
import path from "node:path";
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));

async function load({ page, base }, query = "known=1") {
  await page.mouse.move(0, 0);
  await page.goto(base + "?" + query);
  await page.waitForFunction(() => window.audit?.initialCalled);
  await page.waitForSelector(".progress-window-summary");
  if (!query.includes("fail=1")) await page.waitForFunction(() => document.querySelector('.progress-window-summary').getAttribute('aria-label').includes(String(window.audit.snapshot.total_count)));
}
async function revealed(page, value, height = value ? 190 : 28) {
  await page.waitForFunction((expected, size) => document.querySelector('.download-progress-window')?.dataset.revealed === String(expected)
    && document.getElementById('root').getBoundingClientRect().height === size, {}, value, height);
}
async function open(page) { await page.hover('.progress-window-summary'); await revealed(page, true); }

export async function checkCompact(context) {
  const { page, record, output } = context;
  await load(context);
  await revealed(page, false);
  assert.equal(await page.$eval('.progress-window-summary [role="progressbar"]', el => Number(el.getAttribute('aria-valuenow'))), 78.125);
  assert.equal(await page.$('#progress-window-details'), null);
  assert.equal(await page.$eval('.download-progress-window', el => el.querySelectorAll('button').length), 1);
  const size = await page.$eval('.download-progress-window', el => ({ width: el.clientWidth, height: el.clientHeight }));
  assert.deepEqual(size, { width: 224, height: 28 });
  record('Idle is one 224x28 strip; aggregate uses the full backend summary, with no hidden controls');
  await load(context, 'count=3&known=1');
  await page.$eval('#audit-backdrop', el => { el.hidden = true; });
  await page.$('.download-progress-window').then(el => el.screenshot({ path: path.join(output, 'compact-preview.png'), omitBackground: true }));
  await load(context, 'count=8');
  assert.equal(await page.$eval('.progress-window-summary [role="progressbar"]', el => el.hasAttribute('aria-valuenow')), false);
  assert.equal(await page.$('.progress-window-summary-track > span'), null);
  record('Unknown aggregate has no fabricated percentage or moving fill');
  await load(context, 'count=0');
  assert.equal(await page.$eval('.progress-window-summary [role="progressbar"]', el => el.hasAttribute('aria-valuenow')), false);
  record('Empty snapshot does not fabricate progress');
}

export async function checkSummaryActivation(context) {
  const { page, record } = context;
  await load(context);
  await open(page);
  await page.click('.progress-window-summary');
  await delay(350);
  await revealed(page, true);
  assert.equal(await page.evaluate(() => window.audit.calls.some(call => call[0] === 'reveal' && call[1] === false)), false);
  record('First click preserves the details already opened by pointer enter and focus');

  await load(context);
  await page.focus('.progress-window-summary');
  await revealed(page, true);
  for (const key of ['Enter', 'Space']) {
    await page.keyboard.press(key);
    await delay(200);
    await revealed(page, true);
  }
  await page.keyboard.press('Escape');
  await revealed(page, false);
  await page.keyboard.press('Enter');
  await delay(200);
  await revealed(page, true);
  record('Enter and Space reliably reveal details, including after Escape without moving focus');

  await load(context);
  await page.evaluate(() => { window.audit.holdReveal = true; });
  await page.hover('.progress-window-summary');
  await page.waitForFunction(() => Boolean(window.audit.releaseReveal));
  await page.click('.progress-window-summary');
  await page.evaluate(() => { window.audit.holdReveal = false; window.audit.releaseReveal(); });
  await revealed(page, true);
  assert.deepEqual(await page.evaluate(() => window.audit.calls.filter(call => call[0] === 'reveal').map(call => call[1])), [true]);
  await page.click('#audit-external-focus');
  await revealed(page, false);
  record('Click during pending reveal preserves one reveal request; leaving still collapses');
}

export async function checkControls(context) {
  const { page, record, output } = context;
  await load(context);
  const before = await page.$eval('.progress-window-summary', el => el.getBoundingClientRect().y);
  await open(page);
  assert.equal(await page.$eval('.progress-window-summary', el => el.getBoundingClientRect().y), before);
  assert.equal(await page.$$('.progress-window-task').then(rows => rows.length), 3);
  assert.equal(await page.$eval('[data-task-id="progress-2"] .progress-window-task-state', el => el.textContent.trim()), '校验中');
  assert.ok(!(await page.$eval('[data-task-id="progress-2"] .progress-window-task-meta', el => el.textContent)).includes('/s'));
  record('Verification state stays explicit and does not masquerade as verification percentage or download speed');
  await page.click('.progress-window-fold');
  await revealed(page, true, 298);
  assert.equal(await page.$$('.progress-window-task').then(rows => rows.length), 6);
  assert.equal(await page.$eval('.progress-window-summary', el => el.getBoundingClientRect().y), before);
  await page.$('.download-progress-window').then(el => el.screenshot({ path: path.join(output, 'expanded-six.png') }));
  record('Hover expands upwards; bottom anchor stays fixed through delayed native resize and 3/6 rows');
  await page.click('[aria-pressed]');
  await page.waitForSelector('[aria-pressed="false"]');
  await page.evaluate(() => { window.audit.failPin = true; });
  await page.click('[aria-pressed]');
  await page.waitForSelector('[role="alert"]');
  assert.equal(await page.$eval('[aria-pressed]', el => el.getAttribute('aria-pressed')), 'false');
  assert.ok(!(await page.$eval('body', el => el.textContent)).includes('synthetic-private'));
  record('Pin changes state; failed pin preserves state without exposing private errors');
  await load(context);
  await page.focus('.progress-window-summary');
  await revealed(page, true);
  await page.mouse.move(0, 0);
  await delay(250);
  assert.ok(await page.$('#progress-window-details'));
  await page.keyboard.press('Escape');
  await revealed(page, false);
  assert.equal(await page.evaluate(() => window.audit.calls.filter(call => call[0] === 'dismiss').length), 0);
  await page.keyboard.press('Escape');
  await page.waitForFunction(() => window.audit.calls.some(call => call[0] === 'dismiss'));
  record('Keyboard focus retains details; first Escape folds and second hides');
  await load(context);
  await open(page);
  await page.click('[data-task-id="progress-0"] .progress-window-task-name');
  await page.waitForFunction(() => window.audit.calls.some(call => call[0] === 'main' && call[1] === 'progress-0'));
  await page.click('[data-task-id="progress-0"] .progress-window-task-action');
  await page.waitForFunction(() => !document.querySelector('[data-task-id="progress-0"]'));
  record('Exact task view and pause actions remain accessible in details');
}

export async function checkRevealRaces(context) {
  const { page, record } = context;
  await load(context);
  await page.evaluate(() => { window.audit.holdReveal = true; });
  await page.hover('.progress-window-summary');
  await page.waitForFunction(() => Boolean(window.audit.releaseReveal));
  await page.mouse.move(0, 0);
  await delay(200);
  await page.evaluate(() => { window.audit.holdReveal = false; window.audit.releaseReveal(); });
  await page.waitForFunction(() => window.audit.calls.some(call => call[0] === 'reveal' && call[1] === false));
  await revealed(page, false);
  assert.deepEqual(await page.evaluate(() => window.audit.calls.filter(call => call[0] === 'reveal').map(call => call[1])), [true, false]);
  record('Rapid enter/leave preserves the final collapse intent while reveal is in flight');
  await load(context);
  await open(page);
  await page.evaluate(() => { window.audit.holdPin = true; });
  await page.click('[aria-pressed]');
  await page.waitForFunction(() => Boolean(window.audit.releasePin));
  await page.click('#audit-external-focus');
  await revealed(page, false);
  await page.evaluate(() => { window.audit.holdPin = false; window.audit.releasePin(); });
  await page.waitForFunction(() => window.audit.snapshot.always_on_top === false);
  await revealed(page, false);
  record('Busy pin operation cannot swallow hover/focus leave or reopen collapsed details');
}

export async function checkRecovery(context) {
  const { page, record } = context;
  await load(context, 'fail=1');
  await open(page);
  await page.waitForSelector('[role="alert"]');
  await page.evaluate(() => { window.audit.failSnapshot = false; });
  await page.click('[role="alert"] button');
  await page.waitForFunction(() => !document.querySelector('[role="alert"]'));
  assert.equal(await page.evaluate(() => window.audit.listenerCount), 1);
  record('Initial read error can reveal an actionable retry without leaking a subscription');
  await load(context, 'race=1&known=1');
  await page.evaluate(() => window.audit.emit({ summary_percent: 12.5 }));
  await delay(220);
  assert.equal(await page.$eval('.progress-window-summary [role="progressbar"]', el => Number(el.getAttribute('aria-valuenow'))), 12.5);
  record('Late initial snapshot cannot replace a newer summary event');
}

export async function checkSettings({ page, base, record }) {
  await page.goto(base + '?mode=settings');
  await page.waitForSelector('input.toggle');
  await page.click('input.toggle');
  assert.ok(await page.$eval('button.secondary', el => el.disabled));
  assert.equal(await page.evaluate(() => window.audit.calls.filter(call => call[0] === 'restore').length), 0);
  await page.goto(base + '?mode=settings&saved=1');
  await page.waitForSelector('button.secondary:not(:disabled)');
  await page.click('button.secondary');
  await page.waitForFunction(() => window.audit.calls.some(call => call[0] === 'restore'));
  record('Restore still respects saved settings and is available for enabled progress');
}

export async function checkMatrix(context) {
  const { page, record, output } = context;
  for (const [width, height] of [[1280, 720], [1440, 900]]) {
    for (const scale of [1, 1.25]) {
      for (const theme of ['light', 'dark']) {
        await page.setViewport({ width, height, deviceScaleFactor: scale });
        await page.emulateMediaFeatures([{ name: 'prefers-color-scheme', value: theme }]);
        await load(context, 'count=500&known=1');
        const transparent = await page.evaluate(() => [document.documentElement, document.body, document.getElementById('root')].every(el => getComputedStyle(el).backgroundColor === 'rgba(0, 0, 0, 0)'));
        assert.ok(transparent, 'The document must remain transparent in both themes');
        await page.$('.download-progress-window').then(el => el.screenshot({ path: path.join(output, `${width}-${height}-${scale}-${theme}-compact.png`) }));
        await open(page);
        const bounds = await page.$eval('.download-progress-window', el => {
          const r = el.getBoundingClientRect();
          return { inside: r.x >= 0 && r.y >= 0 && r.right <= innerWidth && r.bottom <= innerHeight,
            fit: [...el.querySelectorAll('button')].every(button => { const b = button.getBoundingClientRect(); return b.width > 0 && b.left >= r.left && b.right <= r.right + 1; }),
            rows: el.querySelectorAll('.progress-window-task').length, theme: document.documentElement.dataset.theme };
        });
        assert.ok(bounds.inside && bounds.fit && bounds.rows === 3, JSON.stringify(bounds));
        assert.equal(bounds.theme, theme);
        await page.$('.download-progress-window').then(el => el.screenshot({ path: path.join(output, `${width}-${height}-${scale}-${theme}-details.png`) }));
        record(`Transparent compact/details ${width}x${height}, ${scale}, ${theme}, bounded 500-task input`);
      }
    }
  }
}

export async function checkEntryScope({ page, base, record }) {
  const origin = new URL(base).origin;
  await page.setRequestInterception(true);
  const intercept = request => request.url().includes('/src/main.tsx') ? request.abort() : request.continue();
  page.on('request', intercept);
  try {
    await page.goto(origin + '/?view=download-progress');
    assert.ok(await page.evaluate(() => document.documentElement.classList.contains('progress-window-page')));
    assert.ok(await page.evaluate(() => [document.documentElement, document.body, document.getElementById('root')].every(el => getComputedStyle(el).backgroundColor === 'rgba(0, 0, 0, 0)')));
  } finally {
    page.off('request', intercept);
    await page.setRequestInterception(false);
  }
  await page.goto(origin + '/');
  await page.waitForSelector('.workspace');
  assert.equal(await page.evaluate(() => document.documentElement.classList.contains('progress-window-page')), false);
  assert.notEqual(await page.$eval('body', el => getComputedStyle(el).backgroundColor), 'rgba(0, 0, 0, 0)');
  record('Production HTML is transparent before its main bundle and does not make the main window transparent');
}
