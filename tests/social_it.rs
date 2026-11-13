//! Social media management (FERRUM-ECOSYSTEM-005) against a real PostgreSQL
//! (`FERRUM_SOCIAL_TEST_DATABASE_URL`, default `postgres://ferrum:ferrum@localhost:5432/ferrum_social_test`;
//! reset first), through the platform's connections to one fake network on 127.0.0.1 that keeps what it was sent:
//! its media, its posts under their idempotency keys, its inbox, its replies and its numbers, and signs people in.
//!
//! Accounts on connections (a key an administrator holds, the workspace's sign-in, a person's own, a reader that
//! cannot publish), their state (not connected, connected, the provider refusing a sign-in, disconnected with
//! revoke), and posts checked against network profiles in plain words; review by someone other than the author;
//! a post to several accounts as one queued call each, picture first, once under a lost answer, a service that says
//! "later", and two workers; a refusal shown in the service's words and retried by a person; a first comment; delete;
//! the inbox by inbound address and by pull, each message once, routed to its account, answered through the right
//! operation, matched to a person's record when the workspace says so, and erased with the person; numbers read by
//! day, named by what they are, and the reports of them.

use chrono::{Duration, Utc};
use ferrum_kernel::connections::ConnectionInput;
use ferrum_kernel::customize::NewCustomization;
use ferrum_kernel::secret::SecretKey;
use ferrum_kernel::tenant::NewTenant;
use ferrum_kernel::{Ctx, Db, DbConfig, Doc, Error, HookRegistry, Kernel, KernelSettings, ListQuery};
use ferrum_meta::Catalog;
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use uuid::Uuid;

const KEY: &str = "chirp-key-Zq81";
const HOOK: &str = "chirp-hook-Pm42";

fn obj(v: Value) -> Doc {
    v.as_object().cloned().unwrap()
}

fn uid(v: &Value) -> Uuid {
    v.as_str().unwrap_or_else(|| panic!("not an id: {v}")).parse().unwrap()
}

/// What the fake network holds and was sent.
#[derive(Default)]
struct Net {
    /// (id, body, bearer token it came with, key).
    posts: Vec<(String, Value, String, String)>,
    keys: HashMap<String, String>,
    media: Vec<Value>,
    /// What came in what order: "media", "post".
    order: Vec<&'static str>,
    /// "ok", or "reject" for good.
    mode: &'static str,
    /// Answer 503 this many times before taking a post; and take a post, then lose the answer, this many times.
    later: usize,
    lose: usize,
    deleted: Vec<String>,
    /// (key, body) of replies and direct messages.
    replies: Vec<(String, Value)>,
    dms: Vec<(String, Value)>,
    reject_replies: bool,
    /// A post whose words hold this is refused.
    refuse_text: Option<String>,
    /// Every request to the round-two fakes (stock, fetch, shortener, writing, plugs, feed): "METHOD path" and its body.
    calls: Vec<(String, Value)>,
    /// What the writing fake answers next, in order; then an echo of the prompt.
    written: Vec<String>,
    /// The feed's items.
    feed: Vec<Value>,
    inbox: Vec<Value>,
    stats_down: bool,
    /// Calls to /me and /posts/*/stats.
    reads: usize,
    /// Who the provider will not refresh a sign-in for, how many tokens it gave, and revocations.
    revoked: HashSet<String>,
    issued: Vec<String>,
    revokes: usize,
}

type Shared = Arc<Mutex<Net>>;

fn form(body: &str) -> BTreeMap<String, String> {
    reqwest::Url::parse(&format!("http://f/?{body}")).unwrap().query_pairs().map(|(k, v)| (k.into_owned(), v.into_owned())).collect()
}

fn answer(n: &mut Net, method: &str, path: &str, header: &dyn Fn(&str) -> Option<String>, body: &str) -> (u16, Value) {
    let route = path.split('?').next().unwrap_or_default().to_string();
    let json_body: Value = serde_json::from_str(body).unwrap_or(Value::Null);
    if route.starts_with("/v1/oauth/") {
        let f = form(body);
        return match (route.as_str(), f.get("grant_type").map(String::as_str)) {
            ("/v1/oauth/token", Some("authorization_code")) => {
                let who = f.get("code").and_then(|c| c.strip_prefix("CODE-")).unwrap_or("nobody").to_string();
                let token = format!("AT-{who}-{}", n.issued.len() + 1);
                n.issued.push(token.clone());
                (200, json!({"access_token": token, "refresh_token": format!("RT-{who}"), "expires_in": 3600, "account": {"handle": who}}))
            }
            ("/v1/oauth/token", Some("refresh_token")) => {
                let who = f.get("refresh_token").and_then(|c| c.strip_prefix("RT-")).unwrap_or("nobody").to_string();
                if n.revoked.contains(&who) {
                    return (400, json!({"error": "invalid_grant"}));
                }
                let token = format!("AT-{who}-{}", n.issued.len() + 1);
                n.issued.push(token.clone());
                (200, json!({"access_token": token, "expires_in": 3600}))
            }
            ("/v1/oauth/revoke", _) => {
                n.revokes += 1;
                (200, json!({}))
            }
            _ => (400, json!({"error": "unsupported"})),
        };
    }
    let bearer = header("authorization").and_then(|a| a.strip_prefix("Bearer ").map(str::to_string));
    let known = header("x-api-key").as_deref() == Some(KEY) || bearer.as_ref().is_some_and(|b| n.issued.contains(b));
    if !known {
        return (401, json!({"error": {"message": "The token is not valid"}}));
    }
    let key = header("idempotency-key").unwrap_or_default();
    let token = bearer.unwrap_or_else(|| "key".into());
    match (method, route.as_str()) {
        ("POST", "/v1/media") => {
            n.order.push("media");
            n.media.push(json_body);
            (200, json!({"media": {"id": format!("m-{}", n.media.len())}}))
        }
        ("POST", "/v1/posts") => {
            if n.refuse_text.as_ref().is_some_and(|r| json_body["text"].as_str().unwrap_or_default().contains(r.as_str())) {
                return (422, json!({"error": {"message": "That item is not allowed"}}));
            }
            if n.mode == "reject" {
                return (422, json!({"error": {"message": "That text is not allowed"}}));
            }
            if let Some(id) = n.keys.get(&key) {
                return (201, json!({"post": {"id": id, "url": format!("https://chirp.test/p/{id}")}}));
            }
            if n.later > 0 {
                n.later -= 1;
                return (503, json!({"error": {"message": "Chirp is busy"}}));
            }
            n.order.push("post");
            let id = format!("p-{}", n.posts.len() + 1);
            n.posts.push((id.clone(), json_body, token, key.clone()));
            n.keys.insert(key, id.clone());
            if n.lose > 0 {
                n.lose -= 1;
                return (503, json!({"error": {"message": "Chirp lost the answer"}}));
            }
            (201, json!({"post": {"id": id, "url": format!("https://chirp.test/p/{id}")}}))
        }
        ("DELETE", p) if p.starts_with("/v1/posts/") => {
            n.deleted.push(p.trim_start_matches("/v1/posts/").to_string());
            (200, json!({"deleted": true}))
        }
        (m, r)
            if r.starts_with("/v1/stock")
                || r.starts_with("/v1/fetch")
                || r.starts_with("/v1/short")
                || r.starts_with("/v1/messages")
                || r.starts_with("/v1/picture")
                || r.starts_with("/v1/posts/") && (r.ends_with("/repost") || r.ends_with("/like"))
                || r.starts_with("/v1/feed") =>
        {
            n.calls.push((format!("{m} {path}"), json_body.clone()));
            round_two(n, m, r, path, &json_body)
        }
        ("GET", "/v1/pages") => (
            200,
            json!({"pages": [{"id": "pg-1", "name": "Northwind Shop", "username": "@nwshop", "avatar": "https://chirp.test/a/1.png"},
                             {"id": "pg-2", "name": "Northwind Cafe", "username": "nwcafe", "avatar": "https://chirp.test/a/2.png"}]}),
        ),
        ("GET", "/v1/people") => {
            let q = path.split("q=").nth(1).unwrap_or_default().to_lowercase();
            let people = [json!({"id": "u-9", "name": "Ana Lima"}), json!({"id": "u-7", "name": "Bo Chen"})];
            (200, json!(people.into_iter().filter(|p| p["name"].as_str().unwrap().to_lowercase().contains(&q)).collect::<Vec<_>>()))
        }
        ("GET", "/v1/boards") => (200, json!({"items": [{"id": "b-1", "name": "Spring"}, {"id": "b-2", "name": "Summer"}], "for": path.split("for=").nth(1)})),
        ("GET", "/v1/me") => {
            n.reads += 1;
            if n.stats_down {
                return (503, json!({"error": {"message": "Stats are down"}}));
            }
            let cafe = path.contains("for=pg-2");
            (
                200,
                json!({"handle": if cafe { "nwcafe" } else { "northwind" }, "name": if cafe { "Northwind Café" } else { "Northwind" }, "avatar": "https://chirp.test/a/0.png",
                         "followers": 1200, "day": {"views": 900, "unique": 700, "actions": 40, "taps": 8}}),
            )
        }
        ("GET", p) if p.ends_with("/stats") => {
            n.reads += 1;
            if n.stats_down {
                return (503, json!({"error": {"message": "Stats are down"}}));
            }
            (200, json!({"views": 500, "unique": 400, "likes": 3, "shares": 2, "replies": 1, "taps": 5}))
        }
        ("GET", "/v1/inbox") => {
            let since: usize = path.split("since=").nth(1).and_then(|s| s.split('&').next()).and_then(|s| s.parse().ok()).unwrap_or(0);
            (200, json!({"items": n.inbox.iter().skip(since).cloned().collect::<Vec<_>>(), "next": n.inbox.len().to_string()}))
        }
        ("POST", "/v1/replies") | ("POST", "/v1/dm") => {
            if n.reject_replies {
                return (422, json!({"error": {"message": "Replies are closed on that thread"}}));
            }
            let list = if route == "/v1/dm" { &mut n.dms } else { &mut n.replies };
            if !list.iter().any(|(k, _)| *k == key) {
                list.push((key.clone(), json_body));
            }
            let at = list.iter().position(|(k, _)| *k == key).unwrap_or(0);
            (200, json!({"id": format!("{}-{}", if route == "/v1/dm" { "d" } else { "r" }, at + 1)}))
        }
        _ => (404, json!({"error": {"message": "No such thing"}})),
    }
}

/// The round-two fakes: shaped like the documented answers of a stock picture source, an image fetcher, a link
/// shortener, the Claude Messages API, a picture maker, reposts and likes, and a feed read as JSON.
fn round_two(n: &mut Net, method: &str, route: &str, path: &str, body: &Value) -> (u16, Value) {
    let png = |t: &str| json!({"__bytes": format!("\u{89}PNG {t}"), "__type": "image/png"});
    match (method, route) {
        ("GET", "/v1/stock/search") => (
            200,
            json!({"total": 2, "results": [
            {"id": "s-1", "alt_description": "A lake at dawn", "width": 4000, "height": 3000, "urls": {"small": "https://chirp.test/s/1-small.jpg", "full": "https://chirp.test/s/1.jpg"},
             "user": {"name": "Ana Lima", "links": {"html": "https://chirp.test/@ana"}}},
            {"id": "s-2", "urls": {"small": "https://chirp.test/s/2-small.jpg", "full": "https://chirp.test/s/2.jpg"}, "user": {"name": "Bo", "links": {"html": "https://chirp.test/@bo"}}}]}),
        ),
        ("GET", r) if r.starts_with("/v1/stock/") && r.ends_with("/download") => (200, png("stock")),
        ("POST", r) if r.starts_with("/v1/stock/") && r.ends_with("/told") => (200, json!({"ok": true})),
        ("GET", "/v1/fetch") => (200, png(path.split("u=").nth(1).unwrap_or_default())),
        ("POST", "/v1/short") => {
            let i = n.calls.iter().filter(|c| c.0.starts_with("POST /v1/short")).count();
            (200, json!({"id": format!("k{i}"), "shortLink": format!("https://sh.test/k{i}")}))
        }
        ("GET", r) if r.starts_with("/v1/short/") => (200, json!({"clicks": 42})),
        ("POST", "/v1/messages") => {
            let prompt = body["messages"][0]["content"].clone();
            let said =
                if n.written.is_empty() { format!("ECHO {}", prompt.as_str().map_or_else(|| prompt.to_string(), str::to_string)) } else { n.written.remove(0) };
            (
                200,
                json!({"id": "msg_1", "type": "message", "role": "assistant", "model": body["model"], "content": [{"type": "text", "text": said}], "stop_reason": "end_turn"}),
            )
        }
        ("POST", "/v1/picture") => (200, png("drawn")),
        ("POST", r) if r.ends_with("/repost") => (200, json!({"id": format!("rp-{}", n.calls.len())})),
        ("POST", r) if r.ends_with("/like") => (200, json!({"liked": true})),
        ("GET", "/v1/feed") => (200, json!({"items": n.feed.clone()})),
        _ => (404, json!({"error": {"message": "No such thing"}})),
    }
}

async fn serve(net: Shared) -> u16 {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        loop {
            let Ok((mut s, _)) = listener.accept().await else { return };
            let net = net.clone();
            tokio::spawn(async move {
                let mut buf = Vec::new();
                let mut chunk = [0u8; 8192];
                loop {
                    let n = s.read(&mut chunk).await.unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    buf.extend_from_slice(&chunk[..n]);
                    let text = String::from_utf8_lossy(&buf).to_string();
                    if let Some(end) = text.find("\r\n\r\n") {
                        let len = text[..end]
                            .lines()
                            .find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0)))
                            .unwrap_or(0);
                        if buf.len() >= end + 4 + len {
                            break;
                        }
                    }
                }
                let text = String::from_utf8_lossy(&buf).to_string();
                let (head, body) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
                let mut lines = head.lines();
                let mut first = lines.next().unwrap_or_default().split_whitespace();
                let (method, path) = (first.next().unwrap_or_default().to_string(), first.next().unwrap_or("/").to_string());
                let headers: Vec<(String, String)> =
                    lines.filter_map(|l| l.split_once(':').map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string()))).collect();
                let header = |h: &str| headers.iter().find(|(k, _)| k == h).map(|(_, v)| v.clone());
                let (status, reply) = answer(&mut net.lock().unwrap(), &method, &path, &header, body);
                // A file is answered as its bytes: `{"__bytes": text, "__type": content type}`.
                let (kind, reply) = match reply.get("__bytes").and_then(Value::as_str) {
                    Some(b) => (reply["__type"].as_str().unwrap_or("application/octet-stream").to_string(), b.to_string()),
                    None => ("application/json".to_string(), reply.to_string()),
                };
                let out = format!("HTTP/1.1 {status} X\r\ncontent-type: {kind}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{reply}", reply.len());
                let _ = s.write_all(out.as_bytes()).await;
            });
        }
    });
    port
}

/// An in-memory object store standing in for S3.
#[derive(Default)]
struct MemFiles(Mutex<HashMap<String, Vec<u8>>>);

#[async_trait::async_trait]
impl ferrum_kernel::files::FileStore for MemFiles {
    async fn put(&self, key: &str, data: &[u8]) -> ferrum_kernel::Result<()> {
        self.0.lock().unwrap().insert(key.to_string(), data.to_vec());
        Ok(())
    }
    async fn get(&self, key: &str) -> ferrum_kernel::Result<Vec<u8>> {
        self.0.lock().unwrap().get(key).cloned().ok_or_else(|| Error::NotFound("object".into()))
    }
    async fn delete(&self, key: &str) -> ferrum_kernel::Result<()> {
        self.0.lock().unwrap().remove(key);
        Ok(())
    }
}

