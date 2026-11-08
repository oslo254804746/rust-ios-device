// Record the first CDP commands sent by installed Playwright and Puppeteer clients.
// This is a protocol probe against a fake CDP endpoint, not a device or bridge test.
// Install dependencies outside the checkout and set NODE_PATH to that node_modules.
const http = require('node:http');
const { WebSocketServer } = require('ws');
const { chromium } = require('playwright-core');
const puppeteer = require('puppeteer-core');

async function main() {
  const target = { targetId: 'PID:42:page-1', type: 'page', title: 'Fixture', url: 'https://example.test/', attached: false, canAccessOpener: false };
  const log = [];
  const server = http.createServer((req, res) => {
    res.setHeader('Content-Type', 'application/json');
    if (req.url === '/json/version' || req.url === '/json/version/') {
      res.end(JSON.stringify({ Browser: 'Safari', 'Protocol-Version': '1.3', webSocketDebuggerUrl: `ws://127.0.0.1:${server.address().port}/devtools/browser/ios-cli` }));
    } else if (req.url === '/json/list') {
      res.end(JSON.stringify([]));
    } else {
      res.statusCode = 404;
      res.end('{}');
    }
  });
  const wss = new WebSocketServer({ server, path: '/devtools/browser/ios-cli' });
  wss.on('connection', ws => {
    ws.on('message', data => {
      const request = JSON.parse(data.toString());
      log.push({ method: request.method, sessionId: request.sessionId ?? null, params: request.params ?? {} });
      const response = { id: request.id };
      if (request.sessionId) response.sessionId = request.sessionId;
      switch (request.method) {
        case 'Browser.getVersion': response.result = { protocolVersion: '1.3', product: 'Chrome/125.0.0.0', revision: '', userAgent: 'Chrome/125.0.0.0', jsVersion: '' }; break;
        case 'Target.attachToBrowserTarget': response.result = { sessionId: 'browser-session' }; break;
        case 'Target.getTargets': response.result = { targetInfos: [target] }; break;
        case 'Target.getBrowserContexts': response.result = { browserContextIds: [] }; break;
        case 'Target.getTargetInfo': response.result = { targetInfo: target }; break;
        case 'Target.attachToTarget': response.result = { sessionId: 'page-session' }; break;
        case 'Target.setDiscoverTargets':
        case 'Target.setAutoAttach': response.result = {}; break;
        default: response.error = { code: -32601, message: 'Method not implemented' };
      }
      ws.send(JSON.stringify(response));
      if (request.method === 'Target.setDiscoverTargets' && request.params?.discover) {
        ws.send(JSON.stringify({ method: 'Target.targetCreated', params: { targetInfo: target }, ...(request.sessionId ? { sessionId: request.sessionId } : {}) }));
      }
    });
  });
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  const endpoint = `http://127.0.0.1:${server.address().port}`;
  try {
    for (const client of ['playwright', 'puppeteer']) {
      log.length = 0;
      let browser;
      try {
        browser = client === 'playwright'
          ? await chromium.connectOverCDP(endpoint, { timeout: 5000 })
          : await puppeteer.connect({ browserWSEndpoint: `${endpoint.replace('http', 'ws')}/devtools/browser/ios-cli`, protocolTimeout: 5000 });
        const pages = await browser.pages();
        console.log(JSON.stringify({ client, status: 'connected', pages: pages.length, methods: log }));
      } catch (error) {
        console.log(JSON.stringify({ client, status: 'failed', error: String(error.message).slice(0, 200), methods: log }));
      } finally {
        if (browser) await browser.disconnect();
      }
    }
  } finally {
    wss.close();
    server.close();
  }
}

main().catch(error => { console.error(error); process.exitCode = 1; });
