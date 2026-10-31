// Isolated visual/keyboard fixture; synthetic evidence never reaches the native database.
import assert from "node:assert/strict";
import { existsSync } from "node:fs";
import { mkdtemp, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import puppeteer from "puppeteer-core";
import { createAuditServer } from "./ui-server.mjs";

const output = await mkdtemp(path.join(tmpdir(), "maobu-compatibility-ui-"));
const fixture = `<!doctype html><html><head><meta charset="UTF-8"><link rel="icon" href="data:,"></head><body><div id="root" style="padding:16px;width:100%;max-width:1050px;box-sizing:border-box"></div><script type="module">
import React from 'react';import {createRoot} from 'react-dom/client';
import {PlatformCompatibilitySection} from '/src/components/settings/PlatformCompatibilitySection.tsx';import '/src/styles.css';
const params=new URLSearchParams(location.search);document.documentElement.dataset.theme=params.get('theme')||'light';
const record={app_version:'fixture-1',tool_version:'fixture-tool-1',sample:'local-fixture/'+ 'long-sample-name-'.repeat(15),scope:'download',result:'passed'};
const rows=[{platform:'bilibili',level:'verified',notes:'旧记录没有证据，必须显示未验证'},
{platform:'douyin',level:'experimental',notes:'静态能力说明不保证当前可用性'},
{platform:'fixture-full',level:'verified',last_tested_at:'1970-01-01',verification:record},
{platform:'fixture-metadata',level:'experimental',last_tested_at:'1970-01-01',verification:{...record,scope:'metadata'}},
{platform:'fixture-old',level:'verified',last_tested_at:'1970-01-01',verification:{...record,app_version:'old-fixture'}},
{platform:'fixture-failed',level:'experimental',last_tested_at:'1970-01-01',verification:{...record,result:'failed'},known_issues:['可见失败说明']}];
createRoot(document.getElementById('root')).render(React.createElement(PlatformCompatibilitySection,{items:params.has('empty')?[]:rows,appVersion:'fixture-1'}));
</script></body></html>`;
const server = await createAuditServer(output);
server.middlewares.use(async (request, response, next) => {
  if (!request.url?.startsWith("/__compatibility.html") || request.url.includes("html-proxy")) return next();
  response.setHeader("Content-Type", "text/html; charset=utf-8");
  response.end(await server.transformIndexHtml("/__compatibility.html", fixture));
});
server.middlewares.stack.unshift(server.middlewares.stack.pop());
let browser;
const results = [];
try {
  await server.listen();
  browser = await puppeteer.launch({headless: true, executablePath: [
    "C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe", "C:/Program Files/Google/Chrome/Application/chrome.exe"
  ].find(existsSync)});
  const page = await browser.newPage();
  const errors = [];
  page.on("pageerror", error => errors.push(String(error)));
  const url = `http://127.0.0.1:${server.httpServer.address().port}/__compatibility.html`;
  for (const [width,height] of [[1280,720],[1440,900]]) for (const scale of [1,1.25]) for (const theme of ["light","dark"]) {
    await page.setViewport({width,height,deviceScaleFactor:scale});
    await page.goto(url+"?theme="+theme);
    await page.waitForSelector("tbody tr");
    await page.evaluate(zoom => document.documentElement.style.zoom=String(zoom), scale);
    const labels = await page.$$eval("tbody tr", rows => rows.map(row=>row.cells[1].textContent));
    assert.deepEqual(labels,["未验证","未验证","下载样本已验证","仅元数据解析通过","历史样本通过","验证未通过"]);
    assert.ok(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth),"page must not overflow viewport");
    await page.focus("summary");
    await page.keyboard.press("Enter");
    assert.ok(await page.$eval("details", element => element.open),"keyboard expands evidence record");
    await page.screenshot({path:path.join(output,`${width}x${height}-${scale}-${theme}.png`)});
    results.push(`${width}x${height} ${scale} ${theme}: status, long sample, keyboard and viewport passed`);
  }
  await page.goto(url+"?empty=1");
  await page.waitForFunction(()=>document.body.textContent.includes("暂无兼容性记录"));
  assert.deepEqual(errors,[]);
  await writeFile(path.join(output,"results.json"),JSON.stringify(results,null,2));
  console.log(JSON.stringify({output,passed:results.length+1},null,2));
} finally { await browser?.close(); await server.close(); }
