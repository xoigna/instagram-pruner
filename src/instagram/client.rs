use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use serde_json::{json, Value};
use wreq::header::{HeaderMap, HeaderName, HeaderValue, CONTENT_TYPE, COOKIE, USER_AGENT};
use wreq_util::{Emulation, Platform, Profile};

use super::device::{
    jitter_u64, resolve_session_path, DeviceIdentity, SessionStore, API_BASE, API_DOMAIN,
};
use super::error::{IgError, IgResult};
use super::model::{
    ActivityFeedResponse, Collection, CollectionsResponse, Comment, CommentsResponse,
    CurrentUserResponse, DeleteMediaResponse, DirectThread, FeedResponse, InboxResponse, Media,
    MediaItem, StatusResponse, ThreadResponse, User, UserInfoResponse,
};
use super::ratelimit::RateLimiter;

#[derive(Debug, Clone, Default)]
struct LiveAuth {
    mid: String,
    ig_u_rur: String,
    ig_www_claim: String,
    authorization: String,
    user_id: String,
    username: String,
    sessionid: String,
}

pub struct InstagramClient {
    http: wreq::Client,
    device: DeviceIdentity,
    live: Mutex<LiveAuth>,
    rate_limiter: Arc<RateLimiter>,
    base_url: String,
    session_path: PathBuf,
}

impl InstagramClient {
    pub async fn new(
        sessionid: &str,
        base_url: Option<&str>,
        session_path: Option<&str>,
        rate_limiter: Arc<RateLimiter>,
    ) -> IgResult<Self> {
        let session_path = resolve_session_path(session_path);
        let provided = sessionid.trim();
        let mut store = SessionStore::load(&session_path).unwrap_or_default();

        let sessionid = if !provided.is_empty() {
            let sid = normalize_sessionid(provided)?;
            let prev = store.sessionid.clone();
            store.sessionid = sid.clone();

            if prev.is_empty() || prev != sid {
                store.user_id.clear();
                store.username.clear();
                store.authorization.clear();
                store.mid.clear();
                store.ig_u_rur.clear();
                store.ig_www_claim.clear();
            }
            let _ = store.ensure_device();
            sid
        } else if !store.sessionid.is_empty() {
            let _ = store.ensure_device();
            normalize_sessionid(&store.sessionid)?
        } else {
            return Err(IgError::BadSessionId);
        };

        let user_id_hint = if !store.user_id.is_empty() {
            store.user_id.clone()
        } else {
            user_id_from_sessionid(&sessionid)?
        };

        let device = store.device.clone().unwrap_or_else(DeviceIdentity::fresh);

        let authorization = build_authorization(&user_id_hint, &sessionid);

        let base_url = base_url
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(API_BASE)
            .trim_end_matches('/')
            .to_string();

        let emulation = Emulation::builder()
            .profile(Profile::OkHttp4_12)
            .platform(Platform::Android)
            .headers(false)
            .build();

        let http = wreq::Client::builder()
            .emulation(emulation)
            .timeout(Duration::from_secs(30))
            .build()?;

        let live = LiveAuth {
            mid: store.mid.clone(),
            ig_u_rur: store.ig_u_rur.clone(),
            ig_www_claim: if store.ig_www_claim.is_empty() {
                "0".into()
            } else {
                store.ig_www_claim.clone()
            },
            authorization,
            user_id: user_id_hint.clone(),
            username: store.username.clone(),
            sessionid: sessionid.clone(),
        };

        let client = Self {
            http,
            device,
            live: Mutex::new(live),
            rate_limiter,
            base_url,
            session_path,
        };

        match client.fetch_current_user().await {
            Ok(user) => {
                {
                    let mut live = client.live.lock().expect("live auth poisoned");
                    if !user.pk.is_empty() {
                        live.user_id = user.pk;
                    }
                    live.username = user.username;
                    live.authorization = build_authorization(&live.user_id, &live.sessionid);
                }
                client.persist_session();
            }
            Err(e) => {
                tracing::warn!("current_user failed ({e}); trying users/{{id}}/info/");
                let user = client.fetch_user_info(&user_id_hint).await?;
                {
                    let mut live = client.live.lock().expect("live auth poisoned");
                    live.user_id = user.pk;
                    live.username = user.username;
                    live.authorization = build_authorization(&live.user_id, &live.sessionid);
                }
                client.persist_session();
            }
        }

        Ok(client)
    }

