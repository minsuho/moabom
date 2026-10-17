//! 설정, 마지막으로 보던 방송, 그룹을 설정 폴더의 settings.json에 저장하고 불러온다.

use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

use crate::platforms::Platform;
use crate::tiles::{Layout, Quality};

/// 저장해 둔 방송 하나. `input`을 다시 해석해 같은 방송을 연다.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedTile {
    pub input: String,
    pub platform: Platform,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub quality: Option<Quality>,
    #[serde(default = "full_volume")]
    pub volume: u8,
    #[serde(default)]
    pub mix: bool,
}

fn full_volume() -> u8 {
    100
}

/// 이름을 붙여 저장한 방송 묶음.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Group {
    pub name: String,
    pub tiles: Vec<SavedTile>,
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Saved {
    /// 시작할 때 마지막으로 보던 방송을 다시 연다.
    pub restore: bool,
    /// 꺼져 있던 방송이 시작되면 알림을 띄운다.
    pub notify: bool,
    pub layout: Layout,
    pub quality: Quality,
    pub split_soop: bool,
    pub chzzk_hq: bool,
    pub tiles: Vec<SavedTile>,
    /// 소리를 내던 방송의 순서(0부터)
    pub selected: Option<usize>,
    pub groups: Vec<Group>,
}

impl Default for Saved {
    fn default() -> Self {
        Saved {
            restore: true,
            notify: true,
            layout: Layout::Grid,
            quality: Quality::Auto,
            split_soop: true,
            chzzk_hq: true,
            tiles: Vec::new(),
            selected: None,
            groups: Vec::new(),
        }
    }
}

fn path(app: &AppHandle) -> Option<PathBuf> {
    app.path().app_config_dir().ok().map(|d| d.join("settings.json"))
}

/// 파일이 없으면 기본값을 쓴다. 파일이 깨졌으면 기본값으로 시작하되, 덮어쓰기 전에 옆에 백업해 둔다.
pub fn load(app: &AppHandle) -> Saved {
    let Some(path) = path(app) else { return Saved::default() };
    let Ok(text) = std::fs::read_to_string(&path) else { return Saved::default() };
    match serde_json::from_str(&text) {
        Ok(saved) => {
            *LAST.lock().unwrap() = text;
            saved
        }
        Err(e) => {
            eprintln!("[moabom] 설정 파일을 읽지 못해 기본값으로 시작합니다 ({}): {e}", path.display());
            let _ = std::fs::copy(&path, path.with_extension("json.bak"));
            Saved::default()
        }
    }
}

/// 마지막으로 쓴 내용. 바뀐 게 없으면 다시 쓰지 않는다.
static LAST: Mutex<String> = Mutex::new(String::new());

pub fn save(app: &AppHandle, saved: &Saved) {
    let Some(path) = path(app) else { return };
    let Ok(text) = serde_json::to_string_pretty(saved) else { return };
    let mut last = LAST.lock().unwrap();
    if *last == text {
        return;
    }
    // 쓰는 도중 꺼져도 파일이 깨지지 않도록 임시 파일에 쓴 뒤 바꿔치기한다.
    let tmp = path.with_extension("json.tmp");
    let result = path
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|_| std::fs::write(&tmp, &text))
        .and_then(|_| std::fs::rename(&tmp, &path));
    match result {
        Ok(()) => *last = text,
        Err(e) => eprintln!("[moabom] 설정을 저장하지 못했습니다 ({}): {e}", path.display()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_or_partial_files_fill_defaults() {
        let s: Saved = serde_json::from_str(r#"{"tiles":[{"input":"soop:abc","platform":"soop"}]}"#).unwrap();
        assert!(s.restore && s.notify);
        assert_eq!(s.tiles[0].volume, 100);
        assert_eq!(s.tiles[0].quality, None);
    }

    #[test]
    fn round_trip() {
        let mut s = Saved::default();
        s.layout = Layout::Bottom;
        s.groups.push(Group {
            name: "평일".into(),
            tiles: vec![SavedTile {
                input: "chzzk:abc".into(),
                platform: Platform::Chzzk,
                name: Some("이름".into()),
                quality: Some(Quality::Low),
                volume: 40,
                mix: true,
            }],
        });
        let back: Saved = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(back, s);
    }
}
