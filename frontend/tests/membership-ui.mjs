// Browser regression against the real production frontend, isolated mock API.
// Run after frontend build: node frontend/tests/membership-ui.mjs
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { readFile, mkdir, mkdtemp, writeFile } from 'node:fs/promises';
import { resolve, extname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { tmpdir } from 'node:os';
import { spawn } from 'node:child_process';

const web = fileURLToPath(new URL('../../assets/web/', import.meta.url));
const output = resolve(process.env.JX_UI_TEST_OUTPUT ?? `${tmpdir()}/jx-membership-ui-results`);
await mkdir(output, { recursive: true });
const profile = await mkdtemp(resolve(tmpdir(), 'jx-membership-ui-'));
const port = 4176;
const debugPort = 9236;
let expiry = 1793183400;
let state = { logged_in: true, member: true, account: 'uitest', device_suffix: '12345678',
  membership_expires_at: expiry, revision: 1 };
let deadline = performance.now() + 60_000;
let rechargeCount = 0;
let logoutFinished = false;
const pause = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
let config = { language: 'zh-CN', controller_port: 28012, web_port: 28013 };
try {
  // Read-only config to render unchanged device/settings screens realistically.
  const response = await fetch('http://127.0.0.1:28013/api/config/get_config', { signal: AbortSignal.timeout(1500) });
  if (response.ok) config = (await response.json()).data;
} catch { /* The test also works when the original client is not running. */ }
const server = createServer(async (req, res) => {
  const path = new URL(req.url, `http://127.0.0.1:${port}`).pathname;
  const reply = (code, data, message = '成功') => {
    res.writeHead(code, { 'Content-Type': 'application/json' });
    res.end(JSON.stringify({ code, data, message }));
  };
  if (path.startsWith('/api/')) {
    let raw = '';
    for await (const chunk of req) raw += chunk;
    const body = raw ? JSON.parse(raw) : {};
    if (path === '/api/member/status') {
      return reply(200, { ...state, member: state.member && performance.now() < deadline,
        lease_remaining_ms: Math.max(0, deadline - performance.now()) });
    }
    if (path === '/api/member/logout') {
      // Deliberately keep returning stale authenticated status for two seconds.
      await pause(2000);
      state = { ...state, revision: state.revision + 1, member: false, logged_in: false, account: undefined };
      logoutFinished = true;
      return reply(200, state);
    }
    if (path === '/api/member/redeem') {
      rechargeCount += 1;
      await pause(250);
      if (body.card === 'INVALID-CARD') return reply(403, null, '卡密不存在');
      expiry += 86400;
      deadline = performance.now() + 60_000;
      state = { ...state, member: true, revision: state.revision + 1, membership_expires_at: expiry };
      return reply(200, { ...state, lease_remaining_ms: 60_000 });
    }
    if (path === '/api/config/get_config') return reply(200, config);
    if (path === '/api/device/device_list') return reply(200, { controlled_devices: [], adb_devices: [] });
    if (path === '/api/ws/connect') return reply(426, null);
    return reply(200, []);
  }
  const file = resolve(web, `.${path === '/' ? '/index.html' : path}`);
  if (!file.startsWith(resolve(web) + '\\') && !file.startsWith(resolve(web) + '/')) { res.writeHead(403); return res.end(); }
  try {
    const content = await readFile(file);
    res.writeHead(200, { 'Content-Type': ({ '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css', '.png': 'image/png' })[extname(file)] ?? 'application/octet-stream' });
    res.end(content);
  } catch { res.writeHead(404); res.end(); }
});
await new Promise((resolve) => server.listen(port, '127.0.0.1', resolve));
const edge = process.env.JX_TEST_BROWSER ?? 'C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe';
const browser = spawn(edge, ['--headless=new', '--disable-gpu', '--no-first-run', '--no-default-browser-check',
  `--user-data-dir=${profile}`, `--remote-debugging-port=${debugPort}`, `http://127.0.0.1:${port}/`],
  { windowsHide: true, stdio: 'ignore' });
let socket;
let call;
try {
  let page;
  for (let attempt = 0; attempt < 40 && !page; attempt++) {
    try { page = (await (await fetch(`http://127.0.0.1:${debugPort}/json`)).json()).find((entry) => entry.url.startsWith(`http://127.0.0.1:${port}`)); } catch {}
    if (!page) await pause(250);
  }
  assert(page, 'isolated browser did not start');
  socket = new WebSocket(page.webSocketDebuggerUrl);
  await new Promise((resolve, reject) => { socket.addEventListener('open', resolve, { once: true }); socket.addEventListener('error', reject, { once: true }); });
  const pending = new Map();
  let id = 0;
  socket.addEventListener('message', ({ data }) => {
    const message = JSON.parse(data);
    const entry = pending.get(message.id);
    if (!entry) return;
    pending.delete(message.id);
    if (message.error) entry.reject(new Error(message.error.message)); else entry.resolve(message.result);
  });
  call = (method, params = {}) => new Promise((resolve, reject) => {
    const current = ++id;
    pending.set(current, { resolve, reject });
    socket.send(JSON.stringify({ id: current, method, params }));
  });
  const evaluate = async (expression) => {
    const result = await call('Runtime.evaluate', { expression, returnByValue: true, awaitPromise: true });
    if (result.exceptionDetails) throw new Error(JSON.stringify(result.exceptionDetails));
    return result.result.value;
  };
  const waitFor = async (expression) => {
    for (let attempt = 0; attempt < 100; attempt++) {
      if (await evaluate(expression)) return;
      await pause(50);
    }
    throw new Error(`UI condition timed out: ${expression}`);
  };
  const click = async (text) => evaluate(`Array.from(document.querySelectorAll('button')).find(b=>b.textContent.trim()===${JSON.stringify(text)})?.click()`);
  const inputCard = async (value) => evaluate(`(() => { const input=document.querySelector('input[placeholder="请输入卡密"]'); Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value').set.call(input,${JSON.stringify(value)});input.dispatchEvent(new Event('input',{bubbles:true})); })()`);
  const capture = async (name) => {
    const image = await call('Page.captureScreenshot', { format: 'png', captureBeyondViewport: false });
    await writeFile(resolve(output, name), Buffer.from(image.data, 'base64'));
  };
  await call('Page.enable');
  await call('Runtime.enable');
  await call('Emulation.setDeviceMetricsOverride', { width: 1154, height: 648, deviceScaleFactor: 1, mobile: false });
  await call('Page.reload', { ignoreCache: true });
  await waitFor(`!!document.querySelector('.authenticated-app')`);
  await waitFor(`document.body.innerText.includes('USB 设备连接')`);
  assert.equal(await evaluate(`!!document.querySelector('.authenticated-app')`), true);
  const sidebar = await evaluate(`document.querySelector('.ant-layout-sider').innerText`);
  assert(sidebar.includes('设备') && sidebar.includes('编辑映射') && sidebar.includes('设置'));
  assert.equal(await evaluate(`document.querySelector('.membership-toolbar').innerText.includes('2026-10-28 18:30:00')`), true);
  await capture('main.png');
  await click('卡密充值');
  await pause(200);
  await capture('recharge.png');
  await inputCard('INVALID-CARD');
  await click('确认充值');
  await waitFor(`document.querySelector('.ant-modal')?.innerText.includes('卡密不存在')`);
  assert.equal(await evaluate(`document.querySelector('.ant-modal').innerText.includes('卡密不存在')`), true);
  assert.equal(state.membership_expires_at, 1793183400);
  assert.equal(await evaluate(`!!document.querySelector('.authenticated-app')`), true);
  await inputCard('VALID-CARD');
  await evaluate(`(() => { const b=Array.from(document.querySelectorAll('button')).find(b=>b.textContent.trim()==='确认充值'); b.click();b.click(); })()`);
  await waitFor(`document.querySelector('.membership-toolbar')?.innerText.includes('2026-10-29 18:30:00')`);
  assert.equal(rechargeCount, 2, 'one invalid attempt plus exactly one valid redemption');
  assert.equal(await evaluate(`document.querySelector('.membership-toolbar').innerText.includes('2026-10-29 18:30:00')`), true);
  assert.equal(await evaluate(`document.querySelector('.ant-layout-sider').innerText`), sidebar);
  console.log('recharge, duplicate-submit prevention, expiry display, sidebar preservation: PASS');

  // Cloud-origin expiry event; ack must observe the gate, never internal UI.
  deadline = performance.now();
  state = { ...state, member: false, revision: state.revision + 1 };
  await evaluate(`window.__acks=[];window.ipc={postMessage:body=>window.__acks.push({message:JSON.parse(body),protectedVisible:!!document.querySelector('.authenticated-app')})}; document.documentElement.dataset.jxMember='false';window.dispatchEvent(new CustomEvent('jx-membership-status',{detail:${JSON.stringify({ ...state, lease_remaining_ms: 0 })}}))`);
  await waitFor(`window.__acks.at(-1)?.message.member===false`);
  assert.equal(await evaluate(`!!document.querySelector('.authenticated-app')`), false);
  assert.equal(await evaluate(`document.querySelector('.membership-page').innerText.includes('会员未开通或已到期')`), true);
  assert.equal(await evaluate(`window.__acks.at(-1)?.protectedVisible`), false);
  await evaluate(`window.dispatchEvent(new CustomEvent('jx-membership-status',{detail:${JSON.stringify({ ...state, member: true, lease_remaining_ms: 0 })}}))`);
  assert.equal(await evaluate(`!!document.querySelector('.authenticated-app')`), false, 'late already-expired response must never remount protected UI');
  await capture('expired.png');
  // Recharge from expired gate returns to unchanged main page.
  await evaluate(`(() => { const input=document.querySelector('input');Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value').set.call(input,'VALID-CARD');input.dispatchEvent(new Event('input',{bubbles:true})); })()`);
  await click('卡密充值');
  await waitFor(`!!document.querySelector('.authenticated-app')`);
  assert.equal(await evaluate(`!!document.querySelector('.authenticated-app')`), true);
  // Local monotonic lease expires even without a native expiry event.
  deadline = performance.now() + 400;
  state = { ...state, revision: state.revision + 1 };
  await evaluate(`window.dispatchEvent(new CustomEvent('jx-membership-status',{detail:${JSON.stringify({ ...state, lease_remaining_ms: 400 })}}))`);
  await pause(650);
  assert.equal(await evaluate(`!!document.querySelector('.authenticated-app')`), false);
  console.log('native gate-before-resize ack, expiry timer, recharge from expired gate: PASS');

  // Restore verified state, then exercise slow logout with stale status polling.
  deadline = performance.now() + 60_000;
  state = { ...state, member: true, revision: state.revision + 1 };
  await evaluate(`window.dispatchEvent(new CustomEvent('jx-membership-status',{detail:${JSON.stringify({ ...state, lease_remaining_ms: 60_000 })}}))`);
  await pause(100);
  await evaluate(`window.__flash=false;window.__watch=setInterval(()=>{if(document.querySelector('.authenticated-app'))window.__flash=true},10);Array.from(document.querySelectorAll('button')).find(b=>b.textContent.trim()==='退出登录').click();window.__immediateHidden=!document.querySelector('.authenticated-app')`);
  assert.equal(await evaluate(`window.__immediateHidden`), true);
  assert.equal(logoutFinished, false, 'gate appeared while cloud logout was still pending');
  await pause(2300);
  assert.equal(await evaluate(`window.__flash`), false, 'stale polls must not restore protected page');
  assert.equal(await evaluate(`document.querySelector('[role=tab][aria-selected=true]').innerText`), '登录');
  await evaluate(`clearInterval(window.__watch)`);
  await capture('logout.png');
  console.log('immediate logout, slow-network logout, stale-status race: PASS');
  console.log(`Screenshots: ${output}`);
} finally {
  if (socket?.readyState === WebSocket.OPEN) {
    try { await Promise.race([call('Browser.close'), pause(500)]); } catch {}
    socket.close();
  }
  browser.kill();
  server.closeAllConnections();
  await new Promise((resolve) => server.close(resolve));
}
