import assert from "node:assert/strict";
import { existsSync } from "node:fs";
import { mkdtemp, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import puppeteer from "puppeteer-core";
import { createAuditServer } from "./ui-server.mjs";

const output = await mkdtemp(path.join(tmpdir(), "maobu-recording-ui-"));
const server = await createAuditServer(output);
const results = [];
const record = (name, detail) => { results.push({ name, detail, passed:true }); console.log("PASS", name); };
const live = '.task-row[data-task-id="audit-0"]';
const stop = `${live} button[aria-label="停止并保存录制"]`;
const pause = `${live} button[title="暂停"]`;
let browser;
try {
  await server.listen();
  const executablePath = [
    "C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe",
    "C:/Program Files/Google/Chrome/Application/chrome.exe",
  ].find(existsSync);
  assert.ok(executablePath, "只使用已经安装的浏览器");
  browser = await puppeteer.launch({headless:true, executablePath});
  const page = await browser.newPage();
  page.setDefaultTimeout(15000);
  const errors = [];
  page.on("pageerror", error => errors.push(String(error)));
  const url = `http://127.0.0.1:${server.httpServer.address().port}/__audit.html?mode=recording`;
  const load = async () => {
    await page.goto(url);
    await page.waitForSelector(stop);
    await page.waitForSelector("#splash-screen", {hidden:true});
  };
  await page.setViewport({width:1280,height:720,deviceScaleFactor:1});
  await load();
  assert.equal(await page.$$eval('button[aria-label="停止并保存录制"]', els => els.length), 1);
  for (const id of ["audit-1","audit-2","audit-3"]) {
    assert.equal(await page.$(`.task-row[data-task-id="${id}"] button[aria-label="停止并保存录制"]`), null);
  }
  assert.ok(await page.$eval(live + " .task-status", element => element.textContent.includes("录制中")));
  record("仅实际下载中的直播显示独立停止保存按钮", "VoD、已暂停直播、普通任务均不显示");
  const realProgress=await page.$eval(live+' .progress-cell > span',element=>element.textContent);
  assert.match(realProgress.trim(),/^64(?:[.,]0)?\s*K(?:i)?B$/,'65536个真实录制字节应可读地显示，不能用百分比或未知代替');
  record('直播未知总长显示真实录制字节','不把未知总长显示为0%');

  await page.mouse.move(0,0);
  await page.focus(stop);
  await page.keyboard.down("Shift");
  await page.keyboard.press("Tab");
  await page.keyboard.up("Shift");
  await page.waitForFunction(selector => {
    const button=document.querySelector(selector);
    return button === document.activeElement && button.matches(':focus-visible')
      && Number(getComputedStyle(button).opacity) > 0.5;
  },{},pause);
  await page.keyboard.press("Enter");
  await page.waitForFunction(() => window.audit.calls.length === 1);
  assert.deepEqual(await page.evaluate(() => window.audit.calls), [["action","audit-0","pause"]]);
  await page.evaluate(() => window.audit.updateTask({...window.audit.tasks[0],status:'paused'}));
  await page.waitForSelector(stop, {hidden:true});
  record("键盘焦点可见且Enter暂停只发送pause，暂停事件后移除停止保存入口");

  await load();
  await page.mouse.move(0,0);
  await page.focus(pause);
  await page.keyboard.press("Tab");
  assert.equal(await page.$eval(stop, element => element === document.activeElement), true);
  const focused = await page.$eval(stop, element => ({
    visible:element.matches(':focus-visible'),opacity:getComputedStyle(element).opacity,
    outline:getComputedStyle(element).outlineStyle,
  }));
  assert.ok(focused.visible && Number(focused.opacity) > 0.5 && focused.outline !== "none", JSON.stringify(focused));
  await page.evaluate(() => { window.audit.holdAction = 'finish-recording'; });
  await page.keyboard.press("Enter");
  await page.waitForFunction(() => window.audit.calls.length === 1);
  assert.deepEqual(await page.evaluate(() => window.audit.calls), [["action","audit-0","finish-recording"]]);
  assert.equal(await page.$eval(stop, element => element.disabled && element.getAttribute("aria-busy") === "true"), true);
  await page.keyboard.press("Enter");
  assert.equal(await page.evaluate(() => window.audit.calls.length), 1, "保存中不重复发送");
  await page.evaluate(() => window.audit.releaseAction());
  await page.waitForFunction(() => document.body.textContent.includes("录制已停止并保存"));
  record("Tab焦点与Enter独立停止保存", "有可见焦点，finish-recording等待中禁用且不重复触发");

  await load();
  await page.evaluate(() => { window.audit.failAction = 'finish-recording'; });
  await page.click(stop);
  await page.waitForFunction(() => document.body.textContent.includes("原始数据已保留"));
  assert.equal(await page.$eval(stop, element => element.disabled), false);
  assert.equal(await page.evaluate(() => document.body.textContent.includes("录制已停止并保存")), false);
  record("停止失败反馈保留重试入口，未假报保存成功");

  for (const [width,height] of [[1280,720],[1440,900],[780,540]]) {
    for (const scale of [1,1.25]) {
      for (const theme of ["light","dark"]) {
        await page.setViewport({width,height,deviceScaleFactor:scale});
        await page.emulateMediaFeatures([{name:"prefers-color-scheme",value:theme}]);
        await load();
        await page.evaluate(value => { document.documentElement.style.zoom = String(value); }, scale);
        await page.mouse.move(0,0);
        await page.focus(pause);
        await page.keyboard.press("Tab");
        const bounds = await page.$eval(stop, element => {
          const r=element.getBoundingClientRect(); const style=getComputedStyle(element);
          const connection=element.closest('.task-row').querySelector('.connection-count');
          const range=document.createRange();range.selectNodeContents(connection);
          const label=element.closest('.task-status').querySelector('.task-status-label');
          const labelRange=document.createRange();labelRange.selectNodeContents(label);
          const lineTops=[...labelRange.getClientRects()].map(rect=>Math.round(rect.top*100)/100);
          return {x:r.x,y:r.y,right:r.right,bottom:r.bottom,width:innerWidth,height:innerHeight,
            visible:style.visibility,opacity:Number(style.opacity),focus:element.matches(':focus-visible'),
            connectionTextLeft:range.getBoundingClientRect().left,
            singleLine:lineTops.length>0 && new Set(lineTops).size===1 && getComputedStyle(label).whiteSpace==='nowrap',
            fullTitle:label.title===label.textContent && label.getAttribute('aria-label')===label.textContent,
            hit:element.contains(document.elementFromPoint(r.x+r.width/2,r.y+r.height/2))};
        });
        assert.ok(bounds.x>=0 && bounds.y>=0 && bounds.right<=bounds.width+1 && bounds.bottom<=bounds.height+1
          && bounds.hit && bounds.focus && bounds.visible==="visible" && bounds.opacity>0.5
          && bounds.singleLine && bounds.fullTitle && bounds.right+2<=bounds.connectionTextLeft, JSON.stringify(bounds));
        const name=`${width}x${height}-${scale}-${theme}`;
        await page.screenshot({path:path.join(output,name+".png")});
        record(name,"真实TaskRow、完整App/正式CSS，停止按钮在视口内且未被其他元素盖住");
      }
    }
  }
  for (const [width,height] of [[1280,720],[780,540]]) {
    await page.setViewport({width,height,deviceScaleFactor:1.25});
    await load();
    await page.evaluate(async () => {
      document.documentElement.style.zoom='1.25';
      const {setLocale}=await import('/src/i18n.ts');setLocale('en');
    });
    const englishStop=live+' button[aria-label="Stop and save recording"]';
    await page.waitForSelector(englishStop);
    const geometry=await page.$eval(live, element => {
      const button=element.querySelector('button[aria-label="Stop and save recording"]');
      const rect=button.getBoundingClientRect(); const connection=element.querySelector('.connection-count');
      const range=document.createRange();range.selectNodeContents(connection);
      return {right:rect.right,connectionLeft:connection.getBoundingClientRect().left,
        connectionTextLeft:range.getBoundingClientRect().left,viewWidth:innerWidth,
        hit:button.contains(document.elementFromPoint(rect.x+rect.width/2,rect.y+rect.height/2))};
    });
    await page.screenshot({path:path.join(output,`${width}x${height}-1.25-en-dark.png`)});
    console.log("English geometry",width,JSON.stringify(geometry));
    assert.ok(geometry.hit && geometry.right<=geometry.viewWidth+1 && geometry.right+2<=geometry.connectionTextLeft,JSON.stringify(geometry));
    record(`${width}x${height}英文125%`,"Recording文案及两个独立按钮不覆盖连接列");
  }
  assert.deepEqual(errors,[]);
  console.log("Output:",output);
} finally {
  await writeFile(path.join(output,"results.json"),JSON.stringify({date:new Date().toISOString(),results},null,2));
  await browser?.close();
  await server.close();
}
