import assert from 'node:assert/strict';

export async function assertNoInternalCompletion(page) {
  await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
  assert.equal(await page.$('.completion-toast'), null, '完成事件不能重复渲染应用内完成卡片');
  assert.equal(await page.$$eval('.toast-stack [role="status"]', nodes =>
    nodes.some(node => node.textContent.includes('下载完成'))), false);
}

export async function checkCompletionEvents(page, base, record) {
  await page.goto(base + '?count=2');
  await page.waitForSelector('#splash-screen', { hidden: true });
  await page.waitForFunction(() => window.audit?.notificationListenerCount > 0);
  await page.click('.nav-settings');
  await page.waitForSelector('.settings-actions button.primary');
  const settings = await checkNoticeModes(page, record);
  for (const mode of ['desktop', 'system']) {
    for (const duration of [0, 15, 30, 60]) {
      await page.evaluate(({ settings, mode, duration }) => {
        window.audit.updateSettings({ ...settings, completion_notice_mode: mode, completion_notice_seconds: duration, notify_sound_enabled: false });
      }, { settings, mode, duration });
      await page.evaluate(() => { window.audit.emit(); window.audit.emit('audit-1'); window.audit.emit(); });
      await assertNoInternalCompletion(page);
    }
  }
  record('两种模式完成事件均不在主窗重复显示卡片', '完整 App 接收连续完成事件；desktop/system × 0/15/30/60 秒配置均不恢复主窗卡片');
  return settings;
}

async function checkNoticeModes(page, record) {
  const modeSelector = '[role="combobox"][aria-label="完成提醒方式"]';
  const durationSelector = '[role="combobox"][aria-label="完成提醒停留时间"]';
  assert.equal(await page.$eval(modeSelector, node => node.textContent.trim()), '独立桌面提示窗');
  assert.equal(await page.$eval(durationSelector, node => node.textContent.trim()), '30 秒');
  for (const [seconds, label] of [[15, '15 秒'], [30, '30 秒'], [60, '60 秒'], [0, '手动关闭']]) {
    await selectHistoryFilter(page, '完成提醒停留时间', label);
    await page.click('.settings-actions button.primary');
    await page.waitForSelector('.settings-page', { hidden: true });
    assert.equal(await page.evaluate(() => window.audit.saved?.completion_notice_seconds), seconds, '完成提醒时长必须以数字保存');
    await reopenSettings(page);
  }
  await selectHistoryFilter(page, '完成提醒方式', 'Windows 系统通知');
  assert.equal(await page.$(durationSelector), null, '系统模式由 Windows 控制时长');
  assert.ok(await page.$eval('.settings-page', node => node.textContent.includes('外观和停留时间由系统控制')));
  await page.click('.settings-actions button.primary');
  await page.waitForFunction(() => window.audit.saved?.completion_notice_mode === 'system');
  await reopenSettings(page);
  await selectHistoryFilter(page, '完成提醒方式', '独立桌面提示窗');
  assert.equal(await page.$eval(durationSelector, node => node.textContent.trim()), '手动关闭', '切换模式保留桌面提醒时长');
  await selectHistoryFilter(page, '完成提醒停留时间', '30 秒');
  await page.click('.settings-actions button.primary');
  await page.waitForFunction(() => window.audit.saved?.completion_notice_mode === 'desktop' && window.audit.saved.completion_notice_seconds === 30);
  record('完成提醒双模式设置与时长保存', '默认独立桌面提示窗30秒；15/30/60/手动均保存；系统模式隐藏时长并解释通知中心；切回保留用户选择');
  return page.evaluate(() => window.audit.saved);
}

async function reopenSettings(page) {
  await page.waitForSelector('.settings-page', { hidden: true });
  await page.waitForSelector('.toast-stack .toast', { hidden: true });
  await page.click('.nav-settings');
  await page.waitForSelector('[role="combobox"][aria-label="完成提醒方式"]');
}

