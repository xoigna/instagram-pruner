#![allow(dead_code)]

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct User {
    #[serde(default, deserialize_with = "de_id_string")]
    pub pk: String,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub full_name: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Media {
    #[serde(default, deserialize_with = "de_id_string")]
    pub pk: String,

    #[serde(default, deserialize_with = "de_opt_id_string")]
    pub id: Option<String>,
    #[serde(default)]
    pub code: Option<String>,

    #[serde(default)]
    pub taken_at: i64,

    #[serde(default)]
    pub media_type: i32,
    #[serde(default)]
    pub product_type: Option<String>,
    #[serde(default)]
    pub caption: Option<Caption>,
    #[serde(default)]
    pub user: Option<UserShort>,
    #[serde(default)]
    pub image_versions2: Option<ImageVersions>,
    #[serde(default)]
    pub thumbnail_url: Option<String>,
}

impl Media {
    pub fn media_id(&self) -> String {
        if let Some(id) = &self.id {
            if !id.is_empty() {
                return id.clone();
            }
        }
        if let Some(user) = &self.user {
            if !user.pk.is_empty() && !self.pk.is_empty() {
                return format!("{}_{}", self.pk, user.pk);
            }
        }
        self.pk.clone()
    }

    pub fn caption_text(&self) -> String {
        self.caption
            .as_ref()
            .map(|c| c.text.clone())
            .unwrap_or_default()
    }

    pub fn author_username(&self) -> String {
        self.user
            .as_ref()
            .map(|u| u.username.clone())
            .unwrap_or_default()
    }

    pub fn author_id(&self) -> String {
        self.user.as_ref().map(|u| u.pk.clone()).unwrap_or_default()
    }

    pub fn preview_url(&self) -> Option<String> {
        if let Some(url) = &self.thumbnail_url {
            if !url.is_empty() {
                return Some(url.clone());
            }
        }
        self.image_versions2
            .as_ref()
            .and_then(|iv| iv.candidates.first())
            .map(|c| c.url.clone())
    }

    pub fn kind_label(&self) -> &'static str {
        match (self.media_type, self.product_type.as_deref()) {
            (1, _) => "photo",
            (2, Some("clips")) => "reel",
            (2, Some("igtv")) => "igtv",
            (2, _) => "video",
            (8, _) => "album",
            _ => "media",
        }
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct Caption {
    #[serde(default)]
    pub text: String,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct UserShort {
    #[serde(default, deserialize_with = "de_id_string")]
    pub pk: String,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub full_name: String,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct ImageVersions {
    #[serde(default)]
    pub candidates: Vec<ImageCandidate>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct ImageCandidate {
    #[serde(default)]
    pub url: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FeedResponse {
    #[serde(default)]
    pub items: Vec<Media>,
    #[serde(default, deserialize_with = "de_opt_id_string")]
    pub next_max_id: Option<String>,
    #[serde(default)]
    pub more_available: bool,
    #[serde(default)]
    pub status: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DirectThread {
    #[serde(default, deserialize_with = "de_id_string")]
    pub thread_id: String,

    #[serde(default, deserialize_with = "de_opt_id_string")]
    pub pk: Option<String>,
    #[serde(default)]
    pub thread_title: Option<String>,

    #[serde(default)]
    pub is_group: bool,
    #[serde(default)]
    pub users: Vec<UserShort>,
    #[serde(default)]
    pub items: Vec<DirectMessage>,
    #[serde(default)]
    pub has_older: bool,
    #[serde(default, deserialize_with = "de_opt_id_string")]
    pub prev_cursor: Option<String>,
    #[serde(default, deserialize_with = "de_opt_id_string")]
    pub oldest_cursor: Option<String>,
}

impl DirectThread {
    pub fn id(&self) -> String {
        if !self.thread_id.is_empty() {
            return self.thread_id.clone();
        }
        self.pk.clone().unwrap_or_default()
    }

    pub fn display_name(&self) -> String {
        if let Some(title) = &self.thread_title {
            let t = title.trim();
            if !t.is_empty() {
                return t.to_string();
            }
        }
        self.participant_label()
    }

    pub fn participant_label(&self) -> String {
        if self.users.is_empty() {
            return format!("thread:{}", self.id());
        }
        if !self.is_group {
            let u = &self.users[0];
            if !u.username.is_empty() {
                return format!("@{}", u.username);
            }
            if !u.full_name.is_empty() {
                return u.full_name.clone();
            }
            return format!("user:{}", u.pk);
        }

        let labels: Vec<String> = self
            .users
            .iter()
            .map(|u| {
                if !u.username.is_empty() {
                    format!("@{}", u.username)
                } else if !u.full_name.is_empty() {
                    u.full_name.clone()
                } else {
                    u.pk.clone()
                }
            })
            .filter(|s| !s.is_empty())
            .collect();
        match labels.len() {
            0 => format!("group:{}", self.id()),
            1 => labels[0].clone(),
            2 => format!("{}, {}", labels[0], labels[1]),
            n => format!("{}, {} +{}", labels[0], labels[1], n - 2),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct DirectMessage {
    #[serde(default, deserialize_with = "de_id_string")]
    pub item_id: String,

    #[serde(default, deserialize_with = "de_opt_id_string")]
    pub id: Option<String>,
    #[serde(default, deserialize_with = "de_opt_id_string")]
    pub user_id: Option<String>,

    #[serde(default, deserialize_with = "de_timestamp_us")]
    pub timestamp: i64,
    #[serde(default)]
    pub item_type: Option<String>,
    #[serde(default)]
    pub text: Option<String>,
}

impl DirectMessage {
    pub fn message_id(&self) -> String {
        if !self.item_id.is_empty() {
            return self.item_id.clone();
        }
        self.id.clone().unwrap_or_default()
    }

    pub fn timestamp_rfc3339(&self) -> String {
        if self.timestamp <= 0 {
            return String::new();
        }

        let secs = self.timestamp / 1_000_000;
        let nsecs = ((self.timestamp % 1_000_000) * 1000) as u32;
        chrono::DateTime::<chrono::Utc>::from_timestamp(secs, nsecs)
            .map(|dt| dt.to_rfc3339())
            .unwrap_or_default()
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct InboxResponse {
    #[serde(default)]
    pub inbox: InboxBody,
    #[serde(default)]
    pub status: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct InboxBody {
    #[serde(default)]
    pub threads: Vec<DirectThread>,
    #[serde(default, deserialize_with = "de_opt_id_string")]
    pub oldest_cursor: Option<String>,
    #[serde(default)]
    pub has_older: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ThreadResponse {
    #[serde(default)]
    pub thread: Option<DirectThread>,
    #[serde(default)]
    pub status: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UserInfoResponse {
    #[serde(default)]
    pub user: Option<User>,
    #[serde(default)]
    pub status: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CurrentUserResponse {
    #[serde(default)]
    pub user: Option<User>,
    #[serde(default)]
    pub status: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DeleteMediaResponse {
    #[serde(default)]
    pub did_delete: Option<bool>,
    #[serde(default)]
    pub status: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct StatusResponse {
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum MediaItem {
    Wrapped { media: Media },
    Plain(Media),
}

impl MediaItem {
    pub fn into_media(self) -> Media {
        match self {
            Self::Wrapped { media } => media,
            Self::Plain(media) => media,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ActivityFeedResponse {
    #[serde(default)]
    pub items: Vec<MediaItem>,
    #[serde(default, deserialize_with = "de_opt_id_string")]
    pub next_max_id: Option<String>,

    #[serde(default, deserialize_with = "de_opt_id_string")]
    pub max_id: Option<String>,
    #[serde(default)]
    pub more_available: bool,
    #[serde(default)]
    pub status: Option<String>,
}

impl ActivityFeedResponse {
    pub fn next_cursor(&self) -> Option<String> {
        self.next_max_id
            .clone()
            .or_else(|| self.max_id.clone())
            .filter(|s| !s.is_empty())
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Collection {
    #[serde(default, deserialize_with = "de_id_string")]
    pub collection_id: String,
    #[serde(default)]
    pub collection_name: String,
    #[serde(default)]
    pub collection_type: Option<String>,
    #[serde(default)]
    pub collection_media_count: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CollectionsResponse {
    #[serde(default)]
    pub items: Vec<Collection>,
    #[serde(default, deserialize_with = "de_opt_id_string")]
    pub next_max_id: Option<String>,
    #[serde(default, deserialize_with = "de_opt_id_string")]
    pub max_id: Option<String>,
    #[serde(default)]
    pub more_available: bool,
    #[serde(default)]
    pub status: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct Comment {
    #[serde(default, deserialize_with = "de_id_string")]
    pub pk: String,
    #[serde(default)]
    pub text: String,

    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub user: Option<UserShort>,

    #[serde(default, deserialize_with = "de_opt_id_string")]
    pub media_id: Option<String>,

    #[serde(default, deserialize_with = "de_opt_id_string")]
    pub parent_comment_id: Option<String>,
}

impl Comment {
    pub fn author_id(&self) -> String {
        self.user.as_ref().map(|u| u.pk.clone()).unwrap_or_default()
    }

    pub fn author_username(&self) -> String {
        self.user
            .as_ref()
            .map(|u| u.username.clone())
            .unwrap_or_default()
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct CommentsResponse {
    #[serde(default)]
    pub comments: Vec<Comment>,
    #[serde(default, deserialize_with = "de_opt_id_string")]
    pub next_max_id: Option<String>,
    #[serde(default, deserialize_with = "de_opt_id_string")]
    pub next_min_id: Option<String>,
    #[serde(default)]
    pub has_more_comments: bool,
    #[serde(default)]
    pub has_more_headload_comments: bool,
    #[serde(default)]
    pub comment_count: Option<i64>,
    #[serde(default)]
    pub status: Option<String>,
}

fn de_id_string<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(de_opt_id_string(deserializer)?.unwrap_or_default())
}

fn de_opt_id_string<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde_json::Value;
    match Value::deserialize(deserializer)? {
        Value::Null | Value::Bool(_) => Ok(None),
        Value::String(s) => {
            let t = s.trim();
            if t.is_empty() {
                Ok(None)
            } else {
                Ok(Some(t.to_string()))
            }
        }
        Value::Number(n) => Ok(Some(n.to_string())),
        _ => Ok(None),
    }
}

fn de_timestamp_us<'de, D>(deserializer: D) -> Result<i64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::{self, Visitor};
    use std::fmt;

    struct TsVisitor;
    impl<'de> Visitor<'de> for TsVisitor {
        type Value = i64;

        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            f.write_str("timestamp as int or string")
        }

        fn visit_i64<E: de::Error>(self, v: i64) -> Result<Self::Value, E> {
            Ok(v)
        }

        fn visit_u64<E: de::Error>(self, v: u64) -> Result<Self::Value, E> {
            Ok(v as i64)
        }

        fn visit_u128<E: de::Error>(self, v: u128) -> Result<Self::Value, E> {
            Ok(v as i64)
        }

        fn visit_i128<E: de::Error>(self, v: i128) -> Result<Self::Value, E> {
            Ok(v as i64)
        }

        fn visit_f64<E: de::Error>(self, v: f64) -> Result<Self::Value, E> {
            Ok(v as i64)
        }

        fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
            let s = v.trim();
            if s.is_empty() {
                return Ok(0);
            }
            if let Ok(i) = s.parse::<i64>() {
                return Ok(i);
            }
            if let Ok(f) = s.parse::<f64>() {
                return Ok(f as i64);
            }
            Ok(0)
        }

        fn visit_string<E: de::Error>(self, v: String) -> Result<Self::Value, E> {
            self.visit_str(&v)
        }

        fn visit_none<E: de::Error>(self) -> Result<Self::Value, E> {
            Ok(0)
        }

        fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
            Ok(0)
        }

        fn visit_bool<E: de::Error>(self, _v: bool) -> Result<Self::Value, E> {
            Ok(0)
        }
    }

    deserializer.deserialize_any(TsVisitor)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Deserialize)]
    struct TestMsg {
        #[serde(default, deserialize_with = "de_id_string")]
        id: String,
        #[serde(default, deserialize_with = "de_timestamp_us")]
        ts: i64,
    }

    #[test]
    fn deserializes_128bit_id_and_various_timestamps() {
        let json = r#"{"id": 340282366841710301281152737272051356071, "ts": "1710000000.5"}"#;
        let msg: TestMsg = serde_json::from_str(json).unwrap();
        assert_eq!(msg.id, "340282366841710301281152737272051356071");
        assert_eq!(msg.ts, 1710000000);

        let json_empty = r#"{"id": null, "ts": ""}"#;
        let msg_empty: TestMsg = serde_json::from_str(json_empty).unwrap();
        assert_eq!(msg_empty.id, "");
        assert_eq!(msg_empty.ts, 0);

        let json_bool = r#"{"id": false, "ts": false}"#;
        let msg_bool: TestMsg = serde_json::from_str(json_bool).unwrap();
        assert_eq!(msg_bool.id, "");
        assert_eq!(msg_bool.ts, 0);
    }
}
