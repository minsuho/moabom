import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { JSDOM } from 'jsdom';
import { TextDecoder, TextEncoder } from 'node:util';
import { protect } from './build-assets.mjs';
import { parse } from 'acorn';

const root = new URL('../', import.meta.url);
const source = async (file) => readFile(new URL(file, root), 'utf8');
const flush = () => new Promise((r) => setImmediate(r));

// An independent parser verifies that protection preserves operations and branches.
function executionShape(node) {
  if (Array.isArray(node)) return node.map(executionShape);
  if (!node || typeof node !== 'object') return node;
  if (node.type === 'Identifier') return { type: 'Identifier' };
  const result = {};
  for (const [key, value] of Object.entries(node)) {
    if (['start', 'end', 'raw', 'shorthand'].includes(key)) continue;
    const staticKey = !node.computed && ((node.type === 'Property' && key === 'key') ||
      (node.type === 'MemberExpression' && key === 'property'));
    result[key] = staticKey ? { staticKey: value.name ?? value.value } : executionShape(value);
  }
  return result;
}

test('protection preserves JavaScript operations, control flow, literals and public properties', async () => {
  const html = await source('ui/index.html');
  const scripts = [/<script>([\s\S]*?)<\/script>/.exec(html)[1]];
  for (const name of ['video_only', 'chzzk_hq', 'chzzk_sd', 'soop_chat', 'overlay', 'assist', 'quality', 'watchdog']) {
    scripts.push(await source(`src-tauri/src/inject/${name}.js`));
  }
  for (const raw of scripts) {
    const protectedCode = await protect(raw);
    assert.deepEqual(executionShape(parse(protectedCode, { ecmaVersion: 'latest' })),
      executionShape(parse(raw, { ecmaVersion: 'latest' })));
  }
});

function browser(html = '', url = 'https://www.youtube.com/watch?v=fixture') {
  const dom = new JSDOM(html, { url, runScripts: 'outside-only', pretendToBeVisual: true });
  const timers = [];
  dom.window.setInterval = (fn, ms) => { timers.push({ fn, ms }); return timers.length; };
  dom.window.setTimeout = (fn, ms) => { timers.push({ fn, ms }); return timers.length; };
  dom.window.requestAnimationFrame = (fn) => fn();
  dom.window.TextDecoder = TextDecoder;
  dom.window.TextEncoder = TextEncoder;
  dom.window.PointerEvent = dom.window.MouseEvent;
  dom.window.chrome = { webview: { postMessage() {} } };
  return { dom, w: dom.window, timers };
}

test('UI keeps IPC, settings, group actions, keyboard commands and snapshots', async () => {
  const html = await source('ui/index.html');
  const js = /<script>([\s\S]*?)<\/script>/.exec(html)[1];
  async function exercise(code) {
    const { dom, w } = browser(html);
    const calls = [];
    const events = {};
    const snapshot = { tiles: [], groups: [], layout: 'grid', quality: 'auto', restore: true,
      notify: true, splitSoop: true, chzzkHq: true, fullscreen: false, chatShown: false };
    w.__TAURI__ = {
      core: { invoke: async (cmd, args) => {
        calls.push([cmd, args]);
        if (cmd === 'list_tiles') return snapshot;
        if (cmd === 'app_version') return '0.2.1';
        if (cmd === 'search') return [];
        return null;
      } },
      event: { listen: async (event, fn) => { events[event] = fn; } },
    };
    w.eval(code);
    await flush();
    const d = w.document;
    d.getElementById('q').value = 'twitch:fixture';
    d.getElementById('add').dispatchEvent(new w.Event('submit', { bubbles: true, cancelable: true }));
    await flush();
    d.getElementById('quality').value = 'low';
    d.getElementById('quality').dispatchEvent(new w.Event('change'));
    d.getElementById('gname').value = '테스트';
    d.getElementById('gsave').dispatchEvent(new w.Event('submit', { cancelable: true }));
    d.getElementById('q').blur();
    w.dispatchEvent(new w.KeyboardEvent('keydown', { code: 'KeyM', bubbles: true }));
    for (const fn of Object.values(events)) fn({ payload: snapshot });
    await flush();
    assert.ok(calls.some(([cmd]) => cmd === 'add_tile'));
    assert.ok(calls.some(([cmd]) => cmd === 'set_quality'));
    assert.ok(calls.some(([cmd]) => cmd === 'save_group'));
    assert.equal(typeof w.__focusAdd, 'function');
    const result = JSON.parse(JSON.stringify({ calls, body: d.body.innerHTML.replace(/<script>[\s\S]*?<\/script>/, '') }));
    dom.window.close();
    return result;
  }
  assert.deepEqual(await exercise(await protect(js)), await exercise(js));
});

