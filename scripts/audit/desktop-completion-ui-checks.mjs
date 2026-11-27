import assert from 'node:assert/strict';
import path from 'node:path';

const card = '.desktop-completion-card';
export async function loadDesktop(context, query = 'duration=0') {
  await context.page.goto(context.base + '?' + query);
  await context.page.waitForSelector(card);
  await context.page.waitForFunction(() => window.audit.presentedId === window.audit.snapshot.notice?.notice_id);
}

export async function clickDesktopAction(page, label) {
  await page.evaluate(label => {
    const button = [...document.querySelectorAll('.desktop-completion-card button')]
      .find(node => node.textContent.trim() === label || node.getAttribute('aria-label') === label);
    if (!button) throw new Error('Missing completion action: ' + label);
    button.click();
  }, label);
}

export async function checkDesktopActions(context) {
  const { page, record } = context;
  await loadDesktop(context, 'duration=0&count=3');
  assert.match(await page.$eval(card, node => node.textContent.replace(/\s/g, '')), /另有2/);
  for (const [index, label, action] of [[0, '打开文件', 'file'], [1, '打开文件夹', 'folder'], [2, '查看详情', 'details']]) {
    await clickDesktopAction(page, label);
    await page.waitForFunction(count => window.audit.calls.filter(call => call[0] === 'act').length === count, {}, index + 1);
    assert.deepEqual(await page.evaluate(() => window.audit.calls.filter(call => call[0] === 'act').at(-1)),
      ['act', 'desktop-' + index, index + 1, action]);
  }
  await page.waitForSelector(card, { hidden: true });
  record('桌面提醒队列与三个快捷操作', '打开文件/文件夹/详情发送精确 taskId + noticeId；成功依次推进，最后关闭');

  await loadDesktop(context, 'duration=0&count=2');
  await clickDesktopAction(page, '关闭');
  await page.waitForFunction(() => window.audit.snapshot.notice?.notice_id === 2);
  assert.equal(await page.$('.desktop-completion-card').then(Boolean), true);
  await clickDesktopAction(page, '关闭');
  await page.waitForSelector(card, { hidden: true });
  record('手动关闭按队列逐条推进', '关闭当前提醒后保留下一条，不重复或丢失');

  await loadDesktop(context);
  await page.evaluate(() => { window.audit.failAction = 'file'; window.audit.actionError = 'Cookie: synthetic-private; token=synthetic-private'; });
  await clickDesktopAction(page, '打开文件');
  await page.waitForSelector('[role="alert"]');
  assert.equal(await page.evaluate(() => window.audit.snapshot.notice?.notice_id), 1);
  assert.match(await page.$eval('[role="alert"]', node => node.textContent), /保存位置/);
  assert.ok(!(await page.$eval(card, node => node.textContent)).includes('synthetic-private'), '底层错误不应暴露到完成提醒');
  await clickDesktopAction(page, '打开文件夹');
  await page.waitForSelector(card, { hidden: true });
  record('打开失败保留当前提醒并允许重试', '显示可操作中文原因，另一快捷操作成功后关闭');

  await loadDesktop(context);
  await page.focus('.desktop-completion-actions button');
  await page.keyboard.press('Tab');
  assert.equal(await page.evaluate(() => document.activeElement.textContent.trim()), '打开文件夹');
  assert.notEqual(await page.evaluate(() => getComputedStyle(document.activeElement).outlineStyle), 'none');
  await page.keyboard.press('Escape');
  await page.waitForSelector(card, { hidden: true });
  record('桌面提醒键盘操作与焦点可见', 'Tab按顺序切换操作、焦点轮廓可见，Escape关闭当前提醒');
}

export async function checkDesktopRecovery(context) {
  const { page, record } = context;
  await page.goto(context.base + '?duration=0&count=1&race=1');
  await page.waitForFunction(() => window.audit?.initialCalled && window.audit.listenerCount > 0);
  await page.evaluate(() => window.audit.replace('replacement-task'));
  await page.waitForFunction(() => window.audit.snapshot.notice?.task_id === 'replacement-task');
  await page.waitForSelector(card);
  await new Promise(resolve => setTimeout(resolve, 250));
  assert.match(await page.$eval(card, node => node.textContent), /示例文件-2/);
  await clickDesktopAction(page, '查看详情');
  assert.deepEqual(await page.evaluate(() => window.audit.calls.filter(call => call[0] === 'act').at(-1)),
    ['act', 'replacement-task', 2, 'details']);
  record('桌面提醒快照及队首更新恢复', '更新的 noticeId 被动作精确使用，旧快照不使提示状态回退');
  await loadDesktop(context, 'duration=0&presentFail=1');
  assert.ok(await page.evaluate(() => window.audit.presentAttempts >= 2));
  await clickDesktopAction(page, '查看详情');
  await page.waitForSelector(card, { hidden: true });
  record('原生提示窗暂未就绪时自动重试恢复', '首次 present 拒绝，重试成功后卡片仍能执行动作');
}

export async function checkDesktopMatrix(context) {
  const { page, record, output } = context;
  for (const [width, height] of [[1280, 720], [1440, 900]]) {
    for (const scale of [1, 1.25]) {
      for (const theme of ['light', 'dark']) {
        await page.setViewport({ width, height, deviceScaleFactor: scale });
        await page.emulateMediaFeatures([{ name: 'prefers-color-scheme', value: theme }]);
        await loadDesktop(context, 'duration=0&count=3&long=1');
        await page.evaluate(scale => { document.documentElement.style.zoom = String(scale); }, scale);
        await page.waitForFunction(theme => document.documentElement.dataset.theme === theme, {}, theme);
        const fits = await page.$eval(card, node => {
          const rect = node.getBoundingClientRect();
          return rect.left >= 0 && rect.top >= 0 && rect.right <= innerWidth + 1 && rect.bottom <= innerHeight + 1
            && [...node.querySelectorAll('button')].every(button => {
              const bounds = button.getBoundingClientRect();
              return bounds.width > 0 && bounds.height > 0 && bounds.left >= rect.left
                && bounds.right <= rect.right + 1 && bounds.top >= rect.top && bounds.bottom <= rect.bottom + 1;
            });
        });
        assert.ok(fits, '长标题/路径与所有按钮应位于卡片及视口内');
        const name = `${width}x${height}-${scale}-${theme}`;
        await page.screenshot({ path: path.join(output, name + '.png') });
        record('桌面卡片 ' + name, '真实 React 卡片；长中文/特殊字符/路径/3条队列；CSS和设备缩放');
      }
    }
  }
}
