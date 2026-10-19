# Release 코드 보호와 검증

## 목적과 범위

기존 실행 성능과 기능을 우선하면서 배포 파일에서 읽을 수 있는 구현 정보를 줄인다. 대상은 Rust 네이티브 실행 파일, 컨트롤 바의 JavaScript, 방송 웹뷰에 주입되는 자체 JavaScript 8개다. 제3자 uBlock Origin Lite는 수정하지 않는다.

네이티브 코드와 브라우저에서 실행되는 JavaScript의 역공학을 완전히 막을 수는 없다. 이 구성은 심볼·디버그 정보·개발 PC 경로·주석·내부 JavaScript 식별자를 제거하고 최적화된 코드를 배포한다. Tauri IPC 이름, 외부 사이트 API, DOM 선택자, 설정 JSON 필드, 외부에서 호출하는 window 훅은 호환성을 위해 유지한다. 제어 흐름 가상화, 패커, 안티디버그 반복 검사, 문자열 런타임 복호화는 추가하지 않는다.

프레임워크의 오류 메시지용 Rust 타입 이름 같은 일부 문자열과 링커의 섹션 배치 정보(PE debug type 13)는 남을 수 있다. 이것을 전체 이름 난독화나 디컴파일 불가 보장으로 해석하면 안 된다.

## 적용 내용

| 영역 | 설정 | 성능과의 관계 |
|---|---|---|
| Rust | 기존 `opt-level="s"` 유지, fat LTO, codegen unit 1 | 기존 실행 코드 최적화 정책 유지. 수준 3 후보는 파일 크기 약 33% 증가에 비해 확실한 속도 이득이 없어 채택하지 않음 |
| 디버그 정보 | `debug=0`, `strip="symbols"`, MSVC `/DEBUG:NONE` | 디버그 심볼과 CodeView/PDB 참조 제거 |
| 링커 | `/OPT:REF,ICF`, `/INCREMENTAL:NO` | 미사용 코드 제거와 동일 코드 병합, 증분 링크 비활성화 |
| 오류 처리 | 기존 `panic="abort"` 유지 | 기존과 같은 패닉 동작 |
| 소스 경로 | `--remap-path-prefix` | 자체 Rust 파일은 고유한 `/r/N.rs`, 프로젝트·Cargo·도구 경로는 일반 경로로 치환. 실행 시 처리 없음 |
| JavaScript | Terser 5.51.2, 최상위·지역 이름 축약, 주석·공백 제거, `compress=false` | 원래 연산·분기·함수 구조 유지. 속성 이름 변경·소스맵·런타임 로더 없음 |
| 강제 적용 | `build.rs`에서 자산 생성 후 `include_str!` | 직접 Cargo로 컴파일하더라도 원본 주입 스크립트를 실수로 포함하지 않음 |
| 배포 | 허용된 실행 파일·확장·라이선스만 새 staging에 복사 | PDB, Rust 소스, 자체 JS 원본·소스맵을 배포하지 않음 |
| 검증 | 배포 스크립트에서 PE 검사 실패 시 중단 | 심볼·CodeView·자체 소스 경로·선정한 내부 JS 이름의 재유입 검사 |

ASLR, high-entropy ASLR, DEP는 기존에 활성화되어 있던 값을 유지하며 PE 헤더에서 확인한다. 이는 메모리 보호 기능이며 난독화 기능과는 구분한다.

## 재현

Windows x64에서 Rust stable, Tauri CLI 2, Node.js 22 이상, Python 3, MSVC 빌드 도구가 필요하다.

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/release.ps1
```

루트에서 실행한다. npm 잠금 파일을 이용한 설치, JavaScript 동등성 테스트, 잠긴 Rust 의존성으로 Release 빌드, PE 검증, NSIS 설치판과 포터블 생성, SHA-256 기록을 수행한다. CI도 같은 스크립트를 사용한다. 원격 업로드나 Git push는 이 스크립트에 포함하지 않는다.

서명 키 환경 변수가 있으면 기존 공개 키에 대응하는 자동 업데이트 서명도 생성한다. 키가 없으면 빌드별 설정 재정의로 `createUpdaterArtifacts=false`만 적용한다. 저장소의 공개 키와 업데이트 주소는 바꾸지 않는다. 이 경우 설치판·포터블을 직접 배포할 수 있으나 자동 업데이트용 `.sig`는 제공하지 않는다. Authenticode 서명과 Tauri 업데이트 서명은 별개다.

일반 `cargo tauri build`에도 JavaScript 보호와 Release 프로필이 적용된다. **소스 경로 치환과 배포 검사까지 포함하는 기준 명령은 `scripts/release.ps1`이다.** `RUSTFLAGS`를 별도로 설정한 환경에서는 안전한 인수 보존을 위해 `CARGO_ENCODED_RUSTFLAGS`로 옮긴 뒤 실행해야 한다.

```powershell
# 기능 테스트
npm ci --ignore-scripts
npm test
cargo test --locked --manifest-path src-tauri/Cargo.toml

