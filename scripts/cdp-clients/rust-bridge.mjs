// Actual pinned clients talk to the supplied Rust bridge. No synthetic CDP
// server, method overrides, or manufactured success responses live here.
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import puppeteer, { CDPSessionEvent } from 'puppeteer-core';

const versions = JSON.parse(readFileSync(new URL('./package.json', import.meta.url), 'utf8')).dependencies;
const MAX_RECORDS = 512;
const MAX_BYTES = 256 * 1024;
const TIMEOUT = 5000;
const trace = [];
let traceBytes = 0;

function record(direction, message) {
  const value = typeof message === 'string' ? JSON.parse(message) : message;
  const bytes = Buffer.byteLength(JSON.stringify(value));
  assert(trace.length < MAX_RECORDS && bytes <= MAX_BYTES - traceBytes, 'client trace limit');
  traceBytes += bytes;
  trace.push({ direction, message: value });
}

function failure(error, stage) {
  // Retain a method/category, without copying arbitrary remote error strings.
  const match = String(error?.message ?? '').match(/Protocol error \(([A-Za-z0-9_.]+)\)/);
  return {
    stage,
    method: match?.[1] ?? null,
    category: error?.name === 'TimeoutError' ? 'timeout' : 'client_error',
    code: Number.isInteger(error?.code) ? error.code : null,
  };
}

function arguments_() {
  const args = process.argv.slice(2);
  assert.equal(args.length, 4, 'usage: rust-bridge.mjs playwright|puppeteer http://127.0.0.1:PORT TARGET_URL EXPECTATION');
  const [client, address, targetUrl, expectation] = args;
  assert(['playwright', 'puppeteer'].includes(client), 'unknown client');
  const endpoint = new URL(address);
  assert.equal(endpoint.protocol, 'http:');
  assert.equal(endpoint.hostname, '127.0.0.1');
  assert(endpoint.port && endpoint.pathname === '/' && !endpoint.search && !endpoint.hash);
  assert(!endpoint.username && !endpoint.password);
  assert(['connected', 'connect-over-cdp', 'initialization-rejection', 'runtime-workflow', 'console-workflow', 'fetch-workflow', 'metrics-workflow', 'iframe-workflow', 'world-workflow', 'five-domain-workflow', 'native-page-closure', 'native-application-closure', 'native-discovery-page-closure', 'native-discovery-application-closure', 'native-discovery-race-closure'].includes(expectation));
  return { client, endpoint: endpoint.origin, targetUrl, expectation };
}

async function bounded(promise) {
  let timer;
  return Promise.race([promise, new Promise((_, reject) => {
    timer = setTimeout(() => reject(new Error('fixture event absent')), TIMEOUT);
  })]).finally(() => clearTimeout(timer));
}