    #[allow(dead_code)]
    pub fn session_path(&self) -> &Path {
        &self.session_path
    }

    pub fn user_id(&self) -> String {
        self.live
            .lock()
            .expect("live auth poisoned")
            .user_id
            .clone()
    }

    pub fn username(&self) -> String {
        self.live
            .lock()
            .expect("live auth poisoned")
            .username
            .clone()
    }

    async fn fetch_current_user(&self) -> IgResult<User> {
        let resp = self.get("accounts/current_user/", &[]).await?;
        let body: CurrentUserResponse = parse_json(resp).await?;
        body.user
            .ok_or_else(|| IgError::Other("missing user in current_user".into()))
    }

    pub async fn fetch_user_info(&self, user_id: &str) -> IgResult<User> {
        let resp = self.get(&format!("users/{user_id}/info/"), &[]).await?;
        let body: UserInfoResponse = parse_json(resp).await?;
        body.user
            .ok_or_else(|| IgError::Other("missing user in users/info".into()))
    }

    pub async fn user_medias_page(
        &self,
        user_id: &str,
        max_id: Option<&str>,
        count: u32,
    ) -> IgResult<(Vec<Media>, Option<String>)> {
        let count_s = count.to_string();
        let uid = self.user_id();
        let rank_token = format!("{}_{}", uid, self.device.uuid);
        let mut params: Vec<(&str, &str)> = vec![
            ("count", count_s.as_str()),
            ("rank_token", rank_token.as_str()),
            ("ranked_content", "true"),
        ];
        if let Some(cursor) = max_id {
            if !cursor.is_empty() {
                params.push(("max_id", cursor));
            }
        }
        let resp = self.get(&format!("feed/user/{user_id}/"), &params).await?;
        let body: FeedResponse = parse_json(resp).await?;
        let next = if body.more_available {
            body.next_max_id.filter(|s| !s.is_empty())
        } else {
            None
        };
        Ok((body.items, next))
    }

    pub async fn media_delete(&self, media_id: &str) -> IgResult<bool> {
        let data = self.default_data(json!({ "media_id": media_id }));
        let resp = self
            .post_signed(&format!("media/{media_id}/delete/"), &data)
            .await?;
        let body: DeleteMediaResponse = parse_json(resp).await?;
        Ok(body
            .did_delete
            .unwrap_or(body.status.as_deref() == Some("ok")))
    }

    pub async fn liked_medias_page(
        &self,
        max_id: Option<&str>,
    ) -> IgResult<(Vec<Media>, Option<String>)> {
        self.activity_feed_page("feed/liked/", max_id).await
    }

    pub async fn saved_medias_page(
        &self,
        max_id: Option<&str>,
    ) -> IgResult<(Vec<Media>, Option<String>)> {
        self.activity_feed_page("feed/saved/posts/", max_id).await
    }

    pub async fn archived_medias_page(
        &self,
        max_id: Option<&str>,
    ) -> IgResult<(Vec<Media>, Option<String>)> {
        self.activity_feed_page("feed/only_me_feed/", max_id).await
    }

    async fn activity_feed_page(
        &self,
        path: &str,
        max_id: Option<&str>,
    ) -> IgResult<(Vec<Media>, Option<String>)> {
        let mut params: Vec<(&str, &str)> = vec![("include_igtv_preview", "false")];
        if let Some(cursor) = max_id.filter(|s| !s.is_empty()) {
            params.push(("max_id", cursor));
        }
        let resp = self.get(path, &params).await?;
        let body: ActivityFeedResponse = parse_json(resp).await?;
        let next = body.next_cursor();
        let items = body.items.into_iter().map(MediaItem::into_media).collect();
        Ok((items, next))
    }

