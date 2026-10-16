// 화질 제어: 'best'(최고화질) · 'auto'(네트워크 최적화) · 'low'(저화질).
// 플랫폼마다 화질 목록을 읽어 모드에 맞는 화질을 고른 뒤 플레이어에 적용하고, 적용될 때까지 몇 번 다시 시도한다.
// 앱은 처음에 __MV_QUALITY__ 자리에 모드를 넣어 주고, 이후 바꿀 때는 window.__mvSetQuality(mode)를 부른다.
(() => {
  if (window !== window.top || window.__mvSetQuality) return;
  let mode = '__MV_QUALITY__';
  const LOW_TARGET = 360; // 저화질: 이 높이 이상 중 가장 낮은 화질(없으면 가장 낮은 화질)
  const AUTO_FALLBACK = 720; // 자동 화질을 못 찾았을 때의 네트워크 최적화 화질
  const RETRY_MS = 3000;
  const MAX_TRIES = 15;
  const h = location.hostname;

  // 높이 목록에서 모드에 맞는 높이를 고른다.
  function pickHeight(heights, m) {
    const hs = [...new Set(heights.filter((x) => x > 0))].sort((a, b) => b - a);
    if (!hs.length) return null;
    if (m === 'best') return hs[0];
    const target = m === 'low' ? LOW_TARGET : AUTO_FALLBACK;
    const atLeast = hs.filter((x) => x >= target);
    return atLeast.length ? atLeast[atLeast.length - 1] : hs[hs.length - 1];
  }
  const heightOf = (label) => Number((/(\d{3,4})p/.exec(label) || [])[1] || 0);

  const PLATFORMS = {
    youtube: {
      apply(m) {
        const p = document.querySelector('#movie_player');
        if (!p?.getAvailableQualityLevels) return false;
        const levels = p.getAvailableQualityLevels().filter((l) => l !== 'auto');
        if (!levels.length) return false;
        if (m === 'auto') {
          p.setPlaybackQualityRange('auto', 'auto');
          return true;
        }
        // 유튜브 화질 이름 → 높이
        const H = { highres: 4320, hd2880: 2880, hd2160: 2160, hd1440: 1440, hd1080: 1080, hd720: 720, large: 480, medium: 360, small: 240, tiny: 144 };
        const want = pickHeight(levels.map((l) => H[l] || 0), m);
        const level = levels.find((l) => H[l] === want);
        p.setPlaybackQualityRange(level, level);
        return p.getPlaybackQuality() === level;
      },
    },
    chzzk: {
      // 플레이어는 마지막으로 고른 화질을 여기서 읽으므로, 다음 로드부터도 같은 화질로 시작하게 남겨 둔다.
      remember(height) {
        try {
          localStorage.setItem('live-player-video-track', JSON.stringify({ label: `${height}p`, width: Math.round((height * 16) / 9), height }));
        } catch {}
      },
      // 치지직 플레이어 객체는 React 상태에 들어 있고, 화질은 표준 VideoTrack(selected)으로 바꾼다.
      // 화질 메뉴 항목을 스크립트로 눌러서는 바뀌지 않는다.
      tracks() {
        for (let e = document.querySelector('.pzp'); e; e = e.parentElement) {
          const key = Object.keys(e).find((k) => k.startsWith('__reactFiber$'));
          if (!key) continue;
          for (let f = e[key], i = 0; f && i < 40; f = f.return, i++) {
            for (let s = f.memoizedState, j = 0; s && j < 20; s = s.next, j++) {
              const v = s.memoizedState;
              if (v && typeof v === 'object' && v._internalPlayer && v.videoTracks?._getTracks) return v.videoTracks._getTracks();
            }
          }
          break;
        }
        return null;
      },
      apply(m) {
        const all = this.tracks();
        if (!all?.length) return false;
        const current = all.find((t) => t.selected);
        // 일반/저지연 트랙이 따로 있으므로 지금 쓰는 쪽에서 고른다.
        const lowLatency = /low-?latency/i.test(current?.id || '');
        const group = all.filter((t) => /low-?latency/i.test(t.id) === lowLatency);
        let target;
        if (m === 'auto') target = group.find((t) => /^ABR/.test(t.id));
        else {
          const fixed = group.filter((t) => !/^ABR/.test(t.id));
          const want = pickHeight(fixed.map((t) => t.height || heightOf(t.label)), m);
          target = fixed.find((t) => (t.height || heightOf(t.label)) === want);
          if (want) this.remember(want);
        }
        if (!target) return false;
        if (!target.selected) target.selected = true;
        return target.selected;
      },
    },
    soop: {
      apply(m) {
        const buttons = [...document.querySelectorAll('#player .quality_box li button')];
        if (!buttons.length) return false;
        const current = (document.querySelector('#player .btn_quality_mode span')?.textContent || '').trim();
        let target;
        if (m === 'auto') target = buttons.find((b) => b.classList.contains('auto_quality'));
        else {
          const want = pickHeight(buttons.map((b) => heightOf(b.textContent)), m);
          target = buttons.find((b) => heightOf(b.textContent) === want);
        }
        if (!target) return false;
        const label = target.textContent.trim();
        if (current === label) return true;
        target.click();
        return false;
      },
    },
    twitch: {
      // 트위치 플레이어 객체는 React 컴포넌트 속성(mediaPlayerInstance)에 들어 있다.
      player() {
        const v = document.querySelector('video');
        for (let e = v; e; e = e.parentElement) {
          const key = Object.keys(e).find((k) => k.startsWith('__reactFiber$'));
          if (!key) continue;
          for (let f = e[key], i = 0; f && i < 80; f = f.return, i++) {
            const p = f.memoizedProps?.mediaPlayerInstance;
            if (p?.getQualities) return p;
          }
        }
        return null;
      },
      apply(m) {
        const p = this.player();
        if (!p) return false;
        if (m === 'auto') {
          p.setAutoQualityMode(true);
          return p.isAutoQualityMode();
        }
        const qs = p.getQualities();
        if (!qs.length) return false;
        const want = pickHeight(qs.map((q) => q.height || heightOf(q.name)), m);
        const q = qs.find((x) => (x.height || heightOf(x.name)) === want);
        p.setAutoQualityMode(false);
        if (p.getQuality()?.name !== q.name) p.setQuality(q);
        return p.getQuality()?.name === q.name;
      },
    },
  };

  // 킥은 플레이어 객체를 찾을 수 없어 화면의 화질 메뉴를 대신 조작한다.
  // 조작 바는 마우스가 영상 위에 있을 때만 생기므로 마우스 이벤트를 먼저 보낸다.
  PLATFORMS.kick = {
    async apply(m) {
      const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
      const video = (window.__mv && window.__mv.video) || document.querySelector('video');
      const box = video?.parentElement;
      if (!box) return false;
      const r = box.getBoundingClientRect();
      const at = { bubbles: true, cancelable: true, view: window, clientX: r.x + r.width / 2, clientY: r.y + r.height / 2, pointerType: 'mouse' };
      for (const t of ['pointerover', 'pointerenter', 'mouseover', 'mouseenter', 'pointermove', 'mousemove']) box.dispatchEvent(new PointerEvent(t, at));
      await sleep(400);
      const press = (el) => {
        for (const t of ['pointerdown', 'mousedown', 'pointerup', 'mouseup']) {
          el.dispatchEvent(new PointerEvent(t, { bubbles: true, cancelable: true, view: window, pointerType: 'mouse', button: 0 }));
        }
        el.click();
      };
      const buttons = [...box.querySelectorAll('button')];
      const gear = buttons[buttons.length - 1]; // 조작 바의 마지막 버튼이 설정(톱니바퀴)
      if (!gear) return false;
      press(gear);
      await sleep(500);
      // 메뉴 항목: "Auto", "720p60" 같은 글자. 로그인이 필요한 화질은 고를 수 없으니 뺀다.
      const items = [...document.querySelectorAll('span')]
        .filter((s) => !s.children.length && /^(Auto|\d{3,4}p\d*)$/.test(s.textContent.trim()))
        .map((s) => ({ span: s, item: s.closest('[role=menuitemradio],[role=menuitem],[role=option]') || s.parentElement }))
        .filter(({ item }) => !/로그인|log ?in/i.test(item.textContent));
      if (!items.length) {
        press(gear);
        return false;
      }
      let target;
      if (m === 'auto') target = items.find(({ span }) => span.textContent.trim() === 'Auto');
      else {
        const fixed = items.filter(({ span }) => span.textContent.trim() !== 'Auto');
        const want = pickHeight(fixed.map(({ span }) => heightOf(span.textContent)), m);
        target = fixed.find(({ span }) => heightOf(span.textContent) === want);
      }
      if (!target) {
        press(gear);
        return false;
      }
      press(target.item);
      await sleep(300);
      // 메뉴가 남아 있으면 닫는다.
      if (document.querySelector('[role=menu]')) press(gear);
      return true;
    },
  };

  const key = /youtube/.test(h) ? 'youtube' : /chzzk/.test(h) ? 'chzzk' : /sooplive/.test(h) ? 'soop' : /twitch/.test(h) ? 'twitch' : /kick\.com/.test(h) ? 'kick' : null;
  const platform = key && PLATFORMS[key];
  if (!platform) {
    window.__mvSetQuality = () => {};
    return;
  }
  if (key === 'chzzk') platform.remember(mode === 'best' ? 1080 : mode === 'low' ? 360 : 720);

  let tries = 0;
  let timer = 0;
  let lastVideo = null;
  let busy = false;
  async function attempt() {
    clearTimeout(timer);
    // 킥처럼 메뉴를 여닫는 플랫폼은 적용 중에 겹쳐 실행되면 안 된다.
    if (busy) return;
    busy = true;
    let done = false;
    try {
      done = await platform.apply(mode);
    } catch {}
    busy = false;
    if (!done && ++tries < MAX_TRIES) timer = setTimeout(attempt, RETRY_MS);
  }
  window.__mvSetQuality = (m) => {
    mode = m;
    tries = 0;
    attempt();
  };
  // 광고가 끝나거나 플레이어가 영상을 바꾸면 화질이 초기화되는 경우가 있어, 새 영상이 보이면 다시 적용한다.
  setInterval(() => {
    const v = (window.__mv && window.__mv.video) || document.querySelector('video');
    if (v && v !== lastVideo && v.readyState >= 2) {
      lastVideo = v;
      tries = 0;
      attempt();
    }
  }, 2000);
})();