test('all injected scripts initialize and preserve hooks, timers and tokens across platforms', async () => {
  const files = ['video_only', 'chzzk_hq', 'chzzk_sd', 'soop_chat', 'overlay', 'assist', 'quality', 'watchdog'];
  const scripts = await Promise.all(files.map(async (name) => {
    const raw = await source(`src-tauri/src/inject/${name}.js`);
    return { name, raw, protected: await protect(raw) };
  }));
  for (const url of ['https://www.youtube.com/watch?v=fixture', 'https://chzzk.naver.com/live/fixture',
    'https://play.sooplive.com/fixture', 'https://www.twitch.tv/fixture', 'https://kick.com/fixture',
    'https://ci.me/fixture', 'https://play.sooplive.com/fixture?vtype=chat']) {
    async function exercise(key) {
      const { dom, w, timers } = browser('<div id="movie_player"><video></video></div>', url);
      for (const script of scripts) w.eval(script[key].replaceAll('__MV_TOKEN__', 'fixture-token').replaceAll('__MV_QUALITY__', 'auto'));
      w.document.dispatchEvent(new w.Event('DOMContentLoaded'));
      w.__mvSetVideoOnly(false);
      w.__mvSetVideoOnly(true);
      w.__mvSetQuality('low');
      w.__mvOverlay?.set({ index: 1, total: 2, volume: 30, quality: 'low', offline: true });
      w.__mvOverlay?.toggleHelp();
      await flush();
      const hooks = Object.keys(w).filter((k) => k.startsWith('__mv')).sort();
      const result = { hooks, timers: timers.map((t) => t.ms), videoOnly: w.__mv.videoOnly,
        body: w.document.body.innerHTML, userAgent: w.navigator.userAgent };
      dom.window.close();
      return result;
    }
    assert.deepEqual(await exercise('protected'), await exercise('raw'), url);
  }
});

test('quality selection and retry path produce the same player calls', async () => {
  const js = await source('src-tauri/src/inject/quality.js');
  async function exercise(code) {
    const { dom, w, timers } = browser('<div id="movie_player"></div>');
    const calls = [];
    const player = w.document.getElementById('movie_player');
    let level = 'hd720';
    player.getAvailableQualityLevels = () => ['hd1080', 'hd720', 'medium', 'small'];
    player.setPlaybackQualityRange = (lo, hi) => { level = lo; calls.push([lo, hi]); };
    player.getPlaybackQuality = () => level;
    w.eval(code.replaceAll('__MV_QUALITY__', 'best'));
    for (const mode of ['low', 'auto', 'best']) {
      w.__mvSetQuality(mode);
      await flush();
    }
    for (const timer of [...timers]) timer.fn();
    await flush();
    dom.window.close();
    return calls;
  }
  const actual = await exercise(await protect(js));
  assert.deepEqual(actual, await exercise(js));
  assert.ok(actual.some(([level]) => level === 'medium'));
  assert.ok(actual.some(([level]) => level === 'auto'));
  assert.ok(actual.some(([level]) => level === 'hd1080'));
});
