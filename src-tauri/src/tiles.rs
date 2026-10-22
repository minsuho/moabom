//! 타일(방송 하나 = 웹뷰 하나) 관리: 생성/삭제, 격자 배치, 창 분리, 채팅, 음소거, 단축키 명령.

use std::collections::BTreeMap;
use std::sync::{Mutex, OnceLock};

use serde::Serialize;
use tauri::{
    webview::{Color, NewWindowFeatures, NewWindowResponse, PageLoadEvent},
    AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, Url, Webview, WebviewBuilder,
    WebviewUrl, WebviewWindowBuilder, Window, WindowEvent, Wry,
};

use crate::platforms::{self, Platform};
use crate::store::{self, Group, Saved, SavedTile, WinGeom};

pub const MAIN: &str = "main";
pub const UI: &str = "ui";
pub const BAR_H: f64 = 52.0;
const CHAT_W: f64 = 340.0;
const SOOP_PER_PROFILE: usize = 3;
/// 킥은 화면 폭이 이보다 좁으면 플레이어 조작 바(화질 메뉴 포함)를 만들지 않는다.
const KICK_MIN_WIDTH: f64 = 1024.0;
const BROWSER_ARGS: &str = "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection \
                            --autoplay-policy=no-user-gesture-required";
const VIDEO_ONLY_JS: &str = include_str!(concat!(env!("MOABOM_ASSETS_DIR"), "/inject/video_only.js"));
const CHZZK_HQ_JS: &str = include_str!(concat!(env!("MOABOM_ASSETS_DIR"), "/inject/chzzk_hq.js"));
const CHZZK_SD_JS: &str = include_str!(concat!(env!("MOABOM_ASSETS_DIR"), "/inject/chzzk_sd.js"));
const SOOP_CHAT_JS: &str = include_str!(concat!(env!("MOABOM_ASSETS_DIR"), "/inject/soop_chat.js"));
const WATCHDOG_JS: &str = include_str!(concat!(env!("MOABOM_ASSETS_DIR"), "/inject/watchdog.js"));
const OVERLAY_JS: &str = include_str!(concat!(env!("MOABOM_ASSETS_DIR"), "/inject/overlay.js"));
const ASSIST_JS: &str = include_str!(concat!(env!("MOABOM_ASSETS_DIR"), "/inject/assist.js"));
const QUALITY_JS: &str = include_str!(concat!(env!("MOABOM_ASSETS_DIR"), "/inject/quality.js"));

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
    /// 붙어 있는 창. 0은 메인 창, 1부터는 따로 띄운 창 번호다.
    window: u32,
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
    /// 방송을 열 때 입력한 값. 다시 열 때(마지막 방송·그룹) 이 값을 다시 해석한다.
    #[serde(skip)]
    input: String,
    /// 플랫폼 API에서 읽은 방송인 이름. 꺼진 방송은 페이지에서 이름을 읽을 수 없어 이걸 쓴다.
    #[serde(skip)]
    nick: Option<String>,
    /// 이 방송의 음량(0~100). 페이지의 영상 음량을 바꾼다.
    volume: u8,
    /// 선택하지 않아도 소리를 같이 낸다.
    mix: bool,
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

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Layout {
    Grid,
    /// 선택한 방송을 크게, 나머지는 오른쪽 세로 줄로
    Focus,
    /// 선택한 방송을 크게, 나머지는 아래 가로 줄로
    Bottom,
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
    restore: bool,
    notify: bool,
    groups: Vec<Group>,
    /// 열려 있는 따로 띄운 창 번호들
    windows: Vec<u32>,
}

pub struct Tiles(Mutex<Inner>);

struct Inner {
    tiles: Vec<Tile>,
    next_id: u32,
    /// 소리와 단축키의 대상이 되는 타일
    selected: Option<u32>,
    /// 선택한 타일까지 포함해 전부 음소거
    muted: bool,
    layout: Layout,
    /// 전체 화면인 창 번호들(0은 메인 창)
    fullscreen: Vec<u32>,
    /// 검색창 아래에 펼친 검색 결과 패널(창 기준 좌표). 방송 위에 겹쳐 그린다.
    panel: Option<Rect>,
    /// 마지막 배치에서 각 타일의 (타일, 창 번호, 창 안 위치). 끌어서 옮길 때 놓을 곳을 찾는 데 쓴다.
    rects: Vec<(u32, u32, Rect)>,
    /// 끌고 있는 타일과, 지금 마우스 아래에 있는(놓으면 자리를 바꿀) 타일
    drag: Option<(u32, Option<u32>)>,
    split_soop: bool,
    chzzk_hq: bool,
    /// 모든 방송에 적용하는 기본 화질(방송별로 따로 정할 수 있다)
    quality: Quality,
    /// F 키로 격자에서 바꿀 때 쓸 크게 보기 배치(마지막으로 고른 것)
    big_layout: Layout,
    restore: bool,
    notify: bool,
    groups: Vec<Group>,
    /// 마지막 방송을 다시 여는 중. 다 열기 전에는 저장하지 않는다(중간에 꺼지면 목록이 줄어든다).
    restoring: bool,
    /// 열려 있는 따로 띄운 창 번호들(작은 번호부터)
    windows: Vec<u32>,
    /// 창 번호별 마지막 위치·크기. 창을 닫아도 남겨 두고, 같은 번호 창을 다시 열 때 쓴다.
    geoms: BTreeMap<u32, WinGeom>,
    /// 마지막으로 포커스를 받은 따로 띄운 창
    last_window: u32,
}

impl Default for Tiles {
    fn default() -> Self {
        Tiles(Mutex::new(Inner {
            tiles: Vec::new(),
            next_id: 0,
            selected: None,
            muted: false,
            layout: Layout::Grid,
            fullscreen: Vec::new(),
            panel: None,
            rects: Vec::new(),
            drag: None,
            split_soop: true,
            chzzk_hq: true,
            quality: Quality::Auto,
            big_layout: Layout::Focus,
            restore: true,
            notify: true,
            groups: Vec::new(),
            restoring: false,
            windows: Vec::new(),
            geoms: BTreeMap::new(),
            last_window: 0,
        }))
    }
}

impl Inner {
    fn snapshot(&self) -> Snapshot {
        Snapshot {
            tiles: self.tiles.clone(),
            selected: self.selected,
            muted: self.muted,
            chat_shown: self.chat_shown(),
            layout: self.layout,
            fullscreen: self.fullscreen.contains(&0),
            split_soop: self.split_soop,
            chzzk_hq: self.chzzk_hq,
            quality: self.quality,
            restore: self.restore,
            notify: self.notify,
            groups: self.groups.clone(),
            windows: self.windows.clone(),
        }
    }

    fn saved_tiles(&self) -> Vec<SavedTile> {
        self.tiles
            .iter()
            .map(|t| SavedTile {
                input: t.input.clone(),
                platform: t.platform,
                name: t.name.clone().or_else(|| t.nick.clone()),
                quality: t.quality,
                volume: t.volume,
                mix: t.mix,
                window: t.window,
            })
            .collect()
    }

