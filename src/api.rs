use super::{MAX_ARTIST_LEN, MAX_TITLE_LEN, MAX_DESCRIPTION_LEN};
use anyhow::*;
use hyper::header::{
    ACCEPT, ACCEPT_ENCODING, ACCEPT_LANGUAGE, AUTHORIZATION, CONNECTION, DNT,
    ORIGIN, REFERER, USER_AGENT,
};
use reqwest::Client;
use serde::Deserialize;
use serde_json::Value;
use unicode_truncate::UnicodeTruncateStr;

#[derive(Clone, Debug, Deserialize)]
pub struct ClientToken {
    pub client_id: String,
    pub auth: String,
}

fn make_resolve_url(client_id: &str, url: &str) -> String {
    let client_id = urlencoding::encode(client_id);
    let url = urlencoding::encode(url);
    format!("https://api-v2.soundcloud.com/resolve?client_id={client_id}&url={url}")
}

/// makes a request to the soundcloud api and parses the result as json
async fn api_request(url: &str, auth: &str) -> Result<Value> {
    let client = Client::new();

    // TODO: replace fake user agent with something like https://github.com/FixTweet/FixTweet/blob/main/src/helpers/useragent.ts
    let text = client.get(url)
        .header(ACCEPT, "application/json, text/javascript, */*; q=0.01")
        .header(ACCEPT_ENCODING, "gzip, deflate, br")
        .header(ACCEPT_LANGUAGE, "en-US,en;q=0.5")
        .header(AUTHORIZATION, auth)
        .header(CONNECTION, "keep-alive")
        .header(DNT, 1)
        .header(ORIGIN, "https://soundcloud.com")
        .header(REFERER, "https://soundcloud.com/")
        .header("Sec-Fetch-Dest", "empty")
        .header("Sec-Fetch-Mode", "cors")
        .header("Sec-Fetch-Site", "same-site")
        .header(USER_AGENT, "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/114.0.0.0 Safari/537.36")
        .header("sec-ch-ua", "\"Not.A/Brand\";v=\"8\", \"Chromium\";v=\"114\", \"Google Chrome\";v=\"114\"")
        .header("sec-ch-ua-mobile", "?0")
        .header("sec-ch-ua-platform", "\"Linux\"")
        .send()
        .await?
        .text()
        .await?;

    let json = serde_json::from_str(&text)?;

    Ok(json)
}

/// stores the info of a track that we care about
#[derive(Clone, Default, Debug)]
pub struct TrackInfo {
    pub artwork_url: String,
    pub permalink_url: String,
    pub artist_name: String,
    pub title: String,
    pub description: String,
    pub playback_count: u32,
    pub likes_count: u32,
    pub reposts_count: u32,
    pub comment_count: u32,
}

/// stores the info of a playlist that we care about
#[derive(Clone, Default, Debug)]
pub struct PlaylistInfo {
    pub artwork_url: String,
    pub permalink_url: String,
    pub artist_name: String,
    pub title: String,
    pub description: String,
    pub track_count: u32,
    pub likes_count: u32,
    pub reposts_count: u32,
}

#[derive(Clone, Debug)]
pub enum ResolveInfo {
    Track(TrackInfo),
    Playlist(PlaylistInfo),
}

impl ResolveInfo {
    pub fn artwork_url(&self) -> &str {
        match self {
            Self::Track(info) => &info.artwork_url,
            Self::Playlist(info) => &info.artwork_url,
        }
    }

    pub fn permalink_url(&self) -> &str {
        match self {
            Self::Track(info) => &info.permalink_url,
            Self::Playlist(info) => &info.permalink_url,
        }
    }

    pub fn artist_name(&self) -> &str {
        match self {
            Self::Track(info) => &info.artist_name,
            Self::Playlist(info) => &info.artist_name,
        }
    }

    pub fn title(&self) -> &str {
        match self {
            Self::Track(info) => &info.title,
            Self::Playlist(info) => &info.title,
        }
    }

    pub fn description(&self) -> &str {
        match self {
            Self::Track(info) => &info.description,
            Self::Playlist(info) => &info.description,
        }
    }

