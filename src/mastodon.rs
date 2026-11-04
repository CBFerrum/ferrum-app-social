//! A connector for Mastodon (any server speaking its REST API, docs.joinmastodon.org), so a brand's
//! account there can be one of Social's accounts. A connection's config names the server
//! (`{"instance": "https://mastodon.social"}`) and its credentials hold an access token of the
//! account with the read and write scopes. Through the platform's connections:
//! * `posts`: sent as new statuses (`POST /api/v1/statuses`, with the delivery's key as Mastodon's
//!   `Idempotency-Key`, so a retried send is one status), and read back from the account's own
//!   statuses with what they did: favourites, boosts and replies as engagements. Mastodon counts no
//!   impressions.
//! * `messages`: mentions of the account (`GET /api/v1/notifications?types[]=mention`), each a
//!   comment when it answers one of its statuses and a direct message when only it can see it.
//! * `replies`: sent as statuses answering the message's status, as open as it (a direct message
//!   is answered directly) and naming its author.

use async_trait::async_trait;
use ferrum_kernel::connect::{Connection, Connector, ConnectorInfo, Incoming, Outgoing, Pulled};
use ferrum_kernel::{Error, Kernel, Result};
use serde_json::{Map, Value, json};
use std::time::Duration;

/// Statuses and notifications a page.
const PAGE: usize = 40;

pub struct Mastodon;

/// A Mastodon status's HTML as plain text: paragraphs and line breaks kept, tags dropped,
/// entities read.
pub(crate) fn plain(html: &str) -> String {
    let spaced = html.replace("</p><p>", "\n\n").replace("<br />", "\n").replace("<br/>", "\n").replace("<br>", "\n");
    let mut out = String::with_capacity(spaced.len());
    let mut in_tag = false;
    for c in spaced.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&#39;", "'").replace("&apos;", "'").replace("&amp;", "&").trim().to_string()
}

/// What a refusal of `path` from the server means here: its limits become the platform's waiting.
fn failure(status: u16, path: &str, retry_after: Option<u64>, text: &str) -> Error {
    let said = serde_json::from_str::<Value>(text)
        .ok()
        .and_then(|v| v.get("error").and_then(Value::as_str).map(str::to_string))
        .unwrap_or_else(|| text.chars().take(200).collect());
    match status {
        401 | 403 => Error::Unauthorized(format!("Mastodon refused the token: {said}")),
        404 => Error::NotFound(format!("Mastodon has no {path}")),
        422 => Error::invalid(format!("Mastodon refused it: {said}")),
        429 => Error::Wait("Mastodon asks to slow down".into(), retry_after.unwrap_or(300)),
        _ => Error::Unavailable(format!("Mastodon answered {status}: {said}")),
    }
}

impl Mastodon {
    fn base(c: &Connection) -> Result<String> {
        let base = c.config.get("instance").and_then(Value::as_str).map(|s| s.trim().trim_end_matches('/').to_string());
        base.filter(|b| !b.is_empty()).ok_or_else(|| Error::invalid("the connection's config names its Mastodon server: {\"instance\": \"https://…\"}"))
    }

    fn token(c: &Connection) -> Result<String> {
        c.credentials
            .get("token")
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| Error::invalid("the connection's credentials hold the account's access token: {\"token\": \"…\"}"))
    }

    /// One call to the server, its answer read as JSON; what Mastodon says of limits and failures
    /// becomes the platform's waiting and refusals.
    async fn call(k: &Kernel, c: &Connection, method: reqwest::Method, path: &str, body: Option<Value>, key: Option<&str>) -> Result<Value> {
        let url = format!("{}{path}", Self::base(c)?);
        let (client, url) = k.vetted_client(&url, k.settings.webhooks_allow_private, Duration::from_secs(20), "Ferrum Social").await?;
        let mut req = client.request(method, url).bearer_auth(Self::token(c)?);
        if let Some(key) = key {
            req = req.header("Idempotency-Key", key);
        }
        if let Some(body) = body {
            req = req.json(&body);
        }
        let answer = req.send().await.map_err(|e| Error::Unavailable(format!("Mastodon cannot be reached: {e}")))?;
        let status = answer.status().as_u16();
        let reset = answer.headers().get("retry-after").and_then(|v| v.to_str().ok()).and_then(|v| v.parse::<u64>().ok());
        let text = answer.text().await.unwrap_or_default();
        if (200..300).contains(&status) {
            return serde_json::from_str(&text).map_err(|e| Error::Unavailable(format!("Mastodon answered something else than JSON: {e}")));
        }
        Err(failure(status, path, reset, &text))
    }

    /// The account the token is for.
    async fn me(k: &Kernel, c: &Connection) -> Result<Value> {
        Self::call(k, c, reqwest::Method::GET, "/api/v1/accounts/verify_credentials", None, None).await
    }

    /// A page's query: a full page, older than `cursor` when there is one (Mastodon lists newest first).
    fn page(cursor: Option<&str>) -> String {
        format!("limit={PAGE}{}", cursor.map(|c| format!("&max_id={c}")).unwrap_or_default())
    }

    /// What the next page starts after: the last id, when the page was full.
    fn next(items: &[Value]) -> Option<String> {
        (items.len() >= PAGE).then(|| items.last().and_then(|i| i["id"].as_str()).map(str::to_string)).flatten()
    }
}