    pub async fn collections(&self) -> IgResult<Vec<Collection>> {
        const TYPES: &str = r#"["ALL_MEDIA_AUTO_COLLECTION","PRODUCT_AUTO_COLLECTION","MEDIA"]"#;
        let mut out = Vec::new();
        let mut cursor: Option<String> = None;

        for _ in 0..10 {
            let mut params: Vec<(&str, &str)> = vec![("collection_types", TYPES)];
            if let Some(c) = cursor.as_deref().filter(|s| !s.is_empty()) {
                params.push(("max_id", c));
            }
            let resp = self.get("collections/list/", &params).await?;
            let body: CollectionsResponse = parse_json(resp).await?;
            out.extend(body.items);
            let next = body
                .next_max_id
                .or(body.max_id)
                .filter(|s| !s.is_empty() && Some(s.as_str()) != cursor.as_deref());
            match next {
                Some(n) if body.more_available => cursor = Some(n),
                _ => break,
            }
        }
        Ok(out)
    }

    pub async fn collection_medias_page(
        &self,
        collection_id: &str,
        max_id: Option<&str>,
    ) -> IgResult<(Vec<Media>, Option<String>)> {
        self.activity_feed_page(&format!("feed/collection/{collection_id}/"), max_id)
            .await
    }

    pub async fn media_unlike(&self, media_id: &str) -> IgResult<bool> {
        let data = self.action_data(json!({ "media_id": media_id }));
        self.post_status(&format!("media/{media_id}/unlike/"), &data)
            .await
    }

    pub async fn media_unsave(&self, media_id: &str) -> IgResult<bool> {
        let data = self.action_data(json!({
            "media_id": media_id,
            "module_name": "feed_timeline",
        }));
        self.post_status(&format!("media/{media_id}/unsave/"), &data)
            .await
    }

    pub async fn media_unsave_from_collection(
        &self,
        media_id: &str,
        collection_id: &str,
    ) -> IgResult<bool> {
        let data = self.action_data(json!({
            "media_id": media_id,
            "module_name": "feed_timeline",
            "removed_collection_ids": format!("[{collection_id}]"),
        }));
        self.post_status(&format!("media/{media_id}/unsave/"), &data)
            .await
    }

    pub async fn media_comments_page(
        &self,
        media_id: &str,
        max_id: Option<&str>,
    ) -> IgResult<(Vec<Comment>, Option<String>)> {
        let mut params: Vec<(&str, &str)> = vec![
            ("can_support_threading", "true"),
            ("permalink_enabled", "false"),
        ];
        if let Some(cursor) = max_id.filter(|s| !s.is_empty()) {
            params.push(("max_id", cursor));
        }
        let resp = self
            .get(&format!("media/{media_id}/comments/"), &params)
            .await?;
        let body: CommentsResponse = parse_json(resp).await?;
        let next = body
            .next_max_id
            .or(body.next_min_id)
            .filter(|s| !s.is_empty());
        Ok((body.comments, next))
    }

    pub async fn comments_bulk_delete(
        &self,
        media_id: &str,
        comment_ids: &[String],
    ) -> IgResult<bool> {
        let joined = comment_ids.join(",");
        let data = self.action_data(json!({
            "comment_ids_to_delete": joined,
            "container_module": "self_comments_v2_newsfeed_you",
        }));
        self.post_status(&format!("media/{media_id}/comment/bulk_delete/"), &data)
            .await
    }

    pub async fn direct_inbox_page(
        &self,
        cursor: Option<&str>,
        thread_message_limit: u32,
    ) -> IgResult<(Vec<DirectThread>, Option<String>)> {
        let limit = thread_message_limit.to_string();
        let mut params: Vec<(&str, &str)> = vec![
            ("visual_message_return_type", "unseen"),
            ("thread_message_limit", limit.as_str()),
            ("persistentBadging", "true"),
            ("limit", "20"),
            ("is_prefetching", "false"),
            ("include_old_mrs", "false"),
            ("no_pending_badge", "true"),
        ];
        if let Some(c) = cursor.filter(|s| !s.is_empty()) {
            params.push(("cursor", c));
            params.push(("direction", "older"));
            params.push(("fetch_reason", "page_scroll"));
        } else {
            params.push(("fetch_reason", "initial_snapshot"));
        }
        let resp = self.get("direct_v2/inbox/", &params).await?;
        let body: InboxResponse = parse_json(resp).await?;
        let next = if body.inbox.has_older {
            body.inbox.oldest_cursor.filter(|s| !s.is_empty())
        } else {
            None
        };
        Ok((body.inbox.threads, next))
    }

