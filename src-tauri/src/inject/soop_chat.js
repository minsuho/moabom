// 숲 채팅. 채팅 전용 페이지(?vtype=chat)는 플레이어가 window.open으로 여는 팝업이어야 동작한다
// (팝업이 window.opener의 방송 정보를 읽는다). 그래서 플레이어 페이지가 직접 팝업을 열고,
// 앱이 그 팝업 웹뷰를 받아 채팅 영역에 붙인다. 플레이어 페이지와 채팅 팝업 양쪽에 주입된다.
(() => {
  if (!/(^|\.)sooplive\.(com|co\.kr)$/.test(location.hostname)) return;

  if (new URLSearchParams(location.search).get('vtype') === 'chat') {
    // 채팅 팝업: 좁은 채팅 영역에서 오른쪽이 잘리지 않게 최소 너비를 푼다.
    // 숲 스타일이 더 강해서 스타일시트로는 안 되므로 요소에 인라인 !important로 건다.
    const set = (el, props) => {
      for (const [k, v] of Object.entries(props)) el.style.setProperty(k, v, 'important');
    };
    const fit = () => {
      for (const el of document.querySelectorAll('#chatting_area, #chatbox')) set(el, { 'min-width': '0', width: '100%' });
      // 채팅 위에 뜨는 창(참여 인원, 채팅 관리 등)은 폭이 360px로 고정돼 있어 좁은 채팅 영역에서 왼쪽이 잘린다.
      // 오른쪽에 붙는 창은 양옆 여백만 두고 채팅 폭에 맞추고, 가운데 뜨는 창(선물·후원 등)은 최대 폭만 줄인다.
      const max = innerWidth - 16;
      for (const el of document.querySelectorAll('.chat_layer')) {
        if (el.offsetWidth > max) set(el, { left: '8px', right: '8px', width: 'auto', 'min-width': '0' });
      }
      for (const el of document.querySelectorAll('[class*="_layer"], .modal-layer')) {
        if (!el.classList.contains('chat_layer') && el.offsetWidth > max) set(el, { 'max-width': `${max}px`, 'min-width': '0' });
      }
    };
    document.addEventListener('DOMContentLoaded', fit);
    setInterval(fit, 1000);
    return;
  }

  window.__mvOpenChat = () => {
    const id = location.pathname.split('/').filter(Boolean)[0];
    if (id) window.open(`https://play.sooplive.com/${id}?vtype=chat`, `mv_chat_${id}`, 'width=340,height=640');
  };
})();
