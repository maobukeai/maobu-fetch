import assert from 'node:assert/strict';
import { existsSync } from 'node:fs';
import { mkdtemp, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import puppeteer from 'puppeteer-core';
import { createAuditServer } from './ui-server.mjs';
import { desktopCompletionFixture } from './desktop-completion-fixture.mjs';
import { checkDesktopActions, checkDesktopRecovery, checkDesktopMatrix } from './desktop-completion-ui-checks.mjs';
import { checkDesktopTimers } from './desktop-completion-timers.mjs';

const output = await mkdtemp(path.join(tmpdir(), 'maobu-desktop-completion-ui-'));
const server = await createAuditServer(output, { '/__desktop-completion-audit.html': desktopCompletionFixture });
const results = [], errors = [];
const record = (name, detail) => { results.push({ name, detail, passed: true }); console.log('PASS', name); };
let browser;
try {
  await server.listen();
  const executablePath = ['C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe',
    'C:/Program Files/Google/Chrome/Application/chrome.exe'].find(existsSync);
  assert.ok(executablePath, 'Use an existing browser without installing software');
  browser = await puppeteer.launch({ executablePath, headless: true });
  const page = await browser.newPage();
  page.on('pageerror', error => errors.push(error.message));
  const base = 'http://127.0.0.1:' + server.httpServer.address().port + '/__desktop-completion-audit.html';
  const context = { page, base, output, record };
  await checkDesktopActions(context);
  await checkDesktopRecovery(context);
  await checkDesktopMatrix(context);
  if (!process.argv.includes('--no-timers')) await checkDesktopTimers({ executablePath, base, record, errors });
  assert.deepEqual(errors, [], 'No uncaught browser errors');
} finally {
  await writeFile(path.join(output, 'results.json'), JSON.stringify({
    scope: 'Actual desktop completion React view and real browser timers; queue, IPC, native sizing/position/show are isolated substitutes. Rust and native verification validate the OS window and trusted file actions.',
    results, errors,
  }, null, 2));
  await browser?.close();
  await server.close();
  console.log('Evidence:', output);
}