    pub async fn direct_thread(
        &self,
        thread_id: &str,
        amount: u32,
        cursor: Option<&str>,
    ) -> IgResult<DirectThread> {
        let limit = amount.clamp(1, 100).to_string();
        let mut params: Vec<(&str, &str)> = vec![
            ("visual_message_return_type", "unseen"),
            ("limit", limit.as_str()),
            ("direction", "older"),
        ];
        if let Some(c) = cursor.filter(|s| !s.is_empty()) {
            params.push(("cursor", c));
        }
        let resp = self
            .get(&format!("direct_v2/threads/{thread_id}/"), &params)
            .await?;
        let body: ThreadResponse = parse_json(resp).await?;
        body.thread
            .ok_or_else(|| IgError::Other(format!("missing thread {thread_id}")))
    }

    pub async fn direct_message_unsend(&self, thread_id: &str, message_id: &str) -> IgResult<bool> {
        let mut data = self.default_data(json!({}));
        if let Value::Object(map) = &mut data {
            map.remove("device_id");
        }
        let resp = self
            .post_signed(
                &format!("direct_v2/threads/{thread_id}/items/{message_id}/delete/"),
                &data,
            )
            .await?;
        let body: StatusResponse = parse_json(resp).await?;
        Ok(body.status.as_deref() == Some("ok"))
    }

    pub async fn direct_thread_hide(&self, thread_id: &str) -> IgResult<bool> {
        let data = json!({
            "should_move_future_requests_to_spam": false,
            "_uuid": self.device.uuid,
        });
        let resp = self
            .post_form(
                &format!("direct_v2/threads/{thread_id}/hide/"),
                &data,
                false,
            )
            .await?;
        let body: StatusResponse = parse_json(resp).await?;
        Ok(body.status.as_deref() == Some("ok"))
    }

    fn default_data(&self, extra: Value) -> Value {
        let uid = self.user_id();
        let mut map = serde_json::Map::new();
        map.insert("_uuid".into(), json!(self.device.uuid));
        map.insert("_uid".into(), json!(uid));
        map.insert("device_id".into(), json!(self.device.android_device_id));
        if let Value::Object(extra_map) = extra {
            for (k, v) in extra_map {
                map.insert(k, v);
            }
        }
        Value::Object(map)
    }

    fn action_data(&self, extra: Value) -> Value {
        self.default_data(extra_with_radio(extra))
    }

    async fn post_status(&self, path: &str, data: &Value) -> IgResult<bool> {
        let resp = self.post_signed(path, data).await?;
        let body: StatusResponse = parse_json(resp).await?;
        Ok(body.status.as_deref() == Some("ok"))
    }

    pub(crate) async fn raw_get_json(
        &self,
        path: &str,
        params: &[(&str, &str)],
    ) -> IgResult<Value> {
        let resp = self.get(path, params).await?;
        parse_json(resp).await
    }

    async fn get(&self, path: &str, params: &[(&str, &str)]) -> IgResult<wreq::Response> {
        self.request("GET", path, params, None).await
    }

    async fn post_signed(&self, path: &str, data: &Value) -> IgResult<wreq::Response> {
        self.request("POST", path, &[], Some((data, true))).await
    }

    async fn post_form(&self, path: &str, data: &Value, signed: bool) -> IgResult<wreq::Response> {
        self.request("POST", path, &[], Some((data, signed))).await
    }

