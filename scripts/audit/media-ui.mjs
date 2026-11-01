import assert from "node:assert/strict";
import { existsSync } from "node:fs";
import { mkdtemp, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import puppeteer from "puppeteer-core";
import { createAuditServer } from "./ui-server.mjs";
import { mediaUiFixture } from "./media-ui-fixture.mjs";
import { checkDouyinSubmit } from "./media-submit-checks.mjs";

const output = await mkdtemp(path.join(tmpdir(), "maobu-media-ui-"));
const server = await createAuditServer(output);
server.middlewares.use(async (request, response, next) => {
  if (!request.url?.startsWith("/__media-audit.html") || request.url.includes("html-proxy")) return next();
  response.setHeader("Content-Type", "text/html; charset=utf-8");
  response.end(await server.transformIndexHtml("/__media-audit.html", mediaUiFixture));
});
// Place the fixture before Vite's SPA fallback, which otherwise serves the production entry.
server.middlewares.stack.unshift(server.middlewares.stack.pop());
let browser;
const results = [];
const inputStyles = [];
const luminance = (color) => color.match(/[\d.]+/g).slice(0,3).map(Number)
  .map(value => value / 255).map(value => value <= .04045 ? value / 12.92 : ((value+.055)/1.055)**2.4)
  .reduce((sum,value,index) => sum + value * [.2126,.7152,.0722][index],0);
const contrast = (foreground,background) => {
  const values = [luminance(foreground),luminance(background)].sort((a,b)=>b-a);
  return (values[0]+.05)/(values[1]+.05);
};
try {
  await server.listen();
  browser = await puppeteer.launch({ headless:true, executablePath:[
    "C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe",
    "C:/Program Files/Google/Chrome/Application/chrome.exe",
  ].find(existsSync) });
  const page = await browser.newPage();
  const errors = [];
  page.on("pageerror", error => { errors.push(String(error)); console.error(String(error)); });
  const base = `http://127.0.0.1:${server.httpServer.address().port}/__media-audit.html`;
  await checkDouyinSubmit(page, base, results);
  const analyze = async () => {
    await page.waitForSelector(".new-task-form");
    await page.evaluate(() => [...document.querySelectorAll("button")].find(button => button.textContent.trim() === "分析媒体").click());
    await page.waitForSelector(".media-result-card");
  };
  for (const [ffmpeg, expected] of [[true,"merged-1080"],[false,"direct-360"]]) {
    await page.goto(base + "?ffmpeg=" + ffmpeg);
    await analyze();
    await page.waitForFunction(value => document.querySelector('[aria-label="视频格式选择"]').textContent.includes(value), {}, expected);
    if(!ffmpeg)assert.ok((await page.$eval('[aria-label="视频格式选择"]',element=>element.textContent)).includes('约'));
    await page.click(".confirm-btn");
    await page.waitForFunction(() => window.audit.calls.length === 1);
    assert.equal(await page.evaluate(() => window.audit.calls[0].media.format_id), expected);
    assert.equal(await page.evaluate(() => window.audit.probeOptions[0].include_collection), true);
    results.push(`FFmpeg=${ffmpeg}: ${expected}`);
  }
  // A failed second probe must not retain the old selected ID or submit a silent replacement.
  await page.goto(base + "?ffmpeg=false");
  await analyze();
  await page.evaluate(() => { window.audit.result = {...window.audit.result,formats:[{id:'direct-360',label:'silent replacement',has_video:true,has_audio:false}]}; });
  await analyze();
  await page.waitForFunction(() => document.querySelector('.inline-error')?.textContent.includes('音轨'));
  await page.click(".confirm-btn");
  assert.equal(await page.evaluate(() => window.audit.calls.length), 0);
  results.push("Failed reprobe clears stale selection and prevents silent submission");

  await page.goto(base + "?mode=collection&ffmpeg=false");
  await analyze();
  await page.waitForSelector('[role="checkbox"][aria-label^="P1"]');
  await page.focus('[role="checkbox"][aria-label^="P1"]');
  await page.keyboard.press("Space");
  await page.waitForFunction(() => document.querySelector('[role="checkbox"][aria-label^="P1"]').getAttribute('aria-checked') === 'false');
  await page.focus('[aria-label="画质偏好"]');
  await page.keyboard.press("ArrowDown");
  await page.keyboard.press("End");
  await page.keyboard.press("Enter");
  await page.click(".confirm-btn");
  await page.waitForFunction(() => window.audit.calls.length === 1);
  const request = await page.evaluate(() => window.audit.calls[0]);
  assert.ok(request.url.endsWith("?p=2"));
  assert.equal(request.media.format_id, "best[height<=480][vcodec!=none][acodec!=none]");
  assert.equal(request.media.requires_ffmpeg, false);
  results.push("Collection keyboard deselection, exact P2 URL, 480P audible selector");

  await page.goto(base + "?mode=original&ffmpeg=false");
  await analyze();
  await page.waitForFunction(() => document.querySelector('[aria-label="视频格式选择"]').textContent.includes('待检测'));
  await page.click(".confirm-btn");
  await page.waitForFunction(() => window.audit.calls.length === 1);
  assert.equal(await page.evaluate(() => window.audit.calls[0].file_name),"actual-source.mp4");
  assert.equal(await page.evaluate(() => window.audit.calls[0].media.format_id),"original");
  results.push("PikPak explicit original file keeps actual filename and unprobed media label");

  await page.goto(base + "?mode=unknown");
  await analyze();
  assert.ok(!(await page.$('.inline-error')), 'Resolved unknown-audio metadata is a successful manual-choice state');
  await page.waitForFunction(() => document.querySelector('.media-result-card [role="status"]')?.textContent.includes('已找到原片'));
  assert.ok((await page.$eval('.media-result-card [role="status"]', element => element.textContent)).includes('来源未提供音轨信息，请选择原片后下载'));
  await page.click('.confirm-btn');
  assert.equal(await page.evaluate(() => window.audit.calls.length),0);
  await page.focus('[aria-label="视频格式选择"]');
  await page.keyboard.press('ArrowDown');
  await page.keyboard.press('Enter');
  const description=await page.$eval('[aria-label="视频格式选择"]',element=>element.textContent);
  assert.ok(description.includes('音轨未确认'));
  assert.ok(!description.includes('无声'));
  assert.equal(await page.$('.media-result-card [role="status"]'), null);
  await page.click('.confirm-btn');
  await page.waitForFunction(() => window.audit.calls.length===1);
  assert.equal(await page.evaluate(()=>window.audit.calls[0].media.format_id),'unknown-4k');
  results.push('Resolved unknown audio shows neutral manual-choice guidance; explicit selection is required and does not label it silent');

  for (const [formats, expected] of [
    [[{id:'merge-only',label:'merge-only',has_video:true,has_audio:true,requires_ffmpeg:true}], 'FFmpeg'],
    [[], '未找到'],
  ]) {
    await page.goto(base + '?ffmpeg=false');
    await page.evaluate(formats => { window.audit.result = {...window.audit.result,formats}; }, formats);
    await analyze();
    await page.waitForFunction(text => document.querySelector('.inline-error')?.textContent.includes(text), {}, expected);
    assert.equal(await page.$('.media-result-card [role="status"]'), null);
    await page.click('.confirm-btn');
    assert.equal(await page.evaluate(() => window.audit.calls.length), 0);
  }
  results.push('Missing FFmpeg and empty format results remain actionable errors and cannot submit');

  for (const [width,height] of [[1280,720],[1440,900]]) for (const scale of [1,1.25]) for (const theme of ["light","dark"]) {
    await page.setViewport({width,height,deviceScaleFactor:scale});
    await page.emulateMediaFeatures([{name:"prefers-color-scheme",value:theme}]);
    await page.goto(base + "?mode=collection&ffmpeg=false");
    await analyze();
    await page.evaluate((zoom,color) => {
      document.documentElement.style.zoom=String(zoom);
      document.documentElement.dataset.theme=color;
      document.querySelector('.episode-picker-list').scrollIntoView({block:'center'});
    },scale,theme);
    await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
    const bounds = await page.$eval(".confirm-btn",button => {const r=button.getBoundingClientRect();return {top:r.top,left:r.left,right:r.right,bottom:r.bottom,width:innerWidth,height:innerHeight};});
    assert.ok(bounds.top >= 0 && bounds.left >= 0 && bounds.right <= bounds.width + 1 && bounds.bottom <= bounds.height + 1,JSON.stringify(bounds));
    assert.ok(await page.$eval('[role="checkbox"][aria-label^="P1"]',row => {const r=row.getBoundingClientRect();return r.top >= 0 && r.bottom <= innerHeight;}));
    inputStyles.push(await page.$$eval('.new-task-form input', (inputs,mode) => ({mode,
      control:getComputedStyle(document.documentElement).getPropertyValue('--control'),
      inputs:inputs.map(input=>({value:input.value,type:input.type,disabled:input.disabled,parent:input.parentElement.className,
        background:getComputedStyle(input).backgroundColor,color:getComputedStyle(input).color}))
    }),`${width}x${height}-${scale}-${theme}`));
    for (const input of inputStyles.at(-1).inputs.filter(input => input.type === "text" && !input.disabled)) {
      assert.ok(contrast(input.color,input.background) >= 4.5,
        `Unreadable ${theme} input: ${input.color} on ${input.background}`);
    }
    await page.screenshot({path:path.join(output,`${width}x${height}-${scale}-${theme}.png`)});
  }
  assert.deepEqual(errors,[]);
  await writeFile(path.join(output,"results.json"),JSON.stringify({results,inputStyles,screenshots:8,passed:true},null,2));
  console.log("Media component regression passed:",output,results);
} finally {
  await browser?.close();
  await server.close();
}