async function puppeteerWorkflow(endpoint, targetUrl, expectation) {
  const token = process.env.IOS_CDP_FIXTURE_TOKEN;
  const headers = token ? { Authorization: `Bearer ${token}` } : {};
  let browser;
  let session;
  let result;
  const steps = [];
  try {
    browser = await puppeteer.connect({
      browserWSEndpoint: `${endpoint.replace('http:', 'ws:')}/devtools/browser/ios-cli`,
      headers,
      protocolTimeout: TIMEOUT,
      defaultViewport: null,
      logger: prefix => {
        if (prefix === 'puppeteer:protocol:SEND ►') return message => record('send', message);
        if (prefix === 'puppeteer:protocol:RECV ◀') return message => record('receive', message);
        return undefined;
      },
    });
    steps.push('initialized');
    let target;
    try {
      target = await browser.waitForTarget(target => target.url() === targetUrl, { timeout: 1000 });
    } catch {
      const { targetInfos } = await browser._connection.send('Target.getTargets');
      const info = targetInfos.find(t => t.url === targetUrl);
      assert(info, 'discovered target absent');
      await browser._connection.send('Target.attachToTarget', { targetId: info.targetId, flatten: true });
      target = await browser.waitForTarget(target => target.url() === targetUrl, { timeout: TIMEOUT });
    }
    steps.push('discovered');
    session = await target.createCDPSession();
    steps.push('attached');
    const contexts = [];
    const frameContexts = new Map();
    const worlds = new Map();
    const destroyedContexts = [];
    let resolveContext;
    const announced = new Promise(resolve => { resolveContext = resolve; });
    session.on('Runtime.executionContextCreated', event => {
      contexts.push(event.context.id);
      worlds.set(event.context.id, event.context);
      if (event.context.auxData?.frameId && event.context.auxData.isDefault) frameContexts.set(event.context.auxData.frameId, event.context);
      resolveContext(event.context.id);
    });
    session.on('Runtime.executionContextDestroyed', event => {
      destroyedContexts.push(event.executionContextId);
      for (const [frame,context] of frameContexts) if (context.id === event.executionContextId) frameContexts.delete(frame);
    });
    await session.send('Runtime.enable');
    let timer;
    const contextId = await Promise.race([
      announced,
      new Promise((_, reject) => { timer = setTimeout(() => reject(new Error('fixture context absent')), TIMEOUT); }),
    ]).finally(() => clearTimeout(timer));
    assert(Number.isSafeInteger(contextId));
    steps.push('context-selected');
    const evaluated = await session.send('Runtime.evaluate', { expression: '6*7', contextId, returnByValue: true });
    assert.equal(evaluated.result.type, 'number');
    assert.equal(evaluated.result.value, 42);
    steps.push('evaluated');
    let unknown;
    const unknownContextId = contexts.includes(contextId + 1) ? Math.max(...contexts) + 1000 : contextId + 1;
    try {
      await session.send('Runtime.evaluate', { expression: '6*7', contextId: unknownContextId });
    } catch (error) {
      unknown = failure(error, 'unknown-context');
    }
    assert(unknown, 'unknown context unexpectedly succeeded');
    assert.equal(unknown.method, 'Runtime.evaluate');
    if (expectation === 'console-workflow') {
      const consolePromise = new Promise(resolve => session.once('Runtime.consoleAPICalled', resolve));
      await session.send('Runtime.evaluate', { expression: 'fixture:console-log', contextId });
      const event = await bounded(consolePromise);
      assert.equal(event.type, 'log');
      assert.equal(event.args[0].value, 'test console log from native');
      assert.equal(event.executionContextId, contextId);
      assert.equal(event.timestamp, 12345);
      steps.push('console-verified');
      await session.detach();
      session = undefined;
      steps.push('detached');
      result = { status: 'console-workflow', steps, contexts, evaluated: 42, console_event: event };
    } else if (expectation === 'fetch-workflow') {
      async function pause(kind) {
        const paused = new Promise(resolve => session.once('Fetch.requestPaused', resolve));
        await session.send('Runtime.evaluate', {expression:`fixture:fetch:${kind}`});
        const event = await bounded(paused);
        assert.equal(event.requestId, `native-${kind}`);
        return event;
      }
      await session.send('Fetch.enable');
      const continued = await pause('continue');
      await session.send('Fetch.continueRequest', {requestId:continued.requestId, method:'POST', postData:'b2s=', headers:[{name:'X-Fixture',value:'request'}]});
      await assert.rejects(session.send('Fetch.continueRequest', {requestId:continued.requestId}));
      const fulfilled = await pause('fulfill');
      await session.send('Fetch.fulfillRequest', {requestId:fulfilled.requestId,responseCode:201,body:'b2s='});
      const failed = await pause('failed');
      await session.send('Fetch.failRequest', {requestId:failed.requestId,errorReason:'Aborted'});
      await session.send('Fetch.disable');
      await session.send('Fetch.enable',{patterns:[{urlPattern:'*',requestStage:'Response'}]});
      const response = await pause('response');
      assert.equal(response.responseStatusCode, 200);
      await session.send('Fetch.fulfillRequest',{requestId:response.requestId,responseCode:201,body:'b2s='});
      const cancelled = await pause('cancelled');
      await session.send('Fetch.disable');
      await assert.rejects(session.send('Fetch.continueRequest',{requestId:cancelled.requestId}));
      steps.push('continue-verified','fulfill-verified','fail-verified','response-verified','pending-disabled');
      await session.detach(); session = undefined; steps.push('detached');
      result = {status:'fetch-workflow',steps,contexts};
    } else if (expectation === 'metrics-workflow') {
      const sample = new Promise(resolve => session.once('Performance.metrics', resolve));
      await session.send('Performance.enable');
      const event = await bounded(sample);
      const resultMetrics = await session.send('Performance.getMetrics');
      assert.deepEqual(resultMetrics.metrics, event.metrics);
      assert.equal(event.metrics.find(m => m.name === 'Timestamp').value, 12.5);
      assert.equal(event.metrics.find(m => m.name === 'WebKitJavaScriptMemory').value, 4096);
      assert(!event.metrics.some(m => m.name === 'JSHeapUsedSize'));
      await session.send('HeapProfiler.collectGarbage');
      await session.send('Performance.disable');
      await assert.rejects(session.send('Performance.getMetrics'));
      steps.push('metrics-verified', 'tracking-stopped');
      await session.detach(); session = undefined; steps.push('detached');
      result = {status:'metrics-workflow', steps, contexts, metrics:event.metrics};
    } else if (expectation === 'world-workflow') {
      await session.send('Page.getFrameTree');
      const arrived = new Promise(resolve => {
        const listener = event => {
          if (event.context.id === 302) {
            session.off('Runtime.executionContextCreated', listener);
            resolve();
          }
        };
        session.on('Runtime.executionContextCreated', listener);
      });
      await session.send('Runtime.evaluate', {expression:'fixture:worlds', contextId});
      await bounded(arrived);
      for (const id of [301, 302]) {
        const world = worlds.get(id);
        assert.equal(world.origin, 'https://example.invalid');
        assert.equal(world.auxData.frameId, '1.3');
        assert.equal(world.auxData.isDefault, false);
        assert.equal(world.auxData.type, 'isolated');
        assert.equal((await session.send('Runtime.evaluate', {expression:'6*7', contextId:id, returnByValue:true})).result.value, 42);
      }
      const consoleEvent = new Promise(resolve => session.once('Runtime.consoleAPICalled', resolve));
      await session.send('Runtime.evaluate', {expression:'fixture:world-console', contextId:301});
      const logged = await bounded(consoleEvent);
      assert.equal(logged.executionContextId, 301);
      assert.equal(logged.timestamp, 12500);
      assert.equal(JSON.parse(logged.args[0].objectId).injectedScriptId, 301);
      assert.equal(frameContexts.get('1.3').id, 101);
      await session.send('Page.getFrameTree');
      assert(!worlds.has(303), 'retired world was revived by a late frame tree');
      const committed = new Promise(resolve => {
        const listener = event => {
          if (event.frame.id === '1.3') {
            session.off('Page.frameNavigated', listener); resolve();
          }
        };
        session.on('Page.frameNavigated', listener);
      });
      await session.send('Page.navigate', {url:'https://example.invalid/world-next', frameId:'1.3'});
      await bounded(committed);
      for (const id of [101, 301, 302]) {
        assert(destroyedContexts.includes(id));
        await assert.rejects(session.send('Runtime.evaluate', {expression:'6*7', contextId:id}));
      }
      steps.push('worlds-evaluated', 'object-console-associated', 'late-world-retired', 'worlds-destroyed');
      await session.detach(); session = undefined; steps.push('detached');
      result = {status:'world-workflow', steps, contexts, destroyedContexts};
    } else if (expectation === 'iframe-workflow') {
      const { frameTree } = await session.send('Page.getFrameTree');
      assert.equal(frameTree.childFrames[0].frame.securityOrigin, frameTree.frame.securityOrigin);
      assert.notEqual(frameTree.childFrames[1].frame.securityOrigin, frameTree.frame.securityOrigin);
      const commits = [];
      session.on('Page.frameNavigated', event => commits.push(event.frame));
      for (const [frameId, url, origin] of [
        ['1.3', 'https://example.invalid/navigated-child', 'https://example.invalid'],
        ['1.5', 'https://cross.invalid/navigated-child', 'https://cross.invalid'],
      ]) {
        const context = frameContexts.get(frameId);
        assert(context, 'native frame context was not mapped');
        assert.equal(context.origin, origin);
        assert.equal(context.auxData.isDefault, true);
        const evaluated = await session.send('Runtime.evaluate', {expression:'6*7', contextId:context.id, returnByValue:true});
        assert.equal(evaluated.result.value, 42);
        const recreated = new Promise(resolve => {
          const listener = event => {
            if (event.context.auxData?.frameId === frameId && event.context.id !== context.id) {
              session.off('Runtime.executionContextCreated', listener); resolve(event.context);
            }
          };
          session.on('Runtime.executionContextCreated', listener);
        });
        const committed = new Promise(resolve => {
          const listener = event => {
            if (event.frame.id === frameId && event.frame.url === url) {
              session.off('Page.frameNavigated', listener); resolve(event.frame);
            }
          };
          session.on('Page.frameNavigated', listener);
        });
        const navigation = await session.send('Page.navigate', {url, frameId});
        assert.equal(navigation.frameId, frameId);
        await bounded(committed);
        const nextContext = await bounded(recreated);
        assert(destroyedContexts.includes(context.id));
        assert.equal(nextContext.origin, origin);
        const nextEvaluated = await session.send('Runtime.evaluate', {expression:'6*7',contextId:nextContext.id,returnByValue:true});
        assert.equal(nextEvaluated.result.value,42);
      }
      assert.deepEqual(commits.map(frame => frame.id), ['1.3', '1.5']);
      assert.equal(commits[1].url, 'https://cross.invalid/navigated-child');
      const refreshed = (await session.send('Page.getFrameTree')).frameTree;
      assert.deepEqual(refreshed.childFrames.map(child => child.frame.url), commits.map(frame => frame.url));
      steps.push('same-origin-navigated', 'cross-origin-navigated');
      await session.detach(); session = undefined; steps.push('detached');
      result = {status: 'iframe-workflow', steps, contexts, commits, destroyedContexts};
    } else if (expectation === 'five-domain-workflow') {
      // Domain 1: Console / Runtime is already active and verified
      // Domain 2: Page / iframe
      const { frameTree } = await session.send('Page.getFrameTree');
      assert(frameTree && frameTree.frame && frameTree.frame.id);
      assert(Array.isArray(frameTree.childFrames));
      steps.push('frame-tree-verified');

      // Domain 3: Input & Screenshot
      const screenshot = await session.send('Page.captureScreenshot', { format: 'png' });
      assert(typeof screenshot.data === 'string' && screenshot.data.length > 0);
      const pngHeader = Buffer.from(screenshot.data, 'base64').subarray(0, 8);
      assert.deepEqual([...pngHeader], [137, 80, 78, 71, 13, 10, 26, 10]);
      steps.push('screenshot-verified');

      let inputRejected = false;
      try {
        await session.send('Input.dispatchMouseEvent', { type: 'mousePressed', x: 10, y: 10, button: 'left', clickCount: 1 });
        await session.send('Input.dispatchKeyEvent', { type: 'keyDown', key: 'a', text: 'a' });
        await session.send('Input.insertText', { text: 'hello' });
        await session.send('Input.dispatchTouchEvent', { type: 'touchStart', touchPoints: [{ x: 10, y: 10 }] });
        steps.push('input-verified');
        await session.send('Input.synthesizePinchGesture', { x: 10, y: 10, scaleFactor: 1.5 });
      } catch (err) {
        inputRejected = true;
      }
      assert(inputRejected, 'native input injection must be rejected over WebInspector');
      steps.push('input-rejected');

      // Domain 4: Fetch & Download
      await session.send('Fetch.enable', { patterns: [{ urlPattern: '*' }] });
      steps.push('fetch-enabled');
      await session.send('Fetch.disable');
      steps.push('fetch-disabled');

      let downloadRejected = false;
      try {
        await session.send('Page.setDownloadBehavior', { behavior: 'allow', downloadPath: '/tmp' });
        steps.push('download-verified');
        await session.send('Page.setDownloadBehavior', { behavior: 'invalid-mode' });
      } catch (err) {
        downloadRejected = true;
      }
      assert(downloadRejected, 'download behavior must be rejected');
      steps.push('download-rejected');

      // Domain 5: Memory & Performance
      let perfRejected = false;
      try {
        await session.send('Performance.getMetrics');
      } catch (err) {
        perfRejected = true;
      }
      assert(perfRejected, 'performance metrics must be rejected');
      steps.push('perf-rejected');

      let memRejected = false;
      try {
        await session.send('Memory.getDOMCounters');
      } catch (err) {
        memRejected = true;
      }
      assert(memRejected, 'memory DOM counters must be rejected');
      steps.push('mem-rejected');

      await session.detach();
      session = undefined;
      steps.push('detached');
      result = { status: 'five-domain-workflow', steps, contexts, evaluated: 42 };
    } else if (expectation.startsWith('native-')) {
      const preserved = await session.send('Runtime.evaluate', { expression: 'fixture:unrelated', contextId });
      assert.equal(preserved.result.value, 42);
      steps.push('unrelated-preserved');
      const destroyed = new Promise(resolve => session.once('Runtime.executionContextDestroyed', resolve));
      const detached = new Promise(resolve => session.once(CDPSessionEvent.Disconnected, resolve));
      await session.send('Runtime.evaluate', { expression: 'fixture:native-close', contextId });
      // Polling uses another native discovery socket; the selected socket has
      // no closure message in the two independent-discovery fixtures.
      const event = await bounded(destroyed);
      assert.equal(event.executionContextId, contextId);
      assert.equal(event.executionContextUniqueId, 'jscontext.1');
      steps.push('context-destroyed');
      await bounded(detached);
      session = undefined;
      steps.push('native-detached');
      result = { status: expectation, steps, contexts, evaluated: 42, unknown, destroyed: event };
    } else {
      await session.detach();
      session = undefined;
      steps.push('detached');
      result = { status: 'runtime-workflow', steps, contexts, evaluated: 42, unknown };
    }
    return result;
  } catch (error) {
    result = { status: 'failed', steps, failure: failure(error, steps.at(-1) ?? 'initialization') };
    return result;
  } finally {
    const cleanup = [];
    if (session) {
      try { await session.detach(); } catch (error) { cleanup.push(failure(error, 'detach')); }
    }
    // Disconnect closes this client's socket; Browser.close would mutate the
    // remote browser and is not part of this read-only fixture workflow.
    if (browser) {
      try { await browser.disconnect(); steps.push('disconnected'); }
      catch (error) { cleanup.push(failure(error, 'disconnect')); }
    }
    if (cleanup.length) result.cleanup_failures = cleanup;
  }
}

