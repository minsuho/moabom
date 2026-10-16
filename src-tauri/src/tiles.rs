//! 타일(방송 하나 = 웹뷰 하나) 관리: 생성/삭제, 격자 배치, 창 분리, 채팅, 음소거, 단축키 명령.

use std::sync::{Mutex, OnceLock};

use serde::Serialize;
use tauri::{
    webview::{Color, NewWindowFeatures, NewWindowResponse, PageLoadEvent},
    AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, Url, Webview, WebviewBuilder,
    WebviewUrl, WebviewWindowBuilder, Window, WindowEvent, Wry,
};

use crate::platforms::{self, Platform};

pub const MAIN: &str = "main";
pub const UI: &str = "ui";
pub const BAR_H: f64 = 52.0;
const CHAT_W: f64 = 340.0;
const SOOP_PER_PROFILE: usize = 3;
/// 킥은 화면 폭이 이보다 좁으면 플레이어 조작 바(화질 메뉴 포함)를 만들지 않는다.
const KICK_MIN_WIDTH: f64 = 1024.0;
const BROWSER_ARGS: &str = "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection \
                            --autoplay-policy=no-user-gesture-required";
const VIDEO_ONLY_JS: &str = include_str!("inject/video_only.js");
const CHZZK_HQ_JS: &str = include_str!("inject/chzzk_hq.js");
const CHZZK_SD_JS: &str = include_str!("inject/chzzk_sd.js");
const SOOP_CHAT_JS: &str = include_str!("inject/soop_chat.js");
const WATCHDOG_JS: &str = include_str!("inject/watchdog.js");
const OVERLAY_JS: &str = include_str!("inject/overlay.js");
const ASSIST_JS: &str = include_str!("inject/assist.js");
const QUALITY_JS: &str = include_str!("inject/quality.js");

/// 웹뷰에 주입하는 스크립트. `all_frames`면 iframe 안에서도 실행된다.
#[derive(Clone)]
struct Script {
    source: String,
    all_frames: bool,
}

fn main_frame(source: &str) -> Script {
    Script { source: source.to_string(), all_frames: false }
}

/// 타일 오버레이가 앱에 보내는 메시지에 붙는 토큰. 실행마다 새로 만들어 사이트 스크립트가 흉내 내지 못하게 한다.
fn token() -> &'static str {
    static TOKEN: OnceLock<String> = OnceLock::new();
    TOKEN.get_or_init(|| {
        use std::hash::{BuildHasher, Hasher};
        let part = || std::collections::hash_map::RandomState::new().build_hasher().finish();
        format!("{:016x}{:016x}", part(), part())
    })
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Tile {
    id: u32,
    platform: Platform,
    channel: String,
    url: String,
    chat_url: Option<String>,
    profile: String,
    detached: bool,
    chat_open: bool,
    video_only: bool,
    /// 페이지에서 읽은 방송인 이름과 방송 제목
    name: Option<String>,
    title: Option<String>,
    /// 채팅 웹뷰가 숲 팝업에서 온 것인지(Tauri가 아닌 WebView2로 직접 옮겨야 한다)
    #[serde(skip)]
    popup_chat: bool,
    /// 이 방송만 따로 정한 화질. 없으면 전체 화질을 따른다.
    quality: Option<Quality>,
    /// 페이지 로드가 한 번이라도 끝났는지(처음 이동이 걸려 멈추는 경우를 감지하는 데 쓴다)
    #[serde(skip)]
    loaded: bool,
    /// 방송 중인지. 확인하지 못했으면 None
    live: Option<bool>,
    /// 방송인 프로필 사진(종료 화면에 쓴다)
    #[serde(skip)]
    image: Option<String>,
}

/// 화질 모드. 플랫폼별 적용은 inject/quality.js가 맡는다.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Quality {
    /// 최고화질
    Best,
    /// 네트워크 최적화(플랫폼 자동 화질)
    Auto,
    /// 저화질
    Low,
}

