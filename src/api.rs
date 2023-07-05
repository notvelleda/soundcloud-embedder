//! handles interactions with soundcloud's api

use super::{MAX_ARTIST_LEN, MAX_DESCRIPTION_LEN, MAX_TITLE_LEN};
use anyhow::*;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use unicode_truncate::UnicodeTruncateStr;

pub fn make_resolve_url(client_id: &str, url: &str) -> String {
    let client_id = urlencoding::encode(client_id);
    let url = urlencoding::encode(url);
    format!("https://api-v2.soundcloud.com/resolve?client_id={client_id}&url={url}")
}

/// stores the info of a track that we care about
#[derive(Clone, Default, Debug, Serialize, Deserialize)]
pub struct TrackInfo {
    pub artwork_url: String,
    pub permalink_url: String,
    pub stream_url: String,
    pub artist_name: String,
    pub title: String,
    pub description: String,
    pub playback_count: u32,
    pub likes_count: u32,
    pub reposts_count: u32,
    pub comment_count: u32,
}

/// stores the info of a playlist that we care about
#[derive(Clone, Default, Debug, Serialize, Deserialize)]
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

#[derive(Clone, Debug, Serialize, Deserialize)]
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
            Self::Track(info) => format!("{} ▶    {} ❤️    {} 🔁    {} 💬", info.playback_count, info.likes_count, info.reposts_count, info.comment_count),
            Self::Playlist(info) => format!("{} 🎵    {} ❤️    {} 🔁", info.track_count, info.likes_count, info.reposts_count),
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
pub async fn resolve(client_id: &str, url: &str) -> Result<ResolveInfo> {
    // make api request and parse to json
    let body = match crate::requests::api_request(&make_resolve_url(client_id, url)).await? {
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
            } else if let Some(Value::Object(user)) = body.get("user") && let Some(Value::String(value)) = user.get("avatar_url") {
                info.artwork_url = value.to_string();
            }

            if let Some(Value::String(value)) = body.get("permalink_url") {
                info.permalink_url = value.to_string();
            }

            if let Some(Value::Object(media)) = body.get("media") && let Some(Value::Array(transcodings)) = media.get("transcodings") {
                for value in transcodings.iter() {
                    if let Some(Value::String(preset)) = value.get("preset")
                        && preset.starts_with("opus")
                        && let Some(Value::Object(format)) = value.get("format")
                        && let Some(Value::String(protocol)) = format.get("protocol")
                        && protocol == "hls"
                        && let Some(Value::String(url)) = value.get("url") {
                        info.stream_url = url.to_string();
                        break;
                    }
                }
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
            } else if let Some(Value::Object(user)) = body.get("user") && let Some(Value::String(value)) = user.get("avatar_url") {
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
