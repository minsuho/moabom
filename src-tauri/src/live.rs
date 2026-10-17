//! 방송 중인지 확인한다. 꺼진 방송은 오버레이가 종료 화면으로 덮고, 다시 켜지면 앱이 새로고침한다.
//! 확인할 방법이 없거나 요청이 실패하면 `None`을 돌려주고, 그때는 화면을 바꾸지 않는다.

use serde_json::{json, Value};

use crate::platforms::Platform;
use crate::search::{client, TWITCH_CLIENT_ID};

pub struct Status {
    pub live: bool,
    pub image: Option<String>,
    /// 방송인 이름. 꺼진 방송은 페이지에서 이름을 읽을 수 없어 여기서 가져온다.
    pub name: Option<String>,
}

fn s(v: &Value) -> Option<String> {
    v.as_str().filter(|s| !s.is_empty()).map(str::to_string)
}

async fn get_json(req: reqwest::RequestBuilder) -> Option<Value> {
    req.send().await.ok()?.error_for_status().ok()?.json().await.ok()
}

pub async fn check(platform: Platform, channel: &str) -> Option<Status> {
    let c = client().ok()?;
    match platform {
        Platform::Soop => {
            let body = url::form_urlencoded::Serializer::new(String::new())
                .extend_pairs([("bid", channel), ("type", "live"), ("player_type", "html5")])
                .finish();
            let req = c
                .post("https://live.sooplive.co.kr/afreeca/player_live_api.php")
                .header("Content-Type", "application/x-www-form-urlencoded")
                .body(body);
            let v = get_json(req).await?;
            let result = v["CHANNEL"]["RESULT"].as_i64()?;
            let prefix: String = channel.chars().take(2).collect();
            let station = get_json(c.get(format!("https://chapi.sooplive.co.kr/api/{channel}/station"))).await;
            Some(Status {
                live: result == 1,
                image: Some(format!("https://profile.img.sooplive.co.kr/LOGO/{prefix}/{channel}/{channel}.jpg")),
                name: station.and_then(|v| s(&v["station"]["user_nick"])),
            })
        }
        Platform::Chzzk => {
            let status = get_json(c.get(format!("https://api.chzzk.naver.com/polling/v3/channels/{channel}/live-status"))).await?;
            let info = get_json(c.get(format!("https://api.chzzk.naver.com/service/v1/channels/{channel}"))).await;
            Some(Status {
                live: status["content"]["status"].as_str() == Some("OPEN"),
                image: info.as_ref().and_then(|v| s(&v["content"]["channelImageUrl"])),
                name: info.as_ref().and_then(|v| s(&v["content"]["channelName"])),
            })
        }
        Platform::Twitch => {
            let body = json!({
                "query": "query($l:String!){user(login:$l){displayName profileImageURL(width:150) stream{id}}}",
                "variables": { "l": channel },
            });
            let v = get_json(c.post("https://gql.twitch.tv/gql").header("Client-Id", TWITCH_CLIENT_ID).json(&body)).await?;
            let user = v["data"]["user"].as_object()?;
            Some(Status {
                live: !user["stream"].is_null(),
                image: s(&user["profileImageURL"]),
                name: s(&user["displayName"]),
            })
        }
        Platform::Kick => {
            let v = get_json(c.get(format!("https://kick.com/api/v2/channels/{channel}"))).await?;
            Some(Status {
                live: !v["livestream"].is_null(),
                image: s(&v["user"]["profile_pic"]).or_else(|| s(&v["user"]["profilePic"])),
                name: s(&v["user"]["username"]),
            })
        }
        // 유튜브·씨미는 페이지가 알아서 안내하므로 확인하지 않는다.
        Platform::Youtube | Platform::Cime => None,
    }
}
