//! A connector for Bluesky (any AT Protocol server: bsky.social or a self-hosted PDS,
//! docs.bsky.app), so a brand's account there can be one of Social's accounts. A connection's
//! config may name the account's server (`{"service": "https://bsky.social"}`, the default), and
//! its credentials hold the account's handle and an app password (`{"identifier", "app_password"}`).
//! A session is opened with them (`com.atproto.server.createSession`) and kept for an hour, since
//! servers limit how often one is opened; an expired one is opened again. Through the platform's
//! connections:
//! * `posts`: sent as `app.bsky.feed.post` records, a link made a link facet, a picture uploaded as a
//!   blob (`com.atproto.repo.uploadBlob`, within Bluesky's 1 MB) and embedded with what it shows as
//!   its alt text, under a record key
//!   made from the delivery's key: a retried send finds the record already there
//!   (`com.atproto.repo.getRecord`) rather than making a second. Read back from the account's own
//!   posts without reposts, with likes, reposts, replies and quotes as engagements. Bluesky counts
//!   no impressions.
//! * `messages`: replies, mentions and quotes of the account
//!   (`app.bsky.notification.listNotifications`): a reply is a comment, the others mentions. Direct
//!   messages are Bluesky's chat, which is not read.
//! * `replies`: posts answering the message's post, in its thread.

use async_trait::async_trait;
use chrono::Utc;
use ferrum_kernel::connect::{Connection, Connector, ConnectorInfo, Incoming, Outgoing, Pulled};
use ferrum_kernel::{Error, Kernel, Result};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};
use uuid::Uuid;

/// Posts and notifications a page.
const PAGE: usize = 50;
/// The largest picture Bluesky takes, in bytes.
pub(crate) const MAX_PICTURE: usize = 1_000_000;
/// How long an opened session is used before another is opened.
const SESSION_FOR: Duration = Duration::from_secs(3600);
const POST: &str = "app.bsky.feed.post";

pub struct Bluesky;

/// An open session: its access token, the account's DID and handle, and when it was opened.
#[derive(Clone)]
struct Session {
    jwt: String,
    did: String,
    handle: String,
    opened: Instant,
}

/// Sessions by connection, kept for an hour.
static SESSIONS: LazyLock<Mutex<HashMap<Uuid, Session>>> = LazyLock::new(Default::default);

