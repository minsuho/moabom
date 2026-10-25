; NSIS 설치 관리자 훅(tauri.conf.json의 bundle.windows.nsis.installerHooks).
; productName(moabom)은 설치 폴더·레지스트리 키·업데이트 경로에 쓰이므로 영문 그대로 두고,
; 사람이 보는 이름(시작 메뉴·바탕 화면 바로 가기, 설치된 앱 목록)만 한글로 바꾼다.
; 바로 가기 이름이 영문이면 Windows 검색에서 "모아봄"으로 찾을 수 없다.
!define MOABOM_DISPLAY_NAME "모아봄"

; DIR 안의 영문 이름 바로 가기를 한글 이름으로 바꾼다. 바꾸기만 하므로 대상·아이콘·알림용 앱 ID는 그대로다.
!macro MOABOM_RENAME_LNK DIR
  ${If} ${FileExists} "${DIR}\${PRODUCTNAME}.lnk"
    Delete "${DIR}\${MOABOM_DISPLAY_NAME}.lnk"
    Rename "${DIR}\${PRODUCTNAME}.lnk" "${DIR}\${MOABOM_DISPLAY_NAME}.lnk"
  ${EndIf}
!macroend

!macro NSIS_HOOK_POSTINSTALL
  ; 업데이트 설치는 바로 가기를 새로 만들지 않으므로, 예전 버전이 만든 영문 바로 가기도 여기서 바뀐다.
  !insertmacro MOABOM_RENAME_LNK "$SMPROGRAMS"
  !insertmacro MOABOM_RENAME_LNK "$DESKTOP"
  WriteRegStr SHCTX "${UNINSTKEY}" "DisplayName" "${MOABOM_DISPLAY_NAME}"
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  ; 기본 제거 과정은 영문 이름 바로 가기만 지우므로 한글 이름 바로 가기도 지운다(업데이트 중에는 남긴다).
  ${If} $UpdateMode <> 1
    Delete "$SMPROGRAMS\${MOABOM_DISPLAY_NAME}.lnk"
    Delete "$DESKTOP\${MOABOM_DISPLAY_NAME}.lnk"
  ${EndIf}
!macroend
