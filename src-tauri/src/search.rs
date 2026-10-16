//! 방송인 검색. 숲·치지직·트위치·킥을 동시에 검색해 방송 중인 채널을 먼저 보여준다.
//! 각 결과의 `input`은 그대로 `add_tile`에 넘길 수 있는 단축 표기다.

use std::time::Duration;

use serde::Serialize;
use serde_json::{json, Value};

use crate::platforms::Platform;

const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/154.0.0.0 Safari/537.36";
// 트위치 웹사이트가 쓰는 공개 클라이언트 ID
pub const TWITCH_CLIENT_ID: &str = "kimne78kx3ncx6brgo4mv6wki5h1ko";
const PER_PLATFORM: usize = 6;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Hit {
    platform: Platform,
    input: String,
    name: String,
    title: Option<String>,
    live: bool,
    image: Option<String>,
}

fn s(v: &Value) -> Option<String> {
    v.as_str().filter(|s| !s.is_empty()).map(str::to_string)
}

async fn get_json(req: reqwest::RequestBuilder) -> Option<Value> {
    req.send().await.ok()?.error_for_status().ok()?.json().await.ok()
}

async fn soop(c: &reqwest::Client, q: &str) -> Vec<Hit> {
    let req = c.get("https://sch.sooplive.co.kr/api.php").query(&[
        ("m", "bjSearch"),
        ("v", "3.0"),
        ("szKeyword", q),
        ("nPageNo", "1"),
        ("nListCnt", &PER_PLATFORM.to_string()),
        ("szOrder", "score"),
    ]);
    let Some(v) = get_json(req).await else { return vec![] };
    v["DATA"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|d| {
            let id = s(&d["user_id"])?;
            Some(Hit {
                platform: Platform::Soop,
                input: format!("soop:{id}"),
                name: s(&d["user_nick"]).unwrap_or_else(|| id.clone()),
                title: s(&d["station_title"]),
                live: s(&d["broad_no"]).is_some_and(|n| n != "0"),
                image: s(&d["station_logo"]),
            })
        })
        .collect()
}

async fn chzzk(c: &reqwest::Client, q: &str) -> Vec<Hit> {
    let req = c
        .get("https://api.chzzk.naver.com/service/v1/search/channels")
        .query(&[("keyword", q), ("offset", "0"), ("size", &PER_PLATFORM.to_string())]);
    let Some(v) = get_json(req).await else { return vec![] };
    v["content"]["data"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|d| {
            let ch = &d["channel"];
            let id = s(&ch["channelId"])?;
            Some(Hit {
                platform: Platform::Chzzk,
                input: format!("chzzk:{id}"),
                name: s(&ch["channelName"]).unwrap_or_else(|| id.clone()),
                title: s(&d["content"]["live"]["liveTitle"]).or_else(|| s(&ch["channelDescription"])),
                live: ch["openLive"].as_bool().unwrap_or(false),
                image: s(&ch["channelImageUrl"]),
            })
        })
        .collect()
}

async fn twitch(c: &reqwest::Client, q: &str) -> Vec<Hit> {
    let body = json!({
        "query": "query($q:String!){searchFor(userQuery:$q,platform:\"web\",options:{targets:[{index:CHANNEL}]}){channels{items{login displayName profileImageURL(width:70) stream{title}}}}}",
        "variables": { "q": q },
    });
    let req = c.post("https://gql.twitch.tv/gql").header("Client-Id", TWITCH_CLIENT_ID).json(&body);
    let Some(v) = get_json(req).await else { return vec![] };
    v["data"]["searchFor"]["channels"]["items"]
        .as_array()
        .into_iter()
        .flatten()
        .take(PER_PLATFORM)
        .filter_map(|d| {
            let login = s(&d["login"])?;
            Some(Hit {
                platform: Platform::Twitch,
                input: format!("twitch:{login}"),
                name: s(&d["displayName"]).unwrap_or_else(|| login.clone()),
                title: s(&d["stream"]["title"]),
                live: d["stream"].is_object(),
                image: s(&d["profileImageURL"]),
            })
        })
        .collect()
}

async fn kick(c: &reqwest::Client, q: &str) -> Vec<Hit> {
    let req = c.get("https://kick.com/api/search").query(&[("searched_word", q)]);
    let Some(v) = get_json(req).await else { return vec![] };
    v["channels"]
        .as_array()
        .into_iter()
        .flatten()
        .take(PER_PLATFORM)
        .filter_map(|d| {
            let slug = s(&d["slug"])?;
            Some(Hit {
                platform: Platform::Kick,
                input: format!("kick:{slug}"),
                name: s(&d["user"]["username"]).unwrap_or_else(|| slug.clone()),
                title: None,
                live: d["isLive"].as_bool().unwrap_or(false),
                image: s(&d["user"]["profilePic"]).or_else(|| s(&d["user"]["profile_pic"])),
            })
        })
        .collect()
}

pub fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .user_agent(UA)
        .timeout(Duration::from_secs(6))
        .build()
        .map_err(|e| e.to_string())
}

/// 입력이 숲 방송국 아이디인지 확인한다(검색창에 아이디만 넣었을 때 바로 추가하려고 쓴다).
#[tauri::command]
pub async fn soop_exists(id: String) -> bool {
    let Ok(c) = client() else { return false };
    let url = format!("https://chapi.sooplive.co.kr/api/{}/station", id.trim().to_ascii_lowercase());
    match get_json(c.get(url)).await {
        Some(v) => v["station"].is_object(),
        None => false,
    }
}

#[tauri::command]
pub async fn search(query: String) -> Result<Vec<Hit>, String> {
    let q = query.trim().to_string();
    if q.is_empty() {
        return Ok(vec![]);
    }
    let client = client()?;

    // 한 플랫폼이 실패하거나 느려도 나머지 결과는 보여준다.
    let spawn = |f: fn(reqwest::Client, String) -> tauri::async_runtime::JoinHandle<Vec<Hit>>| f(client.clone(), q.clone());
    let handles = [
        spawn(|c, q| tauri::async_runtime::spawn(async move { soop(&c, &q).await })),
        spawn(|c, q| tauri::async_runtime::spawn(async move { chzzk(&c, &q).await })),
        spawn(|c, q| tauri::async_runtime::spawn(async move { twitch(&c, &q).await })),
        spawn(|c, q| tauri::async_runtime::spawn(async move { kick(&c, &q).await })),
    ];
    let mut hits = Vec::new();
    for h in handles {
        hits.extend(h.await.unwrap_or_default());
    }
    // 방송 중인 채널 먼저. 같은 그룹 안에서는 플랫폼별 관련도 순서를 유지한다.
    hits.sort_by_key(|h| !h.live);
    Ok(hits)
}
