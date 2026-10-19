// Test-only CDP harness. Never imported by the application or included in distribution.
import { readFile } from 'node:fs/promises';
import path from 'node:path';

const mode = process.argv[2] || 'metrics';
const root = path.resolve(import.meta.dirname, '..');
const mediaFile = path.join(root, 'artifacts/fixtures/720p30.webm');
const profile = path.join(process.env.LOCALAPPDATA, 'io.github.moabom/profiles/default/EBWebView');
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

class CDP {
  pending = new Map();
  nextId = 0;
  static async connect(url) {
    const c = new CDP();
    c.ws = new WebSocket(url);
    await new Promise((resolve, reject) => { c.ws.onopen = resolve; c.ws.onerror = reject; });
    c.ws.onmessage = (event) => {
      const message = JSON.parse(event.data);
      const p = c.pending.get(message.id);
      if (!p) return;
      c.pending.delete(message.id);
      if (message.error) p.reject(new Error(JSON.stringify(message.error)));
      else p.resolve(message.result);
    };
    return c;
  }
  call(method, params = {}) {
    return new Promise((resolve, reject) => {
      const id = ++this.nextId;
      this.pending.set(id, { resolve, reject });
      this.ws.send(JSON.stringify({ id, method, params }));
    });
  }
  async evaluate(expression) {
    const r = await this.call('Runtime.evaluate', { expression, returnByValue: true, awaitPromise: true });
    if (r.exceptionDetails) throw new Error(JSON.stringify(r.exceptionDetails));
    return r.result.value;
  }
  close() { this.ws.close(); }
}

async function targets() {
  for (let i = 0; i < 120; i++) {
    try {
      const port = Number((await readFile(path.join(profile, 'DevToolsActivePort'), 'utf8')).split('\n')[0]);
      const response = await fetch(`http://127.0.0.1:${port}/json/list`);
      const list = (await response.json()).filter((p) => p.type === 'page');
      if (list.length === 2) return list.sort((a, b) => a.id.localeCompare(b.id));
    } catch (e) {
      if (i === 119) throw e;
    }
    await sleep(500);
  }
  throw new Error('Expected two stream webviews on the test-only diagnostic endpoint');
}

const pages = await targets();
const clients = await Promise.all(pages.map((p) => CDP.connect(p.webSocketDebuggerUrl)));
try {
  if (mode === 'prepare') {
    const media = await readFile(mediaFile);
    await Promise.all(clients.map(async (c) => {
      await c.call('Page.stopLoading');
      await c.call('Network.enable');
      await c.call('Network.setBlockedURLs', { urls: ['http://*', 'https://*'] });
    }));
    const html = `<!doctype html><html><head><title>Moabom 720p30 fixture</title></head><body style="margin:0;background:black"><div id="movie_player"><video autoplay muted loop controls playsinline style="width:100vw;height:100vh;object-fit:contain" src="data:video/webm;base64,${media.toString('base64')}"></video></div></body></html>`;
    const url = `data:text/html;base64,${Buffer.from(html).toString('base64')}`;
    await Promise.all(clients.map((c) => c.call('Page.navigate', { url })));
    for (let i = 0; i < 100; i++) {
      const ready = await Promise.all(clients.map((c) => c.evaluate(`(() => {
        const v = document.querySelector('video');
        if (v) { v.muted = true; v.loop = true; v.play().catch(() => {}); }
        return document.title === 'Moabom 720p30 fixture' && !!v && v.readyState >= 3 && v.currentTime > 0;
      })()`).catch((e) => {
        if (/context was destroyed|Cannot find context/i.test(e.message)) return false;
        throw e;
      })));
      if (ready.every(Boolean)) break;
      if (i === 99) {
        const diagnostics = await Promise.all(clients.map((c) => c.evaluate(`(() => {
          const v = document.querySelector('video');
          return { title:document.title, url:location.href.slice(0,45), ready:v?.readyState,
            error:v?.error?.message, time:v?.currentTime, duration:v?.duration };
        })()`)));
        throw new Error('Local video fixtures did not start: ' + JSON.stringify(diagnostics));
      }
      await sleep(200);
    }
    await Promise.all(clients.map(async (c) => {
      await c.call('Page.resetNavigationHistory');
      await c.call('HeapProfiler.collectGarbage');
    }));
  }
  const metrics = await Promise.all(clients.map((c) => c.evaluate(`(() => {
    const v = document.querySelector('video');
    if (!v) throw new Error('No fixture video');
    const q = v.getVideoPlaybackQuality();
    return {timestampMs:performance.now(), title:document.title,
      viewportWidth:innerWidth, viewportHeight:innerHeight, devicePixelRatio,
      visibility:document.visibilityState,
      time:v.currentTime, duration:v.duration, width:v.videoWidth, height:v.videoHeight,
      paused:v.paused, totalFrames:q.totalVideoFrames, droppedFrames:q.droppedVideoFrames,
      videoOnly:!!window.__mv?.videoOnly, overlay:!!window.__mvOverlay,
      qualityHook:typeof window.__mvSetQuality, watchdog:!!window.__mvWatchdog};
  })()`)));
  console.log(JSON.stringify(metrics));
} finally {
  clients.forEach((c) => c.close());
}
