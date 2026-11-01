import assert from 'node:assert/strict';
import path from 'node:path';
import { loadCompletion, waitCompletionDetails } from './progress-completion-checks.mjs';

const summaryActions = page => page.$$eval('.progress-window-completed-summary .progress-window-icon', buttons => buttons.map(button => {
  const rect = button.getBoundingClientRect();
  return { label: button.getAttribute('aria-label'), x: rect.x, y: rect.y, width: rect.width, height: rect.height };
}));

export async function checkCompletionPolish(context) {
  const { page, output, record } = context;
  await page.setViewport({ width: 1280, height: 720, deviceScaleFactor: 1 });
  for (const theme of ['light', 'dark']) {
    await page.emulateMediaFeatures([{ name: 'prefers-color-scheme', value: theme }]);
    await loadCompletion(context);
    await page.evaluate(() => {
      window.audit.emit({ tasks: window.audit.snapshot.tasks.map(task => ({ ...task,
        file_name: 'completion-check (1).txt', total_bytes: 98304, downloaded_bytes: 98304,
      })) });
      window.audit.complete('progress-0');
    });
    await page.waitForSelector('.progress-window-completed-summary');
    await page.waitForFunction(expected => document.documentElement.dataset.theme === expected, {}, theme);
    const compactActions = await summaryActions(page);
    await page.$('.download-progress-window').then(element => element.screenshot({
      path: path.join(output, `completion-polish-${theme}-compact.png`), omitBackground: true,
    }));
    await page.hover('.progress-window-completed-reveal');
    await waitCompletionDetails(page);
    assert.equal(await page.$eval('#root', element => element.getBoundingClientRect().height), 94);
    assert.equal(await page.$('.progress-window-footer'), null, 'Completion details have no second footer');
    assert.equal(await page.$$('.lucide-x').then(elements => elements.length), 1, 'Only one close action is visible');
    assert.equal(await page.$$('.progress-window-header .lucide-x').then(elements => elements.length), 0);
    assert.equal(await page.$eval('.progress-window-completed-meta', element => element.textContent.trim()), '96.0 KB');
    assert.equal(await page.$('.progress-window-completed-row .progress-window-completed-open'), null, 'A single-file row reuses the stable bottom actions');
    assert.equal(await page.$('.progress-window-completed-row .progress-window-completed-folder'), null);
    assert.equal(await page.$eval('.download-progress-window', element =>
      (element.textContent.match(/已完成\s*1\s*项/g) || []).length), 1, 'Completion count appears once');
    const expandedActions = await summaryActions(page);
    assert.deepEqual(expandedActions, compactActions, 'Hover expansion preserves each quick-action hit target');
    const fit = await page.$eval('.download-progress-window', element => {
      const root = element.getBoundingClientRect();
      return [...element.querySelectorAll('button')].every(button => {
        const rect = button.getBoundingClientRect();
        return rect.width > 0 && rect.left >= root.left && rect.right <= root.right + 1
          && rect.top >= root.top && rect.bottom <= root.bottom + 1;
      });
    });
    assert.ok(fit, 'Polished single-file controls fit inside the compact native bounds');
    await page.$('.download-progress-window').then(element => element.screenshot({
      path: path.join(output, `completion-polish-${theme}-details.png`), omitBackground: true,
    }));
    await page.focus('.progress-window-completed-reveal');
    await page.keyboard.press('Escape');
    await page.waitForFunction(() => !window.audit.snapshot.revealed && document.getElementById('root').clientHeight === 28);
    assert.equal(await page.evaluate(() => document.activeElement === document.querySelector('.progress-window-completed-reveal')), true);
    await page.keyboard.press('Enter');
    await waitCompletionDetails(page);
    await page.click('.progress-window-completed-summary .progress-window-completed-folder');
    await page.waitForFunction(() => window.audit.calls.some(call => call[0] === 'open-completed' && call[1] === 'progress-0' && call[2] === 'folder'));
    await page.evaluate(() => { window.audit.failOpen = true; });
    await page.click('.progress-window-completed-summary .progress-window-completed-open');
    await page.waitForSelector('[role="alert"]');
    assert.equal(await page.$('.progress-window-completed-row'), null, 'Single-file errors use the row area instead of squeezing content below it');
    const errorFit = await page.$eval('.progress-window-error', element => {
      const error = element.getBoundingClientRect();
      const header = document.querySelector('.progress-window-header').getBoundingClientRect();
      const summary = document.querySelector('.progress-window-completed-summary').getBoundingClientRect();
      const text = element.querySelector('span');
      const retry = element.querySelector('button').getBoundingClientRect();
      return error.height >= 36 && error.top >= header.bottom - 1 && error.bottom <= summary.top + 1
        && retry.top >= error.top && retry.bottom <= error.bottom && text.scrollHeight <= text.clientHeight + 1;
    });
    assert.ok(errorFit, 'Single-file error text and retry fit above the stable bottom actions');
    await page.$('.download-progress-window').then(element => element.screenshot({
      path: path.join(output, `completion-polish-${theme}-error.png`), omitBackground: true,
    }));
    record(`Polished completion ${theme}: one count/close/footer, 94px single-file layout, stable quick actions and keyboard reveal`);
  }
}
