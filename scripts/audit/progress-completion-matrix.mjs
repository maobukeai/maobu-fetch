import assert from 'node:assert/strict';
import path from 'node:path';
import { loadCompletion, waitCompletionDetails } from './progress-completion-checks.mjs';

export async function checkCompletionMatrix(context) {
  const { page, record, output } = context;
  for (const [width, height] of [[1280, 720], [1440, 900], [780, 540]]) {
    for (const scale of width === 780 ? [1] : [1, 1.25]) {
      for (const theme of ['light', 'dark']) {
        await page.setViewport({ width, height, deviceScaleFactor: scale });
        await page.emulateMediaFeatures([{ name: 'prefers-color-scheme', value: theme }]);
        await loadCompletion(context, 0);
        await page.evaluate(() => window.audit.seedCompleted(500));
        await page.waitForSelector('.progress-window-completed-summary');
        await page.waitForFunction(expected => document.documentElement.dataset.theme === expected, {}, theme);
        assert.equal(await page.evaluate(() => window.audit.snapshot.completed_count), 6);
        assert.equal(await page.$eval('#root', el => el.getBoundingClientRect().height), 28);
        assert.equal(await page.$eval('.progress-window-completed-summary', el => {
          const root = el.getBoundingClientRect();
          return [...el.querySelectorAll('button')].every(button => {
            const rect = button.getBoundingClientRect();
            return rect.width > 0 && rect.left >= root.left && rect.right <= root.right + 1;
          });
        }), true);
        await page.$('.download-progress-window').then(el => el.screenshot({ path: path.join(output, `${width}-${height}-${scale}-${theme}-completed-compact.png`), omitBackground: true }));
        await page.hover('.progress-window-completed-reveal');
        await waitCompletionDetails(page);
        assert.equal(await page.$$eval('.progress-window-completed-row', rows => rows.length), 3);
        await page.click('.progress-window-fold');
        await waitCompletionDetails(page);
        assert.equal(await page.$$eval('.progress-window-completed-row', rows => rows.length), 6);
        await page.click('[data-completed-id="completed-1"] .progress-window-completed-folder');
        await page.waitForFunction(() => window.audit.calls.some(call => call[0] === 'open-completed'
          && call[1] === 'completed-1' && call[2] === 'folder'));
        const bounds = await page.$eval('.download-progress-window', el => {
          const root = el.getBoundingClientRect();
          return { inside: root.x >= 0 && root.y >= 0 && root.right <= innerWidth && root.bottom <= innerHeight,
            fit: [...el.querySelectorAll('button')].every(button => {
              const rect = button.getBoundingClientRect();
              return rect.width > 0 && rect.left >= root.left && rect.right <= root.right + 1;
            }), outsideButtons: [...el.querySelectorAll('button')].map(button => {
              const rect = button.getBoundingClientRect();
              return { name:button.getAttribute('aria-label') || button.textContent, width:rect.width, left:rect.left-root.left, right:rect.right-root.right };
            }).filter(button => button.width <= 0 || button.left < 0 || button.right > 1), theme: document.documentElement.dataset.theme };
        });
        assert.ok(bounds.inside && bounds.fit, JSON.stringify(bounds));
        assert.equal(bounds.theme, theme);
        assert.equal(await page.evaluate(() => [document.documentElement, document.body, document.getElementById('root')]
          .every(el => getComputedStyle(el).backgroundColor === 'rgba(0, 0, 0, 0)')), true);
        await page.$('.download-progress-window').then(el => el.screenshot({ path: path.join(output, `${width}-${height}-${scale}-${theme}-completed-details.png`) }));
        record(`Completed compact/actions and bounded long-name 3/6 rows ${width}x${height}, ${scale}, ${theme}`);
      }
    }
  }
}
