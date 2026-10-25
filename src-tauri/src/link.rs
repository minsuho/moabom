//! 지통실(streams.helmhq.xyz)에서 여는 링크: `moabom://open?v=<방송 주소들>&mode=add|replace&group=<이름>`
//!
//! - v: 방송 주소(또는 `platform:id`)를 쉼표로, 12개까지. 하나씩 platforms::resolve로 확인한다.
//! - mode: add(기본, 열려 있는 방송에 더한다) / replace(열린 방송을 닫고 연다)
//! - group: 지통실 컨텐츠 이름 (확인 창에 보여 준다)
//!
//! 링크는 바로 열지 않고 컨트롤 바에 확인 창을 띄운다(처음에는 꼭 묻고, '다음부터 묻지 않기'를 고르면 바로 연다).
//! 모아봄이 꺼져 있으면 링크로 켜진 뒤 컨트롤 바가 뜨면 가져가고, 켜져 있으면 single-instance가 넘겨준다.

use serde::Serialize;
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager};

pub const SCHEME: &str = "moabom";
const MAX_ITEMS: usize = 12;

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Link {
    pub items: Vec<String>,
    pub replace: bool,
    pub group: Option<String>,
}

/// 컨트롤 바가 뜨기 전에 온 링크
#[derive(Default)]
pub struct PendingLink(pub Mutex<Option<Link>>);

/// 링크 주소를 읽는다. 모아봄 링크가 아니거나 열 방송이 없으면 None
pub fn parse(raw: &str) -> Option<Link> {
    let u = url::Url::parse(raw.trim()).ok()?;
    if u.scheme() != SCHEME || u.host_str() != Some("open") {
        return None;
    }
    let mut items: Vec<String> = Vec::new();
    let mut replace = false;
    let mut group = None;
    for (k, v) in u.query_pairs() {
        match k.as_ref() {
            "v" => {
                for s in v.split(',').map(str::trim).filter(|s| !s.is_empty()) {
                    if items.len() < MAX_ITEMS && !items.iter().any(|x| x == s) {
                        items.push(s.to_string());
                    }
                }
            }
            "mode" => replace = v == "replace",
            "group" => group = Some(v.chars().take(60).collect::<String>()).filter(|g| !g.trim().is_empty()),
            _ => {}
        }
    }
    if items.is_empty() { None } else { Some(Link { items, replace, group }) }
}

/// 실행 인자에서 모아봄 링크를 찾는다
pub fn from_args<I: IntoIterator<Item = String>>(args: I) -> Option<Link> {
    args.into_iter().find_map(|a| parse(&a))
}

/// 링크를 컨트롤 바로 넘긴다. 아직 안 떴으면 맡아 두고, 떠 있으면 알린다(둘 다 해 두면 늦게 뜬 바도 가져간다)
pub fn deliver(app: &AppHandle, link: Link) {
    *app.state::<PendingLink>().0.lock().unwrap() = Some(link.clone());
    let _ = app.emit("open-link", link);
}

/// 컨트롤 바가 떴을 때 맡아 둔 링크를 가져간다 (한 번만)
#[tauri::command]
pub fn take_link(state: tauri::State<'_, PendingLink>) -> Option<Link> {
    state.0.lock().unwrap().take()
}

/// 확인 창에서 '열기'. replace면 열린 방송을 닫고, 아니면 이미 열린 방송은 건너뛴다
#[tauri::command]
pub async fn open_link(app: AppHandle, items: Vec<String>, replace: bool) -> Result<(), String> {
    *app.state::<PendingLink>().0.lock().unwrap() = None;
    let failed = crate::tiles::open_inputs(&app, items.into_iter().take(MAX_ITEMS).collect(), replace).await?;
    if failed > 0 {
        crate::tiles::toast(&app, &format!("방송 {failed}개를 열지 못했습니다."));
    }
    Ok(())
}

/// moabom:// 을 이 실행 파일로 연결한다. 켤 때마다 지금 경로로 다시 쓴다(설치판·포터블 모두, 옮겨도 따라간다).
/// 현재 사용자(HKCU)에만 쓰므로 관리자 권한이 필요 없다.
#[cfg(windows)]
pub fn register() {
    let Ok(exe) = std::env::current_exe() else { return };
    let exe = exe.to_string_lossy();
    let base = format!("Software\\Classes\\{SCHEME}");
    let results = [
        crate::notify::set_reg_string(&base, "", "URL:모아봄"),
        crate::notify::set_reg_string(&base, "URL Protocol", ""),
        crate::notify::set_reg_string(&format!("{base}\\DefaultIcon"), "", &format!("\"{exe}\",0")),
        crate::notify::set_reg_string(&format!("{base}\\shell\\open\\command"), "", &format!("\"{exe}\" \"%1\"")),
    ];
    if let Some(Err(e)) = results.into_iter().find(Result::is_err) {
        eprintln!("moabom:// 등록 실패: {e}");
    }
}

#[cfg(not(windows))]
pub fn register() {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 링크_읽기() {
        let l = parse("moabom://open?v=https%3A%2F%2Fplay.sooplive.co.kr%2Fabc,chzzk:xyz,chzzk:xyz&mode=replace&group=%EC%98%88%EC%8B%9C").unwrap();
        assert_eq!(l.items, vec!["https://play.sooplive.co.kr/abc", "chzzk:xyz"]);
        assert!(l.replace);
        assert_eq!(l.group.as_deref(), Some("예시"));
        assert!(!parse("moabom://open?v=a").unwrap().replace);
    }

    #[test]
    fn 아닌_링크는_무시() {
        assert_eq!(parse("https://example.com/open?v=a"), None);
        assert_eq!(parse("moabom://other?v=a"), None);
        assert_eq!(parse("moabom://open?v="), None);
        assert_eq!(parse("not a url"), None);
    }

    #[test]
    fn 열두_개까지() {
        let v: Vec<String> = (0..20).map(|i| format!("chzzk:c{i}")).collect();
        assert_eq!(parse(&format!("moabom://open?v={}", v.join(","))).unwrap().items.len(), 12);
    }

    #[test]
    fn 실행_인자에서_찾기() {
        let args = vec!["moabom.exe".to_string(), "moabom://open?v=chzzk:a".to_string()];
        assert_eq!(from_args(args).unwrap().items, vec!["chzzk:a"]);
        assert_eq!(from_args(vec!["moabom.exe".to_string()]), None);
    }
}
