//! 입력한 주소(또는 `soop:아이디` 같은 단축 표기)를 플랫폼별 재생/채팅 주소로 변환한다.

use serde::Serialize;
use url::Url;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Platform {
    Soop,
    Chzzk,
    Cime,
    Youtube,
    Twitch,
    Kick,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Resolved {
    pub platform: Platform,
    pub channel: String,
    pub player_url: String,
    pub chat_url: Option<String>,
}

pub fn resolve(input: &str) -> Result<Resolved, String> {
    let input = input.trim();
    if input.is_empty() {
        return Err("주소를 입력하세요".into());
    }
    if let Some(r) = resolve_shorthand(input) {
        return Ok(r);
    }
    // 치지직 채널 ID(32자리 16진수)는 앞에 chzzk: 없이도 알아볼 수 있다.
    if is_chzzk_id(input) {
        return Ok(chzzk(&input.to_ascii_lowercase()));
    }

    let with_scheme = if input.contains("://") {
        input.to_string()
    } else {
        format!("https://{input}")
    };
    let url = Url::parse(&with_scheme).map_err(|_| format!("올바른 주소가 아닙니다: {input}"))?;
    let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
    let host = host
        .strip_prefix("www.")
        .or_else(|| host.strip_prefix("m."))
        .unwrap_or(&host)
        .to_string();
    let segs: Vec<&str> = url
        .path_segments()
        .map(|s| s.filter(|p| !p.is_empty()).collect())
        .unwrap_or_default();

    let unsupported = || format!("지원하지 않는 주소입니다: {input}");

    if host.ends_with("sooplive.co.kr") || host.ends_with("sooplive.com") || host.ends_with("afreecatv.com") {
        let id = match segs.as_slice() {
            ["station", id, ..] => *id,
            [id, ..] => *id,
            [] => return Err("숲 방송국 아이디가 없습니다".into()),
        };
        return Ok(soop(id));
    }
    if host == "chzzk.naver.com" {
        let id = match segs.as_slice() {
            ["live", id, ..] => *id,
            [id, ..] if is_chzzk_id(id) => *id,
            _ => return Err(unsupported()),
        };
        return Ok(chzzk(id));
    }
    if host == "ci.me" {
        let channel = segs.first().ok_or_else(unsupported)?.to_string();
        return Ok(Resolved {
            platform: Platform::Cime,
            channel,
            player_url: url.to_string(),
            chat_url: None,
        });
    }
    if host == "youtu.be" {
        let id = segs.first().ok_or_else(unsupported)?;
        return Ok(youtube_video(id));
    }
    if host.ends_with("youtube.com") {
        if let Some((_, v)) = url.query_pairs().find(|(k, _)| k == "v") {
            return Ok(youtube_video(&v));
        }
        return match segs.as_slice() {
            ["live", id, ..] | ["embed", id, ..] => Ok(youtube_video(id)),
            [handle, ..] if handle.starts_with('@') => Ok(youtube_channel(&format!("{handle}/live"))),
            ["channel", id, ..] => Ok(youtube_channel(&format!("channel/{id}/live"))),
            _ => Err(unsupported()),
        };
    }
    if host.ends_with("twitch.tv") {
        let ch = segs.first().ok_or_else(unsupported)?;
        return Ok(twitch(ch));
    }
    if host.ends_with("kick.com") {
        let ch = segs.first().ok_or_else(unsupported)?;
        return Ok(kick(ch));
    }
    Err(unsupported())
}

fn resolve_shorthand(input: &str) -> Option<Resolved> {
    let (prefix, id) = input.split_once(':')?;
    let id = id.trim();
    if id.is_empty() || id.starts_with("//") {
        return None;
    }
    Some(match prefix.trim().to_ascii_lowercase().as_str() {
        "soop" | "숲" => soop(id),
        "chzzk" | "치지직" => chzzk(id),
        "yt" | "youtube" | "유튜브" => {
            if id.starts_with('@') {
                youtube_channel(&format!("{id}/live"))
            } else {
                youtube_video(id)
            }
        }
        "twitch" | "트위치" => twitch(id),
        "kick" | "킥" => kick(id),
        _ => return None,
    })
}

fn is_chzzk_id(s: &str) -> bool {
    s.len() == 32 && s.chars().all(|c| c.is_ascii_hexdigit())
}

fn soop(id: &str) -> Resolved {
    Resolved {
        platform: Platform::Soop,
        channel: id.to_string(),
        player_url: format!("https://play.sooplive.com/{id}"),
        chat_url: None,
    }
}

fn chzzk(id: &str) -> Resolved {
    Resolved {
        platform: Platform::Chzzk,
        channel: id.to_string(),
        player_url: format!("https://chzzk.naver.com/live/{id}"),
        chat_url: Some(format!("https://chzzk.naver.com/live/{id}/chat")),
    }
}

fn youtube_video(id: &str) -> Resolved {
    Resolved {
        platform: Platform::Youtube,
        channel: id.to_string(),
        player_url: format!("https://www.youtube.com/watch?v={id}"),
        chat_url: Some(format!("https://www.youtube.com/live_chat?is_popout=1&v={id}")),
    }
}

fn youtube_channel(path: &str) -> Resolved {
    Resolved {
        platform: Platform::Youtube,
        channel: path.trim_end_matches("/live").to_string(),
        player_url: format!("https://www.youtube.com/{path}"),
        // 영상 ID를 알아야 채팅 주소를 만들 수 있다. 재생 후 주소에서 다시 찾는다.
        chat_url: None,
    }
}

fn twitch(ch: &str) -> Resolved {
    let ch = ch.to_ascii_lowercase();
    Resolved {
        platform: Platform::Twitch,
        player_url: format!("https://www.twitch.tv/{ch}"),
        chat_url: Some(format!("https://www.twitch.tv/popout/{ch}/chat?popout=")),
        channel: ch,
    }
}

fn kick(ch: &str) -> Resolved {
    let ch = ch.to_ascii_lowercase();
    Resolved {
        platform: Platform::Kick,
        player_url: format!("https://kick.com/{ch}"),
        chat_url: Some(format!("https://kick.com/popout/{ch}/chat")),
        channel: ch,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(input: &str) -> (Platform, String) {
        let r = resolve(input).unwrap();
        (r.platform, r.channel)
    }

    #[test]
    fn soop_urls() {
        assert_eq!(p("https://play.sooplive.co.kr/abc123/281234567"), (Platform::Soop, "abc123".into()));
        assert_eq!(p("ch.sooplive.co.kr/abc123"), (Platform::Soop, "abc123".into()));
        assert_eq!(p("https://www.sooplive.co.kr/station/abc123"), (Platform::Soop, "abc123".into()));
        assert_eq!(p("play.afreecatv.com/abc123"), (Platform::Soop, "abc123".into()));
        assert_eq!(p("https://play.sooplive.com/abc123/297663883"), (Platform::Soop, "abc123".into()));
        assert_eq!(p("soop:abc123"), (Platform::Soop, "abc123".into()));
    }

    #[test]
    fn chzzk_urls() {
        let id = "0123456789abcdef0123456789abcdef";
        assert_eq!(p(&format!("https://chzzk.naver.com/live/{id}")), (Platform::Chzzk, id.into()));
        assert_eq!(p(&format!("chzzk.naver.com/{id}")), (Platform::Chzzk, id.into()));
        assert!(resolve("chzzk.naver.com/lives").is_err());
        assert_eq!(p(id), (Platform::Chzzk, id.into()));
    }

    #[test]
    fn youtube_urls() {
        assert_eq!(p("https://www.youtube.com/watch?v=dQw4w9WgXcQ"), (Platform::Youtube, "dQw4w9WgXcQ".into()));
        assert_eq!(p("youtu.be/dQw4w9WgXcQ"), (Platform::Youtube, "dQw4w9WgXcQ".into()));
        assert_eq!(p("youtube.com/live/dQw4w9WgXcQ?si=x"), (Platform::Youtube, "dQw4w9WgXcQ".into()));
        let r = resolve("https://youtube.com/@somebody").unwrap();
        assert_eq!(r.player_url, "https://www.youtube.com/@somebody/live");
        assert_eq!(r.chat_url, None);
    }

    #[test]
    fn twitch_kick_urls() {
        let r = resolve("https://www.twitch.tv/SomeOne").unwrap();
        assert_eq!(r.channel, "someone");
        assert_eq!(r.chat_url.as_deref(), Some("https://www.twitch.tv/popout/someone/chat?popout="));
        assert_eq!(p("kick.com/streamer"), (Platform::Kick, "streamer".into()));
    }

    #[test]
    fn rejects_unknown() {
        assert!(resolve("https://example.com/x").is_err());
        assert!(resolve("   ").is_err());
    }
}