    async fn request(
        &self,
        method: &str,
        path: &str,
        params: &[(&str, &str)],
        body: Option<(&Value, bool)>,
    ) -> IgResult<wreq::Response> {
        for attempt in 0..8u32 {
            if self.rate_limiter.is_cancelled() {
                return Err(IgError::Cancelled);
            }
            self.rate_limiter.wait().await;
            if self.rate_limiter.is_cancelled() {
                return Err(IgError::Cancelled);
            }

            let url = format!("{}/{}", self.base_url, path.trim_start_matches('/'));
            let mut builder = match method {
                "GET" => self.http.get(&url),
                "POST" => self.http.post(&url),
                other => return Err(IgError::Other(format!("unsupported method {other}"))),
            };
            builder = builder.headers(self.base_headers());
            if !params.is_empty() {
                builder = builder.query(params);
            }
            if let Some((data, signed)) = body {
                let form = if signed {
                    generate_signature(data)
                } else {
                    form_encode(data)
                };
                builder = builder
                    .header(
                        CONTENT_TYPE,
                        "application/x-www-form-urlencoded; charset=UTF-8",
                    )
                    .body(form);
            }

            let resp = match builder.send().await {
                Ok(r) => r,
                Err(e) if attempt < 2 && !self.rate_limiter.is_cancelled() => {
                    tracing::warn!("network error on {path}: {e}; retrying (attempt {attempt})");
                    tokio::time::sleep(Duration::from_millis(500 * (attempt as u64 + 1))).await;
                    continue;
                }
                Err(e) => return Err(e.into()),
            };
            self.absorb_response_headers(&resp);

            let status = resp.status().as_u16();
            let retry_after_ms = parse_retry_after_ms(&resp);

            if (500..600).contains(&status) && attempt < 2 && !self.rate_limiter.is_cancelled() {
                tracing::warn!(
                    "HTTP {status} server error on {path}; retrying (attempt {attempt})"
                );
                tokio::time::sleep(Duration::from_millis(1000 * (attempt as u64 + 1))).await;
                continue;
            }
            if status == 429 {
                let wait = self
                    .rate_limiter
                    .observe_rate_limit(retry_after_ms.unwrap_or(60_000));
                tracing::warn!("HTTP 429 on {path}; cooldown {wait}ms (attempt {attempt})");
                if attempt + 1 == 8 {
                    return Err(IgError::RateLimited {
                        retry_after_ms: wait,
                        global: true,
                        scope: "user".into(),
                    });
                }
                continue;
            }

            if status == 404 {
                self.rate_limiter.observe_success();
                return Err(IgError::NotFound {
                    resource: path.to_string(),
                    status,
                });
            }

            if !(200..300).contains(&status) {
                let text = resp.text().await.unwrap_or_default();
                if let Ok(v) = serde_json::from_str::<Value>(&text) {
                    if let Some(msg) = v.get("message").and_then(|m| m.as_str()) {
                        let lower = msg.to_ascii_lowercase();
                        if lower.contains("login_required") {
                            self.rate_limiter.observe_success();
                            return Err(IgError::LoginRequired);
                        }
                        if lower.contains("challenge") {
                            self.rate_limiter.observe_success();
                            return Err(IgError::ChallengeRequired {
                                message: msg.to_string(),
                            });
                        }
                        if lower.contains("feedback")
                            || lower.contains("please wait a few minutes")
                            || lower.contains("rate limit")
                        {
                            let wait = self
                                .rate_limiter
                                .observe_rate_limit(retry_after_ms.unwrap_or(120_000));
                            if attempt + 1 < 8 {
                                tracing::warn!("throttled ({msg}) on {path}; cooldown {wait}ms");
                                continue;
                            }
                            return Err(IgError::FeedbackRequired {
                                message: msg.to_string(),
                            });
                        }
                        if status == 403 {
                            self.rate_limiter.observe_success();
                            return Err(IgError::Forbidden {
                                resource: path.to_string(),
                                status,
                            });
                        }
                        if status == 401 {
                            self.rate_limiter.observe_success();
                            return Err(IgError::InvalidSession { status });
                        }
                        self.rate_limiter.observe_success();
                        return Err(IgError::Api {
                            status,
                            message: msg.to_string(),
                        });
                    }
                }
                if status == 403 {
                    self.rate_limiter.observe_success();
                    return Err(IgError::Forbidden {
                        resource: path.to_string(),
                        status,
                    });
                }
                if status == 401 {
                    self.rate_limiter.observe_success();
                    return Err(IgError::InvalidSession { status });
                }
                self.rate_limiter.observe_success();
                return Err(IgError::Api {
                    status,
                    message: truncate_err(&text),
                });
            }

            self.rate_limiter.observe_success();

            if attempt == 0 && jitter_u64(0, 9) == 0 {
                self.persist_session();
            }
            return Ok(resp);
        }
        Err(IgError::Other("exhausted retries".into()))
    }

