//! YouTube Music search, straight from music.youtube.com's own API (the
//! WEB_REMIX InnerTube client the website uses) — one HTTP request, no
//! yt-dlp process: songs come back with clean titles, every credited
//! artist, the album, the length and square album art. Results are normal
//! YouTube video ids, so streaming/downloading them is unchanged (and gets
//! Premium audio when the account's cookies are Premium).

use serde_json::{Value, json};
use std::time::Duration;

const API: &str = "https://music.youtube.com/youtubei/v1/search?prettyPrint=false";
const CLIENT_VERSION: &str = "1.20251110.01.00";
/// The "Songs" filter of the search page.
const SONGS: &str = "EgWKAQIIAWoKEAkQBRAKEAMQBA%3D%3D";

fn runs_text(v: &Value) -> String {
    v["runs"].as_array().map(|r| r.iter().filter_map(|x| x["text"].as_str()).collect::<String>()).unwrap_or_default()
}

fn walk<'a>(v: &'a Value, out: &mut Vec<&'a Value>) {
    match v {
        Value::Object(m) => {
            if let Some(r) = m.get("musicResponsiveListItemRenderer") {
                out.push(r);
            }
            for x in m.values() {
                walk(x, out);
            }
        }
        Value::Array(a) => a.iter().for_each(|x| walk(x, out)),
        _ => {}
    }
}

fn parse_duration(s: &str) -> i64 {
    s.split(':').try_fold(0i64, |acc, p| p.trim().parse::<i64>().ok().map(|n| acc * 60 + n)).unwrap_or(0)
}

/// Square artwork at `size` px (the API hands out 60/120 px googleusercontent URLs).
fn art(url: &str, size: u32) -> String {
    match url.rfind("=w") {
        Some(i) if url.contains("googleusercontent.com") => format!("{}=w{size}-h{size}-l90-rj", &url[..i]),
        _ => url.to_string(),
    }
}

/// One search-result row → the app's track shape.
pub fn parse_item(r: &Value) -> Option<Value> {
    let vid = r["playlistItemData"]["videoId"].as_str().or_else(|| r["overlay"]["musicItemThumbnailOverlayRenderer"]["content"]["musicPlayButtonRenderer"]["playNavigationEndpoint"]["watchEndpoint"]["videoId"].as_str())?;
    let cols = r["flexColumns"].as_array()?;
    let col = |i: usize| cols.get(i).map(|c| &c["musicResponsiveListItemFlexColumnRenderer"]["text"]);
    let title = col(0).map(runs_text).filter(|t| !t.is_empty())?;
    let mut artists: Vec<String> = Vec::new();
    let (mut album, mut duration) = (None, 0i64);
    if let Some(runs) = col(1).and_then(|c| c["runs"].as_array()) {
        for x in runs {
            let text = x["text"].as_str().unwrap_or("").trim();
            let page = x["navigationEndpoint"]["browseEndpoint"]["browseEndpointContextSupportedConfigs"]["browseEndpointContextMusicConfig"]["pageType"].as_str();
            match page {
                Some("MUSIC_PAGE_TYPE_ARTIST" | "MUSIC_PAGE_TYPE_USER_CHANNEL") => artists.push(text.to_string()),
                Some("MUSIC_PAGE_TYPE_ALBUM") => album = Some(text.to_string()),
                _ if text.contains(':') && text.chars().all(|c| c.is_ascii_digit() || c == ':') => duration = parse_duration(text),
                _ => {}
            }
        }
        if artists.is_empty() {
            // Unlinked artist names: the first run before " • ".
            if let Some(t) = runs.first().and_then(|x| x["text"].as_str()) {
                artists.push(t.trim().to_string());
            }
        }
    }
    let thumbs = r["thumbnail"]["musicThumbnailRenderer"]["thumbnail"]["thumbnails"].as_array();
    let thumb = thumbs.and_then(|t| t.last()).and_then(|t| t["url"].as_str()).map(|u| art(u, 544)).unwrap_or_else(|| format!("https://i.ytimg.com/vi/{vid}/hqdefault.jpg"));
    let artist = artists.join(", ");
    Some(json!({
        "id": vid,
        "title": title,
        "artist": if artist.is_empty() { "Unknown Artist".to_string() } else { artist },
        "album": album,
        "duration_seconds": duration,
        "thumbnail_url": thumb,
        "source": "youtube",
        "youtube_url": format!("https://music.youtube.com/watch?v={vid}"),
        "ytmusic": true,
    }))
}

/// Songs matching `query` (None when YouTube Music didn't answer).
pub async fn search(http: &reqwest::Client, query: &str, limit: usize) -> Option<Vec<Value>> {
    let body = json!({
        "context": {"client": {"clientName": "WEB_REMIX", "clientVersion": CLIENT_VERSION, "hl": "en", "gl": "US"}},
        "query": query,
        "params": SONGS,
    });
    let res = http
        .post(API)
        .timeout(Duration::from_secs(10))
        .header("Origin", "https://music.youtube.com")
        .header("Referer", "https://music.youtube.com/")
        .header("User-Agent", "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0 Safari/537.36")
        .json(&body)
        .send()
        .await
        .ok()?;
    if !res.status().is_success() {
        tracing::warn!("YouTube Music search answered {}", res.status());
        return None;
    }
    let v: Value = res.json().await.ok()?;
    let mut rows = Vec::new();
    walk(&v, &mut rows);
    let mut seen = std::collections::HashSet::new();
    Some(rows.into_iter().filter_map(parse_item).filter(|t| seen.insert(t["id"].as_str().unwrap_or("").to_string())).take(limit).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_song_row() {
        let r = json!({
            "playlistItemData": {"videoId": "JhulBGMA7G4"},
            "thumbnail": {"musicThumbnailRenderer": {"thumbnail": {"thumbnails": [{"url": "https://lh3.googleusercontent.com/abc=w60-h60-l90-rj"}, {"url": "https://lh3.googleusercontent.com/abc=w120-h120-l90-rj"}]}}},
            "flexColumns": [
                {"musicResponsiveListItemFlexColumnRenderer": {"text": {"runs": [{"text": "Harder, Better, Faster, Stronger"}]}}},
                {"musicResponsiveListItemFlexColumnRenderer": {"text": {"runs": [
                    {"text": "Daft Punk", "navigationEndpoint": {"browseEndpoint": {"browseEndpointContextSupportedConfigs": {"browseEndpointContextMusicConfig": {"pageType": "MUSIC_PAGE_TYPE_ARTIST"}}}}},
                    {"text": " • "},
                    {"text": "Discovery", "navigationEndpoint": {"browseEndpoint": {"browseEndpointContextSupportedConfigs": {"browseEndpointContextMusicConfig": {"pageType": "MUSIC_PAGE_TYPE_ALBUM"}}}}},
                    {"text": " • "}, {"text": "3:47"}]}}}
            ]
        });
        let t = parse_item(&r).unwrap();
        assert_eq!(t["id"], "JhulBGMA7G4");
        assert_eq!(t["artist"], "Daft Punk");
        assert_eq!(t["album"], "Discovery");
        assert_eq!(t["duration_seconds"], 227);
        assert_eq!(t["thumbnail_url"], "https://lh3.googleusercontent.com/abc=w544-h544-l90-rj");
    }
}