impl Quality {
    fn as_str(self) -> &'static str {
        match self {
            Quality::Best => "best",
            Quality::Auto => "auto",
            Quality::Low => "low",
        }
    }

    fn next(self) -> Self {
        match self {
            Quality::Best => Quality::Auto,
            Quality::Auto => Quality::Low,
            Quality::Low => Quality::Best,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Layout {
    Grid,
    Focus,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    tiles: Vec<Tile>,
    selected: Option<u32>,
    muted: bool,
    chat_shown: Option<u32>,
    layout: Layout,
    fullscreen: bool,
    split_soop: bool,
    chzzk_hq: bool,
    quality: Quality,
}

pub struct Tiles(Mutex<Inner>);

struct Inner {
    tiles: Vec<Tile>,
    next_id: u32,
    /// 소리와 단축키의 대상이 되는 타일
    selected: Option<u32>,
    /// 선택한 타일까지 포함해 전부 음소거
    muted: bool,
    /// 메인 창 오른쪽 채팅 영역에 보이는 채팅(한 번에 하나)
    chat_shown: Option<u32>,
    layout: Layout,
    fullscreen: bool,
    /// 검색창 아래에 펼친 검색 결과 패널(창 기준 좌표). 방송 위에 겹쳐 그린다.
    panel: Option<Rect>,
    /// 마지막 배치에서 메인 창에 붙은 타일들의 위치(창 기준). 끌어서 옮길 때 놓을 곳을 찾는 데 쓴다.
    rects: Vec<(u32, Rect)>,
    /// 끌고 있는 타일과, 지금 마우스 아래에 있는(놓으면 자리를 바꿀) 타일
    drag: Option<(u32, Option<u32>)>,
    split_soop: bool,
    chzzk_hq: bool,
    /// 모든 방송에 적용하는 기본 화질(방송별로 따로 정할 수 있다)
    quality: Quality,
}

impl Default for Tiles {
    fn default() -> Self {
        Tiles(Mutex::new(Inner {
            tiles: Vec::new(),
            next_id: 0,
            selected: None,
            muted: false,
            chat_shown: None,
            layout: Layout::Grid,
            fullscreen: false,
            panel: None,
            rects: Vec::new(),
            drag: None,
            split_soop: true,
            chzzk_hq: true,
            quality: Quality::Auto,
        }))
    }
}

impl Inner {
    fn snapshot(&self) -> Snapshot {
        Snapshot {
            tiles: self.tiles.clone(),
            selected: self.selected,
            muted: self.muted,
            chat_shown: self.chat_shown,
            layout: self.layout,
            fullscreen: self.fullscreen,
            split_soop: self.split_soop,
            chzzk_hq: self.chzzk_hq,
            quality: self.quality,
        }
    }

    fn get(&self, id: u32) -> Result<&Tile, String> {
        self.tiles.iter().find(|t| t.id == id).ok_or_else(|| "타일이 없습니다".to_string())
    }

    fn get_mut(&mut self, id: u32) -> Result<&mut Tile, String> {
        self.tiles.iter_mut().find(|t| t.id == id).ok_or_else(|| "타일이 없습니다".to_string())
    }

    /// 숲은 저장 공간(쿠키·사이트 데이터) 하나당 3개까지만 볼 수 있으므로 3개씩 묶어 따로 배정한다.
    fn assign_profile(&self, platform: Platform) -> String {
        if platform != Platform::Soop || !self.split_soop {
            return "default".into();
        }
        (1..)
            .map(|n| format!("soop-{n}"))
            .find(|name| self.tiles.iter().filter(|t| &t.profile == name).count() < SOOP_PER_PROFILE)
            .unwrap()
    }

    /// 타일 웹뷰에 주입할 스크립트. 플랫폼별 보정은 영상만 보기 스크립트보다 먼저 실행된다.
    fn tile_scripts(&self, platform: Platform, quality: Quality) -> Vec<Script> {
        let mut scripts = Vec::new();
        match platform {
            Platform::Chzzk => scripts.push(main_frame(if self.chzzk_hq { CHZZK_HQ_JS } else { CHZZK_SD_JS })),
            Platform::Soop => scripts.push(main_frame(SOOP_CHAT_JS)),
            _ => {}
        }
        scripts.push(main_frame(VIDEO_ONLY_JS));
        scripts.push(main_frame(&OVERLAY_JS.replace("__MV_TOKEN__", token())));
        scripts.push(main_frame(ASSIST_JS));
        scripts.push(main_frame(&QUALITY_JS.replace("__MV_QUALITY__", quality.as_str())));
        scripts.push(main_frame(WATCHDOG_JS));
        scripts
    }

    /// 방송에 실제로 적용되는 화질.
    fn quality_of(&self, t: &Tile) -> Quality {
        t.quality.unwrap_or(self.quality)
    }

    /// 단축키가 향할 타일: 선택한 타일, 없으면 첫 번째 타일.
    fn target(&self) -> Option<u32> {
        self.selected.or_else(|| self.tiles.first().map(|t| t.id))
    }
}

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

fn tile_label(id: u32) -> String {
    format!("tile-{id}")
}
fn chat_label(id: u32) -> String {
    format!("chat-{id}")
}
fn win_label(id: u32) -> String {
    format!("win-{id}")
}
fn tile_id(label: &str) -> Option<u32> {
    label.strip_prefix("tile-")?.parse().ok()
}

fn build_webview(
    app: &AppHandle,
    label: &str,
    url: &str,
    profile: &str,
    scripts: &[Script],
) -> Result<WebviewBuilder<Wry>, String> {
    let dir = app.path().app_local_data_dir().map_err(err)?.join("profiles").join(profile);
    let mut args = BROWSER_ARGS.to_string();
    // 개발용: MOABOM_DEBUG=1 이면 프로필마다 원격 디버깅 포트를 연다 (default=9222, soop-N=9222+N).
    if cfg!(debug_assertions) && std::env::var_os("MOABOM_DEBUG").is_some() {
        let n: u16 = profile.strip_prefix("soop-").and_then(|n| n.parse().ok()).unwrap_or(0);
        args.push_str(&format!(" --remote-debugging-port={}", 9222 + n));
    }
    let mut b = WebviewBuilder::new(label, WebviewUrl::External(url.parse().map_err(err)?))
        .data_directory(dir)
        .additional_browser_args(&args);
    // 광고 차단(uBO Lite). 같은 저장 공간을 쓰는 웹뷰는 확장 설정이 모두 같아야 하므로 타일·채팅 모두에 건다.
    if let Some(ext_dir) = extensions_dir(app) {
        b = b.browser_extensions_enabled(true).extensions_path(ext_dir);
    }
    let mut b = b
        .background_color(Color(0, 0, 0, 255))
        .on_new_window({
            let app = app.clone();
            let opener = label.to_string();
            move |url, features| popup_window(&app, &opener, url, features)
        })
        .on_page_load(|wv, payload| {
            if payload.event() == PageLoadEvent::Finished {
                restore_page_state(&wv);
            }
        });
    for s in scripts {
        b = if s.all_frames {
            b.initialization_script_for_all_frames(&s.source)
        } else {
            b.initialization_script(&s.source)
        };
    }
    Ok(b)
}

/// 번들에 포함된 확장 폴더(하위 폴더 하나가 확장 하나). 개발 중에는 빌드가 target 폴더로 복사해 둔다.
fn extensions_dir(app: &AppHandle) -> Option<std::path::PathBuf> {
    let dir = app.path().resource_dir().ok()?.join("extensions");
    dir.is_dir().then_some(dir)
}

/// 사이트가 띄우는 팝업은 막고, 숲 플레이어가 여는 채팅 팝업만 받아서 채팅 영역에 붙인다.
/// 숲 채팅 페이지는 자기를 연 플레이어 창(window.opener)에서 방송 정보를 읽기 때문에 이렇게 열어야 동작한다.
fn popup_window(app: &AppHandle, opener: &str, url: Url, features: NewWindowFeatures) -> NewWindowResponse<Wry> {
    let soop_chat = url.host_str().is_some_and(|h| h.contains("sooplive"))
        && url.query_pairs().any(|(k, v)| k == "vtype" && v == "chat");
    let Some(id) = tile_id(opener).filter(|_| soop_chat) else { return NewWindowResponse::Deny };
    if app.get_webview(&chat_label(id)).is_some() {
        return NewWindowResponse::Deny;
    }
    // 같은 실행 환경(window_features)으로 만들어야 opener 관계가 유지된다. 창은 숨긴 채 웹뷰만 옮겨 쓴다.
    let built = WebviewWindowBuilder::new(app, chat_label(id), WebviewUrl::External(url))
        .window_features(features)
        .visible(false)
        .skip_taskbar(true)
        .initialization_script(SOOP_CHAT_JS)
        .build();
    match built {
        Ok(window) => {
            adopt_popup_chat(app, id);
            NewWindowResponse::Create { window }
        }
        Err(_) => NewWindowResponse::Deny,
    }
}

/// 숨긴 팝업 창의 채팅 웹뷰를 채팅 영역(메인 창 또는 분리 창)으로 옮긴다.
fn adopt_popup_chat(app: &AppHandle, id: u32) {
    let app = app.clone();
    // 팝업 생성 콜백이 끝난 뒤에 옮겨야 하므로 잠깐 기다렸다가 처리한다.
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(300));
        let tile = {
            let state = app.state::<Tiles>();
            let g = state.0.lock().unwrap();
            match g.get(id) {
                // 그사이 다른 채팅을 골랐거나 방송을 닫았으면 버린다.
                Ok(t) if t.detached || g.chat_shown == Some(id) => t.clone(),
                _ => {
                    drop(g);
                    close_chat(&app, id);
                    return;
                }
            }
        };
        let moved = app
            .get_webview(&chat_label(id))
            .zip(host_window(&app, &tile).ok())
            .map(|(wv, host)| {
                // 팝업 창에 딸린 웹뷰는 창 크기를 따라가도록 설정돼 있어서 꺼 둔다.
                let _ = wv.set_auto_resize(false);
                set_parent_native(&wv, &host)
            })
            .unwrap_or(false);
        if moved {
            if let Ok(t) = app.state::<Tiles>().0.lock().unwrap().get_mut(id) {
                t.chat_open = true;
                t.popup_chat = true;
            }
            refresh(&app);
        } else {
            close_chat(&app, id);
            toast(&app, "숲 채팅을 열지 못했습니다.");
        }
    });
}

/// 페이지 안 상태(사이트 보기, 오버레이)는 새로고침하면 사라지므로 로드가 끝날 때마다 다시 적용한다.
fn restore_page_state(wv: &Webview) {
    let Some(id) = tile_id(wv.label()) else { return };
    let app = wv.app_handle();
    let state = app.state::<Tiles>();
    let Ok(tile) = state.0.lock().unwrap().get(id).cloned() else { return };
    if !tile.video_only {
        let _ = wv.eval("window.__mvSetVideoOnly && window.__mvSetVideoOnly(false)");
    }
    let quality = state.0.lock().unwrap().quality_of(&tile);
    let _ = wv.eval(format!("window.__mvSetQuality && window.__mvSetQuality('{}')", quality.as_str()));
    push_overlay(app);
}

/// 타일이 지금 붙어 있는 창(메인 또는 분리 창).
fn host_window(app: &AppHandle, tile: &Tile) -> Result<Window, String> {
    let label = if tile.detached { win_label(tile.id) } else { MAIN.to_string() };
    app.get_window(&label).ok_or_else(|| "창을 찾을 수 없습니다".to_string())
}

fn emit(app: &AppHandle) {
    let snap = app.state::<Tiles>().0.lock().unwrap().snapshot();
    let _ = app.emit_to(UI, "tiles", snap);
}

fn toast(app: &AppHandle, msg: &str) {
    let _ = app.emit_to(UI, "toast", msg);
}

fn refresh(app: &AppHandle) {
    apply_audio(app);
    relayout(app);
    push_overlay(app);
    emit(app);
}

