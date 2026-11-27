import assert from 'node:assert/strict';
import { setTimeout as delay } from 'node:timers/promises';
import puppeteer from 'puppeteer-core';
import { loadDesktop, clickDesktopAction } from './desktop-completion-ui-checks.mjs';

const visible = async page => assert.ok(await page.$('.desktop-completion-card'), '当前提醒仍应可操作');
const closed = (page, timeout = 19000) => page.waitForSelector('.desktop-completion-card', { hidden: true, timeout });

async function runScenario({ executablePath, base, record, errors }, name, duration, verify) {
  const scenarioBrowser = await puppeteer.launch({ executablePath, headless: true });
  const page = await scenarioBrowser.newPage();
  page.on('pageerror', error => errors.push(error.message));
  try {
    await page.setViewport({ width: 1280, height: 720 });
    await loadDesktop({ page, base }, `duration=${duration}`);
    await page.mouse.move(0, 0);
    await verify(page);
    record(name, '真实 React 倒计时与真实浏览器时间；独立测试页，队列 IPC 为替身');
  } finally {
    await scenarioBrowser.close();
  }
}

export async function checkDesktopTimers(context) {
  const scenarios = [
    ...[15, 30, 60].map(seconds => [`桌面提醒${seconds}秒自动关闭`, seconds, async page => {
      await delay((seconds - 1) * 1000);
      await visible(page);
      await closed(page, 8000);
      assert.deepEqual(await page.evaluate(() => window.audit.calls.filter(call => call[0] === 'act').map(call => call[3])), ['dismiss']);
    }]),
    ['桌面手动模式超过61秒仍保留', 0, async page => {
      await delay(61000);
      await visible(page);
      assert.equal(await page.evaluate(() => window.audit.calls.filter(call => call[0] === 'act').length), 0);
      await clickDesktopAction(page, '关闭');
      await closed(page);
    }],
    ['桌面鼠标悬停暂停并在移开后恢复', 15, async page => {
      await page.hover('.desktop-completion-card');
      await delay(16000);
      await visible(page);
      await page.mouse.move(0, 0);
      await closed(page);
    }],
    ['桌面键盘焦点暂停并在移开后恢复', 15, async page => {
      await page.focus('.desktop-completion-card button');
      await delay(16000);
      await visible(page);
      await page.evaluate(() => document.activeElement.blur());
      await closed(page);
    }],
    ['桌面打开操作执行中暂停计时', 15, async page => {
      await page.evaluate(() => { window.audit.holdAction = 'file'; });
      await clickDesktopAction(page, '打开文件');
      await page.waitForFunction(() => Boolean(window.audit.releaseAction));
      await delay(16000);
      await visible(page);
      assert.equal(await page.evaluate(() => window.audit.calls.filter(call => call[0] === 'act').length), 1);
      await page.evaluate(() => window.audit.releaseAction());
      await closed(page);
    }],
    ['桌面错误暂停计时并保留重试入口', 15, async page => {
      await page.evaluate(() => { window.audit.failAction = 'file'; });
      await clickDesktopAction(page, '打开文件');
      await page.waitForSelector('[role="alert"]');
      await delay(16000);
      await visible(page);
      await clickDesktopAction(page, '打开文件夹');
      await closed(page);
    }],
    ['队列数量变化不重置当前提醒时长', 15, async page => {
      await delay(10000);
      await page.evaluate(() => window.audit.append('next-queued-task'));
      await page.waitForFunction(() => window.audit.snapshot.notice?.task_id === 'next-queued-task', { timeout: 9000 });
      assert.deepEqual(await page.evaluate(() => window.audit.calls.filter(call => call[0] === 'act')[0]), ['act', 'desktop-0', 1, 'dismiss']);
      await clickDesktopAction(page, '关闭');
      await closed(page);
    }],
    ['相同任务新noticeId重新计时且旧计时不越界', 15, async page => {
      await delay(10000);
      await page.evaluate(() => window.audit.replace('desktop-0'));
      await delay(6000);
      await visible(page);
      assert.equal(await page.evaluate(() => window.audit.calls.filter(call => call[0] === 'act').length), 0);
      await closed(page);
      assert.deepEqual(await page.evaluate(() => window.audit.calls.filter(call => call[0] === 'act')[0]), ['act', 'desktop-0', 2, 'dismiss']);
    }],
  ];
  const results = await Promise.allSettled(scenarios.map(([name, duration, verify]) => runScenario(context, name, duration, verify)));
  for (const result of results) if (result.status === 'rejected') throw result.reason;
}
