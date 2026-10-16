// 자동 복구: 영상이 멈춘 채로 있으면 먼저 재생을 다시 시도하고, 그래도 안 되면 새로고침한다.
// 사용자가 직접 멈춘 경우는 건드리지 않고, 끝난 방송을 계속 새로고침하지 않도록 횟수를 제한한다.
(() => {
  if (window.__mvWatchdog) return;
  window.__mvWatchdog = true;

  const RETRY_PLAY_MS = 15000;
  const RELOAD_MS = 30000;
  const MAX_RELOADS = 3;
  const RELOAD_WINDOW_MS = 5 * 60 * 1000;
  const KEY = '__mv_reloads';

  let lastInput = 0;
  let userPaused = false;
  const markInput = () => (lastInput = Date.now());
  addEventListener('pointerdown', markInput, true);
  addEventListener('keydown', markInput, true);
  // 클릭·키 입력 직후의 일시정지는 사용자가 한 것으로 본다.
  document.addEventListener('pause', (e) => {
    if (e.target instanceof HTMLVideoElement && Date.now() - lastInput < 1500) userPaused = true;
  }, true);
  document.addEventListener('play', (e) => {
    if (e.target instanceof HTMLVideoElement) userPaused = false;
  }, true);

  function currentVideo() {
    if (window.__mv && window.__mv.video && window.__mv.video.isConnected) return window.__mv.video;
    let best = null;
    for (const v of document.querySelectorAll('video')) {
      if (!best || v.readyState > best.readyState) best = v;
    }
    return best;
  }

  function takeReloadSlot() {
    try {
      const now = Date.now();
      const recent = JSON.parse(sessionStorage.getItem(KEY) || '[]').filter((t) => now - t < RELOAD_WINDOW_MS);
      if (recent.length >= MAX_RELOADS) return false;
      recent.push(now);
      sessionStorage.setItem(KEY, JSON.stringify(recent));
      return true;
    } catch {
      return false;
    }
  }

  let video = null;
  let lastTime = -1;
  let lastProgress = Date.now();
  let retried = false;

  // 페이지 앱이 아예 뜨지 않아 영상 요소조차 없는 경우(유튜브에서 가끔 발생)도 새로고침한다.
  // 로그인 페이지나 사이트 보기 중에는 영상이 없는 게 정상이므로, 방송 페이지에서 영상만 보기일 때만 적용한다.
  const NO_VIDEO_RELOAD_MS = 45000;
  const STREAM_PAGE = /^(play\.sooplive\.(com|co\.kr)|chzzk\.naver\.com|www\.youtube\.com|www\.twitch\.tv|kick\.com|ci\.me)$/;
  const started = Date.now();
  const expectsVideo = () => STREAM_PAGE.test(location.hostname) && !!(window.__mv && window.__mv.videoOnly);

  setInterval(() => {
    // 앱이 방송 종료로 판단한 동안에는 새로고침하지 않는다(다시 켜지면 앱이 새로고침한다).
    if (window.__mvOffline) return;
    const v = currentVideo();
    const now = Date.now();
    if (!v) {
      if (expectsVideo() && now - started > NO_VIDEO_RELOAD_MS && now - lastInput > RELOAD_MS && takeReloadSlot()) {
        location.reload();
      }
      return;
    }
    if (v !== video) {
      video = v;
      lastTime = v.currentTime;
      lastProgress = now;
      retried = false;
      return;
    }
    // 사용자가 멈췄거나 조작 중이거나 창이 가려져 있으면 멈춘 것으로 치지 않는다.
    if (userPaused || v.ended || document.hidden || now - lastInput < RELOAD_MS) {
      lastProgress = now;
      return;
    }
    // 일부 플레이어(숲)는 멈춘 상태에서도 라이브 지연을 맞추려고 재생 위치를 옮기므로,
    // 실제로 재생 중이고 위치가 앞으로 갈 때만 진행으로 본다.
    const progressed = !v.paused && v.readyState >= 3 && v.currentTime > lastTime;
    lastTime = v.currentTime;
    if (progressed) {
      lastProgress = now;
      retried = false;
      return;
    }
    const stalled = now - lastProgress;
    if (stalled > RETRY_PLAY_MS && !retried) {
      retried = true;
      v.play().catch(() => {});
    }
    if (stalled > RELOAD_MS && takeReloadSlot()) {
      location.reload();
    }
  }, 2000);
})();