async function playwrightClosure(chromium, endpoint, targetUrl, headers, expectation) {
  let browser;
  let session;
  let result;
  const steps = [];
  try {
    // Public option leaves the existing default context's download policy alone.
    // The default connectOverCDP rejection gate below still uses its defaults.
    browser = await chromium.connectOverCDP(endpoint, { headers, timeout: TIMEOUT, noDefaults: true });
    steps.push('initialized-no-defaults');
    session = await browser.newBrowserCDPSession();
    await session.send('Target.setAutoAttach', { autoAttach: false, waitForDebuggerOnStart: false, flatten: true });
    const { targetInfos } = await session.send('Target.getTargets');
    const target = targetInfos.find(target => target.url === targetUrl);
    assert(target, 'fixture target absent');
    steps.push('discovered');
    const pending = new Map();
    const contexts = [];
    let resolveCreated;
    let resolveDestroyed;
    let resolveDetached;
    const created = new Promise(resolve => { resolveCreated = resolve; });
    const destroyed = new Promise(resolve => { resolveDestroyed = resolve; });
    const detached = new Promise(resolve => { resolveDetached = resolve; });
    let childId;
    session.on('Target.receivedMessageFromTarget', event => {
      if (event.sessionId !== childId) return;
      const message = JSON.parse(event.message);
      if (message.method === 'Runtime.executionContextCreated') {
        contexts.push(message.params.context.id);
        resolveCreated(message.params.context.id);
      }
      if (message.method === 'Runtime.executionContextDestroyed') resolveDestroyed(message.params);
      if (pending.has(message.id)) { pending.get(message.id)(message); pending.delete(message.id); }
    });
    session.on('Target.detachedFromTarget', event => { if (event.sessionId === childId) resolveDetached(); });
    ({ sessionId: childId } = await session.send('Target.attachToTarget', { targetId: target.targetId, flatten: false }));
    steps.push('attached-nonflat');
    let unknownSessionRejected = false;
    try { await session.send('Target.sendMessageToTarget', { sessionId: 'absent-fixture-session', message: '{"id":1,"method":"Runtime.enable"}' }); }
    catch { unknownSessionRejected = true; }
    assert(unknownSessionRejected, 'unknown session unexpectedly succeeded');
    let nextId = 1000;
    async function send(method, params = {}) {
      const id = ++nextId;
      const response = new Promise(resolve => pending.set(id, resolve));
      try {
        await session.send('Target.sendMessageToTarget', { sessionId: childId, message: JSON.stringify({ id, method, params }) });
        return await bounded(response);
      } finally { pending.delete(id); }
    }
    assert((await send('Runtime.enable')).result);
    const contextId = await bounded(created);
    steps.push('context-selected');
    const evaluated = await send('Runtime.evaluate', { expression: '6*7', contextId, returnByValue: true });
    assert.equal(evaluated.result.result.value, 42);
    steps.push('evaluated');
    const unknown = await send('Runtime.evaluate', { expression: '6*7', contextId: contextId + 1 });
    assert.equal(unknown.error.code, -32602);
    const preserved = await send('Runtime.evaluate', { expression: 'fixture:unrelated', contextId });
    assert.equal(preserved.result.result.value, 42);
    steps.push('unrelated-preserved');
    await send('Runtime.evaluate', { expression: 'fixture:native-close', contextId });
    if (expectation.includes('discovery')) await session.send('Target.getTargets');
    const event = await bounded(destroyed);
    assert.equal(event.executionContextId, contextId);
    assert.equal(event.executionContextUniqueId, 'jscontext.1');
    steps.push('context-destroyed');
    await bounded(detached);
    steps.push('native-detached');
    result = { status: expectation, steps, contexts, evaluated: 42, unknown: { code: unknown.error.code }, unknown_session_rejected: unknownSessionRejected, destroyed: event };
    return result;
  } catch (error) {
    result = { status: 'failed', steps, failure: failure(error, steps.at(-1) ?? 'initialization') };
    return result;
  } finally {
    const cleanup = [];
    // Browser.close releases this root browser CDPSession and the connection.
    try { if (browser) { await browser.close(); steps.push('disconnected'); } } catch (error) { cleanup.push(failure(error, 'disconnect')); }
    if (cleanup.length) result.cleanup_failures = cleanup;
  }
}