/// 각 타일 오버레이에 번호·선택·채팅 같은 표시 상태를 알려 준다.
fn push_overlay(app: &AppHandle) {
    // 잠금은 값을 모으는 동안만 잡는다. 웹뷰 호출은 메인 스레드를 기다리는데, 메인 스레드도
    // 페이지 로드 처리 중에 이 잠금을 기다릴 수 있어 잠금을 쥔 채 부르면 교착 상태가 된다.
    let updates: Vec<(u32, serde_json::Value)> = {
        let state = app.state::<Tiles>();
        let g = state.0.lock().unwrap();
        g.tiles
            .iter()
            .enumerate()
            .map(|(i, t)| {
                let s = serde_json::json!({
                    "index": i + 1,
                    "total": g.tiles.len(),
                    "selected": g.selected == Some(t.id),
                    "muted": g.muted,
                    "chat": t.chat_open || g.chat_shown == Some(t.id),
                    "detached": t.detached,
                    "videoOnly": t.video_only,
                    "fullscreen": g.fullscreen && !t.detached,
                    "quality": g.quality_of(t).as_str(),
                    "offline": t.live == Some(false),
                    "image": t.image,
                    "dragging": g.drag.is_some_and(|(src, _)| src == t.id),
                    "dropTarget": g.drag.is_some_and(|(_, dst)| dst == Some(t.id)),
                });
                (t.id, s)
            })
            .collect()
    };
    for (id, s) in updates {
        if let Some(wv) = app.get_webview(&tile_label(id)) {
            let _ = wv.eval(format!("window.__mvOverlay && window.__mvOverlay.set({s})"));
        }
    }
}

// ---------- 오버레이 → 앱 메시지 ----------

#[cfg(windows)]
fn listen_messages(app: &AppHandle, wv: &Webview, id: u32) {
    use webview2_com::{CoTaskMemPWSTR, WebMessageReceivedEventHandler};
    use windows_core::PWSTR;
    let app = app.clone();
    let _ = wv.with_webview(move |p| unsafe {
        let Ok(core) = p.controller().CoreWebView2() else { return };
        let handler = WebMessageReceivedEventHandler::create(Box::new(move |_, args| {
            let Some(args) = args else { return Ok(()) };
            let mut raw = PWSTR::null();
            if args.TryGetWebMessageAsString(&mut raw).is_ok() {
                on_tile_message(&app, id, &CoTaskMemPWSTR::from(raw).to_string());
            }
            Ok(())
        }));
        let mut registration = 0i64;
        let _ = core.add_WebMessageReceived(&handler, &mut registration);
    });
}

#[cfg(not(windows))]
fn listen_messages(_app: &AppHandle, _wv: &Webview, _id: u32) {}

fn on_tile_message(app: &AppHandle, id: u32, text: &str) {
    let Ok(msg) = serde_json::from_str::<serde_json::Value>(text) else { return };
    // Tauri 내부 IPC 메시지나 사이트가 보낸 메시지는 토큰이 없으므로 무시된다.
    if msg["t"].as_str() != Some(token()) {
        return;
    }
    let cmd = msg["cmd"].as_str().unwrap_or_default().to_string();
    let arg = msg["arg"].as_i64().unwrap_or(0);
    if cmd == "meta" {
        set_meta(app, id, &msg["data"]);
        return;
    }
    if cmd == "touched" {
        if app.state::<Tiles>().0.lock().unwrap().panel.is_some() {
            let _ = app.emit_to(UI, "close-panel", ());
        }
        return;
    }
    if cmd == "drag" {
        on_drag(app, id, &msg["data"]);
        return;
    }
    if cmd == "ready" {
        if let Ok(t) = app.state::<Tiles>().0.lock().unwrap().get_mut(id) {
            t.loaded = true;
        }
        // 오버레이가 준비되기 전에 보낸 상태(방송 종료 등)를 놓쳤을 수 있어 다시 보낸다.
        push_overlay(app);
        return;
    }
    let target = if msg["self"].as_bool() == Some(true) { Some(id) } else { None };
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(e) = run_command(app.clone(), Some(id), target, cmd, arg).await {
            toast(&app, &e);
        }
    });
}

/// 오버레이에서 타일을 끄는 중이면 마우스 아래 타일을 표시하고, 놓으면 두 타일의 자리를 바꾼다.
/// 좌표는 끄는 타일 기준이라 그 타일의 위치를 더해 창 기준으로 바꾼다.
fn on_drag(app: &AppHandle, id: u32, data: &serde_json::Value) {
    let (x, y) = (data["x"].as_f64().unwrap_or(f64::NAN), data["y"].as_f64().unwrap_or(f64::NAN));
    let end = data["end"].as_bool().unwrap_or(false);
    let changed = {
        let state = app.state::<Tiles>();
        let mut g = state.0.lock().unwrap();
        let Some(origin) = g.rects.iter().find(|(t, _)| *t == id).map(|(_, r)| *r) else { return };
        let (px, py) = (origin.x + x, origin.y + y);
        let target = g
            .rects
            .iter()
            .find(|(t, r)| *t != id && px >= r.x && px < r.x + r.w && py >= r.y && py < r.y + r.h)
            .map(|(t, _)| *t);
        if end {
            g.drag = None;
            if let Some(dst) = target {
                let i = g.tiles.iter().position(|t| t.id == id);
                let j = g.tiles.iter().position(|t| t.id == dst);
                if let (Some(i), Some(j)) = (i, j) {
                    g.tiles.swap(i, j);
                }
            }
            true
        } else {
            let next = Some((id, target));
            let changed = g.drag != next;
            g.drag = next;
            changed
        }
    };
    if end {
        refresh(app);
    } else if changed {
        push_overlay(app);
    }
}

fn set_meta(app: &AppHandle, id: u32, data: &serde_json::Value) {
    let clean = |v: &serde_json::Value| v.as_str().map(str::trim).filter(|s| !s.is_empty()).map(str::to_string);
    let detached = {
        let state = app.state::<Tiles>();
        let mut g = state.0.lock().unwrap();
        let Ok(t) = g.get_mut(id) else { return };
        t.name = clean(&data["name"]);
        t.title = clean(&data["title"]);
        t.detached.then(|| (t.name.clone().unwrap_or_else(|| t.channel.clone()), t.title.clone()))
    };
    if let Some((name, title)) = detached {
        if let Some(win) = app.get_window(&win_label(id)) {
            let text = match title {
                Some(title) => format!("{name} - {title}"),
                None => name,
            };
            let _ = win.set_title(&text);
        }
    }
    emit(app);
}

// ---------- 음소거 ----------

#[cfg(windows)]
fn set_muted(wv: &Webview, muted: bool) {
    use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2_8;
    use windows_core::Interface;
    let _ = wv.with_webview(move |p| unsafe {
        if let Ok(core) = p.controller().CoreWebView2() {
            if let Ok(core8) = core.cast::<ICoreWebView2_8>() {
                let _ = core8.SetIsMuted(muted);
            }
        }
    });
}

#[cfg(not(windows))]
fn set_muted(wv: &Webview, muted: bool) {
    let _ = wv.eval(format!(
        "document.querySelectorAll('video').forEach(v => v.muted = {muted})"
    ));
}

/// 선택한 타일만 소리를 낸다. 사이트 볼륨과 상관없이 웹뷰 단위로 막는다.
fn apply_audio(app: &AppHandle) {
    let mutes: Vec<(u32, bool)> = {
        let state = app.state::<Tiles>();
        let g = state.0.lock().unwrap();
        g.tiles.iter().map(|t| (t.id, g.muted || g.selected != Some(t.id))).collect()
    };
    for (id, muted) in mutes {
        if let Some(wv) = app.get_webview(&tile_label(id)) {
            set_muted(&wv, muted);
        }
    }
}

// ---------- 배치 ----------

