// Real production device UI with delayed responses and USB transitions.
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { readFile, mkdtemp } from 'node:fs/promises';
import { resolve, extname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { tmpdir } from 'node:os';
import { spawn } from 'node:child_process';
const web = fileURLToPath(new URL('../../assets/web/', import.meta.url));
const profile = await mkdtemp(resolve(tmpdir(), 'jx-device-ui-'));
const port = 4177, debugPort = 9237;
const pause = ms => new Promise(resolve => setTimeout(resolve, ms));
let device = { id: 'TEST-USB', status: 'offline' };
let controlled = [];
let projection = { phase: 'idle', scid: null, message: '' };
let starts = 0, stops = 0, restarts = 0;
let delayNext = false, releaseOld;
const snapshot = () => ({ adb_devices: [{ ...device }], controlled_devices: [...controlled], projection: { ...projection } });
const server = createServer(async (req, res) => {
  const path = new URL(req.url, `http://127.0.0.1:${port}`).pathname;
  const reply = data => { res.writeHead(200, { 'Content-Type': 'application/json' }); res.end(JSON.stringify({ code: 200, data, message: '成功' })); };
  if (path.startsWith('/api/')) {
    for await (const chunk of req) { /* consume body */ }
    if (path === '/api/member/status') return reply({ logged_in: true, member: true, account: 'uitest', revision: 1, lease_remaining_ms: 120000, membership_expires_at: 2000000000 });
    if (path === '/api/config/get_config') return reply({ language: 'zh-CN', controller_port: 28012, web_port: 28013 });
    if (path === '/api/device/device_list') {
      const data = snapshot();
      if (delayNext) { delayNext = false; await new Promise(resolve => releaseOld = resolve); }
      return reply(data);
    }
    if (path === '/api/device/control_device') {
      starts++;
      await pause(200);
      projection = { phase: 'streaming', scid: `session-${starts}`, message: '' };
      controlled = [{ device_id: device.id, name: 'Test tablet', main: true, device_size: [1920, 1080], scid: projection.scid, socket_ids: [] }];
      return reply({});
    }
    if (path === '/api/device/decontrol_device') {
      stops++;
      await pause(100);
      controlled = [];
      projection = { phase: 'idle', scid: null, message: '' };
      return reply({});
    }
    if (path === '/api/device/adb_restart') {
      restarts++;
      await pause(150);
      controlled = [];
      device.status = 'device';
      projection = { phase: 'idle', scid: null, message: '' };
      return reply(snapshot());
    }
    return reply([]);
  }
  const file = resolve(web, `.${path === '/' ? '/index.html' : path}`);
  if (!file.startsWith(resolve(web) + '\\') && !file.startsWith(resolve(web) + '/')) { res.writeHead(403); return res.end(); }
  try {
    res.writeHead(200, { 'Content-Type': ({ '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css' })[extname(file)] ?? 'application/octet-stream' });
    res.end(await readFile(file));
  } catch { res.end(); }
});
await new Promise(resolve => server.listen(port, '127.0.0.1', resolve));
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
  await call('Page.enable');
  await call('Runtime.enable');
  await waitFor(`!!document.querySelector('.usb-device-card')`);
  const disabled = `Array.from(document.querySelectorAll('.usb-device-card button')).find(b=>b.textContent.trim()==='投屏')?.disabled`;
  assert.equal(await evaluate(disabled), true, 'offline must disable projection');
  device.status = 'device';
  await waitFor(`Array.from(document.querySelectorAll('.usb-device-card button')).find(b=>b.textContent.trim()==='投屏')?.disabled === false`);
  console.log('USB offline -> online poll restores projection button: PASS');

  // Hold an obsolete offline refresh while restart returns the actual online state.
  device.status = 'offline';
  delayNext = true;
  for (let i = 0; i < 100 && !releaseOld; i++) await pause(50);
  assert(releaseOld, 'periodic device refresh did not run');
  await click('重启');
  for (let i = 0; i < 100 && restarts === 0; i++) await pause(20);
  assert.equal(restarts, 1);
  await pause(300);
  releaseOld();
  await pause(150);
  assert.equal(await evaluate(disabled), false, 'late offline response must not overwrite successful restart');
  console.log('restart and stale device-response race: PASS');

  await evaluate(`(()=>{const b=Array.from(document.querySelectorAll('.usb-device-card button')).find(b=>b.textContent.trim()==='投屏');b.click();b.click();b.click()})()`);
  await waitFor(`!!document.querySelector('.anticon-disconnect')`);
  assert.equal(starts, 1, 'duplicate projection click started multiple sessions');
  await evaluate(`document.querySelector('.anticon-disconnect').closest('a').click()`);
  await waitFor(`Array.from(document.querySelectorAll('.usb-device-card button')).find(b=>b.textContent.trim()==='投屏')?.disabled === false`);
  assert.equal(stops, 1);
  await click('投屏');
  await waitFor(`!!document.querySelector('.anticon-disconnect')`);
  assert.equal(starts, 2, 'second projection must start after cleanup');
  console.log('duplicate-click guard, stop, second projection: PASS');

  controlled = [];
  device.status = 'offline';
  projection = { phase: 'failed', scid: null, message: 'USB连接已断开，请检查调试授权' };
  await waitFor(`document.body.innerText.includes('USB连接已断开，请检查调试授权')`);
  assert.equal(await evaluate(disabled), true);
  device.status = 'unauthorized';
  await waitFor(`document.body.innerText.includes('等待 USB 调试授权')`);
  assert.equal(await evaluate(disabled), true);
  device.status = 'device';
  projection = { phase: 'idle', scid: null, message: '' };
  await waitFor(`Array.from(document.querySelectorAll('.usb-device-card button')).find(b=>b.textContent.trim()==='投屏')?.disabled === false`);
  console.log('session failure, authorization loss and recovery: PASS');
} finally {
  if (socket?.readyState === WebSocket.OPEN) {
    try { await Promise.race([call('Browser.close'), pause(500)]); } catch {}
    socket.close();
  }
  browser.kill();
  server.closeAllConnections();
  await new Promise((resolve) => server.close(resolve));
}
