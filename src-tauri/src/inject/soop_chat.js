// 숲 채팅. 채팅 전용 페이지(?vtype=chat)는 플레이어가 window.open으로 여는 팝업이어야 동작한다
// (팝업이 window.opener의 방송 정보를 읽는다). 그래서 플레이어 페이지가 직접 팝업을 열고,
// 앱이 그 팝업 웹뷰를 받아 채팅 영역에 붙인다. 플레이어 페이지와 채팅 팝업 양쪽에 주입된다.
(() => {
  if (!/(^|\.)sooplive\.(com|co\.kr)$/.test(location.hostname)) return;

  if (new URLSearchParams(location.search).get('vtype') === 'chat') {
    // 채팅 팝업: 좁은 채팅 영역에서 오른쪽이 잘리지 않게 최소 너비를 푼다.
    // 숲 스타일이 더 강해서 스타일시트로는 안 되므로 요소에 인라인 !important로 건다.
    const fit = () => {
      for (const el of document.querySelectorAll('#chatting_area, #chatbox')) {
        el.style.setProperty('min-width', '0', 'important');
        el.style.setProperty('width', '100%', 'important');
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
