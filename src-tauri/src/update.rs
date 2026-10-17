//! 새 버전 확인과 설치. 설치판은 내려받아 바로 설치하고, 포터블은 릴리스 페이지를 연다.

use serde::Serialize;
use tauri::AppHandle;
use tauri_plugin_updater::UpdaterExt;

const RELEASES: &str = "https://github.com/minsuho/moabom/releases/latest";

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    version: String,
    notes: Option<String>,
    /// 포터블이면 자동 설치 대신 릴리스 페이지를 연다.
    portable: bool,
}

/// 설치판은 실행 파일 옆에 제거 프로그램이 있다. 없으면 압축을 풀어 쓰는 포터블로 본다.
fn portable() -> bool {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| !dir.join("uninstall.exe").exists()))
        .unwrap_or(true)
}

async fn find(app: &AppHandle) -> Result<Option<tauri_plugin_updater::Update>, String> {
    let updater = app.updater().map_err(|e| e.to_string())?;
    updater.check().await.map_err(|e| {
        eprintln!("[moabom] 업데이트 확인 실패: {e}");
        "업데이트 정보를 받지 못했어요. 인터넷 연결을 확인하고 잠시 뒤에 다시 눌러 주세요.".to_string()
    })
}

/// 새 버전이 있으면 그 정보를, 이미 최신이면 `None`을 돌려준다.
#[tauri::command]
pub async fn check_update(app: AppHandle) -> Result<Option<UpdateInfo>, String> {
    Ok(find(&app).await?.map(|u| UpdateInfo { version: u.version.clone(), notes: u.body.clone(), portable: portable() }))
}

/// 새 버전을 내려받아 설치하고 다시 실행한다. 설치하는 동안 앱은 꺼진다.
#[tauri::command]
pub async fn install_update(app: AppHandle) -> Result<(), String> {
    if portable() {
        return crate::open_url(RELEASES);
    }
    let update = find(&app).await?.ok_or("이미 최신 버전입니다")?;
    update
        .download_and_install(|_, _| {}, || {})
        .await
        .map_err(|e| format!("업데이트를 설치하지 못했습니다: {e}"))?;
    app.restart();
}
