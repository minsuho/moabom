//! Windows 알림. 알림에 "모아봄" 이름과 아이콘이 보이도록 알림 ID(AppUserModelID)를 직접 등록하고 그 ID로 띄운다.
//! 알림 플러그인은 빌드 폴더에서 실행하면 PowerShell 이름으로 띄우고, 시작 메뉴 바로 가기가 없는 포터블에서는
//! 등록되지 않은 ID를 써서 알림이 아예 뜨지 않을 수 있어 직접 처리한다.

use tauri::{AppHandle, Manager};

const ICON_PNG: &[u8] = include_bytes!("../icons/icon.png");

/// 알림 ID 등록: HKCU\Software\Classes\AppUserModelId\<identifier>에 이름과 아이콘 경로를 적는다.
/// 관리자 권한 없이 현재 사용자 영역에만 쓴다. 시작할 때마다 불러 아이콘 위치가 바뀌어도 맞춘다.
#[cfg(windows)]
pub fn register(app: &AppHandle) {
    let icon = match app.path().app_local_data_dir() {
        Ok(dir) => dir.join("notify-icon.png"),
        Err(e) => {
            eprintln!("[moabom] 알림 아이콘 위치를 찾지 못했습니다: {e}");
            return;
        }
    };
    if std::fs::read(&icon).ok().as_deref() != Some(ICON_PNG) {
        let written = icon.parent().map_or(Ok(()), std::fs::create_dir_all).and_then(|_| std::fs::write(&icon, ICON_PNG));
        if let Err(e) = written {
            eprintln!("[moabom] 알림 아이콘을 저장하지 못했습니다: {e}");
        }
    }
    let key = format!(r"Software\Classes\AppUserModelId\{}", app.config().identifier);
    for (name, value) in [("DisplayName", crate::APP_NAME.to_string()), ("IconUri", icon.display().to_string())] {
        if let Err(e) = set_reg_string(&key, name, &value) {
            eprintln!("[moabom] 알림 등록 실패({name}): {e}");
        }
    }
}

#[cfg(windows)]
pub(crate) fn set_reg_string(key: &str, name: &str, value: &str) -> Result<(), String> {
    use windows::Win32::System::Registry::{RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ};
    use windows_core::HSTRING;
    let wide: Vec<u16> = value.encode_utf16().chain([0]).collect();
    let bytes = wide.len() * 2;
    // 하위 키가 없으면 함께 만든다.
    let err = unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            &HSTRING::from(key),
            &HSTRING::from(name),
            REG_SZ.0,
            Some(wide.as_ptr().cast()),
            bytes as u32,
        )
    };
    if err.is_ok() { Ok(()) } else { Err(format!("{err:?}")) }
}

#[cfg(not(windows))]
pub fn register(_app: &AppHandle) {}

/// 알림을 띄운다. 실패해도 앱 동작에는 영향이 없으므로 로그만 남긴다.
pub fn show(app: &AppHandle, title: &str, body: &str) {
    #[cfg(windows)]
    {
        let result = tauri_winrt_notification::Toast::new(&app.config().identifier).title(title).text1(body).show();
        if let Err(e) = result {
            eprintln!("[moabom] 알림을 띄우지 못했습니다: {e}");
        }
    }
    #[cfg(not(windows))]
    let _ = (app, title, body);
}