    fn absorb_response_headers(&self, resp: &wreq::Response) {
        let headers = resp.headers();
        let mut live = self.live.lock().expect("live auth poisoned");
        let mut dirty = false;

        if let Some(v) =
            header_str(headers, "ig-set-x-mid").or_else(|| header_str(headers, "x-mid"))
        {
            if !v.is_empty() && v != live.mid {
                live.mid = v;
                dirty = true;
            }
        }
        if let Some(v) = header_str(headers, "ig-set-ig-u-rur")
            .or_else(|| header_str(headers, "ig-u-rur"))
            .or_else(|| header_str(headers, "x-ig-set-ig-u-rur"))
        {
            if !v.is_empty() && v != live.ig_u_rur {
                live.ig_u_rur = v;
                dirty = true;
            }
        }
        if let Some(v) = header_str(headers, "x-ig-set-www-claim")
            .or_else(|| header_str(headers, "ig-set-www-claim"))
            .or_else(|| header_str(headers, "x-ig-www-claim"))
        {
            if !v.is_empty() && v != "0" && v != live.ig_www_claim {
                live.ig_www_claim = v;
                dirty = true;
            }
        }
        if let Some(v) = header_str(headers, "ig-set-authorization") {
            if v.starts_with("Bearer ") && v != live.authorization {
                live.authorization = v;
                dirty = true;
            }
        }

        drop(live);
        if dirty {
            self.persist_session();
        }
    }

    fn persist_session(&self) {
        let live = self.live.lock().expect("live auth poisoned");
        let store = SessionStore {
            sessionid: live.sessionid.clone(),
            user_id: live.user_id.clone(),
            username: live.username.clone(),
            mid: live.mid.clone(),
            ig_u_rur: live.ig_u_rur.clone(),
            ig_www_claim: live.ig_www_claim.clone(),
            authorization: live.authorization.clone(),
            device: Some(self.device.clone()),
            last_login_unix: Some(chrono::Utc::now().timestamp()),
            version: 1,
        };
        if let Err(e) = store.save(&self.session_path) {
            tracing::warn!(
                "failed to persist session to {}: {e}",
                self.session_path.display()
            );
        }
    }

