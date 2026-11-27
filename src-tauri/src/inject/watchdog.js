// 재생이 멈추면 먼저 play()를 시도하고, 계속 멈춰 있으면 앱에 이 방송의 복구를 요청한다.
(() => {
  if (window.__mvWatchdog) return;
  window.__mvWatchdog = true;

  const TOKEN = '__MV_TOKEN__';
  const PAGE = performance.timeOrigin;
  const RETRY_PLAY_MS = 15000;
  const RECONNECT_MS = 30000;
  const NO_VIDEO_MS = 45000;
  const STREAM_PAGE = /^(play\.sooplive\.(com|co\.kr)|chzzk\.naver\.com|www\.youtube\.com|www\.twitch\.tv|kick\.com|ci\.me)$/;

  function send(cmd) {
    window.chrome?.webview?.postMessage(JSON.stringify({ t: TOKEN, cmd, arg: PAGE }));
  }
  const alive = () => send('stream-alive');
  document.addEventListener('DOMContentLoaded', alive);
  addEventListener('load', alive);
  setInterval(alive, 10000);

  function currentVideo() {
    if (window.__mv?.video?.isConnected) return window.__mv.video;
    let best = null;
    for (const v of document.querySelectorAll('video')) {
      if (!best || v.readyState > best.readyState) best = v;
    }
    return best;
  }

  let pauseInput = -Infinity;
  let pausedVideo = null;
  addEventListener('pointerdown', (e) => {
    const player = window.__mv?.root || currentVideo()?.parentElement;
    if (e.isTrusted && player?.contains(e.target) && !e.target.closest('[data-mv-keep]')) {
      pauseInput = performance.now();
    }
  }, true);
  addEventListener('keydown', (e) => {
    if (e.isTrusted && ['Space', 'KeyK', 'MediaPlayPause'].includes(e.code)
        && !e.target.closest?.('input, textarea, select, [contenteditable]')) pauseInput = performance.now();
  }, true);
  document.addEventListener('pause', (e) => {
    if (e.target === currentVideo() && performance.now() - pauseInput < 1500) pausedVideo = e.target;
  }, true);
  document.addEventListener('play', (e) => {
    if (e.target === pausedVideo) pausedVideo = null;
  }, true);

  let video = null;
  let lastTime = 0;
  let lastFrames = 0;
  let lastProgress = performance.now();
  let lastReport = -Infinity;
  let retried = false;

  setInterval(() => {
    const v = currentVideo();
    const now = performance.now();
    const quality = v?.getVideoPlaybackQuality?.();
    const frames = quality ? quality.totalVideoFrames - quality.droppedVideoFrames : 0;
    // 영상 요소가 바뀌어도 재생이 진행되기 전까지 멈춤 시간을 유지한다.
    const progressed = v && v === video && !v.paused && !v.seeking &&
      (frames > lastFrames || (frames === 0 && lastFrames === 0 && v.currentTime !== lastTime));
    video = v;
    lastTime = v?.currentTime ?? 0;
    lastFrames = frames;

    const expectsVideo = STREAM_PAGE.test(location.hostname) && window.__mv?.videoOnly;
    if (window.__mvOffline || document.hidden || (v && (v === pausedVideo || (v.ended && !window.__mvLive)))
        || (!v && !expectsVideo) || progressed) {
      lastProgress = now;
      retried = false;
      return;
    }

    const stalled = now - lastProgress;
    if (v && !v.ended && stalled >= RETRY_PLAY_MS && !retried) {
      retried = true;
      v.play().catch(() => { /* 재생이 안 되면 아래 복구 요청으로 넘어간다. */ });
    }
    if (stalled >= (v ? RECONNECT_MS : NO_VIDEO_MS) && now - lastReport >= RECONNECT_MS) {
      lastReport = now;
      send('stream-stalled');
    }
  }, 2000);
})();
