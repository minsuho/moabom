// Exercise the exact Rust-embedded script, regardless of minifier identifier/quote choices.
const assert = require('node:assert/strict');
const { readFileSync } = require('node:fs');
const { runInNewContext } = require('node:vm');
let chosen;
const player = {
  getAvailableQualityLevels: () => ['hd1080', 'hd720', 'medium', 'small'],
  setPlaybackQualityRange: (lo) => { chosen = lo; },
  getPlaybackQuality: () => chosen,
};
const window = {};
window.top = window;
runInNewContext(readFileSync(0, 'utf8'), {
  window,
  location: { hostname: 'www.youtube.com' },
  document: { querySelector: (selector) => selector === 'video' ? { readyState: 4 } : player },
  setInterval: (callback) => callback(),
  clearTimeout() {},
  setTimeout() {},
});
assert.equal(chosen, 'medium', 'Low quality must select the 360p track');
