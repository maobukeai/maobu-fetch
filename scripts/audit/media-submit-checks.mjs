import assert from 'node:assert/strict';

async function enterUrl(page, url) {
  await page.waitForSelector('.new-task-form textarea');
  await page.focus('.new-task-form textarea');
  await page.keyboard.down('Control');
  await page.keyboard.press('KeyA');
  await page.keyboard.up('Control');
  await page.keyboard.type(url);
}

export async function checkDouyinSubmit(page, base, results) {
  const shortUrl = 'https://v.douyin.com/SyntheticShare/';
  await page.goto(base + '?mode=unknown&pendingPlatform=1');
  await enterUrl(page, `6.94 作品分享文案 ${shortUrl} 复制此链接，打开抖音观看视频！`);
  await page.click('.confirm-btn');
  await page.waitForFunction(() => window.audit.probeOptions.length === 1 || window.audit.calls.length > 0);
  assert.equal(await page.evaluate(() => window.audit.calls.length), 0, 'A Douyin page must never become a bare HTTP task');
  assert.deepEqual(await page.evaluate(() => window.audit.probeUrls), [shortUrl]);
  await page.waitForSelector('.media-result-card [role="status"]');
  await page.focus('[aria-label="视频格式选择"]');
  await page.keyboard.press('ArrowDown');
  await page.keyboard.press('Enter');
  await page.click('.confirm-btn');
  await page.waitForFunction(() => window.audit.calls.length === 1);
  assert.equal(await page.evaluate(() => window.audit.calls[0].media.format_id), 'unknown-4k');
  results.push('Direct submit of Douyin share text probes first while platform detection is pending; only explicit format selection creates a media task');

  await page.goto(base + '?pendingPlatform=1');
  await enterUrl(page, shortUrl + '\nhttps://example.test/file.bin');
  await page.click('.confirm-btn');
  await page.waitForFunction(() => document.querySelector('.inline-error')?.textContent.includes('逐条添加'));
  assert.deepEqual(await page.evaluate(() => [window.audit.calls.length, window.audit.batchCalls.length, window.audit.probeUrls.length]), [0, 0, 0]);
  results.push('Mixed batches containing Douyin pages require individual analysis and never submit bare pages');

  for (const url of ['https://example.test/file.bin', 'https://v3-web.douyinvod.com/video/tos/file.mp4']) {
    await page.goto(base + '?pendingPlatform=1');
    await enterUrl(page, url);
    await page.click('.confirm-btn');
    await page.waitForFunction(() => window.audit.calls.length === 1);
    assert.equal(await page.evaluate(() => window.audit.probeOptions.length), 0);
    assert.equal(await page.evaluate(() => window.audit.calls[0].url), url);
    assert.equal(await page.evaluate(() => window.audit.calls[0].media), undefined);
  }
  results.push('Ordinary HTTP and direct CDN files still submit directly without media probing');
}
