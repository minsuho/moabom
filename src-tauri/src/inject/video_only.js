// 타일 웹뷰에 주입되는 스크립트: 사이트 UI를 감추고 플랫폼 플레이어(영상 + 컨트롤 바)만 화면 가득 보여준다.
// 플레이어 안의 일시정지·볼륨·화질 같은 원래 조작은 그대로 쓸 수 있다.
// 사이트가 CSS 레이어 안에서 !important를 쓰면 스타일시트로는 이길 수 없어서, 인라인 !important로 덮어쓴다.
(() => {
  if (window.__mv) return;
  const state = (window.__mv = { videoOnly: true, video: null, root: null, touched: [] });

  // 플랫폼별 플레이어 최상위 요소. 영상에서 가장 가까운 것을 쓰고, 없으면 크기로 추정한다.
  const PLAYER_SELECTORS = [
    '#movie_player', // 유튜브
    '.pzp.pzp-pc', // 치지직
    '#player', // 숲
    '.video-player', // 트위치
    '#injected-channel-player', // 킥
  ];

  // 플레이어로 가는 경로(조상)에 있지 않은 형제 요소는 통째로 display:none 한다.
  // VOD 목록·추천·사이드바 같은 화면 밖 요소를 브라우저가 아예 그리지 않고, 지연 로딩 이미지도 불러오지 않는다.
  const HIDE_CSS = `
html[data-mv-on] [data-mv-anc] > *:not([data-mv-anc]):not([data-mv-main]):not([data-mv-keep]) { display: none !important; }`;

  const ROOT_STYLE = {
    visibility: 'visible', position: 'fixed', top: '0', left: '0', right: 'auto', bottom: 'auto',
    width: '100vw', height: '100vh', 'max-width': 'none', 'max-height': 'none', 'min-width': '0', 'min-height': '0',
    margin: '0', transform: 'none', background: '#000', 'z-index': '2147483647',
  };
  const VIDEO_STYLE = {
    width: '100%', height: '100%', left: '0', top: '0',
    'object-fit': 'contain', 'max-width': 'none', 'max-height': 'none',
  };
  // 플레이어와 영상 사이의 래퍼. 비율 고정 크기를 쓰는 플레이어(트위치 등)에서 영상이 작게 남지 않게 한다.
  const FILL_STYLE = { width: '100%', height: '100%', 'max-width': 'none', 'max-height': 'none' };
  // 조상에 이런 속성이 있으면 position:fixed 기준이 뷰포트가 아니게 되거나 플레이어가 잘린다.
  const ANCESTOR_STYLE = {
    transform: 'none', filter: 'none', 'backdrop-filter': 'none', contain: 'none', 'container-type': 'normal',
    perspective: 'none', 'clip-path': 'none', 'will-change': 'auto', overflow: 'visible',
  };
  const PAGE_STYLE = { overflow: 'hidden', background: '#000' };

  function ensureStyle() {
    if (document.getElementById('__mv_style')) return;
    const el = document.createElement('style');
    el.id = '__mv_style';
    el.textContent = HIDE_CSS;
    (document.head || document.documentElement).appendChild(el);
  }

  function override(el, props) {
    state.touched.push([el, el.getAttribute('style')]);
    for (const k in props) el.style.setProperty(k, props[k], 'important');
  }

  function restore() {
    for (const [el, style] of state.touched) {
      if (style === null) el.removeAttribute('style');
      else el.setAttribute('style', style);
      el.removeAttribute('data-mv-anc');
      el.removeAttribute('data-mv-main');
    }
    state.touched = [];
    state.video = null;
    state.root = null;
  }

  function score(v) {
    const playing = !v.paused && !v.ended && v.readyState >= 2;
    return (playing ? 1e9 : 0) + (v.readyState >= 2 ? 1e8 : 0) + v.videoWidth * v.videoHeight;
  }

  function pickVideo() {
    let best = null;
    for (const v of document.querySelectorAll('video')) {
      if (!best || score(v) > score(best)) best = v;
    }
    return best;
  }

  function playerRoot(v) {
    for (const sel of PLAYER_SELECTORS) {
      const p = v.closest(sel);
      if (p) return p;
    }
    // 영상과 크기가 거의 같은 가장 바깥 컨테이너를 플레이어로 본다.
    const vr = v.getBoundingClientRect();
    const area = Math.max(1, vr.width * vr.height);
    let root = v;
    for (let e = v.parentElement; e && e !== document.body; e = e.parentElement) {
      const r = e.getBoundingClientRect();
      if (r.width * r.height > area * 1.6) break;
      root = e;
    }
    return root;
  }

  // 사이트가 렌더링 중에 style 속성을 다시 쓰는 경우가 있어 주기적으로 확인한다.
  // 유튜브처럼 플레이어 요소를 다른 부모로 옮기는 사이트도 있어, 위쪽 경로가 모두 표시돼 있는지도 본다.
  // (새 부모에 표시가 없으면 화면 밖 요소로 숨겨져 플레이어가 0x0이 된다.)
  function intact() {
    const r = state.root;
    if (!r || !r.isConnected || r.style.getPropertyPriority('position') !== 'important') return false;
    for (let e = r.parentElement; e; e = e.parentElement) {
      if (!e.hasAttribute('data-mv-anc')) return false;
    }
    return true;
  }

  // 페이지가 열린 뒤 이 시간이 지나도 재생 가능한 영상이 없으면 오류 문구 등이 보이도록 사이트를 그대로 둔다.
  const GRACE_MS = 8000;
  const started = Date.now();

  function apply() {
    const html = document.documentElement;
    if (!html) return;
    let v = state.videoOnly ? pickVideo() : null;
    if (v && v.readyState === 0 && Date.now() - started > GRACE_MS) v = null;
    if (!v) {
      // 로그인/성인인증 페이지처럼 영상이 없으면 사이트를 그대로 보여준다.
      html.removeAttribute('data-mv-on');
      if (state.root) restore();
      return;
    }
    ensureStyle();
    if (v !== state.video || !intact()) {
      restore();
      const root = playerRoot(v);
      root.setAttribute('data-mv-main', '');
      override(root, ROOT_STYLE);
      if (root !== v) {
        override(v, VIDEO_STYLE);
        for (let e = v.parentElement; e && e !== root; e = e.parentElement) override(e, FILL_STYLE);
      }
      for (let e = root.parentElement; e; e = e.parentElement) {
        e.setAttribute('data-mv-anc', '');
        override(e, e === html || e === document.body ? PAGE_STYLE : ANCESTOR_STYLE);
      }
      state.video = v;
      state.root = root;
      // 플레이어가 바뀐 크기에 맞춰 컨트롤 바를 다시 배치하도록 알린다.
      window.dispatchEvent(new Event('resize'));
    }
    html.setAttribute('data-mv-on', '');
  }

  window.__mvSetVideoOnly = (on) => {
    state.videoOnly = !!on;
    apply();
  };

  setInterval(apply, 1000);
  document.addEventListener('DOMContentLoaded', apply);
})();
