import assert from "node:assert/strict";
import { setTimeout as delay } from "node:timers/promises";
import puppeteer from "puppeteer-core";

async function scenario(base,executablePath,name,duration,verify,record) {
  const browser=await puppeteer.launch({executablePath,headless:true});
  try {
    const page=await browser.newPage();
    await page.goto(base+"?count=1");
    await page.waitForSelector('.task-row');await page.waitForSelector('#splash-screen',{hidden:true});
    if (duration!==30) {
      await page.click('.nav-settings');await page.waitForSelector('select[aria-label="完成提醒停留时间"]');
      await page.select('select[aria-label="完成提醒停留时间"]',String(duration));
      await page.click('.settings-actions button.primary');
      await page.waitForFunction(value=>window.audit.saved?.completion_notice_seconds===value,{},duration);
      await page.evaluate(()=>window.audit.updateSettings(window.audit.saved));
    }
    await page.bringToFront();await page.mouse.move(0,0);
    await page.evaluate(()=>{document.activeElement?.blur();window.audit.emit();});await page.waitForSelector('.completion-toast');
    assert.ok(await page.evaluate(()=>document.hasFocus()&&!document.hidden),'foreground full App must receive the real timer');
    await verify(page,browser);
    record(name,'完整App与真实CompletionToast；独立临时浏览器、真实计时');
  } finally {await browser.close();}
}

export async function verifyCompletionTimers(base,executablePath,record) {
  const scenarios=[
    ['默认30秒自动关闭',30,async page=>{
      await delay(29000);assert.ok(await page.$('.completion-toast'));
      await page.waitForSelector('.completion-toast',{hidden:true,timeout:7000});
    }],
    ['60秒自动关闭',60,async page=>{
      await delay(59000);assert.ok(await page.$('.completion-toast'));
      await page.waitForSelector('.completion-toast',{hidden:true,timeout:7000});
    }],
    ['手动模式超过61秒仍可操作',0,async page=>{
      await delay(31000);await delay(30000);assert.ok(await page.$('.completion-toast'));
      await page.click('.completion-toast .toast-close-btn');await page.waitForSelector('.completion-toast',{hidden:true});
    }],
    ['键盘焦点暂停并在移开后恢复',15,async page=>{
      await page.focus('.completion-toast .toast-action-btn');await delay(16000);assert.ok(await page.$('.completion-toast'));
      await page.evaluate(()=>document.activeElement.blur());await page.mouse.move(0,0);
      await page.waitForSelector('.completion-toast',{hidden:true,timeout:20000});
    }],
    ['窗口失焦隐藏暂停并在返回后恢复',15,async(page,browser)=>{
      const other=await browser.newPage();await other.goto('about:blank');await other.bringToFront();
      await page.waitForFunction(()=>document.hidden||!document.hasFocus());
      await delay(16000);assert.ok(await page.$('.completion-toast'));
      await page.bringToFront();await other.close();await page.mouse.move(0,0);
      await page.waitForFunction(()=>!document.hidden&&document.hasFocus());
      await page.waitForSelector('.completion-toast',{hidden:true,timeout:20000});
    }],
  ];
  const results=await Promise.allSettled(scenarios.map(([name,duration,verify])=>scenario(base,executablePath,name,duration,verify,record)));
  for(const result of results)if(result.status==='rejected')throw result.reason;
}