export async function checkSystemNotificationFocus(page, base, settings, record) {
  const preload = await page.evaluateOnNewDocument(value => { window.__auditSettings = value; }, settings);
  try {
    await page.goto(base + '?count=500&native=1');
    await page.waitForSelector('.task-row');
    await page.waitForSelector('#splash-screen', { hidden: true });
    await page.type('.search-box input', '不存在的搜索内容');
    await page.waitForFunction(() => !document.querySelector('.task-row'));
    await page.click('.nav-settings');
    await page.waitForSelector('.settings-actions button.primary');
    await emitTaskFocus(page, 'audit-0');
    await page.waitForSelector('.details-pane');
    assert.equal(await page.$('.settings-page'), null, '系统通知定位任务时关闭设置页');
    assert.equal(await page.$eval('.search-box input', input => input.value), '');
    assert.equal(await page.$eval('.task-row[data-task-id="audit-0"]', row => row.classList.contains('selected')), true);
    await page.evaluate(() => window.audit.emit());
    await assertNoInternalCompletion(page);
    record('系统通知定位任务、展开详情并清除过滤', '真实 App 的 notification-focus-task 监听；Tauri 官方 IPC/event mocks，无原生命令或用户数据库访问');
    await checkArchivedNotificationFocus(page, record);
  } finally {
    await page.removeScriptToEvaluateOnNewDocument(preload.identifier);
  }
}

async function emitTaskFocus(page, id) {
  await page.evaluate(async id => {
    const { emit } = await import('/node_modules/@tauri-apps/api/event.js');
    await emit('notification-focus-task', id);
  }, id);
}

async function selectHistoryFilter(page, label, value) {
  await page.click(`[role="combobox"][aria-label="${label}"]`);
  await page.waitForSelector('[role="listbox"]');
  await page.evaluate(value => {
    const option = [...document.querySelectorAll('[role="option"]')].find(node => node.textContent.trim() === value);
    if (!option) throw new Error('Missing history option: ' + value);
    option.click();
  }, value);
  await page.waitForSelector('[role="listbox"]', { hidden: true });
}

async function checkArchivedNotificationFocus(page, record) {
  await page.evaluate(() => window.audit.updateTask({ ...window.audit.tasks[5], completed_at: Date.now() - 40 * 86_400_000 }));
  await page.waitForSelector('.task-row[data-task-id="audit-5"]', { hidden: true });
  await page.click('.nav-item[title="历史归档"]');
  await page.waitForSelector('.task-row[data-task-id="audit-5"]');
  await selectHistoryFilter(page, '状态', '失败');
  await selectHistoryFilter(page, '完成日期', '今天');
  assert.equal(await page.$('.task-row[data-task-id="audit-5"]'), null, '测试前置条件：历史筛选隐藏了目标完成任务');
  await page.evaluate(() => [...document.querySelectorAll('.nav-item')]
    .find(node => node.querySelector('span')?.textContent === '全部任务').click());
  await page.waitForSelector('.history-filter-bar', { hidden: true });
  await emitTaskFocus(page, 'audit-5');
  await page.waitForSelector('.history-filter-bar');
  await page.waitForSelector('.task-row[data-task-id="audit-5"].selected');
  assert.ok(await page.$('.task-row[data-task-id="audit-5"]').then(row => row.isIntersectingViewport()), '归档目标任务必须滚动至可见区域');
  assert.ok(await page.$('.details-pane'));
  assert.equal(await page.$eval('[role="combobox"][aria-label="状态"]', node => node.textContent.trim()), '全部状态');
  assert.equal(await page.$eval('[role="combobox"][aria-label="完成日期"]', node => node.textContent.trim()), '全部时间');
  record('旧通知定位已归档任务并清历史筛选', '40 天前完成任务；原有失败/今天筛选与主列表视图均不阻止系统通知展开目标详情');
}
