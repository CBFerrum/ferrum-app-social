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
        ("GET", "/v1/me") => {
            n.reads += 1;
            if n.stats_down {
                return (503, json!({"error": {"message": "Stats are down"}}));
            }
            (200, json!({"handle": "northwind", "followers": 1200, "day": {"views": 900, "unique": 700, "actions": 40, "taps": 8}}))
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
                let reply = reply.to_string();
                let out =
                    format!("HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{reply}", reply.len());
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
        "publish": {"method": "POST", "path": "/posts", "body": "{\"text\": {{ inputs.text_with_link | json }}, \"media_ids\": {{ inputs.media | json }}}",
                    "result": {"id": "/post/id", "url": "/post/url"}, "error_at": err, "idempotency_header": "Idempotency-Key", "inputs": ["text_with_link", "media"]},
        "publish_media": {"method": "POST", "path": "/media", "body": "{\"name\": {{ inputs.name | json }}, \"alt\": {{ inputs.alt | json }}, \"type\": {{ inputs.content_type | json }}, \"size\": {{ inputs.size | json }}, \"data\": {{ inputs.data | json }}}",
                          "result": {"id": "/media/id"}, "error_at": err, "idempotency_header": "Idempotency-Key", "inputs": ["name", "alt", "content_type", "size", "data"]},
        "delete": {"method": "DELETE", "path": "/posts/{{ inputs.id }}", "error_at": err, "idempotency_header": "Idempotency-Key", "inputs": ["id"]},
        "reply": {"method": "POST", "path": "/replies", "body": "{\"text\": {{ inputs.text | json }}, \"in_reply_to\": {{ inputs.in_reply_to | json }}}",
                  "result": {"id": "/id"}, "error_at": err, "idempotency_header": "Idempotency-Key", "inputs": ["text", "in_reply_to"]},
        "send_message": {"method": "POST", "path": "/dm", "body": "{\"to\": {{ inputs.to | json }}, \"text\": {{ inputs.text | json }}}",
                         "result": {"id": "/id"}, "error_at": err, "idempotency_header": "Idempotency-Key", "inputs": ["to", "text"]},
        "metrics": {"path": "/posts/{{ inputs.id }}/stats", "result": {"impressions": "/views", "reach": "/unique", "likes": "/likes", "reposts": "/shares", "replies": "/replies", "clicks": "/taps"},
                    "error_at": err, "inputs": ["id"]},
        "account": {"path": "/me", "result": {"followers": "/followers", "impressions": "/day/views", "reach": "/day/unique", "engagements": "/day/actions", "clicks": "/day/taps"}, "error_at": err},
        "messages": {"path": "/inbox", "query": {"since": "{{ cursor or '' }}"}, "error_at": err},
    });
    let operations: serde_json::Map<String, Value> = ops.iter().map(|o| (o.to_string(), all[o].clone())).collect();
    let mut d = json!({"base": base, "auth": auth, "apps": ["social"], "roles": ["Social Author", "Social Manager"], "operations": operations});
    if inbound {
        let map = json!({"entity": "SocialMessage", "items_at": "/items", "by": {"external_id": "item.id"},
            "set": {"kind": "item.kind == 'dm' ? 'Direct' : (item.kind == 'mention' ? 'Mention' : 'Comment')", "author": "item.from", "author_id": "item.from_id",
                    "text": "item.text", "account_handle": "has(item.to) ? item.to : null", "post_external_id": "has(item.on_post) ? item.on_post : null",
                    "thread": "item.thread"}});
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
    /// Jobs run, time passing between attempts, until none is left.
    async fn work(&self) {
        loop {
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
    let everything = ["publish", "publish_media", "delete", "reply", "send_message", "metrics", "account", "messages"];
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
    refused(
        t.k.insert(&author, "SocialPost", obj(post(&"a".repeat(90), &[nw_key]))).await,
        "invalid",
        "for Northwind Chirp: Chirp takes 100 characters, and this post is 101",
    );
    t.insert(&author, "SocialPost", post(&"a".repeat(89), &[nw_key])).await;
    refused(t.k.insert(&author, "SocialPost", obj(post("Spring is here", &[co_key]))).await, "invalid", "Contoso Chirp is another brand's");
    refused(t.k.insert(&author, "SocialPost", obj(post("Spring is here", &[nw_pictured]))).await, "invalid", "takes no post without an image or a video");
    let png = t.k.store_file(&author, None, "spring.png", "image/png", b"\x89PNG\r\n\x1a\nspring").await.unwrap();
    let big = t.k.store_file(&author, None, "big.png", "image/png", &vec![7u8; 6000]).await.unwrap();
    let pictured_post = |targets: &[Uuid], file: &Value, alt: Option<&str>| {
        let mut p = post("Spring in bloom", targets);
        p["image"] = file["url"].clone();
        p["image_alt"] = json!(alt);
        p
    };
    refused(t.k.insert(&author, "SocialPost", obj(pictured_post(&[nw_key], &png, None))).await, "invalid", "says what the picture shows");
    refused(
        t.k.insert(&author, "SocialPost", obj(pictured_post(&[nw_key], &big, Some("Big")))).await,
        "invalid",
        "takes pictures of at most 5 KB, and this one is 6 KB",
    );
    refused(t.k.insert(&author, "SocialPost", obj(pictured_post(&[nw_words], &png, Some("Sage")))).await, "invalid", "Chirp Words takes no pictures");
    refused(
        t.k.insert(&author, "SocialPost", obj(pictured_post(&[nw_pia], &png, Some("Sage")))).await,
        "invalid",
        "Pia's Chirp's connection cannot publish pictures",
    );
    let mut commented = post("Spring is here", &[nw_words]);
    commented["first_comment"] = json!("More at the link");
    refused(t.k.insert(&author, "SocialPost", obj(commented)).await, "invalid", "Chirp Words takes no first comment");
    let mut bad_link = post("Spring", &[nw_key]);
    bad_link["link"] = json!("northwind dot example");
    refused(t.k.insert(&author, "SocialPost", obj(bad_link)).await, "invalid", "is a web address");
    let mut elsewhere = pictured_post(&[nw_key], &png, Some("Sage"));
    elsewhere["image"] = json!("https://elsewhere.example/sage.png");
    refused(t.k.insert(&author, "SocialPost", obj(elsewhere)).await, "invalid", "is a picture stored here");
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
    let go = |id: Uuid| {
        let (t, author, approver) = (&t, &author, &approver);
        async move {
            t.act(author, "SocialPost", id, "send_for_review", Value::Null).await.unwrap();
            refused(t.act(author, "SocialPost", id, "approve", Value::Null).await, "forbidden", "");
            t.act(approver, "SocialPost", id, "approve", Value::Null).await.expect("approved");
        }
    };
    t.act(&author, "SocialPost", spring_id, "send_for_review", Value::Null).await.unwrap();
    refused(t.act(&approver, "SocialPost", spring_id, "reject", json!({"note": " "})).await, "invalid", "says why");
    t.act(&approver, "SocialPost", spring_id, "reject", json!({"note": "Say which colours"})).await.unwrap();
    t.act(&author, "SocialPost", spring_id, "send_for_review", Value::Null).await.unwrap();
    t.act(&approver, "SocialPost", spring_id, "approve", Value::Null).await.unwrap();
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
    t.insert(&manager, "SocialSettings", json!({"match_crm": true, "crm_entity": "Contact", "crm_field": "x_handle"})).await;
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
    let by_tag = run("social_by_tag", span).await;
    assert!(by_tag.iter().any(|r| r["tag"] == "launch" && r["publications"] == 3 && r["engagements"].as_i64().unwrap() >= 6), "{by_tag:?}");

    // ───────────── who may do what ─────────────
    refused(t.act(&author, "SocialAccount", nw_key, "check", json!({})).await, "forbidden", "");
    assert!(
        t.k.update(&author, "SocialPost", spring_id, obj(json!({"text": "Changed after it went out"})), None).await.is_err(),
        "a post stays as it went out"
    );
}
