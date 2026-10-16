// 타일 오버레이: 마우스를 올리면 위쪽에 "번호 · 방송인 · 제목"과 조작 버튼을 보여주고,
// 키보드 단축키를 앱으로 전달한다. 사이트 스타일과 섞이지 않도록 닫힌 Shadow DOM에 그린다.
// 앱과는 WebView2 메시지 채널로 통신하며, 실행마다 바뀌는 토큰으로 사이트 스크립트의 위조를 막는다.
(() => {
  if (window !== window.top || window.__mvOverlay) return;
  const TOKEN = '__MV_TOKEN__';
  const HIDE_AFTER_MS = 2200;
  const META_REFRESH_MS = 60000;

  const send = (cmd, arg = 0, self = false, data = null) => {
    try {
      window.chrome.webview.postMessage(JSON.stringify({ t: TOKEN, cmd, arg, self, data }));
    } catch {}
  };

  // ---------- 상태 ----------
  let st = { index: 0, total: 0, selected: false, muted: false, chat: false, detached: false, videoOnly: true, fullscreen: false, quality: 'best', zoom: 1, dragging: false, dropTarget: false, offline: false, image: null };
  let meta = { name: '', title: '' };

  // ---------- 화면 ----------
  // 유튜브처럼 Trusted Types를 쓰는 사이트에서는 innerHTML이 막히므로 DOM을 직접 만든다.
  const SVG_NS = 'http://www.w3.org/2000/svg';
  const STROKE = { fill: 'none', stroke: 'currentColor', 'stroke-width': '2', 'stroke-linecap': 'round', 'stroke-linejoin': 'round' };
  const ICON = {
    volume: [['path', { d: 'M4 9v6h4l5 4V5L8 9H4z' }], ['path', { d: 'M16 8.5a5 5 0 0 1 0 7', ...STROKE }]],
    mute: [['path', { d: 'M4 9v6h4l5 4V5L8 9H4z' }], ['path', { d: 'm16 9 5 6m0-6-5 6', ...STROKE }]],
    chat: [['path', { d: 'M4 5h16v11H9l-5 4V5z', ...STROKE }]],
    detach: [['path', { d: 'M14 4h6v6M20 4l-8 8M18 14v6H4V6h6', ...STROKE }]],
    attach: [['path', { d: 'M10 14 4 20m0-6v6h6M10 4h10v10', ...STROKE }]],
    site: [['rect', { x: '3', y: '4', width: '18', height: '16', rx: '2', ...STROKE }], ['path', { d: 'M3 9h18', ...STROKE }]],
    video: [['rect', { x: '3', y: '5', width: '18', height: '14', rx: '2', ...STROKE }], ['path', { d: 'm10 9 5 3-5 3z' }]],
    reload: [['path', { d: 'M20 12a8 8 0 1 1-2.3-5.7M20 4v5h-5', ...STROKE }]],
    close: [['path', { d: 'm6 6 12 12M18 6 6 18', ...STROKE }]],
  };
  function svg(name) {
    const s = document.createElementNS(SVG_NS, 'svg');
    for (const [k, v] of Object.entries({ viewBox: '0 0 24 24', width: '18', height: '18', fill: 'currentColor', 'aria-hidden': 'true' })) s.setAttribute(k, v);
    for (const [tag, attrs] of ICON[name]) {
      const el = document.createElementNS(SVG_NS, tag);
      for (const [k, v] of Object.entries(attrs)) el.setAttribute(k, v);
      s.appendChild(el);
    }
    return s;
  }
  // el('div.cls', { title: '…' }, child, '글자', …)
  function el(spec, attrs, ...children) {
    const [tag, ...classes] = spec.split('.');
    const node = document.createElement(tag);
    if (classes.length) node.className = classes.join(' ');
    for (const [k, v] of Object.entries(attrs || {})) node.setAttribute(k, v);
    for (const c of children) node.append(c);
    return node;
  }
  const setIcon = (button, name) => button.replaceChildren(svg(name));

  const PLATFORM = (() => {
    const h = location.hostname;
    if (/sooplive/.test(h)) return { name: '숲', color: '#0545b1' };
    if (/chzzk/.test(h)) return { name: '치지직', color: '#00d564' };
    if (/youtube/.test(h)) return { name: '유튜브', color: '#ff0033' };
    if (/twitch/.test(h)) return { name: '트위치', color: '#9146ff' };
    if (/kick\.com/.test(h)) return { name: '킥', color: '#53fc18' };
    if (/ci\.me/.test(h)) return { name: '씨미', color: '#ff5c8a' };
    return { name: '', color: '#6366f1' };
  })();

  const CSS = `
  :host { all: initial; }
  * { box-sizing: border-box; font-family: "Pretendard", "Malgun Gothic", system-ui, sans-serif; }
  .frame { position: fixed; inset: 0; pointer-events: none; box-shadow: inset 0 0 0 2px #6366f1; opacity: 0; transition: opacity .15s; }
  .frame.on { opacity: 1; }
  .bar { position: fixed; top: 0; left: 0; right: 0; display: flex; align-items: center; gap: 8px;
    padding: 8px 8px 22px 10px; color: #fff; pointer-events: auto;
    background: linear-gradient(rgba(0,0,0,.78), rgba(0,0,0,.45) 60%, transparent);
    opacity: 0; transform: translateY(-6px); transition: opacity .15s, transform .15s; }
  .bar.show { opacity: 1; transform: none; }
  .bar:not(.show) { pointer-events: none; }
  .idx { flex: none; min-width: 22px; height: 22px; padding: 0 6px; border-radius: 6px; background: rgba(255,255,255,.16);
    font: 700 12px/22px system-ui; text-align: center; }
  .idx.sel { background: #6366f1; }
  .meta { flex: 1; min-width: 0; cursor: pointer; }
  .idx { cursor: grab; }
  .bar.grabbing, .bar.grabbing * { cursor: grabbing !important; }
  .off { position: fixed; inset: 0; display: none; place-items: center; pointer-events: auto; overflow: hidden;
    background: radial-gradient(ellipse at 50% 35%, color-mix(in srgb, var(--pf) 22%, #0b0b0e) 0%, #0b0b0e 70%); color: #e8e8ee; }
  .off.on { display: grid; }
  .offcard { display: flex; flex-direction: column; align-items: center; gap: min(10px, 2.4vmin); text-align: center; padding: 16px; max-width: 100%; }
  .avatar { position: relative; width: min(84px, 26vmin); height: min(84px, 26vmin); border-radius: 50%; flex: none;
    box-shadow: 0 0 0 3px #0b0b0e, 0 0 0 5px rgba(255,255,255,.14); background: #2a2a33; display: grid; place-items: center;
    font: 700 min(30px, 10vmin)/1 "Pretendard", "Malgun Gothic", system-ui, sans-serif; color: rgba(255,255,255,.7); overflow: hidden; }
  .avatar img { width: 100%; height: 100%; object-fit: cover; filter: grayscale(.85) brightness(.8); }
  .offname { font-size: min(17px, 5.2vmin); font-weight: 700; max-width: 90vw; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .badge { font: 800 min(11px, 3.4vmin)/1 system-ui; letter-spacing: .14em; padding: 5px 9px; border-radius: 999px;
    background: rgba(255,255,255,.08); border: 1px solid rgba(255,255,255,.14); color: rgba(255,255,255,.7); }
  .offmsg { font-size: min(13px, 4vmin); color: rgba(255,255,255,.55); line-height: 1.5; }
  .drop { position: fixed; inset: 0; display: none; place-items: center; pointer-events: none;
    border: 3px dashed #818cf8; background: rgba(99,102,241,.22); color: #fff; font: 700 15px/1.4 "Pretendard", "Malgun Gothic", system-ui, sans-serif; }
  .drop.on { display: grid; }
  .drop span { padding: 8px 14px; border-radius: 8px; background: rgba(0,0,0,.6); }
  .frame.moving { opacity: 1; box-shadow: inset 0 0 0 3px #818cf8; }
  .name { font-size: 13px; font-weight: 700; line-height: 1.3; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .pf { display: inline-block; width: 8px; height: 8px; border-radius: 50%; margin-right: 6px; vertical-align: 1px; }
  .title { font-size: 12px; line-height: 1.35; color: rgba(255,255,255,.78); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .btns { flex: none; display: flex; gap: 2px; }
  button { all: unset; display: grid; place-items: center; width: 30px; height: 30px; border-radius: 8px; color: rgba(255,255,255,.88); cursor: pointer; }
  button:hover { background: rgba(255,255,255,.16); color: #fff; }
  button.on { color: #a5b4fc; }
  button.x:hover { background: rgba(239,68,68,.85); }
  button.q { width: auto; padding: 0 8px; font: 700 11px/30px system-ui; letter-spacing: .02em; }
  .help { position: fixed; inset: 0; display: none; place-items: center; background: rgba(0,0,0,.6); pointer-events: auto; }
  .help.show { display: grid; }
  .card { background: #16161b; color: #e8e8ee; border: 1px solid #2a2a33; border-radius: 12px; padding: 16px 20px; max-width: 92%; max-height: 92%; overflow: auto; }
  .card h3 { margin: 0 0 10px; font-size: 14px; }
  .row { display: flex; gap: 14px; font-size: 13px; line-height: 1.9; white-space: nowrap; }
  .keys { min-width: 150px; }
  kbd { display: inline-block; min-width: 20px; padding: 0 6px; margin-right: 2px; border: 1px solid #3a3a46; border-bottom-width: 2px; border-radius: 5px;
    background: #23232b; font: 600 12px/20px system-ui; text-align: center; }`;

  const HELP = [
    [['1', '~', '9'], '방송 선택 (소리 켜기)'],
    [['Tab', '/', 'Shift+Tab'], '다음 / 이전 방송 선택'],
    [['M'], '음소거 켜기/끄기'],
    [['Q'], '화질 바꾸기 (최고 → 자동 → 저화질)'],
    [['F'], '선택한 방송 크게 보기 (포커스 / 격자)'],
    [['F11'], '전체 화면'],
    [['C'], '채팅'],
    [['D'], '창 분리 / 합치기'],
    [['V'], '사이트 보기 / 영상만 보기'],
    [['R'], '새로고침'],
    [['Alt+←', 'Alt+→'], '순서 옮기기'],
    [['Delete'], '방송 닫기'],
    [['N', '/', 'Ctrl+L'], '방송 추가 (주소·검색)'],
    [['?'], '이 도움말'],
  ];

  const host = document.createElement('moabom-overlay');
  host.setAttribute('data-mv-keep', '');
  host.style.cssText = 'all:initial;position:fixed;inset:0;z-index:2147483647;pointer-events:none;display:block;';
  const root = host.attachShadow({ mode: 'closed' });

  const style = document.createElement('style');
  style.textContent = CSS;
  const frame = el('div.frame');
  const idx = el('span.idx');
  const pf = el('span.pf');
  const nm = el('span');
  const titleEl = el('div.title');
  const meta_ = el('div.meta', { title: '클릭: 이 방송 소리 켜기 · 끌기: 위치 옮기기' }, el('div.name', null, pf, nm), titleEl);
  idx.title = '끌어서 위치 옮기기';
  const btn = (cmd, title, icon, cls = '') => {
    const b = el(cls ? `button.${cls}` : 'button', { 'data-cmd': cmd, title });
    if (icon) setIcon(b, icon);
    return b;
  };
  const audioBtn = btn('audio', '', 'mute');
  const chatBtn = btn('chat', '채팅 (C)', 'chat');
  const detachBtn = btn('detach', '', 'detach');
  const siteBtn = btn('siteview', '', 'site');
  const qualityBtn = btn('quality', '', null, 'q');
  const bar = el('div.bar', null, idx, meta_,
    el('div.btns', null, qualityBtn, audioBtn, chatBtn, detachBtn, siteBtn, btn('reload', '새로고침 (R)', 'reload'), btn('close', '닫기 (Delete)', 'close', 'x')));
  const helpRows = HELP.map(([keys, desc]) => {
    const k = el('div.keys');
    for (const key of keys) k.append(key === '/' || key === '~' ? ` ${key} ` : el('kbd', null, key));
    return el('div.row', null, k, el('span', null, desc));
  });
  const help = el('div.help', null, el('div.card', null, el('h3', null, '단축키'), ...helpRows));
  const drop = el('div.drop', null, el('span', null, '여기에 놓으면 자리를 바꿉니다'));
  // 방송 종료 화면: 앱이 방송이 꺼졌다고 알려 주면 사이트 화면을 덮는다.
  const avatar = el('div.avatar');
  const offName = el('span');
  const off = el('div.off', null, el('div.offcard', null,
    avatar,
    el('div.offname', null, offName),
    el('div.badge', null, PLATFORM.name ? `${PLATFORM.name} · OFFLINE` : 'OFFLINE'),
    el('div.offmsg', null, '방송이 종료되었어요', el('br'), '다시 시작하면 자동으로 연결해요')));
  off.style.setProperty('--pf', PLATFORM.color);
  let avatarSrc = null;
  const noScroll = document.createElement('style');
  root.append(style, off, frame, bar, drop, help);

  function render() {
    // 앱이 페이지 확대 비율을 낮춘 경우(좁은 킥 타일) 오버레이만 되돌려 원래 크기로 보이게 한다.
    host.style.zoom = String(1 / (st.zoom || 1));
    idx.textContent = st.index || '';
    idx.classList.toggle('sel', st.selected);
    frame.classList.toggle('on', st.selected && st.total > 1);
    frame.classList.toggle('moving', st.dragging);
    drop.classList.toggle('on', st.dropTarget);
    off.classList.toggle('on', st.offline);
    window.__mvOffline = st.offline;
    // 종료 화면 옆으로 사이트 스크롤바가 보이지 않게 한다.
    noScroll.textContent = st.offline ? 'html, body { overflow: hidden !important; }' : '';
    if (!noScroll.isConnected && document.head) document.head.appendChild(noScroll);
    offName.textContent = meta.name || PLATFORM.name;
    // 사진이 없거나 못 불러오면 이름 첫 글자를 보여 주고, 다음에 다시 시도한다.
    const initial = () => avatar.replaceChildren((meta.name || PLATFORM.name || '?').slice(0, 1));
    if (st.image && avatarSrc !== st.image) {
      avatarSrc = st.image;
      const img = el('img', { alt: '', referrerpolicy: 'no-referrer' });
      img.onerror = () => { avatarSrc = null; initial(); };
      img.src = st.image;
      avatar.replaceChildren(img);
    } else if (!avatar.querySelector('img')) initial();
    pf.style.background = PLATFORM.color;
    nm.textContent = meta.name || PLATFORM.name || location.hostname;
    titleEl.textContent = meta.title || '';
    const audible = st.selected && !st.muted;
    setIcon(audioBtn, audible ? 'volume' : 'mute');
    audioBtn.classList.toggle('on', audible);
    audioBtn.title = audible ? '음소거 (M)' : `이 방송 소리 켜기 (${st.index || '숫자'})`;
    chatBtn.classList.toggle('on', st.chat);
    setIcon(detachBtn, st.detached ? 'attach' : 'detach');
    detachBtn.title = st.detached ? '메인 창으로 합치기 (D)' : '새 창으로 분리 (D)';
    setIcon(siteBtn, st.videoOnly ? 'site' : 'video');
    siteBtn.title = st.videoOnly ? '사이트 보기 (V)' : '영상만 보기 (V)';
    const Q = { best: ['최고', '최고화질'], auto: ['자동', '네트워크 최적화'], low: ['저화질', '저화질'] }[st.quality] || ['최고', '최고화질'];
    qualityBtn.textContent = Q[0];
    qualityBtn.title = `화질: ${Q[1]} (Q로 바꾸기)`;
  }

  // 방송을 누르면 앱에 알려 컨트롤 바의 검색 결과를 닫게 한다(다른 웹뷰라 컨트롤 바는 포커스 변화를 모른다).
  addEventListener('pointerdown', () => send('touched'), true);
  addEventListener('focus', () => send('touched'));

  // ---------- 끌어서 위치 옮기기 ----------
  // 번호·방송인 부분을 잡고 끌면 마우스 위치를 앱에 보내고, 앱이 마우스 아래 타일을 표시한다.
  // 다른 웹뷰 위로 나가도 이벤트를 계속 받도록 포인터를 붙잡는다. 놓으면 앱이 두 타일의 자리를 바꾼다.
  let drag = null;
  let justDragged = false;
  const dragPos = (e) => ({ x: e.clientX * (st.zoom || 1), y: e.clientY * (st.zoom || 1) });
  root.addEventListener('pointerdown', (e) => {
    const handle = e.button === 0 && !st.detached && st.total > 1 && e.target.closest('.idx, .meta');
    if (!handle) return;
    drag = { id: e.pointerId, x: e.clientX, y: e.clientY, on: false, frame: 0, el: handle };
    handle.setPointerCapture(e.pointerId);
  });
  root.addEventListener('pointermove', (e) => {
    if (!drag || e.pointerId !== drag.id) return;
    if (!drag.on) {
      if (Math.hypot(e.clientX - drag.x, e.clientY - drag.y) < 6) return;
      drag.on = true;
      bar.classList.add('grabbing');
    }
    cancelAnimationFrame(drag.frame);
    const pos = dragPos(e);
    drag.frame = requestAnimationFrame(() => send('drag', 0, true, pos));
  });
  const endDrag = (e, cancel) => {
    if (!drag || e.pointerId !== drag.id) return;
    cancelAnimationFrame(drag.frame);
    if (drag.on) {
      send('drag', 0, true, { ...(cancel ? { x: null, y: null } : dragPos(e)), end: true });
      justDragged = true;
      setTimeout(() => (justDragged = false), 0);
    }
    bar.classList.remove('grabbing');
    drag = null;
  };
  root.addEventListener('pointerup', (e) => endDrag(e, false));
  root.addEventListener('pointercancel', (e) => endDrag(e, true));

  root.addEventListener('click', (e) => {
    if (justDragged) return;
    const b = e.target.closest('button');
    if (b) {
      e.stopPropagation();
      send(b.dataset.cmd, 0, true);
    } else if (e.target.closest('.meta')) {
      send('audio', 0, true);
    } else if (e.target.closest('.help')) {
      help.classList.remove('show');
    }
  });

  // ---------- 마우스를 올리면 보이기 ----------
  let hideTimer = 0;
  let overBar = false;
  const show = () => {
    if (drag) return;
    bar.classList.add('show');
    clearTimeout(hideTimer);
    hideTimer = setTimeout(() => !overBar && bar.classList.remove('show'), HIDE_AFTER_MS);
  };
  const hide = () => {
    clearTimeout(hideTimer);
    bar.classList.remove('show');
  };
  addEventListener('mousemove', show, { capture: true, passive: true });
  document.addEventListener('mouseleave', hide);
  bar.addEventListener('mouseenter', () => { overBar = true; show(); });
  bar.addEventListener('mouseleave', () => { overBar = false; show(); });

  // ---------- 키보드 ----------
  const editable = (el) => !!el && (el.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(el.tagName));
  function keyCommand(e) {
    if (e.ctrlKey || e.metaKey) return e.code === 'KeyL' ? ['add'] : null;
    if (e.altKey) {
      if (e.code === 'ArrowLeft') return ['move', -1];
      if (e.code === 'ArrowRight') return ['move', 1];
      return null;
    }
    const digit = /^(?:Digit|Numpad)([1-9])$/.exec(e.code);
    if (digit) return ['select', Number(digit[1])];
    switch (e.code) {
      case 'Tab': return [e.shiftKey ? 'prev' : 'next'];
      case 'KeyM': return ['mute'];
      case 'KeyQ': return ['quality'];
      case 'KeyF': return ['layout'];
      case 'F11': return ['fullscreen'];
      case 'Escape': return st.fullscreen ? ['exitfullscreen'] : help.classList.contains('show') ? ['help'] : null;
      case 'KeyC': return ['chat'];
      case 'KeyD': return ['detach'];
      case 'KeyV': return ['siteview'];
      case 'KeyR': return ['reload'];
      case 'Delete': return ['close'];
      case 'KeyN': return ['add'];
      case 'Slash': return [e.shiftKey ? 'help' : 'add'];
      case 'F1': return ['help'];
      default: return null;
    }
  }
  addEventListener('keydown', (e) => {
    if (e.isComposing || editable(e.target) || editable(document.activeElement)) return;
    const k = keyCommand(e);
    if (!k) return;
    e.preventDefault();
    e.stopImmediatePropagation();
    if (k[0] === 'help') help.classList.toggle('show');
    else send(k[0], k[1] || 0);
  }, true);

  // ---------- 방송인·제목 ----------
  const text = (sel) => (document.querySelector(sel)?.textContent || '').trim();
  const META = {
    async soop() {
      return {
        name: document.title.replace(/\s*-\s*SOOP\s*$/, ''),
        title: text('#broadTitle') || document.querySelector('meta[property="og:title"]')?.content || '',
      };
    },
    async chzzk() {
      const id = location.pathname.split('/')[2];
      const j = await fetch(`https://api.chzzk.naver.com/service/v3/channels/${id}/live-detail`, { credentials: 'include' }).then((r) => r.json());
      return { name: j.content?.channel?.channelName, title: j.content?.liveTitle };
    },
    async youtube() {
      const vd = document.querySelector('#movie_player')?.getVideoData?.() || window.ytInitialPlayerResponse?.videoDetails;
      return vd?.title ? { name: vd.author, title: vd.title } : {};
    },
    async twitch() {
      const login = location.pathname.split('/')[1];
      const j = await fetch('https://gql.twitch.tv/gql', {
        method: 'POST',
        headers: { 'Client-Id': 'kimne78kx3ncx6brgo4mv6wki5h1ko' },
        body: JSON.stringify({ query: 'query($l:String!){user(login:$l){displayName stream{title}}}', variables: { l: login } }),
      }).then((r) => r.json());
      return { name: j.data?.user?.displayName || login, title: j.data?.user?.stream?.title };
    },
    async kick() {
      const slug = location.pathname.split('/')[1];
      const j = await fetch(`/api/v2/channels/${slug}`).then((r) => r.json());
      return { name: j.user?.username || slug, title: j.livestream?.session_title };
    },
  };
  const metaKey = /sooplive/.test(location.hostname) ? 'soop'
    : /chzzk/.test(location.hostname) ? 'chzzk'
    : /youtube/.test(location.hostname) ? 'youtube'
    : /twitch/.test(location.hostname) ? 'twitch'
    : /kick\.com/.test(location.hostname) ? 'kick' : null;

  async function refreshMeta() {
    let m = {};
    try {
      if (metaKey) m = await META[metaKey]();
    } catch {}
    const next = { name: m.name || '', title: m.title || (metaKey ? '' : document.title) };
    if (next.name !== meta.name || next.title !== meta.title) {
      meta = next;
      render();
    }
    // 앱이 메시지를 받기 전에 보낸 첫 전송을 놓칠 수 있어, 바뀌지 않아도 갱신할 때마다 보낸다.
    if (meta.name || meta.title) send('meta', 0, true, meta);
  }

  // ---------- 시작 ----------
  function mount() {
    if (!host.isConnected && document.documentElement) document.documentElement.appendChild(host);
  }
  window.__mvOverlay = {
    set(next) {
      st = { ...st, ...next };
      render();
    },
    toggleHelp() {
      help.classList.toggle('show');
    },
  };
  render();
  mount();
  document.addEventListener('DOMContentLoaded', () => {
    mount();
    // 앱에 방송 페이지가 실제로 열렸다고 알린다(첫 이동이 멈춘 타일을 찾는 데 쓴다).
    // 앱이 메시지를 받을 준비가 늦을 수 있어 몇 번 더 보낸다.
    for (const ms of [0, 5000, 15000]) setTimeout(() => send('ready', 0, true), ms);
  });
  addEventListener('load', () => send('ready', 0, true));
  // 사이트가 문서를 다시 그리면서 오버레이를 지우는 경우를 대비한다.
  setInterval(mount, 2000);
  // 페이지가 정보를 늦게 채우는 경우가 많아 처음에는 짧은 간격으로 여러 번 읽고, 이후에는 주기적으로 갱신한다.
  for (const ms of [1500, 4000, 8000, 15000, 30000]) setTimeout(refreshMeta, ms);
  setInterval(refreshMeta, META_REFRESH_MS);
})();