    fn base_headers(&self) -> HeaderMap {
        let live = self.live.lock().expect("live auth poisoned");
        let mut headers = HeaderMap::new();
        let locale = &self.device.locale;
        let lang = locale.replace('_', "-");

        set(&mut headers, USER_AGENT, &self.device.user_agent);
        set(&mut headers, "Accept-Language", &format!("{lang}, en-US"));
        set(&mut headers, "Accept-Encoding", "gzip");
        set(&mut headers, "X-IG-App-Locale", locale);
        set(&mut headers, "X-IG-Device-Locale", locale);
        set(&mut headers, "X-IG-Mapped-Locale", locale);
        set(
            &mut headers,
            "X-Pigeon-Session-Id",
            &format!("UFS-{}-1", self.device.client_session_id),
        );
        set(
            &mut headers,
            "X-Pigeon-Rawclienttime",
            &format!(
                "{:.3}",
                chrono::Utc::now().timestamp_millis() as f64 / 1000.0
            ),
        );

        set(
            &mut headers,
            "X-IG-Bandwidth-Speed-KBPS",
            &format!("{:.3}", jitter_u64(2_000_000, 3_000_000) as f64 / 1000.0),
        );
        set(
            &mut headers,
            "X-IG-Bandwidth-TotalBytes-B",
            &jitter_u64(5_000_000, 90_000_000).to_string(),
        );
        set(
            &mut headers,
            "X-IG-Bandwidth-TotalTime-MS",
            &jitter_u64(2_000, 9_000).to_string(),
        );
        set(
            &mut headers,
            "X-IG-App-Startup-Country",
            &self.device.country,
        );
        set(
            &mut headers,
            "X-Bloks-Version-Id",
            &self.device.bloks_versioning_id,
        );
        let claim = if live.ig_www_claim.is_empty() {
            "0"
        } else {
            live.ig_www_claim.as_str()
        };
        set(&mut headers, "X-IG-WWW-Claim", claim);
        set(&mut headers, "X-Bloks-Is-Layout-RTL", "false");
        set(&mut headers, "X-Bloks-Is-Panorama-Enabled", "true");
        set(&mut headers, "X-IG-Device-ID", &self.device.uuid);
        set(&mut headers, "X-IG-Family-Device-ID", &self.device.phone_id);
        set(
            &mut headers,
            "X-IG-Android-ID",
            &self.device.android_device_id,
        );
        set(
            &mut headers,
            "X-IG-Timezone-Offset",
            &self.device.timezone_offset.to_string(),
        );
        set(&mut headers, "X-IG-Connection-Type", "WIFI");
        set(&mut headers, "X-IG-Capabilities", "3brTv10=");
        set(&mut headers, "X-IG-App-ID", &self.device.app_id);
        set(&mut headers, "Priority", "u=3");
        set(&mut headers, "Host", API_DOMAIN);
        set(&mut headers, "X-FB-HTTP-Engine", "Tigon/MNS/TCP");
        set(&mut headers, "X-FB-Client-IP", "True");
        set(&mut headers, "X-FB-Server-Cluster", "True");
        set(&mut headers, "X-Tigon-Is-Retry", "False");
        set(&mut headers, "IG-INTENDED-USER-ID", &live.user_id);
        set(&mut headers, "IG-U-DS-USER-ID", &live.user_id);
        if !live.ig_u_rur.is_empty() {
            set(&mut headers, "IG-U-RUR", &live.ig_u_rur);
        }
        if !live.mid.is_empty() {
            set(&mut headers, "X-MID", &live.mid);
        }
        set(
            &mut headers,
            "X-IG-SALT-IDS",
            &jitter_u64(1_061_162_222, 1_061_262_222).to_string(),
        );
        set(&mut headers, "Authorization", &live.authorization);

        let mut cookie = format!("sessionid={}; ds_user_id={}", live.sessionid, live.user_id);
        if !live.mid.is_empty() {
            cookie.push_str(&format!("; mid={}", live.mid));
        }
        set(&mut headers, COOKIE, &cookie);

        headers
    }
}

impl Drop for InstagramClient {
    fn drop(&mut self) {
        self.persist_session();
    }
}

fn set(map: &mut HeaderMap, key: impl AsHeaderName, value: &str) {
    if let Ok(val) = HeaderValue::from_str(value) {
        key.insert(map, val);
    }
}

trait AsHeaderName {
    fn insert(self, map: &mut HeaderMap, val: HeaderValue);
}

impl AsHeaderName for HeaderName {
    fn insert(self, map: &mut HeaderMap, val: HeaderValue) {
        map.insert(self, val);
    }
}

impl AsHeaderName for &'static str {
    fn insert(self, map: &mut HeaderMap, val: HeaderValue) {
        if let Ok(name) = HeaderName::from_bytes(self.as_bytes()) {
            map.insert(name, val);
        }
    }
}

fn header_str(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .or_else(|| {
            headers.iter().find_map(|(k, v)| {
                if k.as_str().eq_ignore_ascii_case(name) {
                    Some(v)
                } else {
                    None
                }
            })
        })
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
}

fn build_authorization(user_id: &str, sessionid: &str) -> String {
    let data = json!({
        "ds_user_id": user_id,
        "sessionid": sessionid,
        "should_use_header_over_cookies": true,
    });
    let b64 = BASE64.encode(serde_json::to_vec(&data).unwrap_or_default());
    format!("Bearer IGT:2:{b64}")
}

fn generate_signature(data: &Value) -> String {
    let dumped = serde_json::to_string(data).unwrap_or_else(|_| "{}".into());
    format!("signed_body=SIGNATURE.{}", urlencoding::encode(&dumped))
}