#[async_trait]
impl Connector for Mastodon {
    fn info(&self) -> ConnectorInfo {
        ConnectorInfo {
            provider: "mastodon".into(),
            label: "Mastodon".into(),
            family: "social".into(),
            api_version: "v1".into(),
            streams: vec!["posts".into(), "messages".into(), "replies".into()],
            writes: vec!["SocialPublication".into(), "SocialReply".into()],
            webhooks: false,
            requirements: "An access token of the account, with the read and write scopes, from its Mastodon server".into(),
            capabilities: vec!["publish_text".into(), "reply".into(), "read_messages".into(), "read_engagement".into()],
        }
    }

    async fn check(&self, k: &Kernel, c: &Connection) -> Result<String> {
        let me = Self::me(k, c).await?;
        Ok(format!("@{}", me["acct"].as_str().or(me["username"].as_str()).unwrap_or("?")))
    }

    async fn pull(&self, k: &Kernel, c: &Connection, stream: &str, cursor: Option<&str>) -> Result<Pulled> {
        let query = Self::page(cursor);
        match stream {
            "posts" => {
                let me = Self::me(k, c).await?;
                let id = me["id"].as_str().unwrap_or_default();
                let page = Self::call(k, c, reqwest::Method::GET, &format!("/api/v1/accounts/{id}/statuses?{query}&exclude_reblogs=true"), None, None).await?;
                let items = page.as_array().cloned().unwrap_or_default();
                let records = items
                    .iter()
                    .map(|s| {
                        let n = |k: &str| s[k].as_i64().unwrap_or(0);
                        let engaged = n("favourites_count") + n("reblogs_count") + n("replies_count");
                        let mut fields = Map::new();
                        fields.insert("text".into(), json!(plain(s["content"].as_str().unwrap_or_default())));
                        fields.insert("engagements".into(), json!(engaged));
                        fields.insert("permalink".into(), s["url"].clone());
                        Incoming {
                            external_id: s["id"].as_str().unwrap_or_default().to_string(),
                            version: Some(format!("{engaged}:{}", s["edited_at"].as_str().unwrap_or_default())),
                            deleted: false,
                            fields,
                        }
                    })
                    .collect();
                Ok(Pulled { records, next: Self::next(&items) })
            }
            "messages" => {
                let me = Self::me(k, c).await?;
                let (my_id, handle) = (
                    me["id"].as_str().unwrap_or_default().to_string(),
                    c.config.get("handle").and_then(Value::as_str).or(me["acct"].as_str()).unwrap_or_default().to_string(),
                );
                let page = Self::call(k, c, reqwest::Method::GET, &format!("/api/v1/notifications?types[]=mention&{query}"), None, None).await?;
                let items = page.as_array().cloned().unwrap_or_default();
                let records = items
                    .iter()
                    .filter(|n| n["status"].is_object())
                    .map(|n| {
                        let s = &n["status"];
                        let kind = if s["visibility"] == "direct" {
                            "direct"
                        } else if s["in_reply_to_account_id"].as_str() == Some(my_id.as_str()) {
                            "comment"
                        } else {
                            "mention"
                        };
                        let mut fields = Map::new();
                        fields.insert("account".into(), json!(handle));
                        fields.insert("kind".into(), json!(kind));
                        fields.insert("author".into(), n["account"]["acct"].clone());
                        fields.insert("text".into(), json!(plain(s["content"].as_str().unwrap_or_default())));
                        fields.insert("post".into(), s["in_reply_to_id"].clone());
                        // The status is what a reply answers, so it is the message's own id there.
                        Incoming { external_id: s["id"].as_str().unwrap_or_default().to_string(), version: None, deleted: false, fields }
                    })
                    .collect();
                Ok(Pulled { records, next: Self::next(&items) })
            }
            _ => Ok(Pulled::default()),
        }
    }

