// 치지직 고화질 모드.
// WebView2(Edge 계열)로 인식되면 치지직은 720p 이상을 P2P 확장 프로그램 전용으로 막는데,
// WebView2는 확장의 네이티브 메시징을 지원하지 않아 공식 커넥터를 쓸 수 없다.
// 페이지가 보는 브라우저 이름만 Opera로 바꾸면 고화질 트랙이 CDN으로 재생된다. (HTTP 헤더는 그대로)
(() => {
  if (!/(^|\.)chzzk\.naver\.com$/.test(location.hostname)) return;
  const desc = Object.getOwnPropertyDescriptor(Navigator.prototype, 'userAgent');
  const ua = desc.get.call(navigator).replace(/Edg\/([\d.]+)/, 'OPR/$1');
  Object.defineProperty(Navigator.prototype, 'userAgent', { get: () => ua, configurable: true });
  // 시작 화질은 화질 제어 스크립트(quality.js)가 정한다.
})();