fn extra_with_radio(extra: Value) -> Value {
    match extra {
        Value::Object(mut map) => {
            map.entry("radio_type")
                .or_insert_with(|| json!("wifi-none"));
            Value::Object(map)
        }
        other => other,
    }
}

fn form_encode(data: &Value) -> String {
    let mut parts = Vec::new();
    if let Value::Object(map) = data {
        for (k, v) in map {
            let val = match v {
                Value::String(s) => s.clone(),
                other => other.to_string().trim_matches('"').to_string(),
            };
            parts.push(format!(
                "{}={}",
                urlencoding::encode(k),
                urlencoding::encode(&val)
            ));
        }
    }
    parts.join("&")
}

fn normalize_sessionid(raw: &str) -> IgResult<String> {
    let s = raw.trim().trim_matches('"');
    let s = if let Some(idx) = s.find("sessionid=") {
        let after = &s[idx + "sessionid=".len()..];
        after.split(';').next().unwrap_or(after).trim()
    } else {
        s.split(';').next().unwrap_or(s).trim()
    };
    if s.len() < 20 {
        return Err(IgError::BadSessionId);
    }
    Ok(urlencoding::decode(s)
        .map(|c| c.into_owned())
        .unwrap_or_else(|_| s.to_string()))
}

fn user_id_from_sessionid(sessionid: &str) -> IgResult<String> {
    let digits: String = sessionid
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    if digits.is_empty() {
        return Err(IgError::BadSessionId);
    }
    Ok(digits)
}

fn parse_retry_after_ms(resp: &wreq::Response) -> Option<u64> {
    resp.headers().get("retry-after").and_then(|v| {
        v.to_str()
            .ok()
            .and_then(|s| s.parse::<f64>().ok())
            .map(|secs| (secs * 1000.0) as u64)
    })
}

async fn parse_json<T: serde::de::DeserializeOwned>(resp: wreq::Response) -> IgResult<T> {
    let bytes = resp.bytes().await?;
    match serde_json::from_slice(&bytes) {
        Ok(val) => Ok(val),
        Err(e) => {
            let preview = String::from_utf8_lossy(&bytes);
            tracing::warn!(
                "JSON parse error ({e}) on response: {}",
                truncate_err(&preview)
            );
            Err(IgError::Json(e))
        }
    }
}

fn truncate_err(s: &str) -> String {
    let t = s.trim();
    if t.chars().count() > 240 {
        format!("{}…", t.chars().take(240).collect::<String>())
    } else {
        t.to_string()
    }
}

pub async fn list_all_threads(
    client: &InstagramClient,
    max_threads: usize,
) -> IgResult<Vec<DirectThread>> {
    let mut out = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let (page, next) = client.direct_inbox_page(cursor.as_deref(), 10).await?;
        let n = page.len();
        out.extend(page);
        if max_threads > 0 && out.len() >= max_threads {
            out.truncate(max_threads);
            break;
        }
        if next.is_none() || n == 0 || next == cursor {
            break;
        }
        cursor = next;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_cookie_form() {
        let s = normalize_sessionid("sessionid=99%3Atokenvalueherexxxxxx; Path=/").unwrap();
        assert!(s.starts_with("99"));
    }
    #[test]
    fn normalize_cookie_header_with_other_cookies() {
        let s = normalize_sessionid(
            "mid=XYZ; ds_user_id=123; sessionid=99%3Atokenvalueherexxxxxx; rur=PRN",
        )
        .unwrap();
        assert!(s.starts_with("99"));
    }

    #[test]
    fn user_id_prefix() {
        assert_eq!(user_id_from_sessionid("4242:abc:def").unwrap(), "4242");
    }

    #[test]
    fn signature_prefix() {
        let body = generate_signature(&json!({"_uuid": "x"}));
        assert!(body.starts_with("signed_body=SIGNATURE."));
    }

    #[test]
    fn authorization_header_shape() {
        let h = build_authorization("1", "1:sess");
        assert!(h.starts_with("Bearer IGT:2:"));
    }
}