    fn saved(&self) -> Saved {
        Saved {
            restore: self.restore,
            notify: self.notify,
            layout: self.layout,
            quality: self.quality,
            split_soop: self.split_soop,
            chzzk_hq: self.chzzk_hq,
            tiles: self.saved_tiles(),
            selected: self.tiles.iter().position(|t| Some(t.id) == self.selected),
            groups: self.groups.clone(),
            windows: self.geoms.clone(),
        }
    }

    /// 실제로 소리를 내는 방송인지: 전체 음소거가 아니고, 선택했거나 같이 듣기를 켠 방송.
    fn audible(&self, t: &Tile) -> bool {
        !self.muted && (self.selected == Some(t.id) || t.mix)
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

    fn in_window(&self, n: u32) -> impl Iterator<Item = &Tile> {
        self.tiles.iter().filter(move |t| t.window == n)
    }

    /// 컨트롤 바 채팅 드롭다운에 보일 채팅: 메인 창에 열린 채팅, 없으면 따로 띄운 창에 열린 채팅.
    fn chat_shown(&self) -> Option<u32> {
        let open = |main: bool| self.tiles.iter().find(|t| t.chat_open && (t.window == 0) == main);
        open(true).or_else(|| open(false)).map(|t| t.id)
    }

    /// 새로 띄울 창 번호: 쓰고 있지 않은 가장 작은 번호.
    fn free_window(&self) -> u32 {
        (1..).find(|n| !self.windows.contains(n)).unwrap()
    }

    /// 새 방송을 열 창. 메인 창에 방송이 있거나 따로 띄운 창이 없으면 메인 창,
    /// 아니면(방송을 모두 떼어 냈으면) 마지막으로 본 따로 띄운 창.
    fn add_target(&self) -> u32 {
        if self.windows.is_empty() || self.in_window(0).next().is_some() {
            0
        } else if self.windows.contains(&self.last_window) {
            self.last_window
        } else {
            self.windows[0]
        }
    }

    /// 따로 띄운 창의 제목: 들어 있는 방송 이름들. 방송이 하나면 방송 제목도 붙인다.
    fn window_title(&self, n: u32) -> String {
        let name = |t: &Tile| t.name.clone().unwrap_or_else(|| t.channel.clone());
        let tiles: Vec<&Tile> = self.in_window(n).collect();
        let body = match tiles.as_slice() {
            [t] => match &t.title {
                Some(title) => format!("{} - {title}", name(t)),
                None => name(t),
            },
            _ => tiles.iter().map(|t| name(t)).collect::<Vec<_>>().join(", "),
        };
        format!("{body} · {} {n}번 창", crate::APP_NAME)
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
/// 따로 띄운 창(번호 n)의 이름
fn window_label(n: u32) -> String {
    format!("grid-{n}")
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
        // 숲은 방송이 끝나면 다시보기(VOD) 페이지로 넘어가 자동 재생한다. 종료 화면 뒤에서 소리가 나므로 막는다.
        .on_navigation(|url| !is_vod_autoplay(url))
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

/// 방송이 끝났을 때 숲이 이동시키는 다시보기 자동 재생 주소인지.
fn is_vod_autoplay(url: &Url) -> bool {
    url.host_str().is_some_and(|h| h.starts_with("vod.sooplive") || h.starts_with("vod.afreecatv"))
        && url.query_pairs().any(|(k, _)| k == "autoplay_from_live")
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
                Ok(t) if t.chat_open => t.clone(),
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
    let _ = wv.eval(format!("window.__mvSetVolume && window.__mvSetVolume({})", tile.volume));
    push_overlay(app);
}

/// 타일이 지금 붙어 있는 창(메인 창 또는 따로 띄운 창).
fn host_window(app: &AppHandle, tile: &Tile) -> Result<Window, String> {
    window_of(app, tile.window).ok_or_else(|| "창을 찾을 수 없습니다".to_string())
}

/// 창 번호(0은 메인 창)로 창을 찾는다.
fn window_of(app: &AppHandle, n: u32) -> Option<Window> {
    if n == 0 { app.get_window(MAIN) } else { app.get_window(&window_label(n)) }
}

/// 상태가 바뀔 때마다 부른다. 컨트롤 바에 알리고 설정 파일에도 저장한다.
fn emit(app: &AppHandle) {
    let (snap, saved, titles) = {
        let state = app.state::<Tiles>();
        let g = state.0.lock().unwrap();
        let titles: Vec<(u32, String)> = g.windows.iter().map(|&n| (n, g.window_title(n))).collect();
        (g.snapshot(), (!g.restoring).then(|| g.saved()), titles)
    };
    for (n, title) in titles {
        if let Some(win) = window_of(app, n) {
            let _ = win.set_title(&title);
        }
    }
    let _ = app.emit_to(UI, "tiles", snap);
    if let Some(saved) = saved {
        store::save(app, &saved);
    }
}

fn toast(app: &AppHandle, msg: &str) {
    let _ = app.emit_to(UI, "toast", msg);
}

fn refresh(app: &AppHandle) {
    prune_windows(app);
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
                    "chat": t.chat_open,
                    "detached": t.window != 0,
                    "videoOnly": t.video_only,
                    "fullscreen": g.fullscreen.contains(&t.window),
                    "quality": g.quality_of(t).as_str(),
                    "offline": t.live == Some(false),
                    "image": t.image,
                    "nick": t.nick,
                    "volume": t.volume,
                    "mix": t.mix,
                    "audible": g.audible(t),
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

/// 오버레이에서 타일을 끄는 중이면 마우스 아래 타일을 표시하고, 놓으면 그 자리로 옮긴다.
/// 같은 창의 방송 위면 두 방송 자리를 바꾸고, 다른 창의 방송 위면 그 창의 그 자리로,
/// 다른 창의 빈 곳(방송이 없는 메인 창 포함)이면 그 창 끝으로 옮긴다.
/// 좌표는 끄는 타일 기준(CSS 픽셀)이고 창마다 화면 배율이 다를 수 있어, 화면 물리 좌표로 바꿔 비교한다.
fn on_drag(app: &AppHandle, id: u32, data: &serde_json::Value) {
    let (x, y) = (data["x"].as_f64().unwrap_or(f64::NAN), data["y"].as_f64().unwrap_or(f64::NAN));
    let end = data["end"].as_bool().unwrap_or(false);
    let (rects, windows) = {
        let state = app.state::<Tiles>();
        let g = state.0.lock().unwrap();
        (g.rects.clone(), std::iter::once(0).chain(g.windows.iter().copied()).collect::<Vec<u32>>())
    };
    // 창마다 (번호, 안쪽 영역의 화면 좌표, 배율). 창 메서드는 잠금 밖에서 부른다.
    let frames: Vec<(u32, Rect, f64)> = windows
        .iter()
        .filter_map(|&n| {
            let win = window_of(app, n)?;
            let (pos, size, scale) = (win.inner_position().ok()?, win.inner_size().ok()?, win.scale_factor().ok()?);
            let r = Rect { x: pos.x as f64, y: pos.y as f64, w: size.width as f64, h: size.height as f64 };
            Some((n, r, scale))
        })
        .collect();
    let frame = |n: u32| frames.iter().find(|f| f.0 == n).map(|f| (f.1, f.2));
    let on_screen = |n: u32, r: Rect| frame(n).map(|(f, s)| Rect { x: f.x + r.x * s, y: f.y + r.y * s, w: r.w * s, h: r.h * s });
    let inside = |r: Rect, px: f64, py: f64| px >= r.x && px < r.x + r.w && py >= r.y && py < r.y + r.h;

    let Some(&(_, src, origin)) = rects.iter().find(|(t, _, _)| *t == id) else { return };
    let (Some((_, scale)), Some(o)) = (frame(src), on_screen(src, origin)) else { return };
    let (px, py) = (o.x + x * scale, o.y + y * scale);
    // 창이 겹쳐 있으면 마우스 아래에 실제로 보이는(맨 앞의) 창을 고른다.
    let hit = window_at(app, &frames, px, py);
    let target = hit.and_then(|h| {
        rects
            .iter()
            .filter(|(t, n, _)| *t != id && *n == h)
            .find(|(_, n, r)| on_screen(*n, *r).is_some_and(|r| inside(r, px, py)))
            .map(|(t, n, _)| (*t, *n))
    });
    let target_window = hit.filter(|&n| n != src);

    if !end {
        let changed = {
            let state = app.state::<Tiles>();
            let mut g = state.0.lock().unwrap();
            let next = Some((id, target.map(|(t, _)| t)));
            let changed = g.drag != next;
            g.drag = next;
            changed
        };
        if changed {
            push_overlay(app);
        }
        return;
    }
    let to = {
        let state = app.state::<Tiles>();
        let mut g = state.0.lock().unwrap();
        g.drag = None;
        let pos = |g: &Inner, id: u32| g.tiles.iter().position(|t| t.id == id);
        match (target, target_window) {
            (Some((dst, n)), _) if n == src => {
                if let (Some(i), Some(j)) = (pos(&g, id), pos(&g, dst)) {
                    g.tiles.swap(i, j);
                }
                None
            }
            (_, Some(n)) => {
                // 다른 창: 놓은 방송 자리(빈 곳이면 그 창의 마지막 방송 뒤)로 순서를 옮긴 뒤 창을 옮긴다.
                if let Some(i) = pos(&g, id) {
                    let tile = g.tiles.remove(i);
                    let at = match target {
                        Some((dst, _)) => pos(&g, dst),
                        None => g.tiles.iter().rposition(|t| t.window == n).map(|j| j + 1),
                    };
                    let at = at.unwrap_or(g.tiles.len());
                    g.tiles.insert(at, tile);
                }
                Some(n)
            }
            _ => None,
        }
    };
    match to {
        // 웹뷰를 다른 창으로 옮기는 일은 이 메시지 처리(메인 스레드)가 끝난 뒤에 한다.
        Some(n) => {
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = move_tiles(&app, &[id], n) {
                    toast(&app, &e);
                }
            });
        }
        None => refresh(app),
    }
}

/// 화면 물리 좌표 (px, py)에 보이는 우리 창 번호. 다른 프로그램 창이나 바탕 화면이면 None.
#[cfg(windows)]
fn window_at(app: &AppHandle, frames: &[(u32, Rect, f64)], px: f64, py: f64) -> Option<u32> {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::UI::WindowsAndMessaging::{GetAncestor, WindowFromPoint, GA_ROOT};
    // 방송 웹뷰는 창 안의 자식 창이므로 맨 위 부모 창으로 올라가 비교한다.
    let top = unsafe { GetAncestor(WindowFromPoint(POINT { x: px as i32, y: py as i32 }), GA_ROOT) };
    frames
        .iter()
        .map(|f| f.0)
        .find(|&n| window_of(app, n).and_then(|w| w.hwnd().ok()).is_some_and(|h| h.0 as isize == top.0 as isize))
}

#[cfg(not(windows))]
fn window_at(_app: &AppHandle, frames: &[(u32, Rect, f64)], px: f64, py: f64) -> Option<u32> {
    frames.iter().find(|(_, r, _)| px >= r.x && px < r.x + r.w && py >= r.y && py < r.y + r.h).map(|f| f.0)
}

fn set_meta(app: &AppHandle, id: u32, data: &serde_json::Value) {
    let clean = |v: &serde_json::Value| v.as_str().map(str::trim).filter(|s| !s.is_empty()).map(str::to_string);
    {
        let state = app.state::<Tiles>();
        let mut g = state.0.lock().unwrap();
        let Ok(t) = g.get_mut(id) else { return };
        // 꺼진 숲 방송은 페이지 제목에 아이디만 있다. 그때는 플랫폼에서 읽은 이름(또는 저장해 둔 이름)을 그대로 둔다.
        let name = clean(&data["name"]).filter(|n| !n.eq_ignore_ascii_case(&t.channel));
        t.name = name.or_else(|| t.nick.clone()).or_else(|| t.name.take());
        t.title = clean(&data["title"]);
    }
    // 따로 띄운 창 제목(방송 이름)도 emit에서 함께 바꾼다.
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

/// 선택한 타일(과 같이 듣기를 켠 타일)만 소리를 낸다. 사이트 볼륨과 상관없이 웹뷰 단위로 막는다.
fn apply_audio(app: &AppHandle) {
    let mutes: Vec<(u32, bool)> = {
        let state = app.state::<Tiles>();
        let g = state.0.lock().unwrap();
        g.tiles.iter().map(|t| (t.id, !g.audible(t))).collect()
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

/// 아래 줄 모드: 주 화면 하나를 위에 크게, 나머지는 아래 가로 줄로.
fn bottom(a: Rect, n: usize, main_idx: usize) -> Vec<Rect> {
    if n <= 1 {
        return grid(a, n);
    }
    let strip_h = (a.h * 0.25).max(120.0).min(a.h * 0.5);
    let main = Rect { h: a.h - strip_h, ..a };
    let side_w = a.w / (n - 1) as f64;
    let mut k = 0;
    (0..n)
        .map(|i| {
            if i == main_idx {
                main
            } else {
                let r = Rect { x: a.x + k as f64 * side_w, y: a.y + main.h, w: side_w, h: strip_h };
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
    // 따로 띄운 창들의 크기. 창 메서드는 잠금 밖에서 부른다(push_overlay 주석 참고).
    let windows = app.state::<Tiles>().0.lock().unwrap().windows.clone();
    let sizes: Vec<(u32, f64, f64)> = windows
        .iter()
        .filter_map(|&n| window_of(app, n).and_then(|win| logical_size(&win)).map(|(ww, wh)| (n, ww, wh)))
        .collect();

    // 배치는 잠금 안에서 계산만 하고, 실제 웹뷰 이동은 잠금을 푼 뒤에 한다(push_overlay 주석 참고).
    let mut moves: Vec<(String, Rect)> = Vec::new();
    let mut rects: Vec<(u32, u32, Rect)> = Vec::new();
    let kick: Vec<u32>;
    let panel: Option<Rect>;
    {
        let state = app.state::<Tiles>();
        let g = state.0.lock().unwrap();
        let main_empty = g.in_window(0).next().is_none();
        let main_full = g.fullscreen.contains(&0);

        // 전체 화면에서는 컨트롤 바를 숨기고 방송만 채운다.
        let top = if main_full { 0.0 } else { BAR_H };
        // 메인 창에 방송이 없으면 컨트롤 바 웹뷰가 창 전체를 채워 안내 화면을 보여준다.
        // 검색 결과가 열려 있으면 그 아래까지 늘리고, 바와 결과 패널 모양으로 잘라 방송 위에 겹친다.
        panel = g.panel.filter(|_| !main_empty && !main_full);
        let ui_h = match panel {
            _ if main_empty => h,
            Some(p) => top.max(p.y + p.h),
            None => top,
        };
        moves.push((UI.to_string(), Rect { x: 0.0, y: 0.0, w, h: ui_h.max(1.0) }));

        // 창마다 방송을 배치하고, 그 창에 열린 채팅은 오른쪽 채팅 영역에 둔다. 따로 띄운 창에는 컨트롤 바가 없다.
        let frames = std::iter::once((0, w, h, top)).chain(sizes.iter().map(|&(n, ww, wh)| (n, ww, wh, 0.0)));
        for (n, ww, wh, top) in frames {
            let tiles: Vec<&Tile> = g.in_window(n).collect();
            let chat = tiles.iter().find(|t| t.chat_open).map(|t| t.id);
            let dock = if chat.is_some() { CHAT_W.min(ww * 0.4) } else { 0.0 };
            let area = Rect { x: 0.0, y: top, w: (ww - dock).max(1.0), h: (wh - top).max(1.0) };
            for (t, r) in tiles.iter().zip(arrange(&g, area, &tiles)) {
                moves.push((tile_label(t.id), r));
                rects.push((t.id, n, r));
            }
            if let Some(id) = chat {
                moves.push((chat_label(id), Rect { x: ww - dock, y: top, w: dock, h: area.h }));
            }
        }
        kick = g.tiles.iter().filter(|t| t.platform == Platform::Kick).map(|t| t.id).collect();
    }
    app.state::<Tiles>().0.lock().unwrap().rects = rects;

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

/// 한 창 안의 방송 자리를 정한다. 크게 볼 방송은 선택한 방송이고, 그 창에 없으면 첫 방송이다.
fn arrange(g: &Inner, area: Rect, tiles: &[&Tile]) -> Vec<Rect> {
    let main_idx = tiles.iter().position(|t| Some(t.id) == g.selected).unwrap_or(0);
    match g.layout {
        Layout::Grid => grid(area, tiles.len()),
        Layout::Focus => focus(area, tiles.len(), main_idx),
        Layout::Bottom => bottom(area, tiles.len(), main_idx),
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

/// 따로 띄운 창 n을 연다(이미 있으면 그 창, 0이면 메인 창). 기억해 둔 위치·크기가 있으면 그 자리에 연다.
fn ensure_window(app: &AppHandle, n: u32) -> Result<Window, String> {
    if let Some(win) = window_of(app, n) {
        return Ok(win);
    }
    let geom = app.state::<Tiles>().0.lock().unwrap().geoms.get(&n).copied();
    let win = tauri::window::WindowBuilder::new(app, window_label(n))
        .title(format!("{} {n}번 창", crate::APP_NAME))
        .inner_size(1280.0, 720.0)
        .min_inner_size(320.0, 180.0)
        .background_color(Color(0, 0, 0, 255))
        .theme(Some(tauri::Theme::Dark))
        .visible(false)
        .build()
        .map_err(err)?;
    restore_geometry(&win, geom);
    // 처음 여는 번호의 창은 메인 창에서 조금씩 비켜 띄워 창끼리 정확히 겹치지 않게 한다.
    if geom.is_none()
        && let Some(pos) = app.get_window(MAIN).and_then(|m| m.outer_position().ok())
    {
        let step = 40 * n as i32;
        let _ = win.set_position(tauri::PhysicalPosition::new(pos.x + step, pos.y + step));
    }
    let _ = win.show();
    {
        let state = app.state::<Tiles>();
        let mut g = state.0.lock().unwrap();
        if let Err(i) = g.windows.binary_search(&n) {
            g.windows.insert(i, n);
        }
        g.last_window = n;
    }
    let app2 = app.clone();
    win.on_window_event(move |e| match e {
        WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => {
            remember_geometry(&app2, n);
            relayout(&app2);
        }
        WindowEvent::Moved(_) => remember_geometry(&app2, n),
        WindowEvent::Focused(true) => app2.state::<Tiles>().0.lock().unwrap().last_window = n,
        WindowEvent::CloseRequested { api, .. } => {
            // 창을 닫으면 방송을 끄지 않고 메인 창으로 돌려보낸다.
            api.prevent_close();
            let app3 = app2.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = merge_window(&app3, n) {
                    toast(&app3, &e);
                }
            });
        }
        _ => {}
    });
    Ok(win)
}

/// 방송들을 창 `to`(0은 메인 창)로 옮긴다. 웹뷰를 새로 만들지 않고 옮겨 붙이므로 방송이 끊기지 않고,
/// 같은 방송이 두 창에서 재생되는 일도 없다. 열려 있던 채팅도 함께 옮기고, 비게 된 창은 닫는다.
fn move_tiles(app: &AppHandle, ids: &[u32], to: u32) -> Result<(), String> {
    // 창을 띄우기 전에 소속부터 바꿔 둔다. 그사이 다른 곳에서 정리가 돌아도 새 창이 빈 창으로 보여 닫히지 않는다.
    let (moving, replaced) = {
        let state = app.state::<Tiles>();
        let mut g = state.0.lock().unwrap();
        let mut moving = Vec::new();
        for t in g.tiles.iter_mut().filter(|t| ids.contains(&t.id) && t.window != to) {
            moving.push((t.clone(), t.window));
            t.window = to;
        }
        // 창마다 채팅 영역에는 채팅 하나만 보인다. 옮겨 가는 채팅이 있으면 첫 채팅만 남기고 나머지는 닫는다.
        let kept = moving.iter().find(|(t, _)| t.chat_open).map(|(t, _)| t.id);
        let replaced: Vec<u32> = match kept {
            Some(kept) => g.in_window(to).filter(|t| t.chat_open && t.id != kept).map(|t| t.id).collect(),
            None => Vec::new(),
        };
        (moving, replaced)
    };
    if moving.is_empty() {
        return Ok(());
    }
    let host = match ensure_window(app, to) {
        Ok(host) => host,
        Err(e) => {
            {
                let state = app.state::<Tiles>();
                let mut g = state.0.lock().unwrap();
                for (t, from) in &moving {
                    if let Ok(tile) = g.get_mut(t.id) {
                        tile.window = *from;
                    }
                }
            }
            refresh(app);
            return Err(format!("창을 띄우지 못했습니다: {e}"));
        }
    };
    for id in replaced {
        close_chat(app, id);
    }
    let mut first_err = None;
    for (t, _) in &moving {
        if let Some(wv) = app.get_webview(&tile_label(t.id))
            && let Err(e) = wv.reparent(&host)
        {
            first_err.get_or_insert(err(e));
        }
        // 위에서 닫은 채팅은 웹뷰가 없어 그냥 넘어간다.
        if let Err(e) = move_chat(app, t, &host) {
            first_err.get_or_insert(e);
        }
    }
    refresh(app);
    first_err.map_or(Ok(()), |e| Err(format!("방송을 옮기지 못했습니다: {e}")))
}

/// 방송이 하나도 없는 따로 띄운 창은 닫는다. 메인 창에 방송이 없으면 메인 창 전체 화면도 푼다.
fn prune_windows(app: &AppHandle) {
    let (empty, unfull_main) = {
        let state = app.state::<Tiles>();
        let mut g = state.0.lock().unwrap();
        let empty: Vec<u32> = g.windows.iter().copied().filter(|&n| g.in_window(n).next().is_none()).collect();
        let unfull_main = g.in_window(0).next().is_none() && g.fullscreen.contains(&0);
        if unfull_main {
            g.fullscreen.retain(|&f| f != 0);
        }
        (empty, unfull_main)
    };
    for n in empty {
        remember_geometry(app, n);
        {
            let state = app.state::<Tiles>();
            let mut g = state.0.lock().unwrap();
            g.windows.retain(|&w| w != n);
            g.fullscreen.retain(|&f| f != n);
        }
        if let Some(win) = window_of(app, n) {
            let _ = win.destroy();
        }
    }
    if unfull_main && let Some(main) = app.get_window(MAIN) {
        let _ = main.set_fullscreen(false);
    }
}

/// 따로 띄운 창 n의 방송을 모두 메인 창으로 돌려보낸다(창은 비면서 닫힌다).
fn merge_window(app: &AppHandle, n: u32) -> Result<(), String> {
    let ids: Vec<u32> = app.state::<Tiles>().0.lock().unwrap().in_window(n).map(|t| t.id).collect();
    move_tiles(app, &ids, 0)
}

/// 따로 띄운 창의 방송을 모두 메인 창으로 합친다.
fn merge_all(app: &AppHandle) -> Result<(), String> {
    let ids: Vec<u32> = app.state::<Tiles>().0.lock().unwrap().tiles.iter().filter(|t| t.window != 0).map(|t| t.id).collect();
    move_tiles(app, &ids, 0)
}

/// 격자 분리(G): 따로 띄운 창의 방송에서 누르면 그 창을 메인 창으로 합친다.
/// 메인 창에서 누르면 메인 창 방송을 통째로 새 창으로 떼어 내고, 메인 창에 방송이 없으면 모든 창을 합친다.
fn toggle_grid(app: &AppHandle, source: Option<u32>) -> Result<(), String> {
    let (from, main_ids, free, any_window) = {
        let state = app.state::<Tiles>();
        let g = state.0.lock().unwrap();
        let from = source.and_then(|s| g.get(s).ok()).map(|t| t.window);
        let main_ids: Vec<u32> = g.in_window(0).map(|t| t.id).collect();
        (from, main_ids, g.free_window(), !g.windows.is_empty())
    };
    match from {
        Some(n) if n != 0 => merge_window(app, n),
        _ if !main_ids.is_empty() => move_tiles(app, &main_ids, free),
        _ if any_window => merge_all(app),
        _ => Err("분리할 방송이 없습니다. 방송을 먼저 추가하세요.".into()),
    }
}

/// 창 n의 위치·크기를 기억한다. 최대화했으면 그 전 위치·크기는 그대로 두고 최대화 여부만 바꾼다.
fn remember_geometry(app: &AppHandle, n: u32) {
    let Some(win) = window_of(app, n).filter(|_| n != 0) else { return };
    if win.is_minimized().unwrap_or(false) || win.is_fullscreen().unwrap_or(false) {
        return;
    }
    let maximized = win.is_maximized().unwrap_or(false);
    let rect = (!maximized).then(|| win.outer_position().ok().zip(win.inner_size().ok())).flatten();
    let state = app.state::<Tiles>();
    let mut g = state.0.lock().unwrap();
    match (rect, g.geoms.get_mut(&n)) {
        (Some((pos, size)), _) => {
            g.geoms.insert(n, WinGeom { x: pos.x, y: pos.y, w: size.width, h: size.height, maximized: false });
        }
        (None, Some(geom)) => geom.maximized = maximized,
        (None, None) => {}
    }
}

/// 기억해 둔 위치·크기로 창을 옮긴다. 그 자리에 모니터가 없으면(모니터를 뺐으면) 기본 위치에 둔다.
fn restore_geometry(win: &Window, geom: Option<WinGeom>) {
    let Some(geom) = geom else { return };
    // 창 위쪽(제목 줄)이 어느 모니터 안에 있어야 잡아서 옮길 수 있다.
    let monitors = win.available_monitors().unwrap_or_default();
    let Some(monitor) = monitors.iter().find(|m| {
        let (p, s) = (m.position(), m.size());
        let (right, bottom) = (p.x + s.width as i32, p.y + s.height as i32);
        geom.x + 100 > p.x && geom.x < right - 100 && geom.y >= p.y - 10 && geom.y + 40 < bottom
    }) else {
        return;
    };
    // 해상도를 낮췄거나 값이 잘못돼도 그 모니터보다 커지지 않게 한다.
    let max = monitor.size();
    let _ = win.set_size(tauri::PhysicalSize::new(
        geom.w.clamp(320, max.width.max(320)),
        geom.h.clamp(180, max.height.max(180)),
    ));
    let _ = win.set_position(tauri::PhysicalPosition::new(geom.x, geom.y));
    if geom.maximized {
        let _ = win.maximize();
    }
}

/// 종료하기 전에 따로 띄운 창 위치까지 포함해 설정을 저장한다.
pub fn save_now(app: &AppHandle) {
    let windows = app.state::<Tiles>().0.lock().unwrap().windows.clone();
    for n in windows {
        remember_geometry(app, n);
    }
    let saved = {
        let state = app.state::<Tiles>();
        let g = state.0.lock().unwrap();
        (!g.restoring).then(|| g.saved())
    };
    if let Some(saved) = saved {
        store::save(app, &saved);
    }
}

// ---------- 타일 조작 ----------

async fn remove_tile(app: &AppHandle, id: u32) -> Result<(), String> {
    close_chat(app, id);
    if let Some(wv) = app.get_webview(&tile_label(id)) {
        let _ = wv.close();
    }
    {
        let state = app.state::<Tiles>();
        let mut g = state.0.lock().unwrap();
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

/// 창 분리(D): 메인 창의 방송은 새 창으로 떼어 내고, 따로 띄운 창의 방송은 메인 창으로 돌려보낸다.
fn toggle_detach(app: &AppHandle, id: u32) -> Result<(), String> {
    let to = {
        let state = app.state::<Tiles>();
        let g = state.0.lock().unwrap();
        if g.get(id)?.window == 0 { g.free_window() } else { 0 }
    };
    move_tiles(app, &[id], to)
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

/// 채팅 켜기/끄기. 채팅은 방송이 있는 창 오른쪽 채팅 영역에 뜨고 창마다 하나만 보인다.
/// 같은 창에 다른 방송 채팅이 열려 있으면 그 채팅은 닫는다.
fn toggle_chat(app: &AppHandle, id: u32) -> Result<(), String> {
    let (tile, others) = {
        let state = app.state::<Tiles>();
        let g = state.0.lock().unwrap();
        let tile = g.get(id)?.clone();
        let others: Vec<u32> = g.in_window(tile.window).filter(|t| t.chat_open && t.id != id).map(|t| t.id).collect();
        (tile, others)
    };
    if tile.chat_open {
        close_chat(app, id);
    } else {
        for other in others {
            close_chat(app, other);
        }
        // 숲 채팅은 팝업이 뜬 뒤에야 붙으므로 미리 열린 것으로 표시해 둔다(채팅 영역 자리를 잡고, 팝업을 받을지 판단한다).
        if let Ok(t) = app.state::<Tiles>().0.lock().unwrap().get_mut(id) {
            t.chat_open = true;
        }
        if let Err(e) = open_chat(app, &tile) {
            close_chat(app, id);
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
    let (changed, came_back, notify) = {
        let state = app.state::<Tiles>();
        let mut g = state.0.lock().unwrap();
        let notify = g.notify;
        let Ok(t) = g.get_mut(id) else { return };
        let before = t.live;
        t.live = Some(status.live);
        if status.image.is_some() {
            t.image = status.image;
        }
        let nick_changed = status.name.is_some() && t.nick != status.name;
        if nick_changed {
            t.nick = status.name;
            if t.name.as_deref().is_none_or(|n| n.eq_ignore_ascii_case(&t.channel)) {
                t.name = t.nick.clone();
            }
        }
        let came_back = before == Some(false) && status.live;
        let name = t.name.clone().or_else(|| t.nick.clone()).unwrap_or_else(|| t.channel.clone());
        (before != t.live || nick_changed, came_back, (came_back && notify).then_some(name))
    };
    if came_back {
        // 꺼져 있는 동안 페이지가 다른 곳으로 갔을 수 있어 새로고침이 아니라 방송 주소로 다시 이동한다.
        let url = app.state::<Tiles>().0.lock().unwrap().get(id).ok().and_then(|t| t.url.parse().ok());
        if let (Some(wv), Some(url)) = (app.get_webview(&tile_label(id)), url) {
            let _ = wv.navigate(url);
        }
    }
    if let Some(name) = notify {
        notify_live(app, &name);
    }
    if changed {
        push_overlay(app);
        emit(app);
    }
}

/// 꺼져 있던 방송이 시작되면 Windows 알림을 띄운다.
fn notify_live(app: &AppHandle, name: &str) {
    crate::notify::show(app, &format!("{name} 방송이 시작됐어요"), "모아봄에서 자동으로 연결했어요.");
}

fn reload_tile(app: &AppHandle, id: u32) -> Result<(), String> {
    let wv = app.get_webview(&tile_label(id)).ok_or("타일이 없습니다")?;
    wv.eval("location.reload()").map_err(err)
}

/// 이 방송 음량을 바꾼다(0~100). 소리를 낼지는 선택·같이 듣기가 정한다.
fn set_volume(app: &AppHandle, id: u32, volume: i64) {
    let volume = volume.clamp(0, 100) as u8;
    if let Ok(t) = app.state::<Tiles>().0.lock().unwrap().get_mut(id) {
        t.volume = volume;
    }
    if let Some(wv) = app.get_webview(&tile_label(id)) {
        let _ = wv.eval(format!("window.__mvSetVolume && window.__mvSetVolume({volume})"));
    }
    push_overlay(app);
    emit(app);
}

/// 사이트 음소거 버튼을 눌렀을 때: 켜면 이 방송 소리를 켜고(선택), 끄면 이 방송 소리를 끈다.
/// 같이 듣기로 듣던 방송이면 같이 듣기만 끄고, 선택한 방송이면 전체 음소거로 바꾼다(스피커 버튼과 같다).
fn set_audible(app: &AppHandle, id: u32, on: bool) -> Result<(), String> {
    {
        let state = app.state::<Tiles>();
        let mut g = state.0.lock().unwrap();
        let tile = g.get(id)?.clone();
        if g.audible(&tile) == on {
            return Ok(());
        }
        if on {
            g.selected = Some(id);
            g.muted = false;
        } else if tile.mix && g.selected != Some(id) {
            g.get_mut(id)?.mix = false;
        } else {
            g.muted = true;
        }
    }
    refresh(app);
    Ok(())
}

/// 같이 듣기: 선택한 방송이 아니어도 이 방송 소리를 함께 낸다(합방을 볼 때).
fn toggle_mix(app: &AppHandle, id: u32) -> Result<(), String> {
    {
        let state = app.state::<Tiles>();
        let mut g = state.0.lock().unwrap();
        let t = g.get_mut(id)?;
        t.mix = !t.mix;
        // 같이 듣기를 켰는데 전체 음소거 중이면 들리지 않으므로 음소거를 푼다.
        if t.mix {
            g.muted = false;
        }
    }
    refresh(app);
    Ok(())
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

/// 창 n(0은 메인 창)을 전체 화면으로 만들거나 푼다.
fn set_fullscreen(app: &AppHandle, n: u32, on: bool) {
    if let Some(win) = window_of(app, n) {
        let _ = win.set_fullscreen(on);
    }
    {
        let state = app.state::<Tiles>();
        let mut g = state.0.lock().unwrap();
        g.fullscreen.retain(|&f| f != n);
        if on {
            g.fullscreen.push(n);
        }
    }
    refresh(app);
}

/// 컨트롤 바의 주소·검색 입력칸으로 키보드 포커스를 옮긴다.
fn focus_add_input(app: &AppHandle) {
    // 따로 띄운 창이 전체 화면이어도 메인 창만 앞으로 가져오면 되므로, 메인 창이 전체 화면일 때만 푼다.
    if app.state::<Tiles>().0.lock().unwrap().fullscreen.contains(&0) {
        set_fullscreen(app, 0, false);
    }
    if let Some(main) = app.get_window(MAIN) {
        let _ = main.unminimize();
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
                g.layout = if g.layout == Layout::Grid { g.big_layout } else { Layout::Grid };
                if g.selected.is_none() {
                    g.selected = id;
                }
            }
            refresh(&app);
        }
        "fullscreen" | "exitfullscreen" => {
            // 방송에서 누르면 그 방송이 있는 창을, 컨트롤 바에서 누르면 메인 창을 전체 화면으로 만든다.
            // 메인 창에 방송이 없으면(모두 떼어 냈으면) 마지막으로 본 창을 전체 화면으로 만든다.
            let (n, on) = {
                let state = app.state::<Tiles>();
                let g = state.0.lock().unwrap();
                let n = source.and_then(|s| g.get(s).ok()).map_or_else(|| g.add_target(), |t| t.window);
                (n, cmd == "fullscreen" && !g.fullscreen.contains(&n))
            };
            set_fullscreen(&app, n, on);
        }
        "add" => focus_add_input(&app),
        "grid" => toggle_grid(&app, source)?,
        "mergeall" => merge_all(&app)?,
        // 메인 창 안내 화면의 '창 보기': 따로 띄운 창을 모두 앞으로 가져온다.
        "showgrid" => {
            let windows = app.state::<Tiles>().0.lock().unwrap().windows.clone();
            for n in windows {
                if let Some(win) = window_of(&app, n) {
                    let _ = win.unminimize();
                    let _ = win.set_focus();
                }
            }
        }
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
                "volume" => set_volume(&app, id, arg),
                "mix" => toggle_mix(&app, id)?,
                "setaudible" => set_audible(&app, id, arg != 0)?,
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
    open_tile(&app, &input, None)
}

/// 방송을 연다. `saved`가 있으면 저장해 둔 이름·화질·음량·같이 듣기도 되살린다.
fn open_tile(app: &AppHandle, input: &str, saved: Option<&SavedTile>) -> Result<(), String> {
    let r = platforms::resolve(input)?;
    let (tile, scripts) = {
        let state = app.state::<Tiles>();
        let mut g = state.0.lock().unwrap();
        g.next_id += 1;
        // 저장해 둔 방송은 있던 창에, 새 방송은 add_target이 고른 창에 연다.
        let window = saved.map_or_else(|| g.add_target(), |s| s.window);
        let tile = Tile {
            id: g.next_id,
            platform: r.platform,
            profile: g.assign_profile(r.platform),
            channel: r.channel,
            url: r.player_url,
            chat_url: r.chat_url,
            window,
            chat_open: false,
            video_only: true,
            // 저장해 둔 이름을 먼저 보여 주고, 페이지가 열리면 최신 이름으로 바뀐다.
            name: saved.and_then(|s| s.name.clone()),
            title: None,
            popup_chat: false,
            quality: saved.and_then(|s| s.quality),
            loaded: false,
            live: None,
            image: None,
            input: input.trim().to_string(),
            nick: None,
            volume: saved.map_or(100, |s| s.volume.min(100)),
            mix: saved.is_some_and(|s| s.mix),
        };
        g.tiles.push(tile.clone());
        if g.selected.is_none() {
            g.selected = Some(tile.id);
        }
        let scripts = g.tile_scripts(tile.platform, g.quality_of(&tile));
        (tile, scripts)
    };
    // 따로 띄운 창에 열 방송이면 그 창부터 띄운다.
    let host = match ensure_window(app, tile.window) {
        Ok(host) => host,
        Err(e) => {
            app.state::<Tiles>().0.lock().unwrap().tiles.retain(|t| t.id != tile.id);
            emit(app);
            return Err(e);
        }
    };
    let builder = build_webview(app, &tile_label(tile.id), &tile.url, &tile.profile, &scripts)?;
    match host.add_child(builder, LogicalPosition::new(0.0, BAR_H), LogicalSize::new(1.0, 1.0)) {
        Ok(wv) => {
            listen_messages(app, &wv, tile.id);
            watch_first_load(app, tile.id);
            let app = app.clone();
            tauri::async_runtime::spawn(async move { check_live(&app, tile.id).await });
        }
        Err(e) => {
            app.state::<Tiles>().0.lock().unwrap().tiles.retain(|t| t.id != tile.id);
            emit(app);
            return Err(err(e));
        }
    }
    refresh(app);
    Ok(())
}

/// 저장한 방송들을 차례로 연다. 실패한 방송은 건너뛰고 몇 개가 안 열렸는지 돌려준다.
fn open_saved(app: &AppHandle, tiles: &[SavedTile]) -> usize {
    let mut failed = 0;
    for s in tiles {
        if let Err(e) = open_tile(app, &s.input, Some(s)) {
            eprintln!("[moabom] {}: {e}", s.input);
            failed += 1;
        }
    }
    failed
}

/// 시작할 때 저장해 둔 설정을 적용하고, 켜 두었으면 마지막으로 보던 방송을 다시 연다.
pub fn restore_session(app: &AppHandle, reopen_tiles: bool) {
    let saved = store::load(app);
    let reopen = {
        let state = app.state::<Tiles>();
        let mut g = state.0.lock().unwrap();
        g.restore = saved.restore;
        g.notify = saved.notify;
        g.layout = saved.layout;
        if saved.layout != Layout::Grid {
            g.big_layout = saved.layout;
        }
        g.quality = saved.quality;
        g.split_soop = saved.split_soop;
        g.chzzk_hq = saved.chzzk_hq;
        g.groups = saved.groups;
        g.geoms = saved.windows;
        let reopen = reopen_tiles && saved.restore && !saved.tiles.is_empty();
        g.restoring = reopen;
        reopen.then_some((saved.tiles, saved.selected))
    };
    emit(app);
    // 따로 띄운 창에 있던 방송은 open_tile이 그 창을 같은 자리에 다시 띄워 연다.
    let Some((tiles, selected)) = reopen else { return };
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let failed = open_saved(&app, &tiles);
        {
            let state = app.state::<Tiles>();
            let mut g = state.0.lock().unwrap();
            // 열지 못한 방송이 있으면 순서가 어긋나므로 저장해 둔 입력값으로 찾는다.
            let want = selected.and_then(|i| tiles.get(i)).map(|s| s.input.trim().to_string());
            if let Some(id) = want.and_then(|w| g.tiles.iter().find(|t| t.input == w).map(|t| t.id)) {
                g.selected = Some(id);
            }
            g.restoring = false;
        }
        refresh(&app);
        if failed > 0 {
            toast(&app, &format!("지난번 방송 중 {failed}개를 열지 못했습니다."));
        }
    });
}

/// 단축키나 컨트롤 바 버튼에서 오는 명령. 대상은 선택한 타일이다.
#[tauri::command]
pub async fn command(app: AppHandle, cmd: String, arg: Option<i64>) -> Result<(), String> {
    run_command(app, None, None, cmd, arg.unwrap_or(0)).await
}

#[tauri::command]
pub async fn set_layout(app: AppHandle, layout: Layout) -> Result<(), String> {
    {
        let state = app.state::<Tiles>();
        let mut g = state.0.lock().unwrap();
        g.layout = layout;
        if layout != Layout::Grid {
            g.big_layout = layout;
        }
    }
    refresh(&app);
    Ok(())
}

/// 컨트롤 바의 채팅 드롭다운: 고른 방송의 채팅을 오른쪽 채팅 영역에 띄운다. `None`이면 채팅 영역을 닫는다.
#[tauri::command]
pub async fn show_chat(app: AppHandle, id: Option<u32>) -> Result<(), String> {
    let (shown, open) = {
        let state = app.state::<Tiles>();
        let g = state.0.lock().unwrap();
        (g.chat_shown(), id.and_then(|id| g.get(id).ok()).is_some_and(|t| t.chat_open))
    };
    match id {
        Some(id) if !open => toggle_chat(&app, id),
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

#[tauri::command]
pub async fn set_restore(app: AppHandle, on: bool) -> Result<(), String> {
    app.state::<Tiles>().0.lock().unwrap().restore = on;
    emit(&app);
    Ok(())
}

#[tauri::command]
pub async fn set_notify(app: AppHandle, on: bool) -> Result<(), String> {
    app.state::<Tiles>().0.lock().unwrap().notify = on;
    emit(&app);
    Ok(())
}

/// 지금 보는 방송들을 그룹으로 저장한다. 같은 이름이 있으면 덮어쓴다.
#[tauri::command]
pub async fn save_group(app: AppHandle, name: String) -> Result<(), String> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("그룹 이름을 입력하세요".into());
    }
    {
        let state = app.state::<Tiles>();
        let mut g = state.0.lock().unwrap();
        if g.tiles.is_empty() {
            return Err("저장할 방송이 없습니다. 방송을 먼저 추가하세요.".into());
        }
        let group = Group { name: name.clone(), tiles: g.saved_tiles() };
        match g.groups.iter_mut().find(|x| x.name == name) {
            Some(x) => *x = group,
            None => g.groups.push(group),
        }
    }
    emit(&app);
    Ok(())
}

#[tauri::command]
pub async fn delete_group(app: AppHandle, name: String) -> Result<(), String> {
    app.state::<Tiles>().0.lock().unwrap().groups.retain(|x| x.name != name);
    emit(&app);
    Ok(())
}

/// 그룹을 연다. `replace`면 지금 보는 방송을 모두 닫고 그룹 방송만 연다.
/// 아니면 이미 열려 있는 방송은 빼고 덧붙인다.
#[tauri::command]
pub async fn open_group(app: AppHandle, name: String, replace: bool) -> Result<(), String> {
    let (group, open) = {
        let state = app.state::<Tiles>();
        let g = state.0.lock().unwrap();
        let group = g.groups.iter().find(|x| x.name == name).cloned().ok_or("그룹이 없습니다")?;
        (group, g.tiles.iter().map(|t| (t.id, t.input.clone())).collect::<Vec<_>>())
    };
    let todo: Vec<SavedTile> = if replace {
        for (id, _) in &open {
            remove_tile(&app, *id).await?;
        }
        group.tiles
    } else {
        group.tiles.into_iter().filter(|s| !open.iter().any(|(_, input)| *input == s.input.trim())).collect()
    };
    let failed = open_saved(&app, &todo);
    if failed > 0 {
        toast(&app, &format!("그룹 방송 중 {failed}개를 열지 못했습니다."));
    }
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
            window: 0,
            chat_open: false,
            video_only: true,
            name: None,
            title: None,
            popup_chat: false,
            quality: None,
            loaded: false,
            live: None,
            image: None,
            input: String::new(),
            nick: None,
            volume: 100,
            mix: false,
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
        // Test behavior instead of relying on variable names/spacing removed by Release minification.
        let quality = scripts.iter().find(|s| s.source.contains("__mvSetQuality")).unwrap();
        use std::io::Write;
        use std::process::{Command, Stdio};
        let mut check = Command::new("node")
            .arg(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../scripts/check-quality.cjs"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn().expect("Node.js is required for the embedded-script behavior test");
        check.stdin.take().unwrap().write_all(quality.source.as_bytes()).unwrap();
        let output = check.wait_with_output().unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
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
    fn bottom_puts_others_in_a_row_below() {
        let r = bottom(A, 3, 1);
        assert_eq!((r[1].x, r[1].y, r[1].w), (0.0, 0.0, 1600.0));
        assert_eq!((r[0].x, r[0].y, r[0].w), (0.0, r[1].h, 800.0));
        assert_eq!((r[2].x, r[2].y), (800.0, r[1].h));
        assert_eq!(r[1].h + r[0].h, 900.0);
    }

    #[test]
    fn mix_tiles_are_audible_unless_muted() {
        let t = Tiles::default();
        let mut g = t.0.lock().unwrap();
        g.tiles.push(tile(1, Platform::Soop, "default".into()));
        g.tiles.push(tile(2, Platform::Chzzk, "default".into()));
        g.selected = Some(1);
        let (a, b) = (g.tiles[0].clone(), g.tiles[1].clone());
        assert!(g.audible(&a));
        assert!(!g.audible(&b));
        g.tiles[1].mix = true;
        let b = g.tiles[1].clone();
        assert!(g.audible(&b));
        g.muted = true;
        assert!(!g.audible(&b));
    }

    #[test]
    fn saved_keeps_order_and_selection() {
        let t = Tiles::default();
        let mut g = t.0.lock().unwrap();
        let mut a = tile(1, Platform::Soop, "soop-1".into());
        a.input = "soop:a".into();
        let mut b = tile(2, Platform::Kick, "default".into());
        b.input = "kick:b".into();
        b.volume = 30;
        g.tiles.extend([a, b]);
        g.selected = Some(2);
        let s = g.saved();
        let inputs: Vec<_> = s.tiles.iter().map(|t| t.input.as_str()).collect();
        assert_eq!(inputs, ["soop:a", "kick:b"]);
        assert_eq!(s.selected, Some(1));
        assert_eq!(s.tiles[1].volume, 30);
    }

    #[test]
    fn blocks_only_soop_vod_autoplay() {
        let u = |s: &str| Url::parse(s).unwrap();
        assert!(is_vod_autoplay(&u("https://vod.sooplive.com/player/209904703/?autoplay_from_live=1&volume=1&muted=0")));
        assert!(!is_vod_autoplay(&u("https://vod.sooplive.com/player/209904703")));
        assert!(!is_vod_autoplay(&u("https://play.sooplive.com/layule")));
    }

    #[test]
    fn new_tiles_go_to_main_unless_everything_is_detached() {
        let t = Tiles::default();
        let mut g = t.0.lock().unwrap();
        assert_eq!(g.add_target(), 0);
        g.windows = vec![1, 3];
        assert_eq!(g.free_window(), 2);
        // 메인 창에 방송이 없으면 마지막으로 본 창, 그 창이 닫혔으면 첫 창에 연다.
        g.last_window = 3;
        assert_eq!(g.add_target(), 3);
        g.last_window = 9;
        assert_eq!(g.add_target(), 1);
        g.tiles.push(tile(1, Platform::Kick, "default".into()));
        assert_eq!(g.add_target(), 0);
    }

    #[test]
    fn chat_dropdown_prefers_main_window_and_titles_list_names() {
        let t = Tiles::default();
        let mut g = t.0.lock().unwrap();
        let mut a = tile(1, Platform::Soop, "default".into());
        a.window = 2;
        a.chat_open = true;
        a.name = Some("가".into());
        let mut b = tile(2, Platform::Chzzk, "default".into());
        b.window = 2;
        b.channel = "nana".into();
        g.tiles.extend([a, b]);
        assert_eq!(g.chat_shown(), Some(1));
        let mut c = tile(3, Platform::Kick, "default".into());
        c.chat_open = true;
        g.tiles.push(c);
        assert_eq!(g.chat_shown(), Some(3));
        assert_eq!(g.window_title(2), format!("가, nana · {} 2번 창", crate::APP_NAME));
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