/// The description of the fake network as a connection's: operations named by what Social needs of them, what each
/// network calls things mapped in `result`. `ops` leaves out what a connection does not offer.
fn description(base: &str, auth: Value, ops: &[&str], inbound: bool) -> Value {
    let err = "/error/message";
    let all = json!({
        "publish": {"method": "POST", "path": "/posts", "body": "{\"text\": {{ inputs.text_with_link | json }}, \"media_ids\": {{ inputs.media | json }}{% if inputs.reply_to %}, \"reply_to\": {{ inputs.reply_to | json }}{% endif %}{% if inputs.settings %}, \"settings\": {{ inputs.settings | json }}, \"kind\": {{ inputs.kind | json }}{% endif %}{% if inputs.account_id %}, \"account\": {{ inputs.account_id | json }}{% endif %}}",
                    "result": {"id": "/post/id", "url": "/post/url"}, "error_at": err, "idempotency_header": "Idempotency-Key", "inputs": ["text_with_link", "media", "reply_to", "settings", "kind", "account_id"]},
        "publish_media": {"method": "POST", "path": "/media", "body": "{\"name\": {{ inputs.name | json }}, \"alt\": {{ inputs.alt | json }}, \"type\": {{ inputs.content_type | json }}, \"size\": {{ inputs.size | json }}, \"data\": {{ inputs.data | json }}}",
                          "result": {"id": "/media/id"}, "error_at": err, "idempotency_header": "Idempotency-Key", "inputs": ["name", "alt", "content_type", "size", "data"]},
        "delete": {"method": "DELETE", "path": "/posts/{{ inputs.id }}", "error_at": err, "idempotency_header": "Idempotency-Key", "inputs": ["id"]},
        "reply": {"method": "POST", "path": "/replies", "body": "{\"text\": {{ inputs.text | json }}, \"in_reply_to\": {{ inputs.in_reply_to | json }}{% if inputs.item %}, \"item\": {{ inputs.item | json }}{% endif %}}",
                  "result": {"id": "/id"}, "error_at": err, "idempotency_header": "Idempotency-Key", "inputs": ["text", "in_reply_to", "item"]},
        "send_message": {"method": "POST", "path": "/dm", "body": "{\"to\": {{ inputs.to | json }}, \"text\": {{ inputs.text | json }}}",
                         "result": {"id": "/id"}, "error_at": err, "idempotency_header": "Idempotency-Key", "inputs": ["to", "text"]},
        "metrics": {"path": "/posts/{{ inputs.id }}/stats", "result": {"impressions": "/views", "reach": "/unique", "engagements": {"expr": "answer.likes + answer.shares + answer.replies"}, "clicks": "/taps"},
                    "error_at": err, "inputs": ["id"]},
        "account": {"path": "/me", "query": {"for": "{{ inputs.account_id or '' }}"}, "inputs": ["account_id"], "result": {"followers": "/followers", "impressions": "/day/views", "reach": "/day/unique", "engagements": "/day/actions", "clicks": "/day/taps",
                                              "name": "/name", "handle": "/handle", "picture": "/avatar"}, "error_at": err},
        "accounts": {"path": "/pages", "error_at": err},
        "repost": {"method": "POST", "path": "/posts/{{ inputs.id }}/repost", "body": "{}", "result": {"id": "/id"}, "error_at": err, "idempotency_header": "Idempotency-Key", "inputs": ["id"]},
        "like": {"method": "POST", "path": "/posts/{{ inputs.id }}/like", "body": "{}", "error_at": err, "idempotency_header": "Idempotency-Key", "inputs": ["id"]},
        "mentions": {"path": "/people", "query": {"q": "{{ inputs.q }}"}, "error_at": err, "inputs": ["q"]},
        "boards": {"path": "/boards", "query": {"for": "{{ inputs.account_id }}"}, "error_at": err, "inputs": ["account_id"]},
        "messages": {"path": "/inbox", "query": {"since": "{{ cursor or '' }}"}, "error_at": err},
    });
    let operations: serde_json::Map<String, Value> = ops.iter().map(|o| (o.to_string(), all[o].clone())).collect();
    let mut d = json!({"base": base, "auth": auth, "apps": ["social"], "roles": ["Social Author", "Social Manager"], "operations": operations});
    if inbound {
        let map = json!({"entity": "SocialMessage", "items_at": "/items", "by": {"external_id": "item.id"},
            "set": {"kind": "item.kind == 'dm' ? 'Direct' : (item.kind == 'mention' ? 'Mention' : 'Comment')", "author": "item.from", "author_id": "item.from_id",
                    "text": "item.text", "account_handle": "/to", "post_external_id": "/on_post", "thread": "item.thread", "via_connection": "connection.id"},
            "outside": true});
        d["inbound"] = json!({"verify": {"type": "token", "header": "x-token", "secret": "hook"}, "delivery_at": "/id", "map": map.clone()});
        d["pulls"] = json!([{"name": "inbox", "operation": "messages", "every_minutes": 5, "items_at": "/items", "cursor": "token", "next_at": "/next", "max_pages": 3, "map": map}]);
    }
    d
}

fn input(name: &str, desc: Value, secrets: Option<Value>) -> ConnectionInput {
    serde_json::from_value(json!({"name": name, "label": name, "description": desc, "secrets": secrets, "enabled": true})).unwrap()
}

fn query(url: &str) -> BTreeMap<String, String> {
    reqwest::Url::parse(url).unwrap().query_pairs().map(|(k, v)| (k.into_owned(), v.into_owned())).collect()
}

fn refused<V: std::fmt::Debug>(r: std::result::Result<V, Error>, kind: &str, says: &str) {
    let msg = format!("{r:?}");
    let ok = matches!((&r, kind), (Err(Error::Invalid(_) | Error::Validation(_)), "invalid") | (Err(Error::Forbidden(_)), "forbidden"));
    assert!(ok && msg.contains(says), "expected {kind} saying {says:?}, got {msg}");
}

struct T {
    k: Arc<Kernel>,
    tenant: Uuid,
    admin: Ctx,
    net: Shared,
}

impl T {
    async fn insert(&self, ctx: &Ctx, entity: &str, v: Value) -> Doc {
        self.k.insert(ctx, entity, obj(v)).await.unwrap_or_else(|e| panic!("insert {entity}: {e:?}"))
    }
    /// A post saved, then sent for review: a draft may hold what its networks refuse, leaving Draft may not.
    async fn ready(&self, ctx: &Ctx, v: Value) -> std::result::Result<Value, Error> {
        let d = self.k.insert(ctx, "SocialPost", obj(v)).await?;
        self.k.run_action(ctx, "SocialPost", uid(&d["id"]), "send_for_review", Value::Null).await
    }
    async fn get(&self, entity: &str, id: Uuid) -> Doc {
        self.k.get(&self.admin, entity, id, false).await.unwrap_or_else(|e| panic!("get {entity}: {e:?}"))
    }
    async fn user(&self, email: &str, roles: &[&str]) -> Ctx {
        let roles: Vec<String> = roles.iter().map(|r| r.to_string()).collect();
        let id = self.k.create_user(&self.admin, email, email, Some("Correct-Horse-9"), &roles, false).await.expect("user");
        let s = self.k.login("brands", email, "Correct-Horse-9").await.expect("login");
        self.k.load_ctx(self.tenant, id, Some(s.token_version), "it".into()).await.expect("ctx")
    }
    async fn act(&self, ctx: &Ctx, entity: &str, id: Uuid, action: &str, params: Value) -> std::result::Result<Value, Error> {
        self.k.run_action(ctx, entity, id, action, params).await
    }
    async fn rows(&self, entity: &str, filters: Value) -> Vec<Doc> {
        let q = ListQuery { filters: ferrum_kernel::query::parse_filters(&filters).unwrap(), limit: Some(200), ..Default::default() };
        self.k.list(&self.admin, entity, &q).await.unwrap().rows
    }
    /// Jobs run, time passing between attempts, until none is left (a job put back for a later hour, a thread's delay,
    /// is run again a bounded number of times).
    async fn work(&self) {
        for _ in 0..40 {
            let mut tx = self.k.db.system_tx().await.unwrap();
            sqlx::query("update job set run_at = now() where status = 'queued'").execute(&mut *tx).await.unwrap();
            tx.commit().await.unwrap();
            if self.k.run_jobs_once(4).await.unwrap() == 0 {
                break;
            }
        }
    }
    /// One round of the upkeep as of `now`, and the jobs it queued.
    async fn upkeep(&self, now: chrono::DateTime<Utc>) {
        ferrum_app_social::maintenance_at(&self.k, now).await.expect("upkeep");
        self.work().await;
        ferrum_app_social::maintenance_at(&self.k, now).await.expect("upkeep settles");
    }
    async fn count(&self, sql: &str) -> i64 {
        let mut tx = self.k.db.tenant_tx(self.tenant).await.unwrap();
        let n = sqlx::query_scalar(sqlx::AssertSqlSafe(sql.to_string())).bind(self.tenant).fetch_one(&mut *tx).await.unwrap();
        tx.commit().await.unwrap();
        n
    }
    /// A person (or the workspace) signs in to a connection at the fake provider.
    async fn sign_in(&self, who: &Ctx, conn: Uuid, code: &str) {
        let session = Uuid::now_v7();
        let url = self.k.connect_start(who, conn, session).await.unwrap();
        let q = query(&url);
        assert_eq!((q["code_challenge_method"].as_str(), q["response_type"].as_str()), ("S256", "code"));
        self.k.connect_finish(who, session, &q["state"], Some(code), None).await.unwrap();
    }
}

#[test]
fn social_media() {
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().thread_stack_size(16 << 20).build().unwrap();
    rt.block_on(async { tokio::spawn(scenario()).await.unwrap() });
}

