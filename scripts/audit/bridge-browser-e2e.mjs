import assert from 'node:assert/strict';
import {readFile,writeFile,mkdir,cp,stat} from 'node:fs/promises';
import {setTimeout as delay} from 'node:timers/promises';
import {createHash} from 'node:crypto';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
import puppeteer from 'puppeteer-core';
import {authenticateBridge,verifyRevocation} from './bridge-auth-checks.mjs';
import {exerciseTasks,downloadWhileOffline} from './bridge-task-checks.mjs';

const config=JSON.parse(await readFile(process.argv[2],'utf8'));
config.resume_probe_delay_ms=Math.min(5000,Math.max(0,Number(process.env.MAOBU_TEST_BRIDGE_PROBE_DELAY_MS)||0));
const address=new URL(config.base_url);assert.equal(address.hostname,'127.0.0.1');assert.notEqual(address.port,'17433');
const root=fileURLToPath(new URL('../../',import.meta.url));
const extension=path.join(config.root,'extension');await mkdir(extension,{recursive:true});
await cp(path.join(root,'extension','src'),path.join(extension,'src'),{recursive:true});
await cp(path.join(root,'extension','manifest.json'),path.join(extension,'manifest.json'));
const protocol=path.join(extension,'src','protocol.js');const original=await readFile(protocol,'utf8');
assert.ok(original.includes('export const API = "http://127.0.0.1:17433";'));
await writeFile(protocol,original.replace('export const API = "http://127.0.0.1:17433";',`export const API = ${JSON.stringify(config.base_url)};`));
const downloads=path.join(config.root,'browser-downloads');await mkdir(downloads,{recursive:true});
await mkdir(config.destination,{recursive:true});
const browserPaths={Edge:'C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe',Chrome:'C:/Program Files/Google/Chrome/Application/chrome.exe'};
let browser;
async function wait(pathname){const deadline=Date.now()+20000;while(Date.now()<deadline){try{await stat(pathname);return;}catch{await delay(25);}}throw new Error('Fixture phase handshake timed out');}
async function publish(pathname,value){await writeFile(pathname+'.tmp',JSON.stringify(value));await (await import('node:fs/promises')).rename(pathname+'.tmp',pathname);}
try {
  browser=await puppeteer.launch({executablePath:browserPaths[config.browser],headless:true,pipe:true,enableExtensions:true,userDataDir:path.join(config.root,'profile'),args:['--enable-unsafe-extension-debugging','--disable-background-networking','--no-first-run']});
  const cdp=await browser.target().createCDPSession();await cdp.send('Browser.setDownloadBehavior',{behavior:'allow',downloadPath:downloads,eventsEnabled:true});
  const id=await browser.installExtension(extension);
  await browser.waitForTarget(target=>target.type()==='service_worker'&&target.url().includes(id),{timeout:15000});
  const page=await browser.newPage();await page.goto(`chrome-extension://${id}/src/options.html`);
  const source=await browser.newPage();await source.goto(new URL('/',config.source_url).href);
  const authentication=await page.evaluate(authenticateBridge,config);
  const tasks=await page.evaluate(exerciseTasks,config);
  await publish(config.phase_file,tasks);await wait(config.revoked_file);
  const revocation=await page.evaluate(verifyRevocation);
  await publish(config.offline_ready_file,{ready:true});await wait(config.offline_file);
  const fallback=await page.evaluate(downloadWhileOffline,config);
  assert.ok(path.resolve(fallback.filename).startsWith(path.resolve(downloads)+path.sep),'Browser fixture must not write outside TEMP download directory');
  const bytes=await readFile(fallback.filename);assert.equal(createHash('sha256').update(bytes).digest('hex'),config.expected_sha256);
  console.log(JSON.stringify({browser:config.browser,version:await browser.version(),authentication,revocation,tasks:{pause:tasks.pause,resume:tasks.resume,cancel:tasks.cancel,completed:tasks.completed},desktop_download_sha256:config.expected_sha256,offline_browser_download:{state:fallback.state,bytes:bytes.length,sha256:config.expected_sha256}}));
} finally {await browser?.close();}
