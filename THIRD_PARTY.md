# 제3자 구성요소

## 브라우저 확장 (`src-tauri/extensions/`)

타일 웹뷰(WebView2)에 불러오는 확장입니다. 하위 폴더 하나가 확장 하나이며, WebView2가 폴더 안 항목을 모두 확장으로 불러오므로 `extensions/`에는 확장 폴더 외의 파일을 두지 않습니다.

### ubol — uBlock Origin Lite

- 출처: https://github.com/uBlockOrigin/uBOL-home (릴리스 `2026.1006.1931`, `uBOLite_2026.1006.1931.edge.zip`)
- 소스: https://github.com/gorhill/uBlock (`platform/mv3`)
- 라이선스: GPLv3 (`src-tauri/extensions/ubol/LICENSE.txt`). 이 확장은 프로그램과 별개의 저작물이며, 이 저장소의 LICENSE가 아닌 GPLv3를 따릅니다.
- 수정 사항: `manifest.json`에서 한국어 필터 목록(`kor-1`)을 기본 사용으로 변경. 그 외는 원본 그대로.