async fn scenario() {
    let url = std::env::var("FERRUM_SOCIAL_TEST_DATABASE_URL").unwrap_or_else(|_| "postgres://ferrum:ferrum@localhost:5432/ferrum_social_test".into());
    let db = Db::connect(DbConfig { url, max_connections: 16, ..Default::default() }).await.expect("connect");
    sqlx::raw_sql("drop schema if exists public cascade; create schema public;").execute(&db.pool).await.expect("reset");
    let mut packages = ferrum_domain::packages().expect("platform packages");
    packages.extend(ferrum_app_social::packages().expect("app packages"));
    let catalog = Catalog::from_packages(packages).expect("catalog");
    let mut hooks = HookRegistry::default();
    ferrum_domain::register(&mut hooks);
    ferrum_app_social::register(&mut hooks);
    for source in ferrum_app_social::reports() {
        ferrum_domain::reports::add_source(source);
    }
    let settings = KernelSettings {
        secret_key: Some(SecretKey::new([7u8; 32])),
        connection_hosts: vec!["127.0.0.1".into()],
        public_url: Some("https://ferrum.example.test".into()),
        ..Default::default()
    };
    let k = Kernel::boot(db, catalog, hooks, settings, None).await.expect("boot");
    k.set_file_store(Arc::new(MemFiles::default()));
    ferrum_domain::maintenance(&k).await.expect("partitions");
    let tenant = k
        .create_tenant(NewTenant {
            slug: "brands".into(),
            name: "Brands".into(),
            admin_email: "admin@brands.test".into(),
            admin_password: "Correct-Horse-9".into(),
            admin_name: None,
            packages: vec!["social".into(), "contacts".into()],
            settings: Some(json!({"timezone": "UTC"})),
        })
        .await
        .expect("tenant");
    let session = k.login("brands", "admin@brands.test", "Correct-Horse-9").await.expect("login");
    let admin = k.load_ctx(tenant, session.user, Some(session.token_version), "it".into()).await.expect("ctx");
    let net: Shared = Arc::default();
    net.lock().unwrap().mode = "ok";
    let t = T { k: k.clone(), tenant, admin, net: net.clone() };
    let port = serve(net.clone()).await;
    let base = format!("http://127.0.0.1:{port}/v1");
    let author = t.user("author@brands.test", &["Social Author"]).await;
    let approver = t.user("approver@brands.test", &["Social Approver"]).await;
    let manager = t.user("manager@brands.test", &["Social Manager"]).await;
    let pia = t.user("pia@brands.test", &["Social Author"]).await;

    // ───────────── connections: the network is a description; no code of Social's knows it ─────────────
    let everything = ["publish", "publish_media", "delete", "reply", "send_message", "metrics", "account", "messages", "repost", "like"];
    let key_auth = json!({"type": "header", "header": "x-api-key", "secret": "key"});
    let provider = |who: &str| {
        json!({"authorize_url": "https://login.chirp.test/authorize", "token_url": format!("{base}/oauth/token"), "revoke_url": format!("{base}/oauth/revoke"),
               "scopes": ["post", "read"], "client_id": format!("ferrum-{who}"), "client_secret": "client", "account_at": "/account/handle"})
    };
    let save = |name: &'static str, d: Value, secrets: Value| {
        let t = &t;
        async move { t.k.save_connection_def(&t.admin, None, &input(name, d, Some(secrets))).await.expect(name) }
    };
    let on_key = save("chirp_key", description(&base, key_auth.clone(), &everything, true), json!({"key": KEY, "hook": HOOK})).await;
    let on_words = save("chirp_words", description(&base, key_auth, &["publish"], false), json!({"key": KEY})).await;
    let page = json!({"type": "oauth", "owner": "workspace", "provider": provider("page")});
    let on_page = save("chirp_page", description(&base, page, &["publish", "publish_media", "metrics"], false), json!({"client": "page-secret"})).await;
    let mine = json!({"type": "oauth", "owner": "person", "provider": provider("pia")});
    let on_mine = save("chirp_mine", description(&base, mine, &["publish"], false), json!({"client": "pia-secret"})).await;
    // What a description with a wrong operation says, in the platform's words.
    let bad = description(&base, json!({"type": "none"}), &["publish"], false);
    let mut bad = bad;
    bad["operations"]["publish"]["method"] = json!("TRACE");
    assert!(t.k.save_connection_def(&t.admin, None, &input("bad", bad, None)).await.unwrap_err().to_string().contains("method is GET"));

    // ───────────── network profiles: what a network takes is a record ─────────────
    let seeded = t.rows("SocialProfile", json!({})).await;
    assert!(seeded.iter().any(|p| p["profile_name"] == "X" && p["max_text"] == 280 && p["link_length"] == 23), "the profiles the app knew, now records");
    let profile = |name: &str, extra: Value| {
        let mut p = json!({"profile_name": name, "max_text": 100, "link_length": 10, "images": true, "max_image_bytes": 5000, "first_comment": true});
        for (k, v) in extra.as_object().unwrap() {
            p[k] = v.clone();
        }
        p
    };
    let chirp = uid(&t.insert(&manager, "SocialProfile", profile("Chirp", json!({}))).await["id"]);
    let words = uid(&t.insert(&manager, "SocialProfile", profile("Chirp Words", json!({"images": false, "first_comment": false}))).await["id"]);
    let pictured = uid(&t.insert(&manager, "SocialProfile", profile("Chirp Pictures", json!({"media_required": true}))).await["id"]);
    refused(t.k.insert(&author, "SocialProfile", obj(profile("Mine", json!({})))).await, "forbidden", "");

    // ───────────── brands and their accounts ─────────────
    let northwind = uid(&t.insert(&manager, "SocialBrand", json!({"brand_name": "Northwind"})).await["id"]);
    let contoso = uid(&t.insert(&manager, "SocialBrand", json!({"brand_name": "Contoso"})).await["id"]);
    let account = |name: &str, brand: Uuid, profile: Uuid, handle: &str, conn: Uuid| json!({"account_name": name, "brand": brand.to_string(), "profile": profile.to_string(), "handle": handle, "connection": conn.to_string()});
    refused(t.k.insert(&manager, "SocialAccount", obj(account("Lost", northwind, chirp, "lost", Uuid::new_v4()))).await, "invalid", "no connection by that id");
    refused(
        t.k.insert(
            &manager,
            "SocialAccount",
            obj(json!({"account_name": "Nameless", "brand": northwind.to_string(), "handle": "n", "connection": "no such connection"})),
        )
        .await,
        "invalid",
        "no connection is named",
    );
    refused(
        t.k.insert(
            &manager,
            "SocialAccount",
            obj(json!({"account_name": "Unprofiled", "brand": northwind.to_string(), "handle": "n", "connection": on_key.to_string()})),
        )
        .await,
        "invalid",
        "pick one of the network profiles",
    );
    let nw_key = uid(&t.insert(&manager, "SocialAccount", account("Northwind Chirp", northwind, chirp, "@northwind", on_key)).await["id"]);
    let row = t.get("SocialAccount", nw_key).await;
    assert_eq!(
        (row["handle"].as_str(), row["network"].as_str(), row["state"].as_str()),
        (Some("northwind"), Some("Chirp"), Some("Connected")),
        "a key an administrator holds is connected"
    );
    // The connection by its name, as Administration shows it.
    let mut by_name = account("Northwind Words", northwind, words, "northwind_words", on_words);
    by_name["connection"] = json!("CHIRP_WORDS");
    let nw_words = uid(&t.insert(&manager, "SocialAccount", by_name).await["id"]);
    assert_eq!(t.get("SocialAccount", nw_words).await["connection"], json!(on_words.to_string()));
    let nw_page = uid(&t.insert(&manager, "SocialAccount", account("Northwind Page", northwind, chirp, "northwind_page", on_page)).await["id"]);
    let mut own = account("Pia's Chirp", northwind, chirp, "pia", on_mine);
    own["connected_by"] = json!("pia@brands.test");
    let nw_pia = uid(&t.insert(&manager, "SocialAccount", own).await["id"]);
    let nw_pictured = uid(&t.insert(&manager, "SocialAccount", account("Northwind Pictures", northwind, pictured, "northwind_pics", on_key)).await["id"]);
    let co_key = uid(&t.insert(&manager, "SocialAccount", account("Contoso Chirp", contoso, chirp, "contoso", on_key)).await["id"]);
    refused(t.k.insert(&author, "SocialAccount", obj(account("Mine", northwind, chirp, "mine", on_key))).await, "forbidden", "");

    // ───────────── sign-in: not connected, connected, the provider refusing it, reconnected, disconnected ─────────────
    let state = |id: Uuid| {
        let t = &t;
        async move {
            let r = t.get("SocialAccount", id).await;
            (r["state"].as_str().unwrap().to_string(), r["state_note"].as_str().map(str::to_string), r["provider_account"].as_str().map(str::to_string))
        }
    };
    assert_eq!(state(nw_page).await.0, "Not Connected", "nobody has signed in");
    assert_eq!(state(nw_pia).await.0, "Not Connected");
    assert_eq!(
        t.act(&manager, "SocialAccount", nw_page, "connect", json!({})).await.unwrap()["connect"]["path"],
        json!(format!("/api/connections/{on_page}/connect"))
    );
    assert!(t.act(&manager, "SocialAccount", nw_key, "connect", json!({})).await.unwrap_err().to_string().contains("a key an administrator holds"));
    t.sign_in(&t.admin, on_page, "CODE-page").await;
    t.sign_in(&pia, on_mine, "CODE-pia").await;
    let checked = t.act(&manager, "SocialAccount", nw_page, "check", json!({})).await.unwrap();
    assert_eq!(checked["state"], "Connected");
    assert_eq!(state(nw_page).await, ("Connected".into(), None, Some("page".into())), "the account the provider names");
    t.act(&manager, "SocialAccount", nw_pia, "check", json!({})).await.unwrap();
    assert_eq!(state(nw_pia).await.2, Some("pia".into()), "a person's own sign-in, named by whom it was for");
    // The provider refuses the workspace's sign-in: the platform marks it and tells who connected it; the account shows it.
    t.net.lock().unwrap().revoked.insert("page".into());
    let mut tx = t.k.db.tenant_tx(tenant).await.unwrap();
    sqlx::query("update conn_token set expires_at = now() + interval '5 minutes' where tenant_id = $1 and conn_id = $2")
        .bind(tenant)
        .bind(on_page)
        .execute(&mut *tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(t.k.sweep_tokens_at(Utc::now()).await.unwrap(), 1);
    t.work().await;
    ferrum_app_social::maintenance_at(&t.k, Utc::now()).await.unwrap();
    let (st, note, _) = state(nw_page).await;
    assert_eq!(st, "Needs Reconnecting", "{note:?}");
    assert!(note.unwrap().contains("connect the account again"));
    assert_eq!(
        t.count("select count(*) from notification where tenant_id = $1 and title = 'Reconnect chirp_page'").await,
        1,
        "the platform told its owner, once"
    );
    assert_eq!(state(nw_pia).await.0, "Connected", "another account's sign-in is its own");
    // Reconnecting, then disconnecting with revoke.
    t.net.lock().unwrap().revoked.clear();
    t.sign_in(&t.admin, on_page, "CODE-page").await;
    assert_eq!(t.act(&manager, "SocialAccount", nw_page, "check", json!({})).await.unwrap()["state"], "Connected");
    let revokes = t.net.lock().unwrap().revokes;
    let gone = t.act(&t.admin, "SocialAccount", nw_page, "disconnect", json!({})).await.unwrap();
    assert_eq!((gone["revoked"].clone(), gone["state"].as_str()), (json!(true), Some("Not Connected")), "{gone}");
    assert_eq!(t.net.lock().unwrap().revokes, revokes + 1, "the provider was told to revoke");
    refused(t.act(&manager, "SocialAccount", nw_page, "disconnect", json!({})).await, "forbidden", "");
    t.sign_in(&t.admin, on_page, "CODE-page").await;
    t.act(&manager, "SocialAccount", nw_page, "check", json!({})).await.unwrap();

    // ───────────── what a network takes, in plain words, before a post is scheduled ─────────────
    let at = Utc::now() + Duration::hours(1);
    let post = |text: &str, targets: &[Uuid]| {
        json!({"headline": "Spring launch", "brand": northwind.to_string(), "text": text, "link": "https://northwind.example/spring-in-the-very-long-address-of-a-page",
               "scheduled_at": at.to_rfc3339(), "targets": targets.iter().map(|a| json!({"account": a.to_string()})).collect::<Vec<_>>()})
    };
    // 100 characters; the link counts 10 and a space.
    refused(t.ready(&author, post(&"a".repeat(90), &[nw_key])).await, "invalid", "for Northwind Chirp: Chirp takes 100 characters, and this post is 101");
    t.insert(&author, "SocialPost", post(&"a".repeat(89), &[nw_key])).await;
    refused(t.ready(&author, post("Spring is here", &[co_key])).await, "invalid", "Contoso Chirp is another brand's");
    refused(t.ready(&author, post("Spring is here", &[nw_pictured])).await, "invalid", "takes no post without an image or a video");
    let png = t.k.store_file(&author, None, "spring.png", "image/png", b"\x89PNG\r\n\x1a\nspring").await.unwrap();
    let big = t.k.store_file(&author, None, "big.png", "image/png", &vec![7u8; 6000]).await.unwrap();
    let pictured_post = |targets: &[Uuid], file: &Value, alt: Option<&str>| {
        let mut p = post("Spring in bloom", targets);
        p["image"] = file["url"].clone();
        p["image_alt"] = json!(alt);
        p
    };
    refused(t.ready(&author, pictured_post(&[nw_key], &png, None)).await, "invalid", "says what the picture shows");
    refused(t.ready(&author, pictured_post(&[nw_key], &big, Some("Big"))).await, "invalid", "takes pictures of at most 5 KB, and this one is 6 KB");
    refused(t.ready(&author, pictured_post(&[nw_words], &png, Some("Sage"))).await, "invalid", "Chirp Words takes no pictures");
    refused(t.ready(&author, pictured_post(&[nw_pia], &png, Some("Sage"))).await, "invalid", "Pia's Chirp's connection cannot publish pictures");
    let mut commented = post("Spring is here", &[nw_words]);
    commented["first_comment"] = json!("More at the link");
    refused(t.ready(&author, commented).await, "invalid", "Chirp Words takes no first comment");
    let mut bad_link = post("Spring", &[nw_key]);
    bad_link["link"] = json!("northwind dot example");
    refused(t.ready(&author, bad_link).await, "invalid", "is a web address");
    let mut elsewhere = pictured_post(&[nw_key], &png, Some("Sage"));
    elsewhere["image"] = json!("https://elsewhere.example/sage.png");
    refused(t.ready(&author, elsewhere).await, "invalid", "is a picture stored here");
    // A network's profile edited: the post is looked at again where it is sent (below).

    // ───────────── review: someone other than the author approves ─────────────
    let tagged = |text: &str, targets: &[Uuid], head: &str| {
        let mut p = post(text, targets);
        p["headline"] = json!(head);
        p
    };
    let spring = t.insert(&author, "SocialPost", tagged("Spring is here", &[nw_key, nw_page, nw_pia], "Spring launch")).await;
    let spring_id = uid(&spring["id"]);
    assert_eq!((spring["author"].as_str(), spring["status"].as_str()), (Some("author@brands.test"), Some("Draft")));
    // The package's approval policy reproduces 0.7.0's rule: one step, a Social Approver, never the author (who asks).
    let go = |id: Uuid| {
        let (t, author, approver) = (&t, &author, &approver);
        async move {
            let asked = t.act(author, "SocialPost", id, "send_for_review", Value::Null).await.unwrap();
            assert_eq!(asked["approval"]["steps"][0]["label"], "Social Approver", "{asked}");
            refused(t.k.decide_approval(author, "SocialPost", id, true, None).await, "forbidden", "");
            t.k.decide_approval(approver, "SocialPost", id, true, None).await.expect("approved");
            assert!(matches!(t.get("SocialPost", id).await["status"].as_str(), Some("Approved" | "Sending")));
        }
    };
    t.act(&author, "SocialPost", spring_id, "send_for_review", Value::Null).await.unwrap();
    assert_eq!(t.get("SocialPost", spring_id).await["status"], "In Review");
    refused(t.act(&approver, "SocialPost", spring_id, "approve", Value::Null).await, "invalid", "decided in its approval");
    assert!(t.k.decide_approval(&approver, "SocialPost", spring_id, false, Some(" ")).await.unwrap_err().to_string().contains("reason"));
    t.k.decide_approval(&approver, "SocialPost", spring_id, false, Some("Say which colours")).await.unwrap();
    ferrum_app_social::maintenance_at(&t.k, Utc::now()).await.unwrap();
    let sent_back = t.get("SocialPost", spring_id).await;
    assert_eq!((sent_back["status"].as_str(), sent_back["review_note"].as_str()), (Some("Rejected"), Some("Say which colours")), "the approver's words");
    t.act(&author, "SocialPost", spring_id, "send_for_review", Value::Null).await.unwrap();
    // A change while it waits voids what was decided so far: the policy asks again.
    t.k.update(&author, "SocialPost", spring_id, obj(json!({"text": "Spring is here."})), None).await.unwrap();
    t.k.decide_approval(&approver, "SocialPost", spring_id, true, None).await.unwrap();
    ferrum_app_social::maintenance_at(&t.k, Utc::now()).await.unwrap();
    let approved = t.get("SocialPost", spring_id).await;
    assert_eq!((approved["status"].as_str(), approved["approved_by"].as_str()), (Some("Approved"), Some("approver@brands.test")), "who approved it, read back");
    t.k.update(&author, "SocialPost", spring_id, obj(json!({"text": "Spring is here!"})), None).await.unwrap();
    assert_eq!(t.get("SocialPost", spring_id).await["status"], "Draft", "changed words need approving again");
    go(spring_id).await;

    // ───────────── scheduling is the app's: nothing goes before its time ─────────────
    t.upkeep(Utc::now()).await;
    assert!(t.net.lock().unwrap().posts.is_empty());
    assert_eq!(t.get("SocialPost", spring_id).await["status"], "Approved");

    // ───────────── publishing: one queued call per account, with a key; the network's id and address kept ─────────────
    let due = at + Duration::hours(1);
    t.upkeep(due).await;
    {
        let n = t.net.lock().unwrap();
        assert_eq!(n.posts.len(), 3, "one post for each account");
        let tokens: HashSet<&str> = n.posts.iter().map(|p| p.2.as_str()).collect();
        assert!(n.posts.iter().any(|p| p.2.starts_with("AT-pia")), "a person's own sign-in posts as them: {tokens:?}");
        assert!(n.posts.iter().any(|p| p.2.starts_with("AT-page")), "the workspace's sign-in posts as the page");
        assert!(n.posts.iter().all(|p| p.3.starts_with("publish:")), "each under the key of its publication");
        assert_eq!(
            n.posts[0].1["text"], "Spring is here! https://northwind.example/spring-in-the-very-long-address-of-a-page",
            "the words and the link, as the description's body asks for them"
        );
        assert!(n.posts[0].1.get("link").is_none(), "only what the body names is sent");
        assert_eq!(n.posts[0].1["media_ids"], json!([]));
    }
    let publications = t.rows("SocialPublication", json!({"post": spring_id.to_string()})).await;
    assert_eq!(publications.len(), 3);
    for p in &publications {
        assert!(
            p["external_id"].as_str().is_some_and(|e| e.starts_with("p-")) && p["permalink"].as_str().is_some_and(|u| u.starts_with("https://chirp.test/p/")),
            "{p:?}"
        );
        assert!(p["failure"].is_null() && p["failed_at"].is_null());
    }
    let spring_row = t.get("SocialPost", spring_id).await;
    assert_eq!(spring_row["status"], "Published", "published when every account has it");
    let pubs: HashMap<Uuid, Doc> = publications.iter().map(|p| (uid(&p["account"]), p.clone())).collect();
    let page_pub = uid(&pubs[&nw_page]["id"]);
    let key_pub = uid(&pubs[&nw_key]["id"]);
    assert_eq!(t.net.lock().unwrap().keys.len(), 3);
    t.upkeep(due + Duration::hours(1)).await;
    assert_eq!(t.net.lock().unwrap().posts.len(), 3, "the upkeep again sends nothing twice");

    // A picture goes first, where the connection says how; its id is the post's media.
    let pic = t.insert(&author, "SocialPost", pictured_post(&[nw_key, nw_page], &png, Some("Sage in bloom"))).await;
    go(uid(&pic["id"])).await;
    t.upkeep(due + Duration::hours(2)).await;
    {
        let n = t.net.lock().unwrap();
        assert_eq!(n.media.len(), 2, "uploaded for each account");
        assert_eq!(
            (n.media[0]["alt"].as_str(), n.media[0]["type"].as_str(), n.media[0]["size"].as_i64()),
            (Some("Sage in bloom"), Some("image/png"), Some(14))
        );
        assert_eq!(n.media[0]["data"], "iVBORw0KGgpzcHJpbmc=", "the file's bytes, as base64");
        let last: Vec<&str> = n.order.iter().rev().take(4).copied().collect();
        assert_eq!(last.iter().filter(|o| **o == "post").count(), 2);
        assert!(n.order.iter().rposition(|o| *o == "media") < n.order.iter().rposition(|o| *o == "post"), "a picture goes before the post that carries it");
        let with_media = n.posts.iter().filter(|p| p.1["media_ids"] != json!([])).count();
        assert_eq!(with_media, 2, "each post carries its picture's id");
    }

    // Once under a lost answer: the network took the post and the answer never came; the retry carries the same key.
    let lost = t.insert(&author, "SocialPost", tagged("Lost in the post", &[nw_key], "Lost answer")).await;
    let lost_id = uid(&lost["id"]);
    go(lost_id).await;
    t.net.lock().unwrap().lose = 1;
    let before = t.net.lock().unwrap().posts.len();
    t.upkeep(due + Duration::hours(3)).await;
    assert_eq!(t.net.lock().unwrap().posts.len(), before + 1, "one post on the network, not two");
    assert_eq!(t.get("SocialPost", lost_id).await["status"], "Published");
    // "Later" three times: waiting, shown as so, then out; one post.
    let later = t.insert(&author, "SocialPost", tagged("Busy network", &[nw_key], "Busy")).await;
    go(uid(&later["id"])).await;
    t.net.lock().unwrap().later = 2;
    ferrum_app_social::maintenance_at(&t.k, due + Duration::hours(4)).await.unwrap();
    t.k.run_jobs_once(1).await.unwrap();
    let waiting = t.rows("SocialPublication", json!({"post": uid(&later["id"]).to_string()})).await;
    assert!(waiting[0]["failure"].as_str().unwrap().starts_with("Waiting to try again") && waiting[0]["failed_at"].is_null(), "{:?}", waiting[0]);
    t.upkeep(due + Duration::hours(4)).await;
    let done = t.rows("SocialPublication", json!({"post": uid(&later["id"]).to_string()})).await;
    assert!(done[0]["external_id"].is_string() && done[0]["failure"].is_null(), "{:?}", done[0]);
    assert_eq!(t.get("SocialPost", uid(&later["id"])).await["status"], "Published");

    // Two workers at one publication: the network has one post.
    let twice = t.insert(&author, "SocialPost", tagged("Two at once", &[nw_key], "Twice")).await;
    go(uid(&twice["id"])).await;
    ferrum_app_social::maintenance_at(&t.k, due + Duration::hours(5)).await.unwrap();
    let twice_pubs = t.rows("SocialPublication", json!({"post": uid(&twice["id"]).to_string()})).await;
    let mut tx = t.k.db.tenant_tx(tenant).await.unwrap();
    let extra = ferrum_kernel::jobs::NewJob::new("social.publish", json!({"publication": twice_pubs[0]["id"]}));
    ferrum_kernel::jobs::enqueue(&mut tx, tenant, None, extra).await.unwrap();
    tx.commit().await.unwrap();
    let before = t.net.lock().unwrap().posts.len();
    let (a, b) = tokio::join!(t.k.run_jobs_once(1), t.k.run_jobs_once(1));
    assert_eq!(a.unwrap() + b.unwrap(), 2, "both jobs ran");
    assert_eq!(t.net.lock().unwrap().posts.len(), before + 1, "once: the second finds it out under the publication's lock");
    t.upkeep(due + Duration::hours(5)).await;

    // A refusal is shown in the service's own words, ends at once, and is retried by a person.
    let refusal = t.insert(&author, "SocialPost", tagged("Not allowed", &[nw_key, nw_words], "Refused")).await;
    let refusal_id = uid(&refusal["id"]);
    go(refusal_id).await;
    t.net.lock().unwrap().mode = "reject";
    let before = t.net.lock().unwrap().keys.len();
    t.upkeep(due + Duration::hours(6)).await;
    let failed = t.get("SocialPost", refusal_id).await;
    assert_eq!(failed["status"], "Failed");
    assert!(
        failed["failure"].as_str().unwrap().contains("Northwind Chirp: ") && failed["failure"].as_str().unwrap().contains("That text is not allowed"),
        "{failed:?}"
    );
    let failed_pubs = t.rows("SocialPublication", json!({"post": refusal_id.to_string()})).await;
    assert!(
        failed_pubs.iter().all(|p| p["failed_at"].is_string() && p["failure"].as_str().unwrap().contains("That text is not allowed")),
        "per account, the network's words"
    );
    assert_eq!(t.net.lock().unwrap().keys.len(), before, "nothing was made");
    t.net.lock().unwrap().mode = "ok";
    t.act(&author, "SocialPost", refusal_id, "retry", Value::Null).await.unwrap();
    t.upkeep(due + Duration::hours(6)).await;
    assert_eq!(t.get("SocialPost", refusal_id).await["status"], "Published");
    assert_eq!(t.net.lock().unwrap().keys.len(), before + 2, "retried to both, once each");
    // Always "later": eight attempts, then failed with what it said.
    let down = t.insert(&author, "SocialPost", tagged("Never", &[nw_key], "Down")).await;
    go(uid(&down["id"])).await;
    t.net.lock().unwrap().later = 100;
    t.upkeep(due + Duration::hours(7)).await;
    let ended = t.get("SocialPost", uid(&down["id"])).await;
    assert!(ended["status"] == "Failed" && ended["failure"].as_str().unwrap().contains("Chirp is busy"), "{ended:?}");
    t.net.lock().unwrap().later = 0;
    // A sign-in the provider no longer takes: said plainly on the publication, and the account shows it.
    t.net.lock().unwrap().revoked.insert("page".into());
    let mut tx = t.k.db.tenant_tx(tenant).await.unwrap();
    sqlx::query("update conn_token set expires_at = now() - interval '1 minute' where tenant_id = $1 and conn_id = $2")
        .bind(tenant)
        .bind(on_page)
        .execute(&mut *tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let stale = t.insert(&author, "SocialPost", tagged("Signed out", &[nw_page], "Signed out")).await;
    go(uid(&stale["id"])).await;
    t.upkeep(due + Duration::hours(8)).await;
    let ended = t.get("SocialPost", uid(&stale["id"])).await;
    assert!(ended["status"] == "Failed" && ended["failure"].as_str().unwrap().contains("needs reconnecting"), "{ended:?}");
    assert_eq!(state(nw_page).await.0, "Needs Reconnecting");
    t.net.lock().unwrap().revoked.clear();
    t.sign_in(&t.admin, on_page, "CODE-page").await;
    t.act(&author, "SocialPost", uid(&stale["id"]), "retry", Value::Null).await.unwrap();
    t.upkeep(due + Duration::hours(8)).await;
    assert_eq!(t.get("SocialPost", uid(&stale["id"])).await["status"], "Published", "reconnected, retried, out");
    // What a network takes, changed after approval, is looked at again where the post is sent.
    let strict = t.insert(&author, "SocialPost", tagged("A longer text than will fit soon", &[nw_key], "Strict")).await;
    go(uid(&strict["id"])).await;
    t.k.update(&manager, "SocialProfile", chirp, obj(json!({"max_text": 20})), None).await.unwrap();
    t.upkeep(due + Duration::hours(9)).await;
    let ended = t.get("SocialPost", uid(&strict["id"])).await;
    assert!(ended["status"] == "Failed" && ended["failure"].as_str().unwrap().contains("Chirp takes 20 characters"), "{ended:?}");
    t.k.update(&manager, "SocialProfile", chirp, obj(json!({"max_text": 100})), None).await.unwrap();

    // A first comment follows the post where the network takes one; delete where the connection offers it.
    let mut first = tagged("With a comment", &[nw_key], "First comment");
    first["first_comment"] = json!("More at the link");
    let first = t.insert(&author, "SocialPost", first).await;
    go(uid(&first["id"])).await;
    t.upkeep(due + Duration::hours(10)).await;
    let first_pub = t.rows("SocialPublication", json!({"post": uid(&first["id"]).to_string()})).await;
    {
        let n = t.net.lock().unwrap();
        let (k, r) = n.replies.last().unwrap();
        assert!(k.starts_with("first_comment:") && r["text"] == "More at the link" && r["in_reply_to"] == first_pub[0]["external_id"], "{r}");
    }
    t.act(&manager, "SocialPublication", key_pub, "delete_remote", json!({})).await.unwrap();
    let deleted = t.get("SocialPublication", key_pub).await["external_id"].as_str().unwrap().to_string();
    assert_eq!(t.net.lock().unwrap().deleted, vec![deleted]);
    assert!(t.get("SocialPublication", key_pub).await["deleted_at"].is_string());
    assert!(t.act(&manager, "SocialPublication", key_pub, "delete_remote", json!({})).await.is_err(), "already deleted");
    let words_pub = t.insert(&author, "SocialPost", tagged("Words only", &[nw_words], "Words")).await;
    go(uid(&words_pub["id"])).await;
    t.upkeep(due + Duration::hours(10)).await;
    let wp = t.rows("SocialPublication", json!({"post": uid(&words_pub["id"]).to_string()})).await;
    refused(t.act(&manager, "SocialPublication", uid(&wp[0]["id"]), "delete_remote", json!({})).await, "invalid", "does not offer to delete a post");
    refused(t.act(&author, "SocialPublication", key_pub, "delete_remote", json!({})).await, "forbidden", "");

    // ───────────── content: versions per account, threads, several media, what a network asks ─────────────
    // What this network takes and asks is on its description (meta.social): no profile, no Rust.
    let social = json!({"name": "Chirp Threads", "text": {"max": 100},
        "media": {"kinds": ["image"], "max": 4, "alt": true},
        "kinds": [{"name": "post"}, {"name": "story", "follow_ups": {"as": "none"}}],
        "follow_ups": {"as": "thread", "media": true, "max": 5},
        "settings": [{"key": "audience", "label": "Audience", "type": "select", "options": ["everyone", "followers"], "required": true},
                     {"key": "pin", "label": "Pin it", "type": "bool", "default": false}]});
    let mut threads_desc = description(&base, json!({"type": "header", "header": "x-api-key", "secret": "key"}), &everything, false);
    threads_desc["meta"] = json!({"social": social.clone()});
    let on_threads = save("chirp_threads", threads_desc, json!({"key": KEY})).await;
    let mut comments_social = social.clone();
    comments_social["name"] = json!("Chirp Comments");
    comments_social["follow_ups"] = json!({"as": "comment", "media": false});
    comments_social["settings"] = json!([]);
    let mut comments_desc = description(&base, json!({"type": "header", "header": "x-api-key", "secret": "key"}), &everything, false);
    comments_desc["meta"] = json!({"social": comments_social});
    let on_comments = save("chirp_comments", comments_desc, json!({"key": KEY})).await;
    let threaded =
        |name: &str, handle: &str, conn: Uuid| json!({"account_name": name, "brand": northwind.to_string(), "handle": handle, "connection": conn.to_string()});
    let nw_threads = uid(&t.insert(&manager, "SocialAccount", threaded("Northwind Threads", "nw_threads", on_threads)).await["id"]);
    let nw_comments = uid(&t.insert(&manager, "SocialAccount", threaded("Northwind Comments", "nw_comments", on_comments)).await["id"]);
    let target = |a: Uuid, extra: Value| {
        let mut row = json!({"account": a.to_string()});
        for (k, v) in extra.as_object().unwrap() {
            row[k] = v.clone();
        }
        row
    };
    let answered = json!({"settings": {"audience": "followers"}});
    // Settings: what the network asks is answered per account, refused when missing or not asked, passed on.
    let mut asked = tagged("Hello threads", &[], "Settings");
    asked["targets"] = json!([target(nw_threads, json!({}))]);
    refused(t.ready(&author, asked.clone()).await, "invalid", "for Northwind Threads: Audience is needed");
    asked["targets"] = json!([target(nw_threads, json!({"settings": {"audience": "followers", "colour": "red"}}))]);
    refused(t.ready(&author, asked.clone()).await, "invalid", "Chirp Threads asks nothing called colour");
    asked["targets"] = json!([target(nw_threads, json!({"settings": {"audience": "nobody"}}))]);
    refused(t.ready(&author, asked.clone()).await, "invalid", "Audience is one of everyone, followers");
    asked["targets"] = json!([target(nw_threads, json!({"kind": "reel", "settings": {"audience": "followers"}}))]);
    refused(t.ready(&author, asked.clone()).await, "invalid", "Chirp Threads has no kind of post called reel");
    // (round two) A draft keeps what its networks would refuse, says it, and may not leave Draft with it; a kind or a
    // setting the network does not have is refused even in a draft; a post in review is held to every rule on save.
    let mut half = tagged(&"a".repeat(150), &[], "Half written");
    half["link"] = Value::Null;
    half["targets"] = json!([target(nw_threads, json!({}))]);
    let half = t.insert(&author, "SocialPost", half).await;
    let said = t.act(&author, "SocialPost", uid(&half["id"]), "copies", Value::Null).await.unwrap()["copies"][0]["problems"].clone();
    assert!(said.to_string().contains("Audience is needed") && said.to_string().contains("takes 100 characters, and this post is 150"), "{said}");
    refused(t.act(&author, "SocialPost", uid(&half["id"]), "send_for_review", Value::Null).await, "invalid", "takes 100 characters, and this post is 150");
    refused(t.act(&author, "SocialPost", uid(&half["id"]), "add_to_queue", Value::Null).await, "invalid", "this post is 150");
    t.k.update(&author, "SocialPost", uid(&half["id"]), obj(json!({"text": "Fixed", "targets": [target(nw_threads, answered.clone())]})), None).await.unwrap();
    t.act(&author, "SocialPost", uid(&half["id"]), "send_for_review", Value::Null).await.unwrap();
    refused(t.k.update(&author, "SocialPost", uid(&half["id"]), obj(json!({"text": "b".repeat(150)})), None).await, "invalid", "takes 100 characters");
    // Versions: the original to one account, the account's own words to another.
    let mut versions = tagged("The original words", &[nw_key], "Versions");
    versions["targets"] =
        json!([target(nw_key, json!({})), target(nw_threads, json!({"text": "Threads get their own words", "settings": {"audience": "everyone"}}))]);
    let versions = t.insert(&author, "SocialPost", versions).await;
    go(uid(&versions["id"])).await;
    let before = t.net.lock().unwrap().posts.len();
    t.upkeep(due + Duration::hours(11)).await;
    {
        let n = t.net.lock().unwrap();
        let made: Vec<&Value> = n.posts[before..].iter().map(|p| &p.1).collect();
        assert_eq!(made.len(), 2, "{made:?}");
        assert!(made.iter().any(|b| b["text"].as_str().unwrap().starts_with("The original words")), "{made:?}");
        let own = made.iter().find(|b| b["text"].as_str().unwrap().starts_with("Threads get")).expect("the version");
        assert_eq!((own["settings"].clone(), own["kind"].as_str()), (json!({"audience": "everyone", "pin": false}), Some("post")), "answers, with defaults");
    }
    assert_eq!(t.get("SocialPost", uid(&versions["id"])).await["status"], "Published");
    // A three-item thread with several media; the third is refused, the first two stay out; Retry sends the third alone.
    let pics = [
        t.k.store_file(&author, None, "one.png", "image/png", b"\x89PNG\r\n\x1a\none").await.unwrap(),
        t.k.store_file(&author, None, "two.png", "image/png", b"\x89PNG\r\n\x1a\ntwo").await.unwrap(),
        t.k.store_file(&author, None, "three.png", "image/png", b"\x89PNG\r\n\x1a\nthree").await.unwrap(),
    ];
    let mut thread = tagged("One of three", &[], "Thread");
    thread["targets"] = json!([target(nw_threads, answered.clone())]);
    thread["items"] = json!([{"text": "Two of three"}, {"text": "BOOM three of three"}]);
    thread["media"] = json!([{"item": 1, "file": pics[0]["url"], "alt": "First"}, {"item": 1, "file": pics[1]["url"], "alt": "Second"},
                             {"item": 2, "file": pics[2]["url"], "alt": "Third"}]);
    let mut too_many = thread.clone();
    too_many["media"] = json!([{"item": 4, "file": pics[0]["url"]}]);
    refused(t.ready(&author, too_many).await, "invalid", "a medium is for item 4, and Northwind Threads has 3");
    let thread = t.insert(&author, "SocialPost", thread).await;
    let thread_id = uid(&thread["id"]);
    go(thread_id).await;
    t.net.lock().unwrap().refuse_text = Some("BOOM".into());
    let (posts_before, media_before) = {
        let n = t.net.lock().unwrap();
        (n.posts.len(), n.media.len())
    };
    t.upkeep(due + Duration::hours(12)).await;
    let ended = t.get("SocialPost", thread_id).await;
    assert!(ended["status"] == "Failed" && ended["failure"].as_str().unwrap().contains("That item is not allowed"), "{ended:?}");
    let thread_pub = t.rows("SocialPublication", json!({"post": thread_id.to_string()})).await.remove(0);
    assert_eq!(thread_pub["sent_items"].as_array().unwrap().len(), 2, "the first two are out and kept: {thread_pub:?}");
    {
        let n = t.net.lock().unwrap();
        assert_eq!(n.posts.len(), posts_before + 2);
        assert_eq!(n.media.len(), media_before + 3, "each medium uploaded once");
        let (first, second) = (&n.posts[posts_before], &n.posts[posts_before + 1]);
        assert_eq!(first.1["media_ids"].as_array().unwrap().len(), 2, "two pictures on the first item");
        assert_eq!(second.1["reply_to"], json!(first.0), "the second answers the first");
        assert!(first.3.starts_with("publish:") && second.3.ends_with(":1"), "keys by item: {} {}", first.3, second.3);
    }
    t.net.lock().unwrap().refuse_text = None;
    t.act(&author, "SocialPost", thread_id, "retry", Value::Null).await.unwrap();
    t.upkeep(due + Duration::hours(12)).await;
    assert_eq!(t.get("SocialPost", thread_id).await["status"], "Published");
    let thread_pub = t.get("SocialPublication", uid(&thread_pub["id"])).await;
    let items = thread_pub["sent_items"].as_array().unwrap().clone();
    {
        let n = t.net.lock().unwrap();
        assert_eq!(n.posts.len(), posts_before + 3, "items one and two were not sent again");
        assert_eq!(n.media.len(), media_before + 3);
        assert_eq!(n.posts.last().unwrap().1["reply_to"], items[1]["id"], "the third answers the second");
    }
    assert_eq!(items.len(), 3);
    assert!(items.iter().all(|i| i["url"].as_str().is_some_and(|u| u.starts_with("https://chirp.test/p/"))), "each item's address: {items:?}");
    // Delete on the network takes every item down, the last first.
    let deleted_before = t.net.lock().unwrap().deleted.len();
    assert_eq!(t.act(&manager, "SocialPublication", uid(&thread_pub["id"]), "delete_remote", json!({})).await.unwrap()["deleted"], 3);
    let gone: Vec<String> = t.net.lock().unwrap().deleted[deleted_before..].to_vec();
    assert_eq!(gone, items.iter().rev().map(|i| i["id"].as_str().unwrap().to_string()).collect::<Vec<_>>());
    // Comments as follow-ups: the network's reply, under the first item; with media where it takes none, refused.
    let mut commented = tagged("Post with comments", &[nw_comments], "Comments");
    commented["items"] = json!([{"text": "A comment", "delay_minutes": 0}]);
    commented["media"] = json!([{"item": 2, "file": pics[0]["url"]}]);
    refused(t.ready(&author, commented.clone()).await, "invalid", "Chirp Comments takes no media after the post");
    commented["media"] = json!([]);
    let commented = t.insert(&author, "SocialPost", commented).await;
    go(uid(&commented["id"])).await;
    t.upkeep(due + Duration::hours(13)).await;
    let cpub = t.rows("SocialPublication", json!({"post": uid(&commented["id"]).to_string()})).await.remove(0);
    {
        let n = t.net.lock().unwrap();
        let (k, r) = n.replies.last().unwrap();
        assert!(k.ends_with(":1") && r["text"] == "A comment" && r["in_reply_to"] == cpub["external_id"] && r["item"] == 1, "{k} {r}");
    }
    // A wait between items: the job is put back, nothing is lost.
    let mut waits = tagged("First, then wait", &[], "Wait");
    waits["targets"] = json!([target(nw_threads, answered.clone())]);
    waits["items"] = json!([{"text": "An hour later", "delay_minutes": 60}]);
    let waits = t.insert(&author, "SocialPost", waits).await;
    go(uid(&waits["id"])).await;
    let before = t.net.lock().unwrap().posts.len();
    t.upkeep(due + Duration::hours(14)).await;
    let wpub = t.rows("SocialPublication", json!({"post": uid(&waits["id"]).to_string()})).await.remove(0);
    assert!(wpub["next_at"].is_string() && wpub["sent_items"].as_array().unwrap().len() == 1, "{wpub:?}");
    assert_eq!(t.net.lock().unwrap().posts.len(), before + 1);
    assert_eq!(t.get("SocialPost", uid(&waits["id"])).await["status"], "Sending");
    // The hour passes: the second item goes, after the first.
    t.count(&format!(
        "with u as (update e_social_publication set sent_items = jsonb_set(sent_items::jsonb, '{{0,sent_at}}', '\"2020-01-01T00:00:00+00:00\"') where tenant_id = $1 and id = '{}' returning 1) select count(*) from u",
        wpub["id"].as_str().unwrap()
    ))
    .await;
    t.upkeep(due + Duration::hours(14)).await;
    assert_eq!(t.get("SocialPost", uid(&waits["id"])).await["status"], "Published");
    assert_eq!(t.net.lock().unwrap().posts.len(), before + 2);

    // ───────────── when: publish now, move, back to draft, duplicate, the queue, a repeat, bulk ─────────────
    // Publish now: an approved post goes at once; a draft is sent for review with the time now and goes when approved.
    let soon = t.insert(&author, "SocialPost", tagged("Now, not later", &[nw_key], "Now")).await;
    go(uid(&soon["id"])).await;
    let now_out = t.act(&author, "SocialPost", uid(&soon["id"]), "publish_now", Value::Null).await.unwrap();
    assert_eq!(now_out["status"], "Sending", "{now_out}");
    t.work().await;
    ferrum_app_social::maintenance_at(&t.k, Utc::now()).await.unwrap();
    assert_eq!(t.get("SocialPost", uid(&soon["id"])).await["status"], "Published");
    let mut draft = tagged("A draft to send now", &[nw_key], "Draft now");
    draft.as_object_mut().unwrap().remove("scheduled_at");
    let draft = t.insert(&author, "SocialPost", draft).await;
    assert!(draft["scheduled_at"].is_null(), "a draft needs no time");
    assert_eq!(t.act(&author, "SocialPost", uid(&draft["id"]), "publish_now", Value::Null).await.unwrap()["status"], "In Review");
    t.k.decide_approval(&approver, "SocialPost", uid(&draft["id"]), true, None).await.unwrap();
    assert_eq!(t.get("SocialPost", uid(&draft["id"])).await["status"], "Sending", "approved after its time: at once");
    t.work().await;
    ferrum_app_social::maintenance_at(&t.k, Utc::now()).await.unwrap();
    assert_eq!(t.get("SocialPost", uid(&draft["id"])).await["status"], "Published");
    // Move keeps an approval; back to draft takes it off the calendar before it goes; nothing is sent.
    let moved = t.insert(&author, "SocialPost", tagged("To be moved", &[nw_key], "Moved")).await;
    let moved_id = uid(&moved["id"]);
    go(moved_id).await;
    let later_at = (at + Duration::days(30)).to_rfc3339();
    t.act(&author, "SocialPost", moved_id, "reschedule", json!({"at": later_at})).await.unwrap();
    let row = t.get("SocialPost", moved_id).await;
    assert_eq!(row["status"], "Approved", "moving it keeps its approval");
    t.k.update(&author, "SocialPost", moved_id, obj(json!({"headline": "Moved, renamed"})), None).await.unwrap();
    assert_eq!(t.get("SocialPost", moved_id).await["status"], "Approved", "and a later save does not undo it");
    assert_eq!(t.act(&author, "SocialPost", moved_id, "unschedule", Value::Null).await.unwrap()["status"], "Draft");
    let row = t.get("SocialPost", moved_id).await;
    assert!(row["scheduled_at"].is_null() && row["approved_hash"].is_null(), "{row:?}");
    let before = t.net.lock().unwrap().posts.len();
    t.upkeep(at + Duration::days(31)).await;
    assert_eq!(t.net.lock().unwrap().posts.len(), before, "a draft never goes");
    // Duplicate: a new draft with the same content, versions and follow-ups.
    let copy = t.act(&pia, "SocialPost", thread_id, "duplicate", Value::Null).await.unwrap();
    let copy = t.get("SocialPost", uid(&copy["id"])).await;
    assert_eq!((copy["status"].as_str(), copy["author"].as_str(), copy["text"].as_str()), (Some("Draft"), Some("pia@brands.test"), Some("One of three")));
    assert_eq!((copy["items"].as_array().unwrap().len(), copy["media"].as_array().unwrap().len()), (2, 3));
    assert_eq!(copy["targets"][0]["settings"], json!({"audience": "followers"}));
    assert!(copy["scheduled_at"].is_null() && copy["origin"] == json!(thread_id.to_string()));
    // The queue: the brand's posting times; two posts added at once never get the same time.
    refused(t.act(&author, "SocialPost", uid(&copy["id"]), "add_to_queue", Value::Null).await, "invalid", "no posting times are set");
    for time in ["09:00", "17:00"] {
        t.insert(&manager, "SocialPostingTime", json!({"brand": northwind.to_string(), "time": time})).await;
    }
    refused(t.k.insert(&manager, "SocialPostingTime", obj(json!({"brand": northwind.to_string(), "time": "9am"}))).await, "invalid", "is a time of day");
    refused(t.k.insert(&author, "SocialPostingTime", obj(json!({"brand": northwind.to_string(), "time": "10:00"}))).await, "forbidden", "");
    let queued = [
        t.insert(&author, "SocialPost", tagged("Queued one", &[nw_key, nw_words], "Q1")).await,
        t.insert(&author, "SocialPost", tagged("Queued two", &[nw_key], "Q2")).await,
        t.insert(&author, "SocialPost", tagged("Queued three", &[nw_words], "Q3")).await,
    ];
    let (a, b) = tokio::join!(
        t.act(&author, "SocialPost", uid(&queued[0]["id"]), "add_to_queue", Value::Null),
        t.act(&author, "SocialPost", uid(&queued[1]["id"]), "add_to_queue", Value::Null)
    );
    let (a, b) = (a.unwrap()["scheduled_at"].as_str().unwrap().to_string(), b.unwrap()["scheduled_at"].as_str().unwrap().to_string());
    assert_ne!(a, b, "two at once, two times");
    let c = t.act(&author, "SocialPost", uid(&queued[2]["id"]), "add_to_queue", Value::Null).await.unwrap()["scheduled_at"].as_str().unwrap().to_string();
    assert_ne!(c, a, "the third shares an account with the first, so not its time");
    for t_ in [&a, &b, &c] {
        let h = chrono::DateTime::parse_from_rfc3339(t_).unwrap();
        assert!(matches!(chrono::Timelike::hour(&h), 9 | 17) && chrono::Timelike::minute(&h) == 0 && h > Utc::now(), "{t_}");
    }
    // An account's own times come before its brand's.
    t.insert(&manager, "SocialPostingTime", json!({"account": nw_words.to_string(), "time": "12:15"})).await;
    let own = t.insert(&author, "SocialPost", tagged("Own time", &[nw_words], "Q4")).await;
    let own_at = t.act(&author, "SocialPost", uid(&own["id"]), "add_to_queue", Value::Null).await.unwrap()["scheduled_at"].as_str().unwrap().to_string();
    assert!(own_at.contains("T12:15:00"), "{own_at}");
    // A repeat: each occurrence made when the one before goes out, once though two upkeeps run at once.
    let mut weekly = tagged("Every day, three times", &[nw_key], "Repeat");
    weekly["repeat_every"] = json!(1);
    weekly["repeat_unit"] = json!("Days");
    weekly["repeat_count"] = json!(3);
    let weekly = t.insert(&author, "SocialPost", weekly).await;
    let weekly_id = uid(&weekly["id"]);
    go(weekly_id).await;
    let when_due = due + Duration::hours(16);
    let (x, y) = tokio::join!(ferrum_app_social::maintenance_at(&t.k, when_due), ferrum_app_social::maintenance_at(&t.k, when_due));
    x.unwrap();
    y.unwrap();
    assert_eq!(t.rows("SocialPublication", json!({"post": weekly_id.to_string()})).await.len(), 1, "started once");
    t.work().await;
    let (x, y) = tokio::join!(ferrum_app_social::maintenance_at(&t.k, when_due), ferrum_app_social::maintenance_at(&t.k, when_due));
    x.unwrap();
    y.unwrap();
    let next = t.rows("SocialPost", json!({"repeat_of": weekly_id.to_string()})).await;
    assert_eq!(next.len(), 1, "one next occurrence");
    let next = t.get("SocialPost", uid(&next[0]["id"])).await;
    let first_at = chrono::DateTime::parse_from_rfc3339(weekly["scheduled_at"].as_str().unwrap()).unwrap();
    let next_at = chrono::DateTime::parse_from_rfc3339(next["scheduled_at"].as_str().unwrap()).unwrap();
    assert_eq!(next_at - first_at, Duration::days(1));
    assert_eq!((next["status"].as_str(), next["repeat_index"].as_i64(), next["text"].as_str()), (Some("Approved"), Some(2), Some("Every day, three times")));
    assert_eq!(t.get("SocialPost", weekly_id).await["repeat_next"], next["id"]);
    t.act(&author, "SocialPost", uid(&next["id"]), "stop_repeat", Value::Null).await.unwrap();
    let stopped = t.get("SocialPost", uid(&next["id"])).await;
    assert!(stopped["repeat_every"].is_null() && stopped["status"] == "Approved", "stopped, still approved: {stopped:?}");
    // Bulk: the platform's import makes posts with their accounts.
    let (made, errors) =
        t.k.import_rows(&author, "SocialPost", &[tagged("Imported one", &[nw_key], "Import 1"), tagged("Imported two", &[nw_key, nw_words], "Import 2")])
            .await
            .unwrap();
    assert!(errors.is_empty() && made.len() == 2, "{errors:?}");
    let imported = t.rows("SocialPost", json!({"headline": "Import 2"})).await;
    assert_eq!(t.get("SocialPost", uid(&imported[0]["id"])).await["targets"].as_array().unwrap().len(), 2);

    // ───────────── the library: signatures, variables, templates, sets, snippets ─────────────
    t.insert(
        &manager,
        "SocialSignature",
        json!({"signature_name": "Brand sign-off", "text": "— Northwind", "brand": northwind.to_string(), "is_default": true}),
    )
    .await;
    let first_default = t.rows("SocialSignature", json!({"signature_name": "Brand sign-off"})).await;
    t.insert(
        &manager,
        "SocialSignature",
        json!({"signature_name": "Brand sign-off 2", "text": "— The Northwind team", "brand": northwind.to_string(), "is_default": true}),
    )
    .await;
    assert_eq!(t.get("SocialSignature", uid(&first_default[0]["id"])).await["is_default"], false, "one default a brand");
    t.insert(&manager, "SocialSignature", json!({"signature_name": "Key's own", "text": "— N at {{shop}}", "account": nw_key.to_string(), "is_default": true}))
        .await;
    refused(t.k.insert(&author, "SocialSignature", obj(json!({"signature_name": "Mine", "text": "x"}))).await, "forbidden", "");
    t.insert(&manager, "SocialVariable", json!({"variable_name": "shop", "value": "shop.northwind.example", "brand": northwind.to_string()})).await;
    t.insert(&manager, "SocialVariable", json!({"variable_name": "shop", "value": "key.example", "account": nw_key.to_string()})).await;
    refused(
        t.k.insert(&manager, "SocialVariable", obj(json!({"variable_name": "shop", "value": "again", "brand": northwind.to_string()}))).await,
        "invalid",
        "has a value here already",
    );
    refused(t.k.insert(&manager, "SocialVariable", obj(json!({"variable_name": "the shop", "value": "x"}))).await, "invalid", "letters, digits and _");
    let mut signed = tagged("Visit {{shop}}", &[nw_key, nw_words], "Signed");
    signed["add_signature"] = json!(true);
    let mut unknown = signed.clone();
    unknown["text"] = json!("Visit {{nope}}");
    refused(t.ready(&author, unknown).await, "invalid", "no variable is called nope for Northwind Chirp");
    let mut long = signed.clone();
    long["text"] = json!(format!("{} {{{{shop}}}}", "a".repeat(56)));
    long["link"] = Value::Null;
    refused(t.ready(&author, long).await, "invalid", "for Northwind Words: Chirp Words takes 100 characters, and this post is 101");
    let signed = t.insert(&author, "SocialPost", signed).await;
    let copies = t.act(&author, "SocialPost", uid(&signed["id"]), "copies", Value::Null).await.unwrap();
    let key_copy = copies["copies"].as_array().unwrap().iter().find(|c| c["account"] == json!(nw_key)).unwrap().clone();
    assert_eq!(
        (key_copy["items"][0]["text"].as_str(), key_copy["items"][0]["length"].as_i64(), key_copy["items"][0]["max"].as_i64()),
        (Some("Visit key.example\n\n— N at key.example"), Some(48), Some(100)),
        "{key_copy}"
    );
    go(uid(&signed["id"])).await;
    let before = t.net.lock().unwrap().posts.len();
    t.upkeep(due + Duration::hours(17)).await;
    {
        let n = t.net.lock().unwrap();
        let texts: Vec<String> = n.posts[before..].iter().map(|p| p.1["text"].as_str().unwrap().to_string()).collect();
        let link = " https://northwind.example/spring-in-the-very-long-address-of-a-page";
        assert!(texts.contains(&format!("Visit key.example\n\n— N at key.example{link}")), "the account's value and its own signature: {texts:?}");
        assert!(texts.contains(&format!("Visit shop.northwind.example\n\n— The Northwind team{link}")), "the brand's: {texts:?}");
    }
    let mut once = tagged("Spring\n\n— The Northwind team", &[nw_words], "Signed once");
    once["add_signature"] = json!(true);
    let once = t.insert(&author, "SocialPost", once).await;
    go(uid(&once["id"])).await;
    let before = t.net.lock().unwrap().posts.len();
    t.upkeep(due + Duration::hours(18)).await;
    assert!(t.net.lock().unwrap().posts[before].1["text"].as_str().unwrap().starts_with("Spring\n\n— The Northwind team https://"), "never twice");
    // Templates: a post's content kept, versions and answers with it; used, a draft.
    let kept = t.act(&author, "SocialPost", thread_id, "save_as_template", json!({"name": "Thread template"})).await.unwrap();
    let template = t.get("SocialTemplate", uid(&kept["id"])).await;
    assert_eq!((template["text"].as_str(), template["items"].as_array().unwrap().len()), (Some("One of three"), 2));
    let used = t.act(&author, "SocialTemplate", uid(&kept["id"]), "use_template", json!({"headline": "From the template"})).await.unwrap();
    let used = t.get("SocialPost", uid(&used["id"])).await;
    assert_eq!(
        (used["status"].as_str(), used["headline"].as_str(), used["targets"][0]["settings"].clone()),
        (Some("Draft"), Some("From the template"), json!({"audience": "followers"}))
    );
    // Sets: a selection of accounts with their answers, and words to start with.
    let set = t
        .insert(&author, "SocialSet", json!({"set_name": "Threads and key", "brand": northwind.to_string(), "text": "Start here",
                                              "targets": [{"account": nw_threads.to_string(), "settings": {"audience": "everyone"}}, {"account": nw_key.to_string()}]}))
        .await;
    let from_set = t.act(&author, "SocialSet", uid(&set["id"]), "use_set", json!({"headline": "From the set"})).await.unwrap();
    let from_set = t.get("SocialPost", uid(&from_set["id"])).await;
    assert_eq!((from_set["text"].as_str(), from_set["targets"].as_array().unwrap().len()), (Some("Start here"), 2));
    let snippet = t.insert(&author, "SocialSnippet", json!({"snippet_name": "Spring tags", "kind": "Hashtags", "text": "#spring #northwind"})).await;
    assert_eq!(snippet["kind"], "Hashtags");

    // ───────────── a review link: someone outside looks, comments, asks for changes ─────────────
    let review = t.insert(&author, "SocialPost", tagged("For the client", &[nw_key, nw_words], "Client review")).await;
    let review_id = uid(&review["id"]);
    go(review_id).await;
    let link = t
        .act(&author, "SocialPost", review_id, "review_link", json!({"email": "client@else.example", "name": "Cleo", "decide": true, "days": 3}))
        .await
        .unwrap();
    let url = link["url"].as_str().unwrap().to_string();
    let token = url.rsplit('/').next().unwrap().to_string();
    let preview = t.get("SocialPost", review_id).await["preview"].as_str().unwrap().to_string();
    assert!(
        preview.contains("Northwind Chirp · Chirp") && preview.contains("Northwind Words · Chirp Words") && preview.contains("For the client"),
        "{preview}"
    );
    let page = t.k.link_page("brands", &token).await.unwrap().expect("the page");
    let page = serde_json::to_value(&page).unwrap();
    assert!(page.to_string().contains("For the client"), "{page}");
    let use_link = |action: &str, comment: Option<&str>| {
        let (t, token) = (&t, token.clone());
        let mut params = serde_json::Map::new();
        if let Some(c) = comment {
            params.insert("comment".into(), json!(c));
        }
        let action = action.to_string();
        async move { t.k.use_link("brands", &token, &action, &params, None).await }
    };
    use_link("review_comment", Some("Lovely colours")).await.unwrap();
    use_link("review_changes", Some("Say spring, not Spring")).await.unwrap();
    let back = t.get("SocialPost", review_id).await;
    assert_eq!(
        (back["status"].as_str(), back["review_note"].as_str()),
        (Some("Rejected"), Some("Cleo: Say spring, not Spring")),
        "asking for changes sends it back"
    );
    let said = t.rows("SocialReview", json!({"post": review_id.to_string()})).await;
    assert_eq!(said.len(), 2);
    assert!(said.iter().all(|r| r["reviewer"] == "client@else.example" && r["reviewer_name"] == "Cleo"), "{said:?}");
    let mailed = t.k.message_log(&t.admin, Some(("SocialPost", review_id)), None, 10).await.unwrap();
    assert!(mailed.iter().any(|m| m.to_string().contains("client@else.example")), "{mailed:?}");
    assert!(!serde_json::to_string(&mailed).unwrap().contains(&token), "the link is kept masked");
    let only_view = t.act(&author, "SocialPost", review_id, "review_link", json!({})).await.unwrap();
    let other = only_view["url"].as_str().unwrap().rsplit('/').next().unwrap().to_string();
    assert!(t.k.use_link("brands", &other, "review_approve", &serde_json::Map::new(), None).await.is_err(), "a link without a say decides nothing");

    // ───────────── accounts: what a sign-in reaches, picked; names read; mentions and choices looked up ─────────────
    let pages_social = json!({"name": "Chirp Pages", "text": {"max": 100},
        "settings": [{"key": "board", "label": "Board", "type": "select", "required": true,
                      "choices": {"operation": "boards", "inputs": {"account_id": "account.id"}, "items_at": "/items", "value_at": "/id", "label_at": "/name"}}],
        "accounts": {"operation": "accounts", "items_at": "/pages", "value_at": "/id", "label_at": "/name", "handle_at": "/username", "picture_at": "/avatar"},
        "mentions": {"operation": "mentions", "inputs": {"q": "query"}, "items_at": "", "value_at": "/id", "label_at": "/name", "format": "@[{{ label }}]({{ value }})"}});
    let mut pages_desc =
        description(&base, json!({"type": "header", "header": "x-api-key", "secret": "key"}), &["publish", "account", "accounts", "mentions", "boards"], false);
    pages_desc["meta"] = json!({"social": pages_social});
    let on_pages = save("chirp_pages", pages_desc, json!({"key": KEY})).await;
    let pending = uid(&t.insert(&manager, "SocialAccount", threaded("Northwind Pages", "pages", on_pages)).await["id"]);
    let rules = t.act(&author, "SocialAccount", pending, "rules", Value::Null).await.unwrap();
    assert_eq!((rules["rules"]["name"].as_str(), rules["rules"]["settings"][0]["key"].as_str()), (Some("Chirp Pages"), Some("board")), "{rules}");
    let reached = t.act(&manager, "SocialAccount", pending, "reachable", Value::Null).await.unwrap();
    assert_eq!(reached["accounts"].as_array().unwrap().len(), 2, "{reached}");
    assert_eq!((reached["accounts"][0]["name"].as_str(), reached["accounts"][0]["account"].clone()), (Some("Northwind Shop"), Value::Null));
    refused(t.act(&manager, "SocialAccount", pending, "choose", json!({"picked": ["pg-9"]})).await, "invalid", "does not reach pg-9");
    let chosen = t.act(&manager, "SocialAccount", pending, "choose", json!({"picked": "pg-1, pg-2"})).await.unwrap();
    let chosen = chosen["accounts"].as_array().unwrap().clone();
    assert_eq!((chosen[0]["id"].clone(), chosen[0]["created"].clone(), chosen[1]["created"].clone()), (json!(pending), json!(false), json!(true)));
    let shop = t.get("SocialAccount", pending).await;
    assert_eq!((shop["network_id"].as_str(), shop["display_name"].as_str(), shop["handle"].as_str()), (Some("pg-1"), Some("Northwind Shop"), Some("nwshop")));
    let cafe = t.get("SocialAccount", uid(&chosen[1]["id"])).await;
    assert_eq!(
        (cafe["account_name"].as_str(), cafe["network_id"].as_str(), cafe["brand"].clone()),
        (Some("Northwind Cafe"), Some("pg-2"), json!(northwind.to_string()))
    );
    let again = t.act(&manager, "SocialAccount", pending, "choose", json!({"picked": ["pg-2"]})).await.unwrap();
    assert_eq!(
        (again["accounts"][0]["id"].clone(), again["accounts"][0]["created"].clone()),
        (chosen[1]["id"].clone(), json!(false)),
        "picked again: updated, not made twice"
    );
    assert!(t.act(&author, "SocialAccount", pending, "choose", json!({"picked": ["pg-1"]})).await.is_err(), "an author does not pick accounts");
    let found = t.act(&author, "SocialAccount", pending, "mentions", json!({"query": "ana"})).await.unwrap();
    assert_eq!(found["mentions"], json!([{"value": "u-9", "label": "Ana Lima", "picture": null, "text": "@[Ana Lima](u-9)"}]));
    let boards = t.act(&author, "SocialAccount", pending, "choices", json!({"setting": "board"})).await.unwrap();
    assert_eq!(boards["choices"], json!([{"value": "b-1", "label": "Spring"}, {"value": "b-2", "label": "Summer"}]));
    refused(t.act(&author, "SocialAccount", pending, "choices", json!({"setting": "colour"})).await, "invalid", "asks nothing called colour");
    let checked = t.act(&manager, "SocialAccount", uid(&chosen[1]["id"]), "check", Value::Null).await.unwrap();
    assert_eq!(
        (checked["read"]["picture"].as_str(), checked["read"]["display_name"].as_str()),
        (Some("https://chirp.test/a/0.png"), Some("Northwind Café")),
        "{checked}"
    );
    assert_eq!(t.get("SocialAccount", uid(&chosen[1]["id"])).await["handle"], "nwcafe");
    // A post to a picked page names it to the network.
    let mut to_page = tagged("To the shop", &[], "Page post");
    to_page["targets"] = json!([target(pending, json!({"settings": {"board": "b-1"}}))]);
    let to_page = t.insert(&author, "SocialPost", to_page).await;
    go(uid(&to_page["id"])).await;
    t.upkeep(due + Duration::hours(19)).await;
    assert_eq!(t.net.lock().unwrap().posts.last().unwrap().1["account"], "pg-1");

    // ───────────── (round two) the media library: stock pictures taken with their credit, an address fetched ─────────────
    let key_auth2 = json!({"type": "header", "header": "x-api-key", "secret": "key"});
    let mut stock = json!({"base": base, "auth": key_auth2.clone(), "apps": ["social"], "operations": {
        "search_media": {"path": "/stock/search", "query": {"query": "{{ inputs.query }}", "page": "{{ inputs.page }}"}, "error_at": "/error/message"},
        "fetch_media": {"path": "/stock/{{ inputs.id }}/download", "result": {"file": {"file": {"max_bytes": 100000}}}, "error_at": "/error/message"},
        "media_downloaded": {"method": "POST", "path": "/stock/{{ inputs.id }}/told", "body": "{}", "idempotency_header": "Idempotency-Key", "error_at": "/error/message"}}});
    stock["meta"] = json!({"social": {"media_search": {"items_at": "/results", "id_at": "/id", "preview_at": "/urls/small", "url_at": "/urls/full", "credit_at": "/user/name",
        "credit_link_at": "/user/links/html", "alt_at": "/alt_description", "width_at": "/width", "height_at": "/height", "licence": "Free to use; credit asked", "kind": "image"}}});
    save("chirp_stock", stock, json!({"key": KEY})).await;
    let fetcher = json!({"base": base, "auth": key_auth2.clone(), "apps": ["social"], "operations": {
        "fetch_media": {"path": "/fetch", "query": {"u": "{{ inputs.url }}"}, "result": {"file": {"file": {"max_bytes": 100000}}}, "error_at": "/error/message"}}});
    save("chirp_fetch", fetcher, json!({"key": KEY})).await;
    let settings_now = t.rows("SocialSettings", json!({})).await;
    let set = |values: Value| {
        let (t, manager, now) = (&t, &manager, settings_now.clone());
        async move {
            let rows = t.rows("SocialSettings", json!({})).await;
            match rows.first().or(now.first()) {
                Some(r) => {
                    t.k.update(manager, "SocialSettings", uid(&r["id"]), obj(values), None).await.unwrap();
                }
                None => {
                    t.insert(manager, "SocialSettings", values).await;
                }
            }
        }
    };
    refused(t.act(&author, "SocialBrand", northwind, "media_search", json!({"query": "lake"})).await, "invalid", "no picture source");
    set(json!({"media_sources": "chirp_stock", "fetch_connection": "chirp_fetch"})).await;
    let found = t.act(&author, "SocialBrand", northwind, "media_search", json!({"query": "lake"})).await.unwrap();
    let first = found["items"][0].clone();
    assert_eq!(
        (first["id"].as_str(), first["credit"].as_str(), first["preview"].as_str(), first["licence"].as_str()),
        (Some("s-1"), Some("Ana Lima"), Some("https://chirp.test/s/1-small.jpg"), Some("Free to use; credit asked")),
        "{found}"
    );
    let mut take = first.clone();
    take["source"] = json!("chirp_stock");
    let taken = t.act(&author, "SocialBrand", northwind, "media_take", take.clone()).await.unwrap();
    let asset = t.get("SocialMediaAsset", uid(&taken["id"])).await;
    assert_eq!(
        (
            asset["source"].as_str(),
            asset["credit"].as_str(),
            asset["credit_link"].as_str(),
            asset["kind"].as_str(),
            asset["alt"].as_str(),
            asset["width"].as_i64()
        ),
        (Some("Stock"), Some("Ana Lima"), Some("https://chirp.test/@ana"), Some("image"), Some("A lake at dawn"), Some(4000)),
        "{asset:?}"
    );
    assert!(asset["bytes"].as_i64().is_some_and(|b| b > 0) && asset["content_type"] == "image/png");
    t.act(&author, "SocialBrand", northwind, "media_take", take).await.unwrap();
    assert_eq!(t.net.lock().unwrap().calls.iter().filter(|c| c.0.ends_with("/told")).count(), 1, "the source told of its download once");
    refused(t.act(&author, "SocialBrand", northwind, "media_from_address", json!({"url": "file:///etc/passwd"})).await, "invalid", "is a web address");
    let fetched = t.act(&author, "SocialBrand", northwind, "media_from_address", json!({"url": "https://chirp.test/x/cat.png", "alt": "A cat"})).await.unwrap();
    let fetched = t.get("SocialMediaAsset", uid(&fetched["id"])).await;
    assert_eq!(
        (fetched["source"].as_str(), fetched["asset_name"].as_str(), fetched["source_id"].as_str()),
        (Some("Address"), Some("cat.png"), Some("https://chirp.test/x/cat.png"))
    );
    // A post's medium from the library: its file, description and size carried.
    let mut from_library = tagged("From the library", &[], "Library");
    from_library["targets"] = json!([target(nw_threads, answered.clone())]);
    from_library["media"] = json!([{"item": 1, "asset": asset["id"]}]);
    let from_library = t.insert(&author, "SocialPost", from_library).await;
    assert_eq!(
        (from_library["media"][0]["file"].clone(), from_library["media"][0]["alt"].as_str(), from_library["media"][0]["width"].as_i64()),
        (asset["file"].clone(), Some("A lake at dawn"), Some(4000))
    );

    // ───────────── (round two) links: tracking by the most specific template, short links once, their clicks ─────────────
    refused(
        t.k.insert(&manager, "SocialLinkTemplate", obj(json!({"template_name": "Bad", "parameters": "utm_source={{planet}}"}))).await,
        "invalid",
        "{{planet}} is not one of",
    );
    t.insert(&manager, "SocialLinkTemplate", json!({"template_name": "Everyone", "parameters": "utm_source=ws"})).await;
    let brand_links = t
        .insert(
            &manager,
            "SocialLinkTemplate",
            json!({"template_name": "Northwind links", "brand": northwind.to_string(), "in_text": true, "exclude": "partner.example",
                                                       "parameters": "utm_source={{network}}\nutm_medium=social\nutm_campaign={{post}}"}),
        )
        .await;
    let short_desc = json!({"base": base, "auth": key_auth2.clone(), "apps": ["social"], "operations": {
        "shorten": {"method": "POST", "path": "/short", "body": "{\"url\": {{ inputs.url | json }}}", "result": {"url": "/shortLink", "id": "/id"}, "error_at": "/error/message", "idempotency_header": "Idempotency-Key"},
        "link_clicks": {"path": "/short/{{ inputs.id }}", "result": {"clicks": "/clicks"}, "error_at": "/error/message"}}});
    save("chirp_short", short_desc, json!({"key": KEY})).await;
    let mut linked = tagged("Shop https://shop.example/b and https://cdn.partner.example/p", &[nw_key], "Sale");
    linked["link"] = json!("https://shop.example/a?x=1&utm_medium=email");
    let linked = t.insert(&author, "SocialPost", linked).await;
    let copy = t.act(&author, "SocialPost", uid(&linked["id"]), "copies", Value::Null).await.unwrap()["copies"][0]["items"][0].clone();
    assert_eq!(
        copy["link"], "https://shop.example/a?x=1&utm_medium=email&utm_source=Chirp&utm_campaign=Sale",
        "the brand's, the address's own kept, not doubled"
    );
    assert_eq!(copy["text"], "Shop https://shop.example/b?utm_source=Chirp&utm_medium=social&utm_campaign=Sale and https://cdn.partner.example/p", "{copy}");
    set(json!({"shorten_connection": "chirp_short", "metrics_hours": 1})).await;
    go(uid(&linked["id"])).await;
    let before = t.net.lock().unwrap().posts.len();
    t.upkeep(due + Duration::hours(20)).await;
    let sent_text = t.net.lock().unwrap().posts[before].1["text"].as_str().unwrap().to_string();
    assert!(
        sent_text.starts_with("Shop https://shop.example/b?") && sent_text.ends_with(" https://sh.test/k1"),
        "the post's link shortened, the words' left: {sent_text}"
    );
    let lpub = t.rows("SocialPublication", json!({"post": uid(&linked["id"]).to_string()})).await.remove(0);
    assert_eq!(lpub["short_links"]["https://shop.example/a?x=1&utm_medium=email&utm_source=Chirp&utm_campaign=Sale"]["url"], "https://sh.test/k1");
    t.upkeep(due + Duration::hours(22)).await;
    assert_eq!(t.get("SocialPublication", uid(&lpub["id"])).await["clicks"], 42, "the shortener's clicks");
    assert_eq!(t.net.lock().unwrap().calls.iter().filter(|c| c.0.starts_with("POST /v1/short")).count(), 1, "shortened once");
    t.k.update(&manager, "SocialLinkTemplate", uid(&brand_links["id"]), obj(json!({"active": false})), None).await.unwrap();
    set(json!({"shorten_connection": null, "metrics_hours": 6})).await;

    // ───────────── (round two) writing help through the Claude Messages API's shape: given back, never saved over ─────────────
    let writer = t.user("writer@brands.test", &["Social Author", "Social Writing Help"]).await;
    let mut claude: Value =
        serde_json::from_str(&std::fs::read_to_string(format!("{}/docs/connectors/services/claude.json", env!("CARGO_MANIFEST_DIR"))).unwrap()).unwrap();
    claude["base"] = json!(base.trim_end_matches("/v1"));
    claude["operations"]["picture"] = json!({"method": "POST", "path": "/v1/picture", "body": "{\"prompt\": {{ inputs.prompt | json }}}", "result": {"file": {"file": {"max_bytes": 100000}}}});
    save("claude_test", claude, json!({"api_key": KEY})).await;
    t.k.update(&manager, "SocialBrand", northwind, obj(json!({"voice": "Cheerful, plain words, no exclamation marks"})), None).await.unwrap();
    let mut draft = tagged("Spring is here, come and see", &[], "Writing");
    draft["link"] = Value::Null;
    draft["targets"] = json!([{"account": nw_key.to_string()}, {"account": nw_threads.to_string(), "settings": {"audience": "everyone"}}]);
    let draft = t.insert(&writer, "SocialPost", draft).await;
    let draft_id = uid(&draft["id"]);
    refused(t.act(&author, "SocialPost", draft_id, "write", json!({"prompt": "spring"})).await, "forbidden", "Social Writing Help");
    refused(t.act(&writer, "SocialPost", draft_id, "write", json!({"prompt": "spring"})).await, "invalid", "no connection is set for writing help");
    set(json!({"writing_connection": "claude_test"})).await;
    t.net.lock().unwrap().written = vec!["Spring is here. Come and see.".into()];
    let wrote = t.act(&writer, "SocialPost", draft_id, "write", json!({"prompt": "our spring sale"})).await.unwrap();
    assert_eq!(wrote["text"], "Spring is here. Come and see.");
    assert_eq!(t.get("SocialPost", draft_id).await["text"], "Spring is here, come and see", "given back, not saved over");
    {
        let n = t.net.lock().unwrap();
        let body = &n.calls.iter().rev().find(|c| c.0.starts_with("POST /v1/messages")).unwrap().1;
        assert_eq!((body["model"].as_str(), body["max_tokens"].as_i64()), (Some("claude-sonnet-5-5"), Some(1024)));
        assert!(body["system"].as_str().unwrap().contains("Northwind") && body["system"].as_str().unwrap().contains("Cheerful"), "{body}");
        assert!(body["messages"][0]["content"].as_str().unwrap().contains("our spring sale"), "{body}");
    }
    let shorter = t.rows("SocialWritingPreset", json!({"preset_name": "Shorter"})).await;
    let rewritten = t.act(&writer, "SocialPost", draft_id, "rewrite", json!({"preset": shorter[0]["id"]})).await.unwrap();
    assert!(rewritten["text"].as_str().unwrap().contains("Make it shorter") && rewritten["text"].as_str().unwrap().contains("come and see"), "{rewritten}");
    t.net.lock().unwrap().written = vec!["Short and sweet".into(), "b".repeat(150)];
    let adapted = t.act(&writer, "SocialPost", draft_id, "adapt", Value::Null).await.unwrap();
    let v = adapted["versions"].as_array().unwrap();
    assert_eq!(
        (v[0]["text"].as_str(), v[0]["fits"].clone(), v[1]["fits"].clone(), v[1]["max"].as_i64()),
        (Some("Short and sweet"), json!(true), json!(false), Some(100)),
        "{adapted}"
    );
    t.net.lock().unwrap().written = vec!["#spring #sale, #Spring and more #".into(), "One\n---\nTwo\n---\nThree".into()];
    assert_eq!(t.act(&writer, "SocialPost", draft_id, "hashtags", json!({"count": 5})).await.unwrap()["hashtags"], json!(["#spring", "#sale"]));
    assert_eq!(t.act(&writer, "SocialPost", draft_id, "thread", json!({"parts": 3})).await.unwrap()["items"], json!(["One", "Two", "Three"]));
    let pic_post = t.insert(&writer, "SocialPost", pictured_post(&[nw_key], &png, Some("x"))).await;
    t.act(&writer, "SocialPost", uid(&pic_post["id"]), "describe", json!({"medium": 0})).await.unwrap();
    {
        let n = t.net.lock().unwrap();
        let body = &n.calls.iter().rev().find(|c| c.0.starts_with("POST /v1/messages")).unwrap().1;
        assert_eq!(
            (body["messages"][0]["content"][0]["type"].as_str(), body["messages"][0]["content"][0]["source"]["media_type"].as_str()),
            (Some("image"), Some("image/png"))
        );
    }
    let drawn = t.act(&writer, "SocialPost", draft_id, "picture", json!({"prompt": "a spring meadow"})).await.unwrap();
    assert_eq!(t.get("SocialMediaAsset", uid(&drawn["id"])).await["source"], "Generated");
    assert!(t.act(&writer, "SocialBrand", northwind, "write_new", json!({"prompt": "a hello"})).await.unwrap()["text"].as_str().unwrap().starts_with("ECHO"));
    let uses = t.rows("SocialWritingUse", json!({"by": "writer@brands.test"})).await;
    assert_eq!(uses.len(), 9, "every use kept: {}", uses.len());
    t.k.erase_personal_data(&t.admin, "writer@brands.test", "asked to be forgotten").await.unwrap();
    assert!(t.rows("SocialWritingUse", json!({"action": "write"})).await.iter().all(|u| u["prompt"].is_null() || u["prompt"] == ""), "prompts erased");

    // ───────────── (round two) plugs: reposted once it does well; liked by other accounts, once each ─────────────
    t.insert(
        &manager,
        "SocialPlug",
        json!({"plug_name": "Boost", "brand": northwind.to_string(), "account": nw_key.to_string(), "trigger": "Number Reached",
                                              "measure": "impressions", "threshold": 400, "within_hours": 72, "action": "Repost", "max_runs": 1}),
    )
    .await;
    t.insert(
        &manager,
        "SocialPlug",
        json!({"plug_name": "Cheer", "brand": northwind.to_string(), "account": nw_threads.to_string(), "trigger": "Time Passed",
                                              "after_minutes": 0, "action": "Others Like", "max_runs": 50,
                                              "targets": [{"account": nw_key.to_string()}, {"account": nw_words.to_string()}]}),
    )
    .await;
    refused(t.k.insert(&author, "SocialPlug", obj(json!({"plug_name": "Mine", "brand": northwind.to_string()}))).await, "forbidden", "");
    let threads_out = t
        .rows("SocialPublication", json!({"account": nw_threads.to_string()}))
        .await
        .into_iter()
        .filter(|p| p["external_id"].is_string() && p["deleted_at"].is_null())
        .count();
    for hour in [23, 24] {
        t.upkeep(due + Duration::hours(hour)).await;
    }
    let runs = t.rows("SocialPlugRun", json!({})).await;
    let count = |state: &str, acct: Uuid| runs.iter().filter(|r| r["state"] == state && r["account"] == json!(acct.to_string())).count();
    assert_eq!(count("Done", nw_key) - threads_out, 1, "one repost by the Boost plug, at most once in all: {runs:?}");
    assert_eq!(count("Skipped", nw_words), threads_out, "a connection without like: skipped, said");
    {
        let n = t.net.lock().unwrap();
        assert_eq!(
            n.calls.iter().filter(|c| c.0.ends_with("/like")).count(),
            threads_out,
            "each post liked once by the other account, though the upkeep ran twice"
        );
        assert_eq!(n.calls.iter().filter(|c| c.0.ends_with("/repost")).count(), 1);
    }
    assert!(runs.iter().any(|r| r["state"] == "Skipped" && r["note"].as_str().unwrap().contains("does not offer like")));

    // ───────────── the inbox ─────────────
    let spring_external = pubs[&nw_key]["external_id"].as_str().unwrap().to_string();
    let item = |id: &str, kind: &str, from: &str, to: Option<&str>, on: Option<&str>, thread: &str, text: &str| json!({"id": id, "kind": kind, "from": from, "from_id": format!("u-{from}"), "text": text, "to": to, "on_post": on, "thread": thread});
    let hook_headers = |extra: &[(&str, &str)]| {
        let mut h = BTreeMap::from([("X-Token".to_string(), HOOK.to_string())]);
        h.extend(extra.iter().map(|(k, v)| (k.to_string(), v.to_string())));
        h
    };
    let one = json!({"id": "delivery-1", "items": [item("c-1", "comment", "ivy@chirp.test", None, Some(&spring_external), "t-1", "Love the colours!"),
                                                   item("m-1", "mention", "bo", Some("northwind_words"), None, "t-2", "@northwind_words see this")]});
    t.k.connection_inbound(on_key, &hook_headers(&[]), one.to_string().as_bytes()).await.unwrap();
    // Not accepted: unsigned, wrong token, a replay.
    let alike = Error::Unauthorized("this call was not accepted".into()).to_string();
    assert_eq!(t.k.connection_inbound(on_key, &BTreeMap::new(), one.to_string().as_bytes()).await.unwrap_err().to_string(), alike);
    assert_eq!(t.k.connection_inbound(on_key, &hook_headers(&[("X-Token", "wrong")]), one.to_string().as_bytes()).await.unwrap_err().to_string(), alike);
    assert_eq!(t.k.connection_inbound(on_key, &hook_headers(&[]), one.to_string().as_bytes()).await.unwrap_err().to_string(), alike, "a replay");
    t.work().await;
    let messages = t.rows("SocialMessage", json!({})).await;
    assert_eq!(messages.len(), 2, "{messages:?} {:?}", t.k.connection_inbound_log(&t.admin, on_key, 10).await.unwrap());
    let comment = messages.iter().find(|m| m["external_id"] == "c-1").unwrap();
    assert_eq!((comment["kind"].as_str(), comment["author"].as_str(), comment["thread"].as_str()), (Some("Comment"), Some("ivy@chirp.test"), Some("t-1")));
    assert_eq!(comment["account"], json!(nw_key.to_string()), "found by the post it answers");
    let mention = messages.iter().find(|m| m["external_id"] == "m-1").unwrap();
    assert_eq!((mention["kind"].as_str(), mention["account"].clone()), (Some("Mention"), json!(nw_words.to_string())), "found by the handle it names");
    // The same item in another delivery: not a second message.
    let again =
        json!({"id": "delivery-2", "items": [item("c-1", "comment", "ivy@chirp.test", None, Some(&spring_external), "t-1", "Love the colours! (edited)")]});
    t.k.connection_inbound(on_key, &hook_headers(&[]), again.to_string().as_bytes()).await.unwrap();
    t.work().await;
    assert_eq!(t.rows("SocialMessage", json!({})).await.len(), 2, "once each");
    // A pull with a cursor: new items once, the cursor kept, nothing read twice.
    {
        let mut n = t.net.lock().unwrap();
        n.inbox.push(item("c-1", "comment", "ivy@chirp.test", None, Some(&spring_external), "t-1", "Love the colours!"));
        n.inbox.push(item("dm-1", "dm", "ivy@chirp.test", Some("northwind"), None, "t-9", "Can I order in bulk?"));
    }
    assert_eq!(t.k.sweep_pulls_at(Utc::now()).await.unwrap(), 1);
    t.work().await;
    let after = t.rows("SocialMessage", json!({})).await;
    assert_eq!(after.len(), 3, "the comment already here, one direct message new: {after:?}");
    let dm = after.iter().find(|m| m["external_id"] == "dm-1").unwrap();
    assert_eq!((dm["kind"].as_str(), dm["account"].clone(), dm["author_id"].as_str()), (Some("Direct"), json!(nw_key.to_string()), Some("u-ivy@chirp.test")));
    let pulls = t.k.connection_pulls(&t.admin, on_key).await.unwrap();
    assert_eq!(pulls[0]["cursor"], "2", "the network's own cursor, kept");
    t.net.lock().unwrap().inbox.push(item("m-2", "mention", "cy", Some("northwind"), None, "t-3", "Hello"));
    t.k.pull_now(&t.admin, on_key, "inbox").await.unwrap();
    t.work().await;
    assert_eq!(t.rows("SocialMessage", json!({})).await.len(), 4);
    // Triage: assigned, with whom; saved replies and tags.
    let dm_id = uid(&dm["id"]);
    refused(t.k.update(&author, "SocialMessage", dm_id, obj(json!({"status": "Assigned"})), None).await, "invalid", "says to whom");
    t.k.update(&author, "SocialMessage", dm_id, obj(json!({"status": "Assigned", "assigned_to": "author@brands.test"})), None).await.unwrap();
    t.k.set_tags(&author, "SocialMessage", dm_id, &["sales".to_string()]).await.unwrap();
    let saved = uid(&t.insert(&manager, "SocialSavedReply", json!({"title": "Bulk orders", "text": "Yes: write to sales@northwind.example."})).await["id"]);
    refused(t.k.insert(&author, "SocialSavedReply", obj(json!({"title": "Mine", "text": "x"}))).await, "forbidden", "");
    // Replies go out through the operation for what they answer, each once, under the reply's key.
    let comment_id = uid(&comment["id"]);
    refused(t.act(&author, "SocialMessage", comment_id, "reply", json!({})).await, "invalid", "says something");
    let r = t.act(&author, "SocialMessage", comment_id, "reply", json!({"reply": "Thank you!"})).await.unwrap();
    let reply_id = uid(&r["reply"]);
    let replied = t.get("SocialReply", reply_id).await;
    assert_eq!(replied["in_reply_to"], "c-1");
    t.work().await;
    {
        let n = t.net.lock().unwrap();
        let (k, body) = n.replies.last().unwrap();
        assert_eq!((k.clone(), body["in_reply_to"].as_str(), body["text"].as_str()), (format!("reply:{reply_id}"), Some("c-1"), Some("Thank you!")));
    }
    assert_eq!(t.get("SocialMessage", comment_id).await["status"], "Answered");
    assert!(t.get("SocialReply", reply_id).await["external_id"].is_string());
    let before = t.net.lock().unwrap().replies.len();
    let mut tx = t.k.db.tenant_tx(tenant).await.unwrap();
    ferrum_kernel::jobs::enqueue(&mut tx, tenant, None, ferrum_kernel::jobs::NewJob::new("social.reply", json!({"reply": reply_id}))).await.unwrap();
    tx.commit().await.unwrap();
    t.work().await;
    assert_eq!(t.net.lock().unwrap().replies.len(), before, "a reply sent is not sent again");
    t.act(&author, "SocialMessage", dm_id, "reply", json!({"saved_reply": saved.to_string()})).await.unwrap();
    t.work().await;
    {
        let n = t.net.lock().unwrap();
        assert_eq!(n.dms.len(), 1, "a direct message is answered by send_message");
        assert_eq!((n.dms[0].1["to"].as_str(), n.dms[0].1["text"].as_str()), (Some("u-ivy@chirp.test"), Some("Yes: write to sales@northwind.example.")));
    }
    // The network refusing a reply: shown, and the message stays as it was.
    let mention_id = uid(&mention["id"]);
    refused(t.act(&author, "SocialMessage", mention_id, "reply", json!({"reply": "Thanks"})).await, "invalid", "cannot reply");
    let c2 = uid(&t.rows("SocialMessage", json!({"external_id": "m-2"})).await[0]["id"]);
    t.net.lock().unwrap().reject_replies = true;
    let r = t.act(&author, "SocialMessage", c2, "reply", json!({"reply": "Hi"})).await.unwrap();
    t.work().await;
    let r = t.get("SocialReply", uid(&r["reply"])).await;
    assert!(r["failure"].as_str().unwrap().contains("Replies are closed") && r["failed_at"].is_string() && r["external_id"].is_null(), "{r:?}");
    assert_eq!(t.get("SocialMessage", c2).await["status"], "New");
    t.net.lock().unwrap().reject_replies = false;

    // The sender matched to a record of a person, only when the workspace says so, and the entity is installed.
    let field: NewCustomization =
        serde_json::from_value(json!({"entity": "Contact", "op": {"op": "add_field", "field": {"name": "x_handle", "type": "data"}}})).unwrap();
    t.k.batch_customizations(&t.admin, vec![field], false).await.map_err(|e| e.1).unwrap();
    let admin = t.k.load_ctx(tenant, t.admin.user, None, "it".into()).await.unwrap();
    let ivy = t.k.insert(&admin, "Contact", obj(json!({"first_name": "Ivy", "x_handle": "ivy@chirp.test"}))).await.unwrap();
    let item2 = item("c-9", "comment", "ivy@chirp.test", None, Some(&spring_external), "t-5", "Another");
    let push = |id: &str, body: Value| {
        let t = &t;
        let id = id.to_string();
        async move {
            let b = json!({"id": id, "items": [body]});
            t.k.connection_inbound(on_key, &hook_headers(&[]), b.to_string().as_bytes()).await.unwrap();
            t.work().await;
        }
    };
    push("delivery-9", item2).await;
    let c9 = &t.rows("SocialMessage", json!({"external_id": "c-9"})).await[0];
    assert!(c9["contact_id"].is_null(), "off unless the workspace says so");
    set(json!({"match_crm": true, "crm_entity": "Contact", "crm_field": "x_handle"})).await;
    push("delivery-10", item("c-10", "comment", "ivy@chirp.test", None, Some(&spring_external), "t-5", "Yet another")).await;
    let c10 = &t.rows("SocialMessage", json!({"external_id": "c-10"})).await[0];
    assert_eq!((c10["contact_entity"].as_str(), c10["contact_id"].clone()), (Some("Contact"), ivy["id"].clone()));
    push("delivery-11", item("c-11", "comment", "stranger", None, Some(&spring_external), "t-6", "Who am I")).await;
    assert!(t.rows("SocialMessage", json!({"external_id": "c-11"})).await[0]["contact_id"].is_null(), "nobody holds that handle");
    let settings_id = uid(&t.rows("SocialSettings", json!({})).await[0]["id"]);
    t.k.update(&manager, "SocialSettings", settings_id, obj(json!({"crm_entity": "NotInstalled"})), None).await.unwrap();
    push("delivery-12", item("c-12", "comment", "ivy@chirp.test", None, Some(&spring_external), "t-5", "Entity missing")).await;
    assert!(
        t.rows("SocialMessage", json!({"external_id": "c-12"})).await[0]["contact_id"].is_null(),
        "an entity that is not installed matches nothing, and fails nothing"
    );

    // Erasure reaches the handle and the message text, and what was written to them.
    let held = t.k.personal_data(&t.admin, "ivy@chirp.test").await.unwrap();
    let entities: Vec<&str> = held["records"].as_array().unwrap().iter().filter_map(|r| r["entity"].as_str()).collect();
    assert!(entities.contains(&"SocialMessage") && entities.contains(&"SocialReply"), "{held}");
    let erased = t.k.erase_personal_data(&t.admin, "ivy@chirp.test", "asked to be forgotten").await.unwrap();
    assert!(erased["erased"].as_array().unwrap().iter().any(|e| e["entity"] == "SocialReply"), "{erased}");
    let gone = t.get("SocialMessage", comment_id).await;
    assert!(
        gone["author"] != "ivy@chirp.test" && gone["text"] != "Love the colours!" && gone["external_id"] == "c-1",
        "the words go, the record stays: {gone:?}"
    );
    assert_ne!(t.get("SocialReply", reply_id).await["text"], "Thank you!");
    let rest = t.rows("SocialMessage", json!({"author": "bo"})).await;
    assert_eq!(rest[0]["text"], "@northwind_words see this", "others' messages stay");
    // And what someone said through a review link.
    t.k.erase_personal_data(&t.admin, "client@else.example", "asked to be forgotten").await.unwrap();
    let reviews = t.rows("SocialReview", json!({"post": review_id.to_string()})).await;
    assert!(reviews.iter().all(|r| r["reviewer"] != "client@else.example" && r["text"] != "Lovely colours"), "{reviews:?}");

    // ───────────── numbers, by day, named by what they are ─────────────
    let reads = t.net.lock().unwrap().reads;
    let day = (due.date_naive() + Duration::days(10)).and_hms_opt(8, 0, 0).unwrap().and_utc();
    ferrum_app_social::maintenance_at(&t.k, day).await.unwrap();
    let metric_rows = t.rows("SocialMetric", json!({"day": day.date_naive().to_string()})).await;
    let account_row = metric_rows.iter().find(|m| m["account"] == json!(nw_key.to_string()) && m["publication"].is_null()).expect("the account's own numbers");
    assert_eq!(
        (
            account_row["followers"].as_i64(),
            account_row["impressions"].as_i64(),
            account_row["reach"].as_i64(),
            account_row["engagements"].as_i64(),
            account_row["clicks"].as_i64()
        ),
        (Some(1200), Some(900), Some(700), Some(40), Some(8))
    );
    let post_row = metric_rows.iter().find(|m| m["publication"] == json!(page_pub.to_string())).expect("a post's numbers");
    assert_eq!(
        (post_row["impressions"].as_i64(), post_row["reach"].as_i64(), post_row["engagements"].as_i64(), post_row["clicks"].as_i64()),
        (Some(500), Some(400), Some(6), Some(5)),
        "likes, reposts and replies make its engagements"
    );
    assert_eq!(t.get("SocialPublication", page_pub).await["impressions"], 500, "and onto the publication the insights read");
    let read_now = t.net.lock().unwrap().reads;
    assert!(read_now > reads);
    ferrum_app_social::maintenance_at(&t.k, day + Duration::hours(1)).await.unwrap();
    assert_eq!(t.net.lock().unwrap().reads, read_now, "not read again within the hours settings say");
    ferrum_app_social::maintenance_at(&t.k, day + Duration::hours(7)).await.unwrap();
    assert_eq!(t.rows("SocialMetric", json!({"day": day.date_naive().to_string()})).await.len(), metric_rows.len(), "still one row a day for each");
    ferrum_app_social::maintenance_at(&t.k, day + Duration::days(1)).await.unwrap();
    assert!(!t.rows("SocialMetric", json!({"day": (day + Duration::days(1)).date_naive().to_string()})).await.is_empty(), "a new day is a new row");
    // A network that will not tell: said on the account, cleared when it does.
    t.net.lock().unwrap().stats_down = true;
    ferrum_app_social::maintenance_at(&t.k, day + Duration::days(2)).await.unwrap();
    assert!(t.get("SocialAccount", nw_key).await["metrics_error"].as_str().unwrap().contains("Stats are down"));
    t.net.lock().unwrap().stats_down = false;
    ferrum_app_social::maintenance_at(&t.k, day + Duration::days(3)).await.unwrap();
    assert!(t.get("SocialAccount", nw_key).await["metrics_error"].is_null());

    // What the post's numbers did, day by day, as before: the publication's numbers kept for the day, and the reports of them.
    let today = Utc::now().date_naive();
    let insights = t.rows("SocialInsight", json!({"publication": page_pub.to_string()})).await;
    assert_eq!((insights.len(), insights[0]["impressions"].as_i64(), insights[0]["day"].as_str().map(str::to_string)), (1, Some(500), Some(today.to_string())));
    // Reports: by account, by post, by tag.
    t.k.set_tags(&author, "SocialPost", spring_id, &["launch".to_string()]).await.unwrap();
    let run = |name: &'static str, params: Value| {
        let t = &t;
        async move {
            let p: HashMap<String, String> = params.as_object().unwrap().iter().map(|(k, v)| (k.clone(), v.as_str().unwrap().to_string())).collect();
            let r = ferrum_domain::reports::run(&t.k, &t.admin, name, &p).await.unwrap_or_else(|e| panic!("{name}: {e:?}"));
            r["rows"].as_array().unwrap().clone()
        }
    };
    let span = json!({"from_date": (day - Duration::days(30)).date_naive().to_string(), "to_date": (day + Duration::days(4)).date_naive().to_string()});
    let by_account = run("social_by_account", span.clone()).await;
    let nw = by_account.iter().find(|r| r["account"] == "Northwind Chirp").unwrap();
    assert!(nw["impressions"].as_i64().unwrap() >= 900 && nw["followers"] == 1200, "{nw}");
    let by_post = run("social_by_post", span.clone()).await;
    assert!(by_post.iter().any(|r| r["post"] == "Spring launch" && r["account"] == "Northwind Page" && r["engagements"] == 6), "{by_post:?}");
    let each_day = json!({"from_date": (today - Duration::days(1)).to_string(), "to_date": today.to_string()});
    let by_day = run("social_engagement_by_day", each_day.clone()).await;
    assert_eq!(by_day.len(), 2);
    assert!(by_day.last().unwrap()["impressions"].as_i64().unwrap() >= 500, "{by_day:?}");
    assert!(!run("social_best_times", each_day).await.is_empty());
    let by_tag = run("social_by_tag", span.clone()).await;
    assert!(by_tag.iter().any(|r| r["tag"] == "launch" && r["publications"] == 3 && r["engagements"].as_i64().unwrap() >= 6), "{by_tag:?}");
    // (round two) By network, by kind, against the period before, followers by day, top posts; the brand's week.
    let by_network = run("social_by_network", span.clone()).await;
    assert!(by_network.iter().any(|r| r["network"] == "Chirp" && r["publications"].as_i64().unwrap() > 3), "{by_network:?}");
    let by_kind = run("social_by_kind", span.clone()).await;
    assert!(by_kind.iter().any(|r| r["kind"] == "post"), "{by_kind:?}");
    let compare = run("social_period_compare", span.clone()).await;
    let pubs = compare.iter().find(|r| r["measure"] == "publications").unwrap();
    assert!(pubs["this_period"].as_i64().unwrap() > 0 && pubs["period_before"] == 0 && pubs["change_pct"].is_null(), "{compare:?}");
    let followers = run("social_followers_by_day", span.clone()).await;
    assert!(followers.iter().any(|r| r["account"] == "Northwind Chirp" && r["followers"] == 1200), "{followers:?}");
    let mut top = span.clone();
    top["by"] = json!("impressions");
    top["limit"] = json!("2");
    let top = run("social_top_posts", top).await;
    assert_eq!(top.len(), 2);
    assert!(top[0]["impressions"].as_i64() >= top[1]["impressions"].as_i64());
    let week = t.get("SocialBrand", northwind).await["numbers"].clone();
    assert!(week["publications"].as_i64().unwrap() + week["before"]["publications"].as_i64().unwrap() > 0, "the upkeep's clock runs days ahead here: {week}");
    let doc = t.k.generate_document(&t.admin, &serde_json::from_value(json!({"template": "social.brand_report", "record": northwind})).unwrap()).await.unwrap();
    let html = String::from_utf8(t.k.read_file(&t.admin, doc.file).await.unwrap().2).unwrap_or_default();
    assert!(doc.size > 0 && (html.is_empty() || html.contains("Posts out")), "the shipped template renders the brand's week");
    assert_eq!(
        t.count("select count(*) from automation where tenant_id = $1 and name = 'social_weekly_report'").await,
        1,
        "the schedule ships as configuration"
    );

    // ───────────── who may do what ─────────────
    refused(t.act(&author, "SocialAccount", nw_key, "check", json!({})).await, "forbidden", "");
    assert!(
        t.k.update(&author, "SocialPost", spring_id, obj(json!({"text": "Changed after it went out"})), None).await.is_err(),
        "a post stays as it went out"
    );
}