    async fn push(&self, k: &Kernel, c: &Connection, change: &Outgoing) -> Result<String> {
        // Sent already (the network gave its id): what is out stays as it went.
        if let Some(id) = &change.external_id {
            return Ok(id.clone());
        }
        let text = change.fields.get("text").and_then(Value::as_str).unwrap_or_default();
        let body = if change.entity == "SocialReply" {
            let to = change.fields.get("in_reply_to").and_then(Value::as_str).ok_or_else(|| Error::invalid("a reply names the status it answers"))?;
            // As open as what it answers, never more (a direct message is answered directly, not in
            // public), and naming its author, so Mastodon delivers it to them.
            let parent = Self::call(k, c, reqwest::Method::GET, &format!("/api/v1/statuses/{to}"), None, None).await?;
            let visibility = parent["visibility"].as_str().filter(|v| matches!(*v, "direct" | "private" | "unlisted")).unwrap_or("public");
            let author = parent["account"]["acct"].as_str().filter(|a| !a.is_empty());
            let status = match author {
                Some(a) if !text.contains(&format!("@{a}")) => format!("@{a} {text}"),
                _ => text.to_string(),
            };
            json!({"status": status, "in_reply_to_id": to, "visibility": visibility})
        } else {
            let link = change.fields.get("link").and_then(Value::as_str).filter(|l| !l.is_empty());
            json!({"status": match link { Some(l) => format!("{text} {l}"), None => text.to_string() }, "visibility": "public"})
        };
        let made = Self::call(k, c, reqwest::Method::POST, "/api/v1/statuses", Some(body), Some(&change.key)).await?;
        made["id"].as_str().map(str::to_string).ok_or_else(|| Error::Unavailable("Mastodon gave no id for the status".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn statuses_as_text() {
        assert_eq!(plain("<p>Spring is here &amp; <a href=\"x\">#sage</a></p><p>Second</p>"), "Spring is here & #sage\n\nSecond");
        assert_eq!(plain("one<br />two &lt;3"), "one\ntwo <3");
        let full: Vec<Value> = (0..PAGE).map(|i| json!({"id": i.to_string()})).collect();
        assert_eq!((Mastodon::next(&full), Mastodon::next(&full[..3])), (Some((PAGE - 1).to_string()), None));
        assert_eq!((Mastodon::page(None), Mastodon::page(Some("9"))), ("limit=40".to_string(), "limit=40&max_id=9".to_string()));
    }

    #[test]
    fn refusals() {
        let refused = |status, retry| failure(status, "/api/v1/statuses", retry, r#"{"error": "nope"}"#);
        assert!(matches!(refused(401, None), Error::Unauthorized(m) if m.ends_with("nope")));
        assert!(matches!(refused(403, None), Error::Unauthorized(_)));
        assert!(matches!(refused(404, None), Error::NotFound(m) if m.ends_with("/api/v1/statuses")));
        assert!(matches!(refused(422, None), Error::Invalid(m) if m.ends_with("nope")));
        assert!(matches!((refused(429, Some(60)), refused(429, None)), (Error::Wait(_, 60), Error::Wait(_, 300))));
        assert!(matches!(failure(502, "/x", None, "<html>bad gateway</html>"), Error::Unavailable(m) if m.ends_with("<html>bad gateway</html>")));
    }

    #[test]
    fn a_connection_names_its_server_and_token() {
        let c = |config, credentials| Connection { id: uuid::Uuid::nil(), tenant: uuid::Uuid::nil(), provider: "mastodon".into(), config, credentials };
        assert_eq!(Mastodon::base(&c(json!({"instance": " https://mastodon.example/ "}), json!({}))).unwrap(), "https://mastodon.example");
        assert!(Mastodon::base(&c(json!({"instance": " "}), json!({}))).is_err());
        assert!(Mastodon::base(&c(json!({}), json!({}))).is_err());
        assert!(Mastodon::token(&c(json!({}), json!({}))).is_err());
        assert_eq!(Mastodon::token(&c(json!({}), json!({"token": "t"}))).unwrap(), "t");
    }
}