async function playwrightInitialization(endpoint, targetUrl, expectation) {
  // Capture the pinned client's real protocol diagnostic sink. This changes
  // neither its CDP methods nor responses and does not expose HTTP headers.
  process.env.DEBUG = 'pw:protocol';
  process.env.DEBUG_COLORS = '0';
  delete process.env.DEBUG_FILE;
  const originalWrite = process.stderr.write;
  let pending = '';
  let diagnosticFailure;
  process.stderr.write = function (chunk, encoding, callback) {
    if (typeof encoding === 'function') callback = encoding;
    try {
      const text = typeof chunk === 'string' ? chunk : chunk.toString('utf8');
      assert(Buffer.byteLength(pending) + Buffer.byteLength(text) <= MAX_BYTES, 'protocol diagnostic line limit');
      pending += text;
      let end;
      while ((end = pending.indexOf('\n')) !== -1) {
        const line = pending.slice(0, end);
        pending = pending.slice(end + 1);
        const match = line.match(/pw:protocol (SEND ►|◀ RECV) (\{.*\})\s*$/);
        assert(match, 'unexpected protocol diagnostic');
        record(match[1] === 'SEND ►' ? 'send' : 'receive', match[2]);
      }
    } catch (error) {
      diagnosticFailure ??= error;
    }
    if (callback) queueMicrotask(() => callback());
    return true;
  };
  const token = process.env.IOS_CDP_FIXTURE_TOKEN;
  const headers = token ? { Authorization: `Bearer ${token}` } : {};
  let browser;
  let result;
  try {
    // DEBUG is read during module initialization, so import after admission to
    // the diagnostic capture. The public connectOverCDP is used unchanged.
    const { chromium } = await import('playwright-core');
    if (expectation.startsWith('native-')) {
      result = await playwrightClosure(chromium, endpoint, targetUrl, headers, expectation);
    } else {
      browser = await chromium.connectOverCDP(endpoint, { headers, timeout: TIMEOUT });
      const pages = browser.contexts().flatMap(ctx => ctx.pages());
      const targetPage = pages.find(p => p.url() === targetUrl) || pages[0];
      const evaluated = targetPage ? await targetPage.evaluate(() => 6 * 7) : null;
      result = { status: expectation, contexts: browser.contexts().length, pages: pages.length, evaluated };
    }
  } catch (error) {
    result = { status: expectation.startsWith('native-') ? 'failed' : 'initialization-rejection', failure: failure(error, 'initialization') };
  } finally {
    // Closing an attached Playwright Browser releases its CDP connection.
    try {
      if (browser) await browser.close();
    } catch (error) {
      result.cleanup_failures = [failure(error, 'disconnect')];
    } finally {
      process.stderr.write = originalWrite;
    }
  }
  assert.equal(pending, '', 'partial protocol diagnostic');
  if (diagnosticFailure) throw diagnosticFailure;
  return result;
}

async function main() {
  const { client, endpoint, targetUrl, expectation } = arguments_();
  const result = client === 'puppeteer'
    ? await puppeteerWorkflow(endpoint, targetUrl, expectation)
    : await playwrightInitialization(endpoint, targetUrl, expectation);
  const download = trace.find(item => item.direction === 'send' && item.message.method === 'Browser.setDownloadBehavior');
  const accepted = download && trace.find(item => item.direction === 'receive' &&
    item.message.id === download.message.id && item.message.sessionId === download.message.sessionId &&
    item.message.result !== undefined);
  const fulfilled = !result.cleanup_failures && result.status === expectation &&
    (client !== 'playwright' || expectation.startsWith('native-') || !!accepted);
  // A reproducible explicit initialization rejection is evidence of a missing
  // contract. It is never labelled as a complete client workflow.
  console.log(JSON.stringify({ schema_version: 1, client, version: versions[`${client}-core`], expectation, fulfilled, ...result, trace, trace_bytes: traceBytes }));
  if (!fulfilled) process.exitCode = 1;
}

main().catch(error => {
  console.log(JSON.stringify({ schema_version: 1, status: 'fixture-error', failure: failure(error, 'harness') }));
  process.exitCode = 1;
});