/// A record key made from the delivery's key: a TID (13 characters of base32-sortable, its top bit
/// clear), as Bluesky's posts take, the same each time the same delivery is tried.
pub(crate) fn rkey(key: &str) -> String {
    const ALPHABET: &[u8; 32] = b"234567abcdefghijklmnopqrstuvwxyz";
    let digest = Sha256::digest(key.as_bytes());
    let mut n = u64::from_be_bytes(digest[..8].try_into().unwrap_or([0; 8])) & 0x7FFF_FFFF_FFFF_FFFF;
    let mut out = [0u8; 13];
    for slot in out.iter_mut().rev() {
        *slot = ALPHABET[(n & 31) as usize];
        n >>= 5;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// A post's text with its link after it, the link made a link facet (by UTF-8 byte offsets).
pub(crate) fn with_link(text: &str, link: Option<&str>) -> (String, Option<Value>) {
    match link.filter(|l| !l.is_empty()) {
        None => (text.to_string(), None),
        Some(l) => {
            let full = if text.is_empty() { l.to_string() } else { format!("{text} {l}") };
            let start = full.len() - l.len();
            let facet = json!([{"index": {"byteStart": start, "byteEnd": full.len()}, "features": [{"$type": "app.bsky.richtext.facet#link", "uri": l}]}]);
            (full, Some(facet))
        }
    }
}

/// A post's address on the web, from its record's AT URI and its author's handle.
pub(crate) fn permalink(uri: &str, handle: &str) -> Option<String> {
    let rkey = uri.rsplit('/').next().filter(|r| !r.is_empty())?;
    Some(format!("https://bsky.app/profile/{handle}/post/{rkey}"))
}

/// What a refusal from the server means here: its limits become the platform's waiting.
fn failure(status: u16, nsid: &str, reset: Option<u64>, text: &str) -> Error {
    let v: Value = serde_json::from_str(text).unwrap_or_default();
    let code = v.get("error").and_then(Value::as_str).unwrap_or_default();
    let said = v.get("message").and_then(Value::as_str).filter(|m| !m.is_empty()).unwrap_or(code);
    let said = if said.is_empty() { text.chars().take(200).collect::<String>() } else { said.to_string() };
    match (status, code) {
        (401, _) | (_, "AuthenticationRequired" | "ExpiredToken" | "InvalidToken") => Error::Unauthorized(format!("Bluesky refused the session: {said}")),
        (404, _) | (_, "RecordNotFound") => Error::NotFound(format!("Bluesky has no such {nsid}")),
        (429, _) => {
            let wait = reset.map(|at| at.saturating_sub(Utc::now().timestamp().max(0) as u64)).filter(|w| *w > 0).unwrap_or(300);
            Error::Wait("Bluesky asks to slow down".into(), wait)
        }
        (400, _) => Error::invalid(format!("Bluesky refused it: {said}")),
        _ => Error::Unavailable(format!("Bluesky answered {status}: {said}")),
    }
}

impl Bluesky {
    fn base(c: &Connection) -> String {
        let s = c.config.get("service").and_then(Value::as_str).map(|s| s.trim().trim_end_matches('/').to_string());
        s.filter(|s| !s.is_empty()).unwrap_or_else(|| "https://bsky.social".into())
    }

    fn login(c: &Connection) -> Result<(String, String)> {
        let field = |k: &str| c.credentials.get(k).and_then(Value::as_str).map(str::trim).filter(|v| !v.is_empty()).map(str::to_string);
        match (field("identifier"), field("app_password")) {
            (Some(i), Some(p)) => Ok((i, p)),
            _ => Err(Error::invalid(
                "the connection's credentials hold the account's handle and an app password: {\"identifier\": \"…\", \"app_password\": \"…\"}",
            )),
        }
    }

    /// One request to the server, its answer read as JSON.
    async fn send(
        k: &Kernel,
        c: &Connection,
        method: reqwest::Method,
        nsid: &str,
        query: &[(&str, &str)],
        body: Option<&Value>,
        jwt: Option<&str>,
    ) -> Result<Value> {
        let mut url = reqwest::Url::parse(&format!("{}/xrpc/{nsid}", Self::base(c)))
            .map_err(|e| Error::invalid(format!("the connection's service is not an address: {e}")))?;
        if !query.is_empty() {
            url.query_pairs_mut().extend_pairs(query);
        }
        let (client, url) = k.vetted_client(url.as_str(), k.settings.webhooks_allow_private, Duration::from_secs(20), "Ferrum Social").await?;
        let mut req = client.request(method, url);
        if let Some(jwt) = jwt {
            req = req.bearer_auth(jwt);
        }
        if let Some(body) = body {
            req = req.json(body);
        }
        let answer = req.send().await.map_err(|e| Error::Unavailable(format!("Bluesky cannot be reached: {e}")))?;
        let status = answer.status().as_u16();
        let reset = answer.headers().get("ratelimit-reset").and_then(|v| v.to_str().ok()).and_then(|v| v.parse::<u64>().ok());
        let text = answer.text().await.unwrap_or_default();
        if (200..300).contains(&status) {
            return Ok(serde_json::from_str(&text).unwrap_or(Value::Null));
        }
        Err(failure(status, nsid, reset, &text))
    }

    /// The connection's session: the one kept, or a new one.
    async fn session(k: &Kernel, c: &Connection) -> Result<Session> {
        if let Some(s) = SESSIONS.lock().ok().and_then(|m| m.get(&c.id).cloned()).filter(|s| s.opened.elapsed() < SESSION_FOR) {
            return Ok(s);
        }
        let (identifier, password) = Self::login(c)?;
        let made = Self::send(
            k,
            c,
            reqwest::Method::POST,
            "com.atproto.server.createSession",
            &[],
            Some(&json!({"identifier": identifier, "password": password})),
            None,
        )
        .await
        .map_err(|e| match e {
            Error::Unauthorized(m) => Error::Unauthorized(m.replace("the session", "the app password")),
            other => other,
        })?;
        let text = |k: &str| made.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
        let s = Session { jwt: text("accessJwt"), did: text("did"), handle: text("handle"), opened: Instant::now() };
        if s.jwt.is_empty() || s.did.is_empty() {
            return Err(Error::Unavailable("Bluesky opened no session".into()));
        }
        if let Ok(mut m) = SESSIONS.lock() {
            m.insert(c.id, s.clone());
        }
        Ok(s)
    }

    /// A call in the connection's session; a session the server no longer takes is opened again,
    /// once.
    async fn call(k: &Kernel, c: &Connection, method: reqwest::Method, nsid: &str, query: &[(&str, &str)], body: Option<&Value>) -> Result<(Value, Session)> {
        let s = Self::session(k, c).await?;
        match Self::send(k, c, method.clone(), nsid, query, body, Some(&s.jwt)).await {
            Err(Error::Unauthorized(_)) => {
                if let Ok(mut m) = SESSIONS.lock() {
                    m.remove(&c.id);
                }
                let s = Self::session(k, c).await?;
                Ok((Self::send(k, c, method, nsid, query, body, Some(&s.jwt)).await?, s))
            }
            other => Ok((other?, s)),
        }
    }

    /// A picture uploaded as a blob in the connection's session (opened again once when the server
    /// no longer takes it): the blob to embed.
    async fn blob(k: &Kernel, c: &Connection, ctype: &str, bytes: Vec<u8>) -> Result<Value> {
        if bytes.len() > MAX_PICTURE {
            return Err(Error::invalid(format!("Bluesky takes pictures up to 1 MB, and this one is {} KB", bytes.len().div_ceil(1000))));
        }
        let url = format!("{}/xrpc/com.atproto.repo.uploadBlob", Self::base(c));
        let (client, url) = k.vetted_client(&url, k.settings.webhooks_allow_private, Duration::from_secs(60), "Ferrum Social").await?;
        for attempt in 0..2 {
            let s = Self::session(k, c).await?;
            let answer = client
                .post(url.clone())
                .bearer_auth(&s.jwt)
                .header(reqwest::header::CONTENT_TYPE, ctype)
                .body(bytes.clone())
                .send()
                .await
                .map_err(|e| Error::Unavailable(format!("Bluesky cannot be reached: {e}")))?;
            let status = answer.status().as_u16();
            let reset = answer.headers().get("ratelimit-reset").and_then(|v| v.to_str().ok()).and_then(|v| v.parse::<u64>().ok());
            let text = answer.text().await.unwrap_or_default();
            if (200..300).contains(&status) {
                let made: Value = serde_json::from_str(&text).unwrap_or_default();
                return made.get("blob").cloned().ok_or_else(|| Error::Unavailable("Bluesky gave no blob for the picture".into()));
            }
            match failure(status, "com.atproto.repo.uploadBlob", reset, &text) {
                Error::Unauthorized(_) if attempt == 0 => {
                    if let Ok(mut m) = SESSIONS.lock() {
                        m.remove(&c.id);
                    }
                }
                e => return Err(e),
            }
        }
        Err(Error::Unauthorized("Bluesky refused the session".into()))
    }

    /// What the next page starts after: the server's cursor, when the page was full.
    fn next(items: &[Value], cursor: &Value) -> Option<String> {
        (items.len() >= PAGE).then(|| cursor.as_str().filter(|c| !c.is_empty()).map(str::to_string)).flatten()
    }
}

#[async_trait]
impl Connector for Bluesky {
    fn info(&self) -> ConnectorInfo {
        ConnectorInfo {
            provider: "bluesky".into(),
            label: "Bluesky".into(),
            family: "social".into(),
            api_version: "atproto".into(),
            streams: vec!["posts".into(), "messages".into(), "replies".into()],
            writes: vec!["SocialPublication".into(), "SocialReply".into()],
            webhooks: false,
            requirements: "The account's handle and an app password made for it (Bluesky's Settings, Privacy and security, App passwords)".into(),
            capabilities: vec!["publish_text".into(), "publish_image".into(), "reply".into(), "read_messages".into(), "read_engagement".into()],
        }
    }

    async fn check(&self, k: &Kernel, c: &Connection) -> Result<String> {
        if let Ok(mut m) = SESSIONS.lock() {
            m.remove(&c.id);
        }
        Ok(format!("@{}", Self::session(k, c).await?.handle))
    }

    async fn pull(&self, k: &Kernel, c: &Connection, stream: &str, cursor: Option<&str>) -> Result<Pulled> {
        let limit = PAGE.to_string();
        match stream {
            "posts" => {
                let s = Self::session(k, c).await?;
                let mut query = vec![("actor", s.did.as_str()), ("limit", limit.as_str()), ("filter", "posts_no_replies")];
                if let Some(cur) = cursor {
                    query.push(("cursor", cur));
                }
                let (page, s) = Self::call(k, c, reqwest::Method::GET, "app.bsky.feed.getAuthorFeed", &query, None).await?;
                let items = page["feed"].as_array().cloned().unwrap_or_default();
                let records = items
                    .iter()
                    // A repost is someone else's post.
                    .filter(|i| i.get("reason").is_none_or(Value::is_null))
                    .map(|i| {
                        let p = &i["post"];
                        let n = |k: &str| p[k].as_i64().unwrap_or(0);
                        let engaged = n("likeCount") + n("repostCount") + n("replyCount") + n("quoteCount");
                        let uri = p["uri"].as_str().unwrap_or_default();
                        let mut fields = Map::new();
                        fields.insert("text".into(), p["record"]["text"].clone());
                        fields.insert("engagements".into(), json!(engaged));
                        fields.insert("permalink".into(), json!(permalink(uri, &s.handle)));
                        Incoming {
                            external_id: uri.to_string(),
                            version: Some(format!("{engaged}:{}", p["indexedAt"].as_str().unwrap_or_default())),
                            deleted: false,
                            fields,
                        }
                    })
                    .collect();
                Ok(Pulled { records, next: Self::next(&items, &page["cursor"]) })
            }
            "messages" => {
                let mut query = vec![("limit", limit.as_str())];
                if let Some(cur) = cursor {
                    query.push(("cursor", cur));
                }
                let (page, s) = Self::call(k, c, reqwest::Method::GET, "app.bsky.notification.listNotifications", &query, None).await?;
                let handle = c.config.get("handle").and_then(Value::as_str).unwrap_or(&s.handle).to_string();
                let items = page["notifications"].as_array().cloned().unwrap_or_default();
                let records = items
                    .iter()
                    .filter(|n| matches!(n["reason"].as_str(), Some("reply" | "mention" | "quote")))
                    .map(|n| {
                        let kind = if n["reason"] == "reply" { "comment" } else { "mention" };
                        let mut fields = Map::new();
                        fields.insert("account".into(), json!(handle));
                        fields.insert("kind".into(), json!(kind));
                        fields.insert("author".into(), n["author"]["handle"].clone());
                        fields.insert("text".into(), n["record"]["text"].clone());
                        // What it answers or quotes: one of the account's posts, by its URI.
                        let about = if n["reason"] == "quote" { n["reasonSubject"].clone() } else { n["record"]["reply"]["parent"]["uri"].clone() };
                        fields.insert("post".into(), about);
                        Incoming { external_id: n["uri"].as_str().unwrap_or_default().to_string(), version: None, deleted: false, fields }
                    })
                    .collect();
                Ok(Pulled { records, next: Self::next(&items, &page["cursor"]) })
            }
            _ => Ok(Pulled::default()),
        }
    }

    async fn push(&self, k: &Kernel, c: &Connection, change: &Outgoing) -> Result<String> {
        // Sent already (the network gave its id): what is out stays as it went.
        if let Some(id) = &change.external_id {
            return Ok(id.clone());
        }
        let s = Self::session(k, c).await?;
        let key = rkey(&change.key);
        // A retried send: the record made the first time is the post.
        let found = Self::call(
            k,
            c,
            reqwest::Method::GET,
            "com.atproto.repo.getRecord",
            &[("repo", s.did.as_str()), ("collection", POST), ("rkey", key.as_str())],
            None,
        )
        .await;
        match found {
            Ok((record, _)) => {
                if let Some(uri) = record["uri"].as_str() {
                    return Ok(uri.to_string());
                }
            }
            Err(Error::NotFound(_) | Error::Invalid(_)) => {}
            Err(e) => return Err(e),
        }
        let text = change.fields.get("text").and_then(Value::as_str).unwrap_or_default();
        let mut record = json!({"$type": POST, "createdAt": Utc::now().to_rfc3339(), "text": text});
        if change.entity == "SocialReply" {
            let to = change.fields.get("in_reply_to").and_then(Value::as_str).ok_or_else(|| Error::invalid("a reply names the post it answers"))?;
            let (posts, _) = Self::call(k, c, reqwest::Method::GET, "app.bsky.feed.getPosts", &[("uris", to)], None).await?;
            let parent = posts["posts"].get(0).cloned().ok_or_else(|| Error::NotFound(format!("Bluesky has no post {to}")))?;
            let at = json!({"uri": parent["uri"], "cid": parent["cid"]});
            // In the thread of what it answers.
            let root = parent["record"]["reply"]["root"].as_object().map(|r| Value::Object(r.clone())).unwrap_or_else(|| at.clone());
            record["reply"] = json!({"root": root, "parent": at});
        } else {
            let (full, facets) = with_link(text, change.fields.get("link").and_then(Value::as_str));
            record["text"] = json!(full);
            if let Some(f) = facets {
                record["facets"] = f;
            }
            if let Some((_, ctype, bytes, alt)) = crate::picture(k, c, change).await? {
                let blob = Self::blob(k, c, &ctype, bytes).await?;
                record["embed"] = json!({"$type": "app.bsky.embed.images", "images": [{"alt": alt, "image": blob}]});
            }
        }
        let body = json!({"repo": s.did, "collection": POST, "rkey": key, "record": record});
        let (made, _) = Self::call(k, c, reqwest::Method::POST, "com.atproto.repo.createRecord", &[], Some(&body)).await?;
        made["uri"].as_str().map(str::to_string).ok_or_else(|| Error::Unavailable("Bluesky gave no address for the post".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_keys_from_deliveries() {
        let k = rkey("conn:pub:posts");
        assert_eq!(k.len(), 13);
        assert_eq!(k, rkey("conn:pub:posts"), "the same delivery, the same key");
        assert_ne!(k, rkey("conn:pub2:posts"));
        assert!("234567ab".contains(k.chars().next().unwrap()), "its top bit clear: {k}");
        assert!(k.chars().all(|c| "234567abcdefghijklmnopqrstuvwxyz".contains(c)));
    }

    #[test]
    fn links_as_facets() {
        let (text, facets) = with_link("Spring is here.", Some("https://northwind.example/spring"));
        assert_eq!(text, "Spring is here. https://northwind.example/spring");
        let f = facets.unwrap();
        assert_eq!((f[0]["index"]["byteStart"].as_u64(), f[0]["index"]["byteEnd"].as_u64()), (Some(16), Some(48)));
        let (text, facets) = with_link("Café ☕", Some("https://x.example"));
        assert_eq!(&text.as_bytes()[facets.unwrap()[0]["index"]["byteStart"].as_u64().unwrap() as usize..], b"https://x.example", "bytes, not characters");
        assert_eq!(with_link("Plain", None), ("Plain".to_string(), None));
        assert_eq!(
            permalink("at://did:plc:x/app.bsky.feed.post/3kabc", "northwind.bsky.social").as_deref(),
            Some("https://bsky.app/profile/northwind.bsky.social/post/3kabc")
        );
    }

    #[test]
    fn refusals() {
        let refused = |status, body: &str| failure(status, POST, None, body);
        assert!(
            matches!(refused(401, r#"{"error": "AuthenticationRequired", "message": "Invalid identifier or password"}"#), Error::Unauthorized(m) if m.ends_with("Invalid identifier or password"))
        );
        assert!(matches!(refused(400, r#"{"error": "ExpiredToken", "message": "Token has expired"}"#), Error::Unauthorized(_)));
        assert!(matches!(refused(400, r#"{"error": "RecordNotFound", "message": "Could not locate record"}"#), Error::NotFound(_)));
        assert!(
            matches!(refused(400, r#"{"error": "InvalidRequest", "message": "Record/text must not be longer than 300 graphemes"}"#), Error::Invalid(m) if m.contains("300 graphemes"))
        );
        assert!(matches!(failure(429, POST, None, "{}"), Error::Wait(_, 300)));
        assert!(matches!(failure(502, POST, None, "<html>bad gateway</html>"), Error::Unavailable(m) if m.ends_with("<html>bad gateway</html>")));
    }
}