#[derive(Clone, Copy, Debug)]
struct Rect {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

/// 16:9 영상이 가장 크게 보이는 열 수를 고른다. 마지막 줄은 가운데 정렬.
fn grid(a: Rect, n: usize) -> Vec<Rect> {
    if n == 0 {
        return Vec::new();
    }
    let video_w = |cols: usize| {
        let rows = n.div_ceil(cols);
        (a.w / cols as f64).min(a.h / rows as f64 * 16.0 / 9.0)
    };
    let cols = (1..=n).max_by(|&x, &y| video_w(x).total_cmp(&video_w(y))).unwrap();
    let rows = n.div_ceil(cols);
    let (cw, ch) = (a.w / cols as f64, a.h / rows as f64);
    let last_row_items = n - (rows - 1) * cols;
    (0..n)
        .map(|i| {
            let (r, c) = (i / cols, i % cols);
            let offset = if r == rows - 1 { (cols - last_row_items) as f64 * cw / 2.0 } else { 0.0 };
            Rect { x: a.x + offset + c as f64 * cw, y: a.y + r as f64 * ch, w: cw, h: ch }
        })
        .collect()
}

/// 포커스 모드: 주 화면 하나를 크게, 나머지는 오른쪽 세로 줄로.
fn focus(a: Rect, n: usize, main_idx: usize) -> Vec<Rect> {
    if n <= 1 {
        return grid(a, n);
    }
    let side_w = (a.w * 0.25).max(200.0).min(a.w * 0.5);
    let main = Rect { w: a.w - side_w, ..a };
    let side_h = a.h / (n - 1) as f64;
    let mut k = 0;
    (0..n)
        .map(|i| {
            if i == main_idx {
                main
            } else {
                let r = Rect { x: a.x + main.w, y: a.y + k as f64 * side_h, w: side_w, h: side_h };
                k += 1;
                r
            }
        })
        .collect()
}

fn place(app: &AppHandle, label: &str, r: Rect) {
    let Some(wv) = app.get_webview(label) else { return };
    let _ = wv.set_position(LogicalPosition::new(r.x.round(), r.y.round()));
    let _ = wv.set_size(LogicalSize::new(r.w.round().max(1.0), r.h.round().max(1.0)));
    if label == UI {
        return;
    }
    // 일반 웹뷰는 Tauri가 만든 컨테이너 창 안에 들어 있고, Tauri는 컨테이너만 옮긴다.
    // 다른 창으로 옮긴 뒤에는 안쪽 웹뷰 크기가 예전 그대로 남아 잘려 보이므로 컨테이너를 꽉 채우게 맞춘다.
    // 숲 팝업 채팅은 컨테이너 없이 창에 바로 붙어 있어서 창 기준 좌표를 그대로 쓴다.
    let popup = tile_id(label.replacen("chat-", "tile-", 1).as_str())
        .filter(|_| label.starts_with("chat-"))
        .is_some_and(|id| app.state::<Tiles>().0.lock().unwrap().get(id).is_ok_and(|t| t.popup_chat));
    if popup {
        set_bounds_native(&wv, r);
    } else {
        // 다른 창으로 옮긴 웹뷰는 Tauri의 위치 지정이 늦게 또는 엉뚱하게 적용돼 잘려 보인다.
        // wry가 하는 일(컨테이너 창 이동 + 안쪽 웹뷰를 컨테이너에 꽉 채우기)을 바로 적용되게 직접 한다.
        set_container_bounds_native(&wv, r);
    }
}

#[cfg(windows)]
fn set_bounds_native(wv: &Webview, r: Rect) {
    use webview2_com::Microsoft::Web::WebView2::Win32::{
        ICoreWebView2Controller3, COREWEBVIEW2_BOUNDS_MODE, COREWEBVIEW2_BOUNDS_MODE_USE_RASTERIZATION_SCALE,
    };
    use windows_core::Interface;
    let scale = wv.window().scale_factor().unwrap_or(1.0);
    let _ = wv.with_webview(move |p| unsafe {
        let controller = p.controller();
        // 숲 팝업에서 온 웹뷰는 좌표에 화면 배율을 스스로 곱하는 모드일 수 있다.
        // 그 경우 물리 픽셀을 그 배율로 나눠 넘겨야 배율이 두 번 들어가지 않는다.
        let mut divide = 1.0;
        if let Ok(c3) = controller.cast::<ICoreWebView2Controller3>() {
            let mut mode = COREWEBVIEW2_BOUNDS_MODE::default();
            let mut raster = 1.0;
            if c3.BoundsMode(&mut mode).is_ok()
                && mode == COREWEBVIEW2_BOUNDS_MODE_USE_RASTERIZATION_SCALE
                && c3.RasterizationScale(&mut raster).is_ok()
                && raster > 0.0
            {
                divide = raster;
            }
        }
        let px = |v: f64| (v * scale / divide).round() as i32;
        let rect = windows::Win32::Foundation::RECT {
            left: px(r.x),
            top: px(r.y),
            right: px(r.x + r.w.max(1.0)),
            bottom: px(r.y + r.h.max(1.0)),
        };
        let _ = controller.SetBounds(rect);
    });
}

#[cfg(not(windows))]
fn set_bounds_native(_wv: &Webview, _r: Rect) {}

/// 웹뷰를 감싼 컨테이너 창(wry가 만든 창)을 r로 옮기고, 안쪽 웹뷰는 컨테이너를 꽉 채운다.
#[cfg(windows)]
fn set_container_bounds_native(wv: &Webview, r: Rect) {
    use windows::Win32::Foundation::{HWND, RECT};
    use windows::Win32::UI::WindowsAndMessaging::{SetWindowPos, SWP_NOACTIVATE, SWP_NOZORDER};
    let scale = wv.window().scale_factor().unwrap_or(1.0);
    let px = |v: f64| (v * scale).round() as i32;
    let (x, y, w, h) = (px(r.x), px(r.y), px(r.w.max(1.0)), px(r.h.max(1.0)));
    let _ = wv.with_webview(move |p| unsafe {
        let controller = p.controller();
        let mut container = HWND::default();
        if controller.ParentWindow(&mut container).is_ok() && !container.is_invalid() {
            let _ = SetWindowPos(container, None, x, y, w, h, SWP_NOACTIVATE | SWP_NOZORDER);
        }
        let _ = controller.SetBounds(RECT { left: 0, top: 0, right: w, bottom: h });
    });
}

#[cfg(not(windows))]
fn set_container_bounds_native(_wv: &Webview, _r: Rect) {}

/// 웹뷰를 다른 창에 붙인다. 팝업에서 만든 웹뷰는 Tauri의 reparent가 적용되지 않아 WebView2에 직접 지정한다.
/// 팝업 창 자체는 숨긴 채 남겨 두므로, 그 창이 닫힐 때까지 웹뷰도 살아 있다.
#[cfg(windows)]
fn set_parent_native(wv: &Webview, host: &Window) -> bool {
    let Ok(hwnd) = host.hwnd() else { return false };
    // HWND는 스레드 간에 넘길 수 없어 정수로 옮긴다.
    let raw = hwnd.0 as isize;
    wv.with_webview(move |p| unsafe {
        let _ = p.controller().SetParentWindow(windows::Win32::Foundation::HWND(raw as *mut _));
    })
    .is_ok()
}

#[cfg(not(windows))]
fn set_parent_native(wv: &Webview, host: &Window) -> bool {
    wv.reparent(host).is_ok()
}

/// 채팅 웹뷰를 다른 창(분리 창 ↔ 메인 창)으로 옮긴다.
fn move_chat(app: &AppHandle, tile: &Tile, host: &Window) -> Result<(), String> {
    let Some(wv) = app.get_webview(&chat_label(tile.id)) else { return Ok(()) };
    if tile.popup_chat {
        set_parent_native(&wv, host).then_some(()).ok_or_else(|| "채팅을 옮기지 못했습니다".to_string())
    } else {
        wv.reparent(host).map_err(err)
    }
}

fn logical_size(win: &Window) -> Option<(f64, f64)> {
    let size = win.inner_size().ok()?.to_logical::<f64>(win.scale_factor().ok()?);
    Some((size.width, size.height))
}

pub fn relayout(app: &AppHandle) {
    let Some(main) = app.get_window(MAIN) else { return };
    let Some((w, h)) = logical_size(&main) else { return };

    // 배치는 잠금 안에서 계산만 하고, 실제 웹뷰 이동은 잠금을 푼 뒤에 한다(push_overlay 주석 참고).
    let mut moves: Vec<(String, Rect)> = Vec::new();
    let detached: Vec<(u32, bool)>;
    let kick: Vec<u32>;
    let panel: Option<Rect>;
    {
        let state = app.state::<Tiles>();
        let g = state.0.lock().unwrap();
        let attached: Vec<&Tile> = g.tiles.iter().filter(|t| !t.detached).collect();
        let shown = g.chat_shown.filter(|id| attached.iter().any(|t| t.id == *id));

        // 전체 화면에서는 컨트롤 바를 숨기고 방송만 채운다.
        let top = if g.fullscreen { 0.0 } else { BAR_H };
        // 방송이 없으면 컨트롤 바 웹뷰가 창 전체를 채워 안내 화면을 보여준다.
        // 검색 결과가 열려 있으면 그 아래까지 늘리고, 바와 결과 패널 모양으로 잘라 방송 위에 겹친다.
        panel = g.panel.filter(|_| !attached.is_empty() && !g.fullscreen);
        let ui_h = match panel {
            _ if attached.is_empty() => h,
            Some(p) => top.max(p.y + p.h),
            None => top,
        };
        moves.push((UI.to_string(), Rect { x: 0.0, y: 0.0, w, h: ui_h.max(1.0) }));

        let dock = if shown.is_some() { CHAT_W.min(w * 0.4) } else { 0.0 };
        let area = Rect { x: 0.0, y: top, w: (w - dock).max(1.0), h: (h - top).max(1.0) };
        let rects = match g.layout {
            Layout::Grid => grid(area, attached.len()),
            Layout::Focus => {
                let main_idx = attached.iter().position(|t| Some(t.id) == g.selected).unwrap_or(0);
                focus(area, attached.len(), main_idx)
            }
        };
        for (t, r) in attached.iter().zip(rects) {
            moves.push((tile_label(t.id), r));
        }
        if let Some(id) = shown {
            moves.push((chat_label(id), Rect { x: w - dock, y: top, w: dock, h: area.h }));
        }
        detached = g.tiles.iter().filter(|t| t.detached).map(|t| (t.id, t.chat_open)).collect();
        kick = g.tiles.iter().filter(|t| t.platform == Platform::Kick).map(|t| t.id).collect();
    }

    app.state::<Tiles>().0.lock().unwrap().rects = moves
        .iter()
        .filter_map(|(label, r)| tile_id(label).map(|id| (id, *r)))
        .collect();

    for (id, chat_open) in detached {
        let Some(win) = app.get_window(&win_label(id)) else { continue };
        let Some((ww, wh)) = logical_size(&win) else { continue };
        let chat_w = if chat_open { CHAT_W.min(ww * 0.4) } else { 0.0 };
        moves.push((tile_label(id), Rect { x: 0.0, y: 0.0, w: ww - chat_w, h: wh }));
        if chat_open {
            moves.push((chat_label(id), Rect { x: ww - chat_w, y: 0.0, w: chat_w, h: wh }));
        }
    }
    for (label, r) in moves {
        place(app, &label, r);
        // 좁은 킥 타일은 확대 비율을 낮춰 킥이 화면 폭을 KICK_MIN_WIDTH로 인식하게 한다(조작 바가 생긴다).
        // 오버레이는 반대로 키워 원래 크기로 보이게 한다.
        if let Some(id) = tile_id(&label).filter(|id| kick.contains(id)) {
            let zoom = (r.w / KICK_MIN_WIDTH).min(1.0);
            if let Some(wv) = app.get_webview(&tile_label(id)) {
                let _ = wv.set_zoom(zoom);
                let _ = wv.eval(format!("window.__mvOverlay && window.__mvOverlay.set({{zoom: {zoom}}})"));
            }
        }
    }
    if let Some(ui) = app.get_webview(UI) {
        set_ui_shape(&ui, w, panel);
    }
}

/// 검색 결과가 열려 있으면 컨트롤 바 웹뷰를 맨 위로 올리고 "바 + 결과 패널" 모양으로 잘라,
/// 결과가 방송을 밀어내지 않고 그 위에 겹쳐 보이게 한다. 닫히면 원래 사각형으로 돌린다.
#[cfg(windows)]
fn set_ui_shape(wv: &Webview, w: f64, panel: Option<Rect>) {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::Graphics::Gdi::{CombineRgn, CreateRectRgn, DeleteObject, SetWindowRgn, HRGN, RGN_OR};
    use windows::Win32::UI::WindowsAndMessaging::{
        SetWindowPos, HWND_TOP, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
    };
    let scale = wv.window().scale_factor().unwrap_or(1.0);
    let px = |v: f64| (v * scale).round() as i32;
    let shape = panel.map(|p| ((0, 0, px(w), px(BAR_H)), (px(p.x), px(BAR_H), px(p.x + p.w), px(p.y + p.h))));
    let _ = wv.with_webview(move |p| unsafe {
        let mut container = HWND::default();
        if p.controller().ParentWindow(&mut container).is_err() || container.is_invalid() {
            return;
        }
        match shape {
            Some((bar, list)) => {
                let rgn = CreateRectRgn(bar.0, bar.1, bar.2, bar.3);
                let list = CreateRectRgn(list.0, list.1, list.2, list.3);
                CombineRgn(Some(rgn), Some(rgn), Some(list), RGN_OR);
                let _ = DeleteObject(list.into());
                // 성공하면 영역은 창이 가져가므로 지우지 않는다.
                if SetWindowRgn(container, Some(rgn), true) == 0 {
                    let _ = DeleteObject(rgn.into());
                }
                let _ = SetWindowPos(container, Some(HWND_TOP), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
            }
            None => {
                SetWindowRgn(container, None::<HRGN>, true);
            }
        }
    });
}

#[cfg(not(windows))]
fn set_ui_shape(_wv: &Webview, _w: f64, _panel: Option<Rect>) {}

// ---------- 창 분리 ----------

fn detach(app: &AppHandle, id: u32) -> Result<(), String> {
    let tile = app.state::<Tiles>().0.lock().unwrap().get(id)?.clone();
    if tile.detached {
        return Ok(());
    }
    let title = tile.name.clone().unwrap_or_else(|| tile.channel.clone());
    let win = tauri::window::WindowBuilder::new(app, win_label(id))
        .title(format!("{title} - {}", crate::APP_NAME))
        .inner_size(960.0, 540.0)
        .background_color(Color(0, 0, 0, 255))
        .theme(Some(tauri::Theme::Dark))
        .build()
        .map_err(err)?;

    // 웹뷰를 새로 만들지 않고 옮겨 붙이므로 방송이 끊기지 않는다.
    if let Some(wv) = app.get_webview(&tile_label(id)) {
        wv.reparent(&win).map_err(err)?;
    }
    move_chat(app, &tile, &win)?;
    {
        let state = app.state::<Tiles>();
        let mut g = state.0.lock().unwrap();
        // 메인 채팅 영역에 보이던 채팅은 방송과 함께 분리 창으로 옮겨 간다.
        if g.chat_shown == Some(id) {
            g.chat_shown = None;
        }
        g.get_mut(id)?.detached = true;
    }

    let app2 = app.clone();
    win.on_window_event(move |e| match e {
        WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => relayout(&app2),
        WindowEvent::CloseRequested { api, .. } => {
            // 분리 창을 닫으면 방송을 끄지 않고 메인 창으로 돌려보낸다.
            api.prevent_close();
            let app3 = app2.clone();
            tauri::async_runtime::spawn(async move {
                let _ = attach(&app3, id);
            });
        }
        _ => {}
    });
    refresh(app);
    Ok(())
}

fn attach(app: &AppHandle, id: u32) -> Result<(), String> {
    let main = app.get_window(MAIN).ok_or("메인 창이 없습니다")?;
    let Some(win) = app.get_window(&win_label(id)) else { return Ok(()) };
    if let Some(wv) = app.get_webview(&tile_label(id)) {
        wv.reparent(&main).map_err(err)?;
    }
    // `if let` 조건에서 잠그면 블록 끝까지 잠금이 유지돼 웹뷰 작업 중 교착 상태가 되므로, 먼저 복사해 둔다.
    let tile = app.state::<Tiles>().0.lock().unwrap().get(id).cloned();
    if let Ok(tile) = tile {
        move_chat(app, &tile, &main)?;
    }
    // 분리 창에서 채팅을 보고 있었다면 메인 채팅 영역으로 가져온다(채팅 영역에는 하나만 보인다).
    let replaced = {
        let state = app.state::<Tiles>();
        let mut g = state.0.lock().unwrap();
        let chat_open = g.get(id).map(|t| t.chat_open).unwrap_or(false);
        let prev = g.chat_shown.filter(|&p| chat_open && p != id);
        if chat_open {
            g.chat_shown = Some(id);
        }
        if let Ok(t) = g.get_mut(id) {
            t.detached = false;
        }
        prev
    };
    if let Some(prev) = replaced {
        close_chat(app, prev);
    }
    let _ = win.destroy();
    refresh(app);
    Ok(())
}

// ---------- 타일 조작 ----------

async fn remove_tile(app: &AppHandle, id: u32) -> Result<(), String> {
    close_chat(app, id);
    if let Some(wv) = app.get_webview(&tile_label(id)) {
        let _ = wv.close();
    }
    if let Some(win) = app.get_window(&win_label(id)) {
        let _ = win.destroy();
    }
    {
        let state = app.state::<Tiles>();
        let mut g = state.0.lock().unwrap();
        if g.chat_shown == Some(id) {
            g.chat_shown = None;
        }
        let idx = g.tiles.iter().position(|t| t.id == id);
        g.tiles.retain(|t| t.id != id);
        if g.selected == Some(id) {
            // 닫은 자리의 다음 방송(없으면 이전 방송)을 선택한다.
            let next = idx.and_then(|i| g.tiles.get(i).or_else(|| g.tiles.last()));
            g.selected = next.map(|t| t.id);
        }
    }
    refresh(app);
    Ok(())
}

fn toggle_detach(app: &AppHandle, id: u32) -> Result<(), String> {
    let detached = app.state::<Tiles>().0.lock().unwrap().get(id)?.detached;
    if detached { attach(app, id) } else { detach(app, id) }
}

/// 채팅 웹뷰를 닫는다. 숲 채팅을 받을 때 쓴 숨긴 팝업 창도 함께 정리한다.
fn close_chat(app: &AppHandle, id: u32) {
    if let Some(wv) = app.get_webview(&chat_label(id)) {
        let _ = wv.close();
    }
    if let Some(win) = app.get_window(&chat_label(id)) {
        let _ = win.destroy();
    }
    if let Ok(t) = app.state::<Tiles>().0.lock().unwrap().get_mut(id) {
        t.chat_open = false;
        t.popup_chat = false;
    }
}

/// 채팅 웹뷰를 만든다. 숲은 플레이어가 직접 팝업을 열게 하고, 그 팝업을 `popup_window`가 받아 붙인다.
fn open_chat(app: &AppHandle, tile: &Tile) -> Result<(), String> {
    if tile.platform == Platform::Soop {
        let wv = app.get_webview(&tile_label(tile.id)).ok_or("타일이 없습니다")?;
        return wv.eval("window.__mvOpenChat && window.__mvOpenChat()").map_err(err);
    }
    let url = tile
        .chat_url
        .as_deref()
        .ok_or("이 방송은 따로 띄울 채팅 주소가 없습니다. '사이트 보기'(V)로 전환해서 보세요.")?;
    let win = host_window(app, tile)?;
    // 타일과 같은 저장 공간을 써야 로그인한 상태로 채팅할 수 있다.
    let builder = build_webview(app, &chat_label(tile.id), url, &tile.profile, &[])?;
    win.add_child(builder, LogicalPosition::new(0.0, 0.0), LogicalSize::new(1.0, 1.0))
        .map_err(err)?;
    if let Ok(t) = app.state::<Tiles>().0.lock().unwrap().get_mut(tile.id) {
        t.chat_open = true;
    }
    Ok(())
}

/// 채팅 켜기/끄기. 메인 창의 채팅 영역에는 한 번에 하나만 보이고, 다른 방송을 고르면 이전 채팅은 닫는다.
/// 분리 창의 방송은 그 창 오른쪽에 따로 띄운다.
fn toggle_chat(app: &AppHandle, id: u32) -> Result<(), String> {
    let (tile, shown) = {
        let state = app.state::<Tiles>();
        let g = state.0.lock().unwrap();
        (g.get(id)?.clone(), g.chat_shown)
    };
    if tile.detached {
        if tile.chat_open {
            close_chat(app, id);
        } else {
            open_chat(app, &tile)?;
        }
    } else if shown == Some(id) {
        close_chat(app, id);
        app.state::<Tiles>().0.lock().unwrap().chat_shown = None;
    } else {
        if let Some(prev) = shown {
            close_chat(app, prev);
        }
        app.state::<Tiles>().0.lock().unwrap().chat_shown = Some(id);
        if let Err(e) = open_chat(app, &tile) {
            app.state::<Tiles>().0.lock().unwrap().chat_shown = None;
            refresh(app);
            return Err(e);
        }
    }
    refresh(app);
    Ok(())
}

fn toggle_video_only(app: &AppHandle, id: u32) -> Result<(), String> {
    let on = {
        let state = app.state::<Tiles>();
        let mut g = state.0.lock().unwrap();
        let t = g.get_mut(id)?;
        t.video_only = !t.video_only;
        t.video_only
    };
    if let Some(wv) = app.get_webview(&tile_label(id)) {
        wv.eval(format!("window.__mvSetVideoOnly && window.__mvSetVideoOnly({on})")).map_err(err)?;
    }
    push_overlay(app);
    emit(app);
    Ok(())
}

/// 방송을 여러 개 한꺼번에 열면 가끔 첫 페이지 이동이 끝나지 않고 about:blank에 멈춰 있다.
/// 그때는 페이지 스크립트(자동 복구 포함)도 돌지 않는다. 방송 페이지는 열리면 오버레이가 "ready"를 보내므로,
/// 30초 안에 소식이 없으면 먼저 다시 이동시키고, 그래도 안 되면 웹뷰를 새로 만든다.
fn watch_first_load(app: &AppHandle, id: u32) {
    let app = app.clone();
    std::thread::spawn(move || {
        for attempt in 0..2 {
            std::thread::sleep(std::time::Duration::from_secs(30));
            let url = {
                let state = app.state::<Tiles>();
                let g = state.0.lock().unwrap();
                match g.get(id) {
                    Ok(t) if !t.loaded => t.url.clone(),
                    _ => return,
                }
            };
            eprintln!("[moabom] 타일 {id}: 30초 안에 열리지 않음, 복구 시도 {}", attempt + 1);
            if attempt == 0 {
                if let (Some(wv), Ok(url)) = (app.get_webview(&tile_label(id)), url.parse()) {
                    let _ = wv.navigate(url);
                }
            } else if let Err(e) = recreate_tile(&app, id) {
                toast(&app, &format!("방송을 다시 여는 데 실패했습니다: {e}"));
            }
        }
    });
}

/// 타일 웹뷰를 닫고 같은 설정으로 새로 만든다.
fn recreate_tile(app: &AppHandle, id: u32) -> Result<(), String> {
    let (tile, scripts) = {
        let state = app.state::<Tiles>();
        let g = state.0.lock().unwrap();
        let tile = g.get(id)?.clone();
        let scripts = g.tile_scripts(tile.platform, g.quality_of(&tile));
        (tile, scripts)
    };
    if let Some(wv) = app.get_webview(&tile_label(id)) {
        let _ = wv.close();
    }
    // 같은 이름의 웹뷰가 정리될 시간을 준다.
    std::thread::sleep(std::time::Duration::from_millis(300));
    let host = host_window(app, &tile)?;
    let builder = build_webview(app, &tile_label(id), &tile.url, &tile.profile, &scripts)?;
    let wv = host
        .add_child(builder, LogicalPosition::new(0.0, BAR_H), LogicalSize::new(1.0, 1.0))
        .map_err(err)?;
    listen_messages(app, &wv, id);
    refresh(app);
    Ok(())
}

/// 방송 페이지에 지금 화질 설정을 적용한다.
fn apply_quality(app: &AppHandle, id: u32) {
    let quality = {
        let state = app.state::<Tiles>();
        let g = state.0.lock().unwrap();
        match g.get(id) {
            Ok(t) => g.quality_of(t),
            Err(_) => return,
        }
    };
    if let Some(wv) = app.get_webview(&tile_label(id)) {
        let _ = wv.eval(format!("window.__mvSetQuality && window.__mvSetQuality('{}')", quality.as_str()));
    }
}

/// 이 방송만 화질을 바꾼다: 최고화질 → 네트워크 최적화 → 저화질 순서.
fn cycle_quality(app: &AppHandle, id: u32) -> Result<(), String> {
    {
        let state = app.state::<Tiles>();
        let mut g = state.0.lock().unwrap();
        let next = g.quality_of(g.get(id)?).next();
        g.get_mut(id)?.quality = Some(next);
    }
    apply_quality(app, id);
    push_overlay(app);
    emit(app);
    Ok(())
}

// ---------- 방송 종료 확인 ----------

const LIVE_CHECK_SECS: u64 = 30;

/// 열린 방송들이 방송 중인지 주기적으로 확인한다.
pub fn watch_live(app: &AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || loop {
        std::thread::sleep(std::time::Duration::from_secs(LIVE_CHECK_SECS));
        let ids: Vec<u32> = app.state::<Tiles>().0.lock().unwrap().tiles.iter().map(|t| t.id).collect();
        for id in ids {
            tauri::async_runtime::block_on(check_live(&app, id));
        }
    });
}

/// 한 방송의 상태를 확인해 바뀌었으면 오버레이에 알린다. 꺼져 있다가 다시 켜졌으면 새로고침한다.
async fn check_live(app: &AppHandle, id: u32) {
    let Some((platform, channel)) = app.state::<Tiles>().0.lock().unwrap().get(id).ok().map(|t| (t.platform, t.channel.clone()))
    else {
        return;
    };
    let Some(status) = crate::live::check(platform, &channel).await else { return };
    let (changed, came_back) = {
        let state = app.state::<Tiles>();
        let mut g = state.0.lock().unwrap();
        let Ok(t) = g.get_mut(id) else { return };
        let before = t.live;
        t.live = Some(status.live);
        if status.image.is_some() {
            t.image = status.image;
        }
        (before != t.live, before == Some(false) && status.live)
    };
    if came_back {
        let _ = reload_tile(app, id);
    }
    if changed {
        push_overlay(app);
        emit(app);
    }
}

fn reload_tile(app: &AppHandle, id: u32) -> Result<(), String> {
    let wv = app.get_webview(&tile_label(id)).ok_or("타일이 없습니다")?;
    wv.eval("location.reload()").map_err(err)
}

fn move_tile(app: &AppHandle, id: u32, delta: i64) {
    {
        let state = app.state::<Tiles>();
        let mut g = state.0.lock().unwrap();
        let Some(i) = g.tiles.iter().position(|t| t.id == id) else { return };
        let j = i as i64 + delta;
        if j >= 0 && (j as usize) < g.tiles.len() {
            g.tiles.swap(i, j as usize);
        }
    }
    refresh(app);
}

fn set_fullscreen(app: &AppHandle, on: bool) {
    if let Some(main) = app.get_window(MAIN) {
        let _ = main.set_fullscreen(on);
    }
    app.state::<Tiles>().0.lock().unwrap().fullscreen = on;
    refresh(app);
}

/// 컨트롤 바의 주소·검색 입력칸으로 키보드 포커스를 옮긴다.
fn focus_add_input(app: &AppHandle) {
    if app.state::<Tiles>().0.lock().unwrap().fullscreen {
        set_fullscreen(app, false);
    }
    if let Some(main) = app.get_window(MAIN) {
        let _ = main.set_focus();
    }
    if let Some(ui) = app.get_webview(UI) {
        let _ = ui.set_focus();
        let _ = ui.eval("window.__focusAdd && window.__focusAdd()");
    }
}

/// 단축키·오버레이 버튼·컨트롤 바가 공통으로 쓰는 명령.
/// `source`는 명령이 들어온 타일, `target`은 명령을 적용할 타일(없으면 선택한 타일)이다.
async fn run_command(
    app: AppHandle,
    source: Option<u32>,
    target: Option<u32>,
    cmd: String,
    arg: i64,
) -> Result<(), String> {
    let (id, ids) = {
        let state = app.state::<Tiles>();
        let g = state.0.lock().unwrap();
        (target.or_else(|| g.target()), g.tiles.iter().map(|t| t.id).collect::<Vec<_>>())
    };
    let select = |id: u32| {
        let state = app.state::<Tiles>();
        let mut g = state.0.lock().unwrap();
        g.selected = Some(id);
        g.muted = false;
    };
    match cmd.as_str() {
        "select" => {
            let Some(&t) = usize::try_from(arg - 1).ok().and_then(|i| ids.get(i)) else { return Ok(()) };
            select(t);
            refresh(&app);
        }
        "next" | "prev" if !ids.is_empty() => {
            let cur = id.and_then(|id| ids.iter().position(|&t| t == id)).unwrap_or(0);
            let step = if cmd == "next" { 1 } else { ids.len() - 1 };
            select(ids[(cur + step) % ids.len()]);
            refresh(&app);
        }
        // 오버레이의 스피커 버튼: 이 방송 소리 켜기, 이미 듣고 있으면 음소거
        "audio" => {
            let Some(id) = id else { return Ok(()) };
            {
                let state = app.state::<Tiles>();
                let mut g = state.0.lock().unwrap();
                if g.selected == Some(id) && !g.muted {
                    g.muted = true;
                } else {
                    g.selected = Some(id);
                    g.muted = false;
                }
            }
            refresh(&app);
        }
        "mute" => {
            {
                let state = app.state::<Tiles>();
                let mut g = state.0.lock().unwrap();
                g.muted = !g.muted;
                if g.selected.is_none() {
                    g.selected = id;
                }
            }
            refresh(&app);
        }
        "layout" => {
            {
                let state = app.state::<Tiles>();
                let mut g = state.0.lock().unwrap();
                g.layout = if g.layout == Layout::Grid { Layout::Focus } else { Layout::Grid };
                if g.selected.is_none() {
                    g.selected = id;
                }
            }
            refresh(&app);
        }
        "fullscreen" | "exitfullscreen" => {
            // 분리된 창에서 누르면 그 창을, 아니면 메인 창을 전체 화면으로 만든다.
            let detached_win = source.and_then(|s| app.get_window(&win_label(s)));
            if let Some(win) = detached_win {
                let on = cmd == "fullscreen" && !win.is_fullscreen().unwrap_or(false);
                let _ = win.set_fullscreen(on);
            } else {
                let on = cmd == "fullscreen" && !app.state::<Tiles>().0.lock().unwrap().fullscreen;
                set_fullscreen(&app, on);
            }
        }
        "add" => focus_add_input(&app),
        "help" => {
            if let Some(wv) = id.and_then(|id| app.get_webview(&tile_label(id))) {
                let _ = wv.eval("window.__mvOverlay && window.__mvOverlay.toggleHelp()");
            }
        }
        _ => {
            let Some(id) = id else { return Ok(()) };
            match cmd.as_str() {
                "chat" => toggle_chat(&app, id)?,
                "detach" => toggle_detach(&app, id)?,
                "siteview" => toggle_video_only(&app, id)?,
                "reload" => reload_tile(&app, id)?,
                "close" => remove_tile(&app, id).await?,
                "move" => move_tile(&app, id, arg),
                "quality" => cycle_quality(&app, id)?,
                _ => {}
            }
        }
    }
    Ok(())
}

// ---------- 컨트롤 바에서 부르는 명령 ----------

#[tauri::command]
pub fn list_tiles(state: tauri::State<'_, Tiles>) -> Snapshot {
    state.0.lock().unwrap().snapshot()
}

#[tauri::command]
pub async fn add_tile(app: AppHandle, input: String) -> Result<(), String> {
    let r = platforms::resolve(&input)?;
    let (tile, scripts) = {
        let state = app.state::<Tiles>();
        let mut g = state.0.lock().unwrap();
        g.next_id += 1;
        let tile = Tile {
            id: g.next_id,
            platform: r.platform,
            profile: g.assign_profile(r.platform),
            channel: r.channel,
            url: r.player_url,
            chat_url: r.chat_url,
            detached: false,
            chat_open: false,
            video_only: true,
            name: None,
            title: None,
            popup_chat: false,
            quality: None,
            loaded: false,
            live: None,
            image: None,
        };
        g.tiles.push(tile.clone());
        if g.selected.is_none() {
            g.selected = Some(tile.id);
        }
        let scripts = g.tile_scripts(tile.platform, g.quality);
        (tile, scripts)
    };
    let main = app.get_window(MAIN).ok_or("메인 창이 없습니다")?;
    let builder = build_webview(&app, &tile_label(tile.id), &tile.url, &tile.profile, &scripts)?;
    match main.add_child(builder, LogicalPosition::new(0.0, BAR_H), LogicalSize::new(1.0, 1.0)) {
        Ok(wv) => {
            listen_messages(&app, &wv, tile.id);
            watch_first_load(&app, tile.id);
            let app = app.clone();
            tauri::async_runtime::spawn(async move { check_live(&app, tile.id).await });
        }
        Err(e) => {
            app.state::<Tiles>().0.lock().unwrap().tiles.retain(|t| t.id != tile.id);
            emit(&app);
            return Err(err(e));
        }
    }
    refresh(&app);
    Ok(())
}

/// 단축키나 컨트롤 바 버튼에서 오는 명령. 대상은 선택한 타일이다.
#[tauri::command]
pub async fn command(app: AppHandle, cmd: String, arg: Option<i64>) -> Result<(), String> {
    run_command(app, None, None, cmd, arg.unwrap_or(0)).await
}

#[tauri::command]
pub async fn set_layout(app: AppHandle, layout: Layout) -> Result<(), String> {
    app.state::<Tiles>().0.lock().unwrap().layout = layout;
    refresh(&app);
    Ok(())
}

/// 컨트롤 바의 채팅 드롭다운: 고른 방송의 채팅을 오른쪽 채팅 영역에 띄운다. `None`이면 채팅 영역을 닫는다.
#[tauri::command]
pub async fn show_chat(app: AppHandle, id: Option<u32>) -> Result<(), String> {
    let shown = app.state::<Tiles>().0.lock().unwrap().chat_shown;
    match id {
        Some(id) if shown != Some(id) => toggle_chat(&app, id),
        None => match shown {
            Some(prev) => toggle_chat(&app, prev),
            None => Ok(()),
        },
        _ => Ok(()),
    }
}

/// 검색 결과 패널을 펼치거나 접는다. 패널은 방송 위에 겹쳐 그린다. 크기가 0이면 닫힌 것으로 본다.
#[tauri::command]
pub async fn set_panel(app: AppHandle, x: f64, width: f64, height: f64) -> Result<(), String> {
    let panel = (width > 0.0 && height > 0.0).then(|| Rect { x: x.max(0.0), y: BAR_H, w: width, h: height.min(600.0) });
    app.state::<Tiles>().0.lock().unwrap().panel = panel;
    relayout(&app);
    Ok(())
}

/// 모든 방송의 화질을 바꾼다. 방송별로 따로 정했던 화질도 이 값으로 맞춘다.
#[tauri::command]
pub async fn set_quality(app: AppHandle, quality: Quality) -> Result<(), String> {
    let ids: Vec<u32> = {
        let state = app.state::<Tiles>();
        let mut g = state.0.lock().unwrap();
        g.quality = quality;
        for t in &mut g.tiles {
            t.quality = None;
        }
        g.tiles.iter().map(|t| t.id).collect()
    };
    for id in ids {
        apply_quality(&app, id);
    }
    push_overlay(&app);
    emit(&app);
    Ok(())
}

/// 이미 열린 타일에는 적용되지 않고, 이후에 추가하는 숲 타일부터 적용된다.
#[tauri::command]
pub async fn set_split_soop(app: AppHandle, on: bool) -> Result<(), String> {
    app.state::<Tiles>().0.lock().unwrap().split_soop = on;
    emit(&app);
    Ok(())
}

/// 이후에 추가하는 치지직 타일부터 적용된다.
#[tauri::command]
pub async fn set_chzzk_hq(app: AppHandle, on: bool) -> Result<(), String> {
    app.state::<Tiles>().0.lock().unwrap().chzzk_hq = on;
    emit(&app);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: Rect = Rect { x: 0.0, y: 0.0, w: 1600.0, h: 900.0 };

    fn tile(id: u32, platform: Platform, profile: String) -> Tile {
        Tile {
            id,
            platform,
            channel: String::new(),
            url: String::new(),
            chat_url: None,
            profile,
            detached: false,
            chat_open: false,
            video_only: true,
            name: None,
            title: None,
            popup_chat: false,
            quality: None,
            loaded: false,
            live: None,
            image: None,
        }
    }

    #[test]
    fn grid_picks_square_layouts() {
        assert_eq!(grid(A, 1).len(), 1);
        let r4 = grid(A, 4);
        assert_eq!((r4[0].w, r4[0].h), (800.0, 450.0));
        let r3 = grid(A, 3);
        // 2x2 격자에 마지막 줄 1개는 가운데
        assert_eq!(r3[2].x, 400.0);
    }

    #[test]
    fn soop_profiles_group_by_three() {
        let t = Tiles::default();
        let mut g = t.0.lock().unwrap();
        for id in 1..=4 {
            let profile = g.assign_profile(Platform::Soop);
            g.tiles.push(tile(id, Platform::Soop, profile));
        }
        let names: Vec<_> = g.tiles.iter().map(|t| t.profile.as_str()).collect();
        assert_eq!(names, ["soop-1", "soop-1", "soop-1", "soop-2"]);
        assert_eq!(g.assign_profile(Platform::Twitch), "default");
    }

    #[test]
    fn overlay_script_carries_token() {
        let g = Tiles::default();
        let g = g.0.lock().unwrap();
        let scripts = g.tile_scripts(Platform::Youtube, Quality::Best);
        let overlay = scripts.iter().find(|s| s.source.contains("__mvOverlay")).unwrap();
        assert!(!overlay.source.contains("__MV_TOKEN__"));
        assert!(overlay.source.contains(token()));
        assert_eq!(token().len(), 32);
    }

    #[test]
    fn quality_mode_reaches_script_and_cycles() {
        let t = Tiles::default();
        let mut g = t.0.lock().unwrap();
        let scripts = g.tile_scripts(Platform::Chzzk, Quality::Low);
        assert!(scripts.iter().any(|s| s.source.contains("let mode = 'low'")));
        assert!(!scripts.iter().any(|s| s.source.contains("__MV_QUALITY__")));

        assert_eq!(Quality::Best.next(), Quality::Auto);
        assert_eq!(Quality::Auto.next(), Quality::Low);
        assert_eq!(Quality::Low.next(), Quality::Best);

        // 방송별 화질이 없으면 전체 화질을 따른다.
        g.quality = Quality::Auto;
        let mut tile = tile(1, Platform::Twitch, "default".into());
        assert_eq!(g.quality_of(&tile), Quality::Auto);
        tile.quality = Some(Quality::Low);
        assert_eq!(g.quality_of(&tile), Quality::Low);
    }

    #[test]
    fn target_falls_back_to_first_tile() {
        let t = Tiles::default();
        let mut g = t.0.lock().unwrap();
        assert_eq!(g.target(), None);
        g.tiles.push(tile(7, Platform::Kick, "default".into()));
        assert_eq!(g.target(), Some(7));
        g.selected = Some(9);
        assert_eq!(g.target(), Some(9));
    }
}
