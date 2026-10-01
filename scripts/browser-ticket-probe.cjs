// S02 browser access gate. Invoked only by an owned Rust host fixture over stdin.
// No Apple endpoint, custom WebSocket headers, HAR, video or raw error logging.
delete process.env.DEBUG;
delete process.env.PWDEBUG;
delete process.env.NODE_DEBUG;
const fs = require('node:fs');
let browser;
let stage = 'prerequisite';
const deadline = setTimeout(() => {
  process.stderr.write(`browser access probe failed: ${stage}\n`);
  process.exit(1);
}, 30000);

async function main() {
  let input = '';
  for await (const chunk of process.stdin) {
    input += chunk;
    if (input.length > 16384) throw new Error('input bound');
  }
  const config = JSON.parse(input);
  if (config.fixture !== 's02-owned-host-fixture-v1') throw new Error('fixture contract');
  for (const value of [config.base, config.origin, config.unapproved_origin]) {
    const url = new URL(value);
    if (url.protocol !== 'http:' || url.hostname !== '127.0.0.1' || !url.port ||
        url.username || url.password || url.pathname !== '/' || url.search || url.hash) {
      throw new Error('owned loopback contract');
    }
  }
  const token = fs.readFileSync(config.token_file, 'utf8');
  if (!/^[0-9a-f]{64}$/.test(token)) throw new Error('private fixture token');
  const { chromium } = require('playwright-core');
  browser = await chromium.launch({ headless: true, executablePath: config.executable });
  const context = await browser.newContext();
  const page = await context.newPage();
  const other = await context.newPage();
  await page.goto(config.origin);
  await other.goto(config.unapproved_origin);
  stage = 'bootstrap';
  const setup = await page.evaluate(async ({ base, token }) => {
    const target = { kind: 'tunnel', udid: 'synthetic-manager-device', port: 12345 };
    const issue = async (selected = target, origin = location.origin, authorized = true) => {
      const response = await fetch(`${base}/auth/ticket`, {
        method: 'POST', credentials: 'omit', cache: 'no-store',
        headers: { 'Content-Type': 'application/json', ...(authorized ? { Authorization: `Bearer ${token}` } : {}) },
        body: JSON.stringify({ origin, target: selected }),
      });
      return { status: response.status, body: await response.json() };
    };
    let checks = 0;
    const check = (value) => { if (!value) throw new Error('contract check'); checks += 1; };
    check((await issue(target, location.origin, false)).status === 401);
    check((await issue({ ...target, udid: 'other-device' })).status === 403);
    check((await issue({ ...target, port: 22 })).status === 403);
    check((await issue(target, 'http://unapproved.invalid')).status === 403);
    const connection = await issue();
    const pending = await issue();
    const tampered = await issue();
    check(connection.status === 200 && pending.status === 200 && tampered.status === 200);
    check(connection.body.expires_in === 30);
    const wsUrl = path => `${base.replace('http:', 'ws:')}${path}`;
    const denied = async path => {
      await new Promise((resolve, reject) => {
        const socket = new WebSocket(wsUrl(path));
        const timer = setTimeout(() => { socket.close(); reject(new Error('denial deadline')); }, 3000);
        socket.onopen = () => { clearTimeout(timer); socket.close(); reject(new Error('unexpected admission')); };
        socket.onerror = () => { clearTimeout(timer); resolve(); };
      });
      checks += 1;
    };
    await denied(`/connect?udid=${target.udid}&port=12345&ticket=${token}`);
    await denied('/connect?udid=synthetic-manager-device&port=12345');
    await denied(tampered.body.websocket_path.replace('port=12345', 'port=22'));
    await denied(tampered.body.websocket_path);
    const socket = new WebSocket(wsUrl(connection.body.websocket_path));
    socket.binaryType = 'arraybuffer';
    const closed = new Promise(resolve => { socket.onclose = () => resolve(true); });
    await new Promise((resolve, reject) => {
      const timer = setTimeout(() => reject(new Error('echo deadline')), 5000);
      socket.onerror = () => { clearTimeout(timer); reject(new Error('connection failed')); };
      socket.onopen = () => socket.send(new TextEncoder().encode('owned-browser-bytes'));
      socket.onmessage = event => {
        clearTimeout(timer);
        if (new TextDecoder().decode(event.data) === 'owned-browser-bytes') resolve();
        else reject(new Error('echo mismatch'));
      };
    });
    checks += 1; // The open connection returned the exact binary payload.
    await denied(connection.body.websocket_path);
    // Keep only short-lived tickets/socket in the page fixture after bootstrap.
    globalThis.ownedAccessFixture = { closed, denied, pending: pending.body.websocket_path };
    return { checks, pending: pending.body.websocket_path };
  }, { base: config.base, token });
  stage = 'unapproved-origin';
  const blocked = await other.evaluate(async ({ base, token, path }) => {
    let bootstrapDenied = false;
    try {
      await fetch(`${base}/auth/ticket`, { method: 'POST', credentials: 'omit',
        headers: { 'Content-Type': 'application/json', Authorization: `Bearer ${token}` },
        body: JSON.stringify({ origin: location.origin, target: { kind: 'tunnel', udid: 'synthetic-manager-device', port: 12345 } }) });
    } catch { bootstrapDenied = true; }
    const handshakeDenied = await new Promise((resolve, reject) => {
      const socket = new WebSocket(`${base.replace('http:', 'ws:')}${path}`);
      const timer = setTimeout(() => { socket.close(); reject(new Error('origin deadline')); }, 3000);
      socket.onopen = () => { clearTimeout(timer); socket.close(); resolve(false); };
      socket.onerror = () => { clearTimeout(timer); resolve(true); };
    });
    return { bootstrapDenied, handshakeDenied };
  }, { base: config.base, token, path: setup.pending });
  if (!blocked.bootstrapDenied || !blocked.handshakeDenied) throw new Error('origin contract');
  stage = config.expiry ? 'expiry' : 'reload';
  if (!config.expiry) {
    fs.writeFileSync(config.policy_file, JSON.stringify({ version: 1, grants: [] }));
    const reloaded = await page.evaluate(async ({ base, token }) => {
      const response = await fetch(`${base}/auth/reload`, { method: 'POST', credentials: 'omit',
        headers: { Authorization: `Bearer ${token}` } });
      return response.status === 200;
    }, { base: config.base, token });
    if (!reloaded) throw new Error('reload contract');
  }
  await page.evaluate(async () => {
    const { closed, denied, pending } = globalThis.ownedAccessFixture;
    await Promise.race([closed, new Promise((_, reject) => setTimeout(() => reject(new Error('revocation deadline')), 20000))]);
    await denied(pending);
    delete globalThis.ownedAccessFixture;
  });
  const report = { status: 'passed', checks: setup.checks + 4,
    playwright: require('playwright-core/package.json').version,
    chromium: browser.version(), revocation: config.expiry ? 'expiry' : 'reload' };
  await context.close(); await browser.close(); browser = undefined;
  clearTimeout(deadline);
  process.stdout.write(`${JSON.stringify(report)}\n`);
}

main().catch(async () => {
  process.stderr.write(`browser access probe failed: ${stage}\n`);
  if (browser) await browser.close().catch(() => {});
  clearTimeout(deadline); process.exitCode = 1;
});
