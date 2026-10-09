// Actual pinned clients talk to the supplied Rust bridge. No synthetic CDP
// server, method overrides, or manufactured success responses live here.
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import puppeteer from 'puppeteer-core';

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
  assert(['initialization-rejection', 'runtime-workflow'].includes(expectation));
  return { client, endpoint: endpoint.origin, targetUrl, expectation };
}

async function puppeteerWorkflow(endpoint, targetUrl) {
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
    const target = await browser.waitForTarget(target => target.url() === targetUrl, { timeout: TIMEOUT });
    steps.push('discovered');
    session = await target.createCDPSession();
    steps.push('attached');
    const contexts = [];
    let resolveContext;
    const announced = new Promise(resolve => { resolveContext = resolve; });
    session.on('Runtime.executionContextCreated', event => {
      contexts.push(event.context.id);
      resolveContext(event.context.id);
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
    try {
      await session.send('Runtime.evaluate', { expression: '6*7', contextId: contextId + 1 });
    } catch (error) {
      unknown = failure(error, 'unknown-context');
    }
    assert(unknown, 'unknown context unexpectedly succeeded');
    assert.equal(unknown.method, 'Runtime.evaluate');
    await session.detach();
    session = undefined;
    steps.push('detached');
    result = { status: 'runtime-workflow', steps, contexts, evaluated: 42, unknown };
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

async function playwrightInitialization(endpoint) {
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
    browser = await chromium.connectOverCDP(endpoint, { headers, timeout: TIMEOUT });
    result = { status: 'connected', contexts: browser.contexts().length };
  } catch (error) {
    result = { status: 'initialization-rejection', failure: failure(error, 'initialization') };
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
    ? await puppeteerWorkflow(endpoint, targetUrl)
    : await playwrightInitialization(endpoint);
  const download = trace.find(item => item.direction === 'send' && item.message.method === 'Browser.setDownloadBehavior');
  const rejected = download && trace.find(item => item.direction === 'receive' &&
    item.message.id === download.message.id && item.message.sessionId === download.message.sessionId &&
    item.message.error?.code === -32601);
  const fulfilled = !result.cleanup_failures && result.status === expectation &&
    (expectation !== 'initialization-rejection' || (result.failure.method === 'Browser.setDownloadBehavior' && !!rejected));
  // A reproducible explicit initialization rejection is evidence of a missing
  // contract. It is never labelled as a complete client workflow.
  console.log(JSON.stringify({ schema_version: 1, client, version: versions[`${client}-core`], expectation, fulfilled, ...result, trace, trace_bytes: traceBytes }));
  if (!fulfilled) process.exitCode = 1;
}

main().catch(error => {
  console.log(JSON.stringify({ schema_version: 1, status: 'fixture-error', failure: failure(error, 'harness') }));
  process.exitCode = 1;
});