    pub fn counts(&self) -> String {
        match self {
            Self::Track(info) => format!(
                "{} ▶    {} ❤️    {} 🔁    {} 💬",
                info.playback_count, info.likes_count, info.reposts_count, info.comment_count
            ),
            Self::Playlist(info) => format!(
                "{} 🎵    {} ❤️    {} 🔁",
                info.track_count, info.likes_count, info.reposts_count
            ),
        }
    }
}

fn truncate_string(string: &str, length: usize) -> String {
    if string.len() > length {
        let (truncated, new_len) = string.unicode_truncate(length - 3);

        let mut new_string = truncated.to_string();

        for _i in new_len..length {
            new_string.push('.');
        }

        new_string
    } else {
        string.to_string()
    }
}

/// resolve a soundcloud url and parse its information
pub async fn resolve(token: &ClientToken, url: &str) -> Result<ResolveInfo> {
    // make api request and parse to json
    let body = match api_request(&make_resolve_url(&token.client_id, url), &token.auth).await? {
        Value::Object(map) => map,
        _ => return Err(anyhow!("invalid response type")),
    };

    // make sure we got data we understand
    let kind = match body.get("kind") {
        Some(Value::String(kind)) => kind,
        kind => return Err(anyhow!("unexpected object kind {kind:?}")),
    };

    match kind.as_ref() {
        "track" => {
            // parse into TrackInfo
            let mut info = TrackInfo::default();

            if let Some(Value::String(value)) = body.get("artwork_url") {
                info.artwork_url = value.to_string();
            }

            if let Some(Value::String(value)) = body.get("permalink_url") {
                info.permalink_url = value.to_string();
            }

            if let Some(Value::Object(user)) = body.get("user") && let Some(Value::String(value)) = user.get("username") {
                info.artist_name = truncate_string(value, MAX_ARTIST_LEN);
            }

            if let Some(Value::String(value)) = body.get("title") {
                info.title = truncate_string(value, MAX_TITLE_LEN);
            }

            if let Some(Value::String(value)) = body.get("description") {
                info.description = truncate_string(value, MAX_DESCRIPTION_LEN);
            }

            if let Some(Value::Number(number)) = body.get("playback_count") && let Some(value) = number.as_u64() {
                info.playback_count = value as u32;
            }

            if let Some(Value::Number(number)) = body.get("likes_count") && let Some(value) = number.as_u64() {
                info.likes_count = value as u32;
            }

            if let Some(Value::Number(number)) = body.get("reposts_count") && let Some(value) = number.as_u64() {
                info.reposts_count = value as u32;
            }

            if let Some(Value::Number(number)) = body.get("comment_count") && let Some(value) = number.as_u64() {
                info.comment_count = value as u32;
            }

            Ok(ResolveInfo::Track(info))
        }
        "playlist" => {
            let mut info = PlaylistInfo::default();

            if let Some(Value::String(value)) = body.get("artwork_url") {
                info.artwork_url = value.to_string();
            }

            if let Some(Value::String(value)) = body.get("permalink_url") {
                info.permalink_url = value.to_string();
            }

            if let Some(Value::Object(user)) = body.get("user") && let Some(Value::String(value)) = user.get("username") {
                info.artist_name = truncate_string(value, MAX_ARTIST_LEN);
            }

            if let Some(Value::String(value)) = body.get("title") {
                info.title = truncate_string(value, MAX_TITLE_LEN);
            }

            if let Some(Value::String(value)) = body.get("description") {
                info.description = truncate_string(value, MAX_DESCRIPTION_LEN);
            }

            if let Some(Value::Number(number)) = body.get("track_count") && let Some(value) = number.as_u64() {
                info.track_count = value as u32;
            }

            if let Some(Value::Number(number)) = body.get("likes_count") && let Some(value) = number.as_u64() {
                info.likes_count = value as u32;
            }

            if let Some(Value::Number(number)) = body.get("reposts_count") && let Some(value) = number.as_u64() {
                info.reposts_count = value as u32;
            }

            Ok(ResolveInfo::Playlist(info))
        }
        kind => Err(anyhow!("unexpected object kind {kind:?}")),
    }
}
