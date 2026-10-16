(() => {
  const _a = ["Lnl0cC1za2lwLWFkLWJ1dHRvbiwueXRwLWFkLXNraXAtYnV0dG9uLC55dHAtYWQtc2tpcC1idXR0b24tbW9kZXJuLC55dHAtYWQtc2tpcC1idXR0b24tY29udGFpbmVyIGJ1dHRvbg==","XijqtJHqs6BccyopPyhza2lwKFxzKmFkKT986rG064SI65uw6riwfOyKpO2CtSkoXHMq6rSR6rOgKT9ccypb4pa24pa6Pnzina/igLpdKiQ=","W2NsYXNzKj0ic2tpcCIgaV0sYnV0dG9uLFtyb2xlPSJidXR0b24iXQ==","LnZvZF9wbGF5ZXJfd3JhcCB2aWRlbw==","LnZvZF9wbGF5ZXJfd3JhcA==","LmFkX2luZm9fYXJlYSwgLnNraXBfYXJlYQ==","bXV0ZWQ=","YXJpYS1kaXNhYmxlZA==","cG9pbnRlcmRvd24sbW91c2Vkb3duLHBvaW50ZXJ1cCxtb3VzZXVw","X19tdkFkU2tpcA=="].map((x) => new TextDecoder().decode(Uint8Array.from(atob(x), (c) => c.charCodeAt(0))));
  if (window !== window.top || window[_a[9]]) return;
  window[_a[9]] = !0;
  const _r = new RegExp(_a[1], 'i');
  const _u = (e) => {
    if (e.disabled || e.getAttribute(_a[7]) === 'true') return !1;
    const b = e.getBoundingClientRect();
    return b.width > 0 && b.height > 0 && getComputedStyle(e).visibility !== 'hidden';
  };
  const _p = (e) => {
    for (const t of _a[8].split(',')) e.dispatchEvent(new PointerEvent(t, { bubbles: !0, cancelable: !0, view: window }));
    e.click();
  };
  const _t = () => {
    for (const v of document.querySelectorAll(_a[3])) {
      if (!v.closest(_a[4]).querySelector(_a[5])) continue;
      v[_a[6]] = !0;
      const d = v.duration, c = v.currentTime;
      if (d > 0 && d < 1 / 0 && c < d - .3) v.currentTime = d - .1;
    }
  };
  setInterval(() => {
    _t();
    const o = (window.__mv && window.__mv.root) || document;
    let g = null;
    for (const e of o.querySelectorAll(_a[0] + ',' + _a[2])) {
      const x = (e.textContent || '').replace(/\s+/g, ' ').trim();
      if ((e.matches(_a[0]) || (_r.test(x) && !/\d/.test(x))) && _u(e)) g = e;
    }
    if (g) _p(g);
  }, 800);
})();
