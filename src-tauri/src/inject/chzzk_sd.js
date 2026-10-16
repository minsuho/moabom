// 치지직 기본 모드: 고화질 확장 프로그램 안내가 뜨면 "설치없이 일반 화질 시청"을 대신 누른다.
(() => {
  if (!/(^|\.)chzzk\.naver\.com$/.test(location.hostname)) return;
  setInterval(() => {
    for (const b of document.querySelectorAll('button')) {
      if (/^설치\s*없이 (일반|기본) 화질 시청$/.test(b.textContent.trim())) b.click();
    }
  }, 1000);
})();