# PE 검사
python scripts/audit-release.py artifacts/release/Moabom-0.2.1-portable/Moabom.exe --enforce

# A/B 성능 측정: 앱이 종료된 상태에서 실행
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/measure-release.ps1 `
  -Baseline artifacts/baseline/Moabom.exe `
  -Candidate artifacts/release/Moabom-0.2.1-portable/Moabom.exe

# 고정 영상 두 개로 비교 (테스트 환경에 PyAV, NumPy 필요)
python scripts/make-media-fixture.py
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/measure-release.ps1 `
  -Baseline artifacts/baseline/Moabom.exe `
  -Candidate artifacts/release/Moabom-0.2.1-portable/Moabom.exe `
  -Fixture -Rounds 3 -Seconds 20 -WarmupSeconds 2 `
  -Output artifacts/performance-fixture-final.json
```

측정 스크립트는 기존 설정 파일을 바이트 단위로 보관하고 `finally`에서 복원한다. 기존 프로필의 쿠키·로그인 데이터는 지우지 않는다. 활성 앱이 있으면 실행하지 않는다. `-Video`는 음량 0, 저화질(360p 목표)의 공개 YouTube 영상 페이지 두 개를 세션 복원으로 연다. 자동 화질에 따른 부하 변동을 줄이기 위한 설정이며, 네트워크·사이트 상태에 따라 실제 영상 재생 여부를 별도로 확인해야 한다.

시작 시간은 프로세스 생성부터 UI Automation에서 입력 가능한 검색창이 발견될 때까지다. CPU·working set·private memory는 앱과 워밍업 시점의 모든 WebView2 자식 프로세스를 합산한다. CPU는 한 논리 코어를 100%로 표시한다. 응답 시간은 `WM_NULL` 왕복이며 JavaScript 렌더링 지연 전체를 나타내지는 않는다. A/B 순서를 매 라운드 뒤집는다. 샘플 수가 적은 로컬 비교는 모든 PC·방송·GPU 환경의 성능 보증이 아니다.

자식 판정에는 PID와 부모 PID뿐 아니라 생성 시각을 사용한다. CPU는 각 프로세스의 샘플 간 누적 CPU 시간 증가분을 합산해, PID 재사용에 따른 다른 앱 혼입과 프로세스 종료에 따른 누적값 감소를 피한다. 초기 도구 검증 중 발견한 잘못된 합산 수치와 재생 대기 중 YouTube 수치는 최종 비교에 사용하지 않는다.

별도로 매 실행에서 포커스/격자 전환 6회를 UI Automation으로 누르고, 설정 JSON에 레이아웃이 반영될 때까지 측정한다. 이 수치는 JavaScript 이벤트와 IPC, Rust 처리, 디스크 기록을 포함한다. 영상 테스트는 재생 슬라이더 값을 전후에 수집해 실제 재생 진행 여부도 기록한다.

`-Fixture`는 외부 사이트의 버퍼링·광고·자동 화질·초기 로딩 편차를 없애기 위한 별도 비교다. 직접 생성한 동일한 VP8 영상(1280×720, 30fps, 8초, 240프레임)을 실제 Release의 스트림 웹뷰 두 개에서 반복 재생한다. 원본과 보호 빌드 모두 같은 파일을 사용한다. 앱의 원래 초기화 스크립트가 실행됐는지 확인하고 재생 프레임 증가·프레임 드롭도 기록한다. 창은 같은 위치·크기로 앞에 표시해 모니터·화면 가림 차이를 줄인다.

이 테스트 실행에만 자식 프로세스 환경 변수 `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=0`을 전달하고, 로컬 CDP 연결로 테스트 영상을 넣는다. 측정 전후에만 진단 값을 읽고 연결을 닫는다. 배포 바이너리나 설정에 디버깅 옵션을 추가하지 않으며 환경 변수도 복원한다. 원본·보호 빌드에 공통으로 적용되므로 **동일 조건에서의 보호 처리 추가 비용 비교**에 사용한다. 외부 플랫폼·광고·P2P·로그인·모든 코덱의 실제 방송 부하를 대체하는 검증은 아니다. [Microsoft의 WebView2 진단 연결 설명](https://learn.microsoft.com/en-us/microsoft-edge/webview2/how-to/debug-visual-studio-code)을 따른다.

테스트 영상 생성에는 PyAV 19.0.1, NumPy 2.3.3을 사용했다. 이 도구와 영상은 배포 파일에 포함하지 않는다. JSON 결과는 `artifacts/`에 기록하며, 최종 수치와 검증 범위는 아래 결과 절에 기록한다.

## 2026-10-20 최종 검증 결과

기준은 변경 전 커밋 `ea704cb`를 별도 worktree에서 `cargo tauri build --no-bundle -- --locked`로 만든 실행 파일이다. 최종 배포 파일은 기존 Rust 최적화 수준 `s`와 JavaScript 실행 구조를 유지하는 구성이다. 아래 수치는 최종 파일 해시에 연결된 측정만 사용한다. 원자료와 세부 해시는 [release-validation.json](release-validation.json)에 정리했다.

- Rust 테스트 **16개**, JavaScript 동작·실행 구조 비교 테스트 **4개** 통과.
- 보호된 주입 스크립트 **8개**가 최종 실행 파일에 정확히 포함됐는지 확인.
- 정상 실행 환경에서 디버깅 인수 없이 UI·설정·버전 표시와 실제 YouTube 영상 재생을 확인. 두 시점의 재생 화면을 저장했다.
- UI의 레이아웃 명령이 JavaScript → IPC → Rust → 설정 저장까지 왕복하는 것을 반복 확인.
- 최종 EXE **5,156,864 bytes**, 기준 EXE **5,192,704 bytes**: 약 **0.69% 감소**.
- CodeView 참조 제거, COFF 심볼·export·디버그 섹션 0, 검사 대상 소스 경로 문자열 **365 → 0**, 검사 대상 내부 JS 함수명 **4 → 0**. ASLR·high-entropy ASLR·DEP 유지.
- NSIS 내부 EXE와 포터블 EXE는 Tauri 배포 유형 표식만 다름을 확인. 확장 파일 **924개** 원본 일치, 포터블의 PDB·소스맵·Rust 소스 제외 및 라이선스 포함 확인.
- 테스트 종료 후 앱 종료, 기존 설정 파일의 바이트 단위 복원, 진단 포트 listener 0개 확인.

환경: Windows 11 Home 10.0.26200, i5-14600KF, 논리 프로세서 20개, RAM 32GiB, WebView2 154.0.4258.62. 각 조건에서 원본·보호 빌드를 각각 3회 실행했고 순서를 번갈아 배치했다. 메모리는 프로세스별 private/working set 합산을 실행별 평균한 뒤 중앙값을 냈다. CPU는 20개 논리 프로세서 전체에 대해 정규화한 값이다.

| 조건 / 지표 | 원본 | 최종 보호 빌드 |
|---|---:|---:|
| 빈 창: 시작 시간 중앙값 | 425.2ms | 458.5ms |
| 빈 창: UI 명령 왕복 중앙값 | 34.8ms | 39.6ms |
| 빈 창: private 메모리 | 163.37MiB | 162.26MiB |
| 빈 창: working set 합계 | 354.07MiB | 354.37MiB |
| 빈 창: CPU 중앙값 | 0.000% | 0.000% |
| 영상 2개: 시작 시간 중앙값 | 494.0ms | 424.9ms |
| 영상 2개: UI 명령 왕복 중앙값 | 51.1ms | 48.5ms |
| 영상 2개: private 메모리 | 723.90MiB | 723.93MiB |
| 영상 2개: working set 합계 | 1,340.67MiB | 1,346.69MiB |
| 영상 2개: CPU 중앙값 | 0.573% | 0.694% |
| 영상 2개: CPU 실행별 범위 | 0.536–0.883% | 0.606–1.198% |
| 영상 2개: 재생 프레임률 중앙값 | 29.962fps | 29.971fps |
| 영상 2개: 측정 구간 추가 프레임 드롭 | 0 | 0 |

프레임률·메모리는 동등한 수준이고, UI 왕복 차이는 수 ms였다. 시작 시간은 빈 창에서 +33.3ms, 세션 복원에서 −69.1ms로 방향이 달랐다. 영상 부하의 전체 CPU 중앙값은 **+0.122%p**였으며 반복 측정 범위는 서로 겹쳤다. 이를 0% 오버헤드 보장으로 표현하지 않는다. 최종 구성은 Rust 최적화 정책과 JS 연산·분기 구조를 유지하고, 런타임 보호 연산을 추가하지 않는 쪽으로 선택했다.

실제 모든 플랫폼의 로그인·P2P·채팅·알림·모든 코덱 조합을 전수 검증한 결과는 아니다. NSIS는 생성·내부 payload 검증을 했고, 설치/삭제 마법사의 전체 왕복은 수행하지 않았다. Authenticode는 `NotSigned`이며, 현재 개인 키가 없어 자동 업데이트용 `.sig`도 생성하지 않았다. 기존 공개 키와 업데이트 주소는 유지했다.

최종 배포 파일은 `artifacts/release/moabom_0.2.1_x64-setup.exe`, `artifacts/release/moabom_0.2.1_x64-portable.zip`이며, SHA-256은 같은 폴더의 `SHA256SUMS.txt`와 검증 JSON에 기록했다. 원격 게시와 Git push는 수행하지 않았다.

## 근거 문서

- [Cargo Release 프로필](https://doc.rust-lang.org/cargo/reference/profiles.html)
- [Rust 소스 경로 치환](https://doc.rust-lang.org/rustc/remap-source-paths.html)
- [MSVC /DEBUG](https://learn.microsoft.com/en-us/cpp/build/reference/debug-generate-debug-info?view=msvc-170), [링커 최적화](https://learn.microsoft.com/en-us/cpp/build/reference/opt-optimizations?view=msvc-170)
- [Terser 옵션](https://github.com/terser/terser)
