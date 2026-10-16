# 제3자 구성요소

## 브라우저 확장 (`src-tauri/extensions/`)

타일 웹뷰(WebView2)에 불러오는 확장입니다. 하위 폴더 하나가 확장 하나이며, WebView2가 폴더 안 항목을 모두 확장으로 불러오므로 `extensions/`에는 확장 폴더 외의 파일을 두지 않습니다.

### ubol — uBlock Origin Lite

- 출처: https://github.com/uBlockOrigin/uBOL-home 릴리스 [`2026.1006.1931`](https://github.com/uBlockOrigin/uBOL-home/releases/tag/2026.1006.1931) (`uBOLite_2026.1006.1931.edge.zip`)
- 소스:
  - 배포 파일 그대로의 소스: https://github.com/uBlockOrigin/uBOL-home/tree/2026.1006.1931
  - 빌드 스크립트와 원본 소스: https://github.com/gorhill/uBlock/tree/b257eeb7c344028cca8c2a248508d6971ec12953/platform/mv3 (빌드 시점인 2026-10-06의 마지막 커밋)
- 라이선스: GPLv3 (`src-tauri/extensions/ubol/LICENSE.txt`). 이 확장은 프로그램과 별개의 저작물이며, 이 저장소의 LICENSE가 아닌 GPLv3를 따릅니다. GPLv3에 따라 누구나 이 확장을 수정하고 재배포할 수 있습니다.
- 수정 사항 (2026-10-12): `manifest.json`에서 한국어 필터 목록(`kor-1`)을 기본 사용으로 변경. 그 외는 원본 그대로.
