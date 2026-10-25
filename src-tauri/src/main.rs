#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod link;
mod live;
mod notify;
mod platforms;
mod search;
mod store;
mod tiles;
mod update;

use tauri::{webview::Color, LogicalPosition, LogicalSize, Manager, Theme, WebviewBuilder, WebviewUrl, WindowEvent};

pub const APP_NAME: &str = "모아봄";
const REPORT_EMAIL: &str = "gitter0001@gmail.com";

#[tauri::command]
fn app_version(app: tauri::AppHandle) -> String {
    app.package_info().version.to_string()
}

/// 오류 제보 메일 쓰기 창을 연다. 버전과 적어 줄 내용을 미리 채운다.
#[tauri::command]
fn open_report(app: tauri::AppHandle) -> Result<(), String> {
    let version = app.package_info().version.to_string();
    let enc = |s: &str| url::form_urlencoded::byte_serialize(s.as_bytes()).collect::<String>().replace('+', "%20");
    let subject = format!("[{APP_NAME} v{version}] 오류 제보");
    let body = format!(
        "모아봄 버전: v{version}\r\nWindows 버전: \r\n플랫폼과 방송(예: 치지직 OOO): \r\n\r\n\
         무엇을 했더니 어떻게 됐나요?\r\n\r\n\r\n(스크린샷이 있으면 첨부해 주세요)\r\n"
    );
    let url = format!("mailto:{REPORT_EMAIL}?subject={}&body={}", enc(&subject), enc(&body));
    open_url(&url).map_err(|e| format!("메일 앱을 열지 못했습니다: {e}"))
}

/// 주소를 기본 프로그램(브라우저, 메일 앱)으로 연다.
/// cmd의 start는 &를 명령 구분자로 해석하므로, 셸을 거치지 않는 방식으로 연다.
pub fn open_url(url: &str) -> Result<(), String> {
    std::process::Command::new("rundll32")
        .args(["url.dll,FileProtocolHandler", url])
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

fn main() {
    tauri::Builder::default()
        // 이미 실행 중이면 새로 띄우지 않고 기존 창을 앞으로 가져온다(같은 WebView2 데이터 폴더를 두 번 쓸 수 없다).
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            if let Some(main) = app.get_window(tiles::MAIN) {
                let _ = main.unminimize();
                let _ = main.set_focus();
            }
            // 켜져 있을 때 지통실 링크를 누르면 새 실행이 인자를 넘겨준다
            if let Some(l) = link::from_args(args) {
                link::deliver(app, l);
            }
        }))
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(tiles::Tiles::default())
        .manage(link::PendingLink::default())
        .setup(|app| {
            let main = tauri::window::WindowBuilder::new(app, tiles::MAIN)
                .title(APP_NAME)
                .inner_size(1280.0, 800.0)
                .min_inner_size(640.0, 400.0)
                .background_color(Color(10, 10, 12, 255))
                .theme(Some(Theme::Dark))
                .build()?;
            let mut ui = WebviewBuilder::new(tiles::UI, WebviewUrl::App("index.html".into()));
            // 개발용: MOABOM_DEBUG=1 이면 컨트롤 바도 원격 디버깅 포트(9230)를 연다.
            if cfg!(debug_assertions) && std::env::var_os("MOABOM_DEBUG").is_some() {
                ui = ui.additional_browser_args(
                    "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection --remote-debugging-port=9230",
                );
            }
            main.add_child(
                ui,
                LogicalPosition::new(0.0, 0.0),
                LogicalSize::new(1280.0, tiles::BAR_H),
            )?;

            let handle = app.handle().clone();
            main.on_window_event(move |e| match e {
                WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => tiles::relayout(&handle),
                // 메인 창을 닫으면 분리된 창까지 모두 종료. 격자 창 위치도 이때 저장한다.
                WindowEvent::CloseRequested { .. } => {
                    tiles::save_now(&handle);
                    handle.exit(0);
                }
                _ => {}
            });
            tiles::relayout(app.handle());
            tiles::watch_live(app.handle());
            notify::register(app.handle());
            // 지통실 링크(moabom://): 이 실행 파일로 연결하고, 링크로 켜졌으면 컨트롤 바에 넘긴다
            link::register();
            if let Some(l) = link::from_args(std::env::args()) {
                link::deliver(app.handle(), l);
            }
            // 개발용 MOABOM_OPEN으로 띄울 때는 지난 방송을 다시 열지 않는다(설정만 불러온다).
            tiles::restore_session(app.handle(), std::env::var_os("MOABOM_OPEN").is_none());

            // 개발용: MOABOM_OPEN="soop:a,soop:b" 로 시작하면 타일을 바로 연다.
            if let Ok(list) = std::env::var("MOABOM_OPEN") {
                let handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    for input in list.split(',').map(str::trim).filter(|s| !s.is_empty()) {
                        if let Err(e) = tiles::add_tile(handle.clone(), input.to_string()).await {
                            eprintln!("{input}: {e}");
                        }
                    }
                });
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            tiles::list_tiles,
            tiles::add_tile,
            tiles::command,
            tiles::show_chat,
            tiles::set_layout,
            tiles::set_panel,
            tiles::set_split_soop,
            tiles::set_chzzk_hq,
            tiles::set_quality,
            tiles::set_restore,
            tiles::set_notify,
            tiles::save_group,
            tiles::delete_group,
            tiles::open_group,
            update::check_update,
            update::install_update,
            search::search,
            search::soop_exists,
            app_version,
            open_report,
            link::take_link,
            link::open_link,
        ])
        .run(tauri::generate_context!())
        .expect("failed to run app");
}
