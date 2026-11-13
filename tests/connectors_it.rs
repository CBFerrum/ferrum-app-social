//! The example connection descriptions under docs/connectors, as the platform takes them and as Social uses them.
//!
//! Shape: each loads as a description, says what its network takes in `meta`, and the two the package ships as data
//! (the networks the earlier releases had connectors for) are the same descriptions as the files.
//!
//! End to end, against a real PostgreSQL (`FERRUM_SOCIAL_TEST_DATABASE_URL`, reset first) and one fake service on
//! 127.0.0.1 that answers in the shapes of three networks' public documentation (the pages named in
//! docs/connectors/README.md): the Mastodon-shaped one signs in with OAuth, takes a picture as multipart, an
//! Idempotency-Key, a 429 with Retry-After, and pages notifications by the Link header's `prev`; the Bluesky-shaped one
//! signs in with an app password (a session, refreshed on a 401), takes a picture as the raw body, creates records
//! and lists notifications by reason; the LinkedIn-shaped one gives a new post's id in the `x-restli-id` header. Each
//! description is the file's own with its address put where the placeholder is.

use chrono::{Duration, Utc};
use ferrum_kernel::connections::{ConnectionInput, Description};
use ferrum_kernel::secret::SecretKey;
use ferrum_kernel::tenant::NewTenant;
use ferrum_kernel::{Ctx, Db, DbConfig, Doc, Error, HookRegistry, Kernel, KernelSettings, ListQuery};
use ferrum_meta::Catalog;
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use uuid::Uuid;

const HANDLE: &str = "north.bsky.test";
const APP_PASSWORD: &str = "app-pass-1234-wxyz";
const PNG: &[u8] = b"\x89PNG\r\n\x1a\nCAT-PICTURE";

fn obj(v: Value) -> Doc {
    v.as_object().cloned().unwrap()
}

fn uid(v: &Value) -> Uuid {
    v.as_str().unwrap_or_else(|| panic!("not an id: {v}")).parse().unwrap()
}

fn example(name: &str) -> String {
    std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("docs/connectors/{name}.json"))).unwrap()
}

// ───────────── shape ─────────────

#[test]
fn the_examples_are_descriptions_the_platform_takes() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/connectors");
    let mut seen = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|e| e != "json") {
            continue;
        }
        let d: Description = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        assert_eq!(d.problem(), None, "{}", path.display());
        assert!(d.apps.iter().any(|a| a == "social") && d.operations.contains_key("publish"), "{}", path.display());
        let caps = &d.meta.as_ref().unwrap_or_else(|| panic!("{} says what its network takes", path.display()))["capabilities"];
        assert!(caps["max_text"].as_u64().is_some_and(|n| n > 0) && caps["name"].is_string(), "{}", path.display());
        seen.push(path.file_stem().unwrap().to_string_lossy().to_string());
    }
    seen.sort();
    assert_eq!(seen, ["bluesky", "linkedin", "mastodon", "x"]);
}

#[test]
fn the_package_ships_the_two_the_earlier_releases_had() {
    let packages = ferrum_app_social::packages().unwrap();
    let shipped = &packages.iter().find(|p| p.manifest.package == "social").unwrap().manifest.connections;
    let names: Vec<&str> = shipped.iter().filter_map(|c| c["name"].as_str()).collect();
    assert_eq!(names, ["mastodon", "bluesky"]);
    for c in shipped {
        let file: Value = serde_json::from_str(&example(c["name"].as_str().unwrap())).unwrap();
        assert_eq!(c["description"], file, "{}: the package's data is the file under docs/connectors", c["name"]);
    }
}

// ───────────── the fake service ─────────────

#[derive(Default)]
struct Net {
    base: String,
    /// Tokens the OAuth endpoints gave, and for whom.
    issued: HashMap<String, String>,
    // Mastodon-shaped.
    statuses: Vec<Value>,
    keys: HashMap<String, String>,
    media: Vec<Value>,
    deleted: Vec<String>,
    notifs: HashMap<String, Vec<Value>>,
    page: usize,
    /// Answer 429 with Retry-After: 120 this many times to a post, and to a read of notifications.
    limit_posts: usize,
    limit_notifs: usize,
    notif_reads: Vec<(String, String)>,
    /// Who the account-lookup calls were made as.
    verified_as: Vec<String>,
    // Bluesky-shaped.
    logins: usize,
    refreshes: usize,
    access: HashSet<String>,
    refresh: HashSet<String>,
    blobs: Vec<Value>,
    records: Vec<Value>,
    bsky_deleted: Vec<String>,
    bsky_notifs: Vec<Value>,
    counts: Value,
    // LinkedIn-shaped.
    li_posts: Vec<Value>,
    li_deleted: Vec<String>,
    /// Every request, "METHOD path".
    seen: Vec<String>,
}

type Shared = Arc<Mutex<Net>>;

struct Resp(u16, Vec<(String, String)>, String);

fn json_resp(status: u16, v: Value) -> Resp {
    Resp(status, vec![], v.to_string())
}

fn form(q: &str) -> Vec<(String, String)> {
    reqwest::Url::parse(&format!("http://f/?{q}")).unwrap().query_pairs().map(|(k, v)| (k.into_owned(), v.into_owned())).collect()
}

/// The parts of a multipart body: (name, filename, content type, content).
fn parts(content_type: &str, body: &str) -> Vec<(String, Option<String>, Option<String>, String)> {
    let Some(boundary) = content_type.split("boundary=").nth(1) else { return vec![] };
    body.split(&format!("--{boundary}"))
        .filter(|p| p.contains("Content-Disposition"))
        .map(|p| {
            let (head, content) = p.split_once("\r\n\r\n").unwrap_or((p, ""));
            let field = |what: &str| head.split(what).nth(1).and_then(|r| r.split('"').next()).map(str::to_string);
            let kind = head.lines().find_map(|l| l.strip_prefix("Content-Type: ")).map(str::to_string);
            (field("name=\"").unwrap_or_default(), field("filename=\""), kind, content.trim_end_matches("\r\n").to_string())
        })
        .collect()
}

fn status_of(n: &Net, s: &Value) -> Value {
    json!({"id": s["id"], "url": format!("https://fedi.test/@north/{}", s["id"].as_str().unwrap()), "content": s["status"], "visibility": s["visibility"],
           "in_reply_to_id": s["in_reply_to_id"], "favourites_count": n.counts["favourites"], "reblogs_count": n.counts["reblogs"], "replies_count": n.counts["replies"]})
}

fn notification(id: i64, acct: &str, visibility: &str, html: &str, reply_to: Option<&str>) -> Value {
    json!({"id": id.to_string(), "type": "mention", "account": {"id": format!("acct-{acct}"), "acct": acct, "username": acct},
           "status": {"id": format!("s{id}"), "visibility": visibility, "content": html, "in_reply_to_id": reply_to}})
}

fn answer(n: &mut Net, method: &str, target: &str, headers: &HashMap<String, String>, body: &[u8]) -> Resp {
    let route = target.split('?').next().unwrap_or_default().to_string();
    let query = target.split_once('?').map(|(_, q)| q).unwrap_or_default().to_string();
    let text = String::from_utf8_lossy(body).to_string();
    let json_body: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    n.seen.push(format!("{method} {target}"));
    let bearer = headers.get("authorization").and_then(|a| a.strip_prefix("Bearer ").map(str::to_string));
    // OAuth, for the Mastodon-shaped and the LinkedIn-shaped.
    if route == "/oauth/token" || route == "/oauth/v2/accessToken" {
        let f: HashMap<String, String> = form(&text).into_iter().collect();
        let who = f.get("code").and_then(|c| c.strip_prefix("CODE-")).unwrap_or("nobody").to_string();
        let token = format!("AT-{who}-{}", n.issued.len() + 1);
        n.issued.insert(token.clone(), who);
        return json_resp(200, json!({"access_token": token, "token_type": "Bearer", "expires_in": 3600}));
    }
    // Bluesky-shaped sessions.
    if route == "/xrpc/com.atproto.server.createSession" {
        if json_body["identifier"] != HANDLE || json_body["password"] != APP_PASSWORD {
            return json_resp(401, json!({"error": "AuthenticationRequired", "message": "Invalid identifier or password"}));
        }
        n.logins += 1;
        let (a, r) = (format!("AJ-{}", n.logins + n.refreshes), format!("RJ-{}", n.logins + n.refreshes));
        n.access.insert(a.clone());
        n.refresh.insert(r.clone());
        return json_resp(200, json!({"accessJwt": a, "refreshJwt": r, "handle": HANDLE, "did": "did:plc:north"}));
    }
    if route == "/xrpc/com.atproto.server.refreshSession" {
        if !bearer.as_ref().is_some_and(|b| n.refresh.contains(b)) {
            return json_resp(400, json!({"error": "ExpiredToken", "message": "Token has been revoked"}));
        }
        n.refreshes += 1;
        let (a, r) = (format!("AJ-{}", n.logins + n.refreshes), format!("RJ-{}", n.logins + n.refreshes));
        n.access.insert(a.clone());
        n.refresh.insert(r.clone());
        return json_resp(200, json!({"accessJwt": a, "refreshJwt": r, "handle": HANDLE, "did": "did:plc:north"}));
    }
    if let Some(call) = route.strip_prefix("/xrpc/") {
        if !bearer.as_ref().is_some_and(|b| n.access.contains(b)) {
            return json_resp(401, json!({"error": "ExpiredToken", "message": "Token has expired"}));
        }
        return bluesky(n, method, call, &query, headers, &text, &json_body, body);
    }
    // LinkedIn-shaped.
    if route.starts_with("/rest/") {
        let who = bearer.as_ref().and_then(|b| n.issued.get(b)).cloned();
        if who.is_none() {
            return json_resp(401, json!({"message": "Empty or invalid access token"}));
        }
        if headers.get("linkedin-version").is_none() || headers.get("x-restli-protocol-version").map(String::as_str) != Some("2.0.0") {
            return json_resp(400, json!({"message": "A Linkedin-Version header is required"}));
        }
        return match (method, route.strip_prefix("/rest/posts")) {
            ("POST", Some("")) => {
                let id = format!("urn:li:share:{}", n.li_posts.len() + 1);
                n.li_posts.push(json!({"id": id, "body": json_body}));
                Resp(201, vec![("x-restli-id".into(), id)], String::new())
            }
            ("DELETE", Some(urn)) => {
                n.li_deleted.push(urn.trim_start_matches('/').to_string());
                Resp(204, vec![], String::new())
            }
            _ => json_resp(404, json!({"message": "No such resource"})),
        };
    }
    // Mastodon-shaped.
    let who = bearer.as_ref().and_then(|b| n.issued.get(b)).cloned();
    let Some(who) = who else { return json_resp(401, json!({"error": "The access token is invalid"})) };
    match (method, route.as_str()) {
        ("POST", "/api/v2/media") => {
            let kind = headers.get("content-type").cloned().unwrap_or_default();
            let ps = parts(&kind, &text);
            let file = ps.iter().find(|p| p.0 == "file");
            let Some(file) = file else { return json_resp(422, json!({"error": "Validation failed: File can't be blank"})) };
            let id = format!("m{}", n.media.len() + 1);
            n.media.push(json!({"id": id, "filename": file.1, "content_type": file.2, "bytes": file.3,
                                "description": ps.iter().find(|p| p.0 == "description").map(|p| p.3.clone()), "who": who}));
            json_resp(200, json!({"id": id, "type": "image"}))
        }
        ("POST", "/api/v1/statuses") => {
            if n.limit_posts > 0 {
                n.limit_posts -= 1;
                return Resp(429, vec![("retry-after".into(), "120".into())], json!({"error": "Too many requests"}).to_string());
            }
            let key = headers.get("idempotency-key").cloned().unwrap_or_default();
            if let Some(id) = n.keys.get(&key) {
                let s = n.statuses.iter().find(|s| s["id"] == *id).cloned().unwrap();
                return json_resp(200, status_of(n, &s));
            }
            let id = format!("s{}", 5000 + n.statuses.len());
            let s = json!({"id": id, "status": json_body["status"], "in_reply_to_id": json_body["in_reply_to_id"], "visibility": json_body["visibility"],
                           "media_ids": json_body["media_ids"], "key": key, "who": who});
            n.keys.insert(key, id);
            n.statuses.push(s.clone());
            json_resp(200, status_of(n, &s))
        }
        ("DELETE", r) if r.starts_with("/api/v1/statuses/") => {
            n.deleted.push(r.trim_start_matches("/api/v1/statuses/").to_string());
            json_resp(200, json!({}))
        }
        ("GET", r) if r.starts_with("/api/v1/statuses/") => {
            let id = r.trim_start_matches("/api/v1/statuses/");
            match n.statuses.iter().find(|s| s["id"] == id).cloned() {
                Some(s) => json_resp(200, status_of(n, &s)),
                None => json_resp(404, json!({"error": "Record not found"})),
            }
        }
        ("GET", "/api/v1/accounts/verify_credentials") => {
            n.verified_as.push(who);
            json_resp(200, json!({"id": "1", "acct": "north", "followers_count": 321}))
        }
        ("GET", "/api/v1/notifications") => {
            n.notif_reads.push((who.clone(), target.to_string()));
            if n.limit_notifs > 0 {
                n.limit_notifs -= 1;
                return Resp(429, vec![("retry-after".into(), "120".into())], json!({"error": "Too many requests"}).to_string());
            }
            let q: HashMap<String, String> = form(&query).into_iter().collect();
            let all = n.notifs.get(&who).cloned().unwrap_or_default();
            let num = |v: &Value| v["id"].as_str().and_then(|i| i.parse::<i64>().ok()).unwrap_or_default();
            let mut page: Vec<Value> = match (q.get("min_id").filter(|m| !m.is_empty()), q.get("max_id")) {
                (Some(min), _) => {
                    let min: i64 = min.parse().unwrap();
                    let mut newer: Vec<Value> = all.iter().filter(|v| num(v) > min).cloned().collect();
                    newer.sort_by_key(num);
                    let mut first: Vec<Value> = newer.into_iter().take(n.page).collect();
                    first.reverse();
                    first
                }
                (None, Some(max)) => {
                    let max: i64 = max.parse().unwrap();
                    let mut older: Vec<Value> = all.iter().filter(|v| num(v) < max).cloned().collect();
                    older.sort_by_key(|v| -num(v));
                    older.into_iter().take(n.page).collect()
                }
                _ => {
                    let mut newest = all.clone();
                    newest.sort_by_key(|v| -num(v));
                    newest.into_iter().take(n.page).collect()
                }
            };
            page.sort_by_key(|v| -num(v));
            let mut out = vec![];
            if let (Some(newest), Some(oldest)) = (page.first(), page.last()) {
                let at = |extra: &str| format!("{}/api/v1/notifications?types%5B%5D=mention&limit=40&{extra}", n.base);
                out.push((
                    "link".to_string(),
                    format!(
                        "<{}>; rel=\"next\", <{}>; rel=\"prev\"",
                        at(&format!("max_id={}", oldest["id"].as_str().unwrap())),
                        at(&format!("min_id={}", newest["id"].as_str().unwrap()))
                    ),
                ));
            }
            Resp(200, out, Value::Array(page).to_string())
        }
        _ => json_resp(404, json!({"error": "Record not found"})),
    }
}

#[allow(clippy::too_many_arguments)]
fn bluesky(n: &mut Net, method: &str, call: &str, query: &str, headers: &HashMap<String, String>, text: &str, body: &Value, raw: &[u8]) -> Resp {
    match (method, call) {
        ("POST", "com.atproto.repo.uploadBlob") => {
            let kind = headers.get("content-type").cloned().unwrap_or_default();
            let cid = format!("bafy-blob-{}", n.blobs.len() + 1);
            n.blobs.push(json!({"cid": cid, "content_type": kind, "bytes": raw.len(), "same": raw == PNG}));
            json_resp(200, json!({"blob": {"$type": "blob", "ref": {"$link": cid}, "mimeType": kind, "size": raw.len()}}))
        }
        ("POST", "com.atproto.repo.createRecord") => {
            if body["repo"] != HANDLE || body["collection"] != "app.bsky.feed.post" || !body["record"]["createdAt"].as_str().is_some_and(|t| t.ends_with('Z')) {
                return json_resp(400, json!({"error": "InvalidRequest", "message": format!("Bad record: {text}")}));
            }
            let rkey = format!("3k{}", n.records.len() + 1);
            let uri = format!("at://did:plc:north/app.bsky.feed.post/{rkey}");
            n.records.push(json!({"uri": uri, "record": body["record"]}));
            json_resp(200, json!({"uri": uri, "cid": format!("bafy-rec-{}", n.records.len())}))
        }
        ("POST", "com.atproto.repo.deleteRecord") => {
            n.bsky_deleted.push(body["rkey"].as_str().unwrap_or_default().to_string());
            json_resp(200, json!({}))
        }
        ("GET", "app.bsky.feed.getPosts") => {
            let uri = form(query).into_iter().find(|(k, _)| k == "uris").map(|(_, v)| v).unwrap_or_default();
            let c = &n.counts;
            json_resp(
                200,
                json!({"posts": [{"uri": uri, "likeCount": c["likes"], "repostCount": c["reposts"], "replyCount": c["replies"], "quoteCount": c["quotes"]}]}),
            )
        }
        ("GET", "app.bsky.actor.getProfile") => json_resp(200, json!({"handle": HANDLE, "followersCount": 77})),
        ("GET", "app.bsky.notification.listNotifications") => {
            let reasons: Vec<String> = form(query).into_iter().filter(|(k, _)| k == "reasons").map(|(_, v)| v).collect();
            let list: Vec<Value> = n.bsky_notifs.iter().filter(|x| reasons.iter().any(|r| *x["reason"].as_str().unwrap_or_default() == **r)).cloned().collect();
            json_resp(200, json!({"notifications": list}))
        }
        _ => json_resp(404, json!({"error": "MethodNotImplemented", "message": format!("No {call}")})),
    }
}

async fn serve(net: Shared) -> u16 {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    net.lock().unwrap().base = format!("http://127.0.0.1:{port}");
    tokio::spawn(async move {
        loop {
            let Ok((mut s, _)) = listener.accept().await else { return };
            let net = net.clone();
            tokio::spawn(async move {
                let mut buf = Vec::new();
                let mut chunk = [0u8; 16384];
                loop {
                    let n = s.read(&mut chunk).await.unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    buf.extend_from_slice(&chunk[..n]);
                    if let Some(end) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                        let head = String::from_utf8_lossy(&buf[..end]).to_ascii_lowercase();
                        let len = head.lines().find_map(|l| l.strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0))).unwrap_or(0);
                        if buf.len() >= end + 4 + len {
                            break;
                        }
                    }
                }
                let end = buf.windows(4).position(|w| w == b"\r\n\r\n").unwrap_or(buf.len());
                let head = String::from_utf8_lossy(&buf[..end]).to_string();
                let body = buf.get(end + 4..).unwrap_or_default().to_vec();
                let mut lines = head.lines();
                let mut first = lines.next().unwrap_or_default().split_whitespace();
                let (method, target) = (first.next().unwrap_or_default().to_string(), first.next().unwrap_or("/").to_string());
                let headers: HashMap<String, String> =
                    lines.filter_map(|l| l.split_once(':').map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string()))).collect();
                let Resp(status, extra, reply) = answer(&mut net.lock().unwrap(), &method, &target, &headers, &body);
                let mut out = format!("HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n", reply.len());
                for (k, v) in extra {
                    out.push_str(&format!("{k}: {v}\r\n"));
                }
                out.push_str(&format!("\r\n{reply}"));
                let _ = s.write_all(out.as_bytes()).await;
            });
        }
    });
    port
}

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

// ───────────── the scenario ─────────────

struct T {
    k: Arc<Kernel>,
    tenant: Uuid,
    admin: Ctx,
    net: Shared,
    base: String,
}

impl T {
    async fn insert(&self, ctx: &Ctx, entity: &str, v: Value) -> Doc {
        self.k.insert(ctx, entity, obj(v)).await.unwrap_or_else(|e| panic!("insert {entity}: {e:?}"))
    }
    async fn get(&self, entity: &str, id: Uuid) -> Doc {
        self.k.get(&self.admin, entity, id, false).await.unwrap_or_else(|e| panic!("get {entity}: {e:?}"))
    }
    async fn rows(&self, entity: &str, filters: Value) -> Vec<Doc> {
        let q = ListQuery { filters: ferrum_kernel::query::parse_filters(&filters).unwrap(), limit: Some(200), ..Default::default() };
        self.k.list(&self.admin, entity, &q).await.unwrap().rows
    }
    async fn user(&self, email: &str, roles: &[&str]) -> Ctx {
        let roles: Vec<String> = roles.iter().map(|r| r.to_string()).collect();
        let id = self.k.create_user(&self.admin, email, email, Some("Correct-Horse-9"), &roles, false).await.expect("user");
        let s = self.k.login("brands", email, "Correct-Horse-9").await.expect("login");
        self.k.load_ctx(self.tenant, id, Some(s.token_version), "it".into()).await.expect("ctx")
    }
    async fn work(&self) {
        loop {
            let mut tx = self.k.db.system_tx().await.unwrap();
            sqlx::query("update job set run_at = now() where status = 'queued' and kind like 'social.%'").execute(&mut *tx).await.unwrap();
            tx.commit().await.unwrap();
            if self.k.run_jobs_once(4).await.unwrap() == 0 {
                break;
            }
        }
    }
    async fn upkeep(&self, now: chrono::DateTime<Utc>) {
        ferrum_app_social::maintenance_at(&self.k, now).await.expect("upkeep");
        self.work().await;
        ferrum_app_social::maintenance_at(&self.k, now).await.expect("upkeep settles");
    }
    /// Pulls now and runs what that queued.
    async fn pull(&self, conn: Uuid, name: &str) {
        self.k.pull_now(&self.admin, conn, name).await.unwrap();
        while self.k.run_jobs_once(4).await.unwrap() > 0 {}
    }
    async fn count(&self, sql: &str) -> i64 {
        let mut tx = self.k.db.tenant_tx(self.tenant).await.unwrap();
        let n = sqlx::query_scalar(sqlx::AssertSqlSafe(sql.to_string())).bind(self.tenant).fetch_one(&mut *tx).await.unwrap();
        tx.commit().await.unwrap();
        n
    }
    async fn sign_in(&self, who: &Ctx, conn: Uuid, code: &str) {
        let session = Uuid::now_v7();
        let url = self.k.connect_start(who, conn, session).await.unwrap();
        let state = reqwest::Url::parse(&url).unwrap().query_pairs().find(|(k, _)| k == "state").unwrap().1.to_string();
        self.k.connect_finish(who, session, &state, Some(code), None).await.unwrap();
    }
    /// A connection from the example `name`, with its addresses put where the placeholders are.
    async fn connect(&self, name: &str, file: &str, secrets: Value, edit: impl FnOnce(&mut Value)) -> Uuid {
        let text = example(file)
            .replace("https://mastodon.example", &self.base)
            .replace("https://bsky.social", &self.base)
            .replace("https://api.linkedin.com", &self.base)
            .replace("https://www.linkedin.com", &self.base);
        let mut d: Value = serde_json::from_str(&text).unwrap();
        edit(&mut d);
        let input: ConnectionInput =
            serde_json::from_value(json!({"name": name, "label": name, "description": d, "secrets": secrets, "enabled": true})).unwrap();
        self.k.save_connection_def(&self.admin, None, &input).await.unwrap_or_else(|e| panic!("{name}: {e:?}"))
    }
    fn net(&self) -> std::sync::MutexGuard<'_, Net> {
        self.net.lock().unwrap()
    }
}

#[test]
fn the_examples_end_to_end() {
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().thread_stack_size(16 << 20).build().unwrap();
    rt.block_on(async { tokio::spawn(scenario()).await.unwrap() });
}

async fn scenario() {
    let url = std::env::var("FERRUM_SOCIAL_TEST_DATABASE_URL").unwrap_or_else(|_| "postgres://ferrum:ferrum@localhost:5432/ferrum_social_test".into());
    let db = Db::connect(DbConfig { url, max_connections: 16, ..Default::default() }).await.expect("connect");
    sqlx::raw_sql("drop schema if exists public cascade; create schema public;").execute(&db.pool).await.expect("reset");
    let mut packages = ferrum_domain::packages().expect("platform packages");
    packages.extend(ferrum_app_social::packages().expect("app packages"));
    let mut hooks = HookRegistry::default();
    ferrum_domain::register(&mut hooks);
    ferrum_app_social::register(&mut hooks);
    let settings = KernelSettings {
        secret_key: Some(SecretKey::new([7u8; 32])),
        connection_hosts: vec!["127.0.0.1".into()],
        public_url: Some("https://ferrum.example.test".into()),
        ..Default::default()
    };
    let k = Kernel::boot(db, Catalog::from_packages(packages).unwrap(), hooks, settings, None).await.expect("boot");
    k.set_file_store(Arc::new(MemFiles::default()));
    ferrum_domain::maintenance(&k).await.expect("partitions");
    let tenant = k
        .create_tenant(NewTenant {
            slug: "brands".into(),
            name: "Brands".into(),
            admin_email: "admin@brands.test".into(),
            admin_password: "Correct-Horse-9".into(),
            admin_name: None,
            packages: vec!["social".into()],
            settings: Some(json!({"timezone": "UTC"})),
        })
        .await
        .expect("tenant");
    let session = k.login("brands", "admin@brands.test", "Correct-Horse-9").await.expect("login");
    let admin = k.load_ctx(tenant, session.user, Some(session.token_version), "it".into()).await.expect("ctx");
    let net: Shared = Arc::default();
    {
        let mut n = net.lock().unwrap();
        n.page = 2;
        n.counts = json!({"favourites": 5, "reblogs": 2, "replies": 1, "likes": 4, "reposts": 3, "quotes": 1});
    }
    let port = serve(net.clone()).await;
    let base = format!("http://127.0.0.1:{port}");
    let t = T { k: k.clone(), tenant, admin, net: net.clone(), base };
    let author = t.user("author@brands.test", &["Social Author"]).await;
    let approver = t.user("approver@brands.test", &["Social Approver"]).await;
    let manager = t.user("manager@brands.test", &["Social Manager"]).await;
    let pia = t.user("pia@brands.test", &["Social Author"]).await;
    let brand = uid(&t.insert(&manager, "SocialBrand", json!({"brand_name": "Northwind"})).await["id"]);

    // The three descriptions, saved as the platform takes them. No network profile names any of them: what each takes
    // is its own `meta`.
    let mastodon = t.connect("mastodon_north", "mastodon", json!({"client_secret": "mastodon-client-secret"}), |_| {}).await;
    let bluesky = t.connect("bluesky_north", "bluesky", json!({"handle": HANDLE, "app_password": APP_PASSWORD}), |_| {}).await;
    let linkedin = t.connect("linkedin_north", "linkedin", json!({"client_secret": "linkedin-client-secret"}), |_| {}).await;
    let account =
        |name: &str, handle: &str, conn: Uuid| json!({"account_name": name, "brand": brand.to_string(), "handle": handle, "connection": conn.to_string()});
    let on_mastodon = uid(&t.insert(&manager, "SocialAccount", account("Northwind Mastodon", "north", mastodon)).await["id"]);
    let on_bluesky = uid(&t.insert(&manager, "SocialAccount", account("Northwind Bluesky", HANDLE, bluesky)).await["id"]);
    let on_linkedin = uid(&t.insert(&manager, "SocialAccount", account("Northwind LinkedIn", "northwind", linkedin)).await["id"]);
    let state = |a: Uuid| {
        let t = &t;
        async move {
            let row = t.get("SocialAccount", a).await;
            (row["state"].as_str().unwrap_or_default().to_string(), row["state_note"].as_str().map(str::to_string))
        }
    };
    assert_eq!(state(on_mastodon).await.0, "Not Connected", "an OAuth account waits for its sign-in");
    assert_eq!(state(on_bluesky).await.0, "Connected", "a session signs itself in from its secrets, on first use");

    // What a network takes comes from the description's meta, where the network's own limit lives.
    let post = |text: &str, targets: &[Uuid], extra: Value| {
        let mut p = json!({"headline": "Spring launch", "brand": brand.to_string(), "text": text, "link": "https://northwind.example/spring",
                           "scheduled_at": (Utc::now() + Duration::hours(1)).to_rfc3339(),
                           "targets": targets.iter().map(|a| json!({"account": a.to_string()})).collect::<Vec<_>>()});
        for (k, v) in extra.as_object().unwrap() {
            p[k] = v.clone();
        }
        p
    };
    let e = t.k.insert(&author, "SocialPost", obj(post(&"a".repeat(490), &[on_mastodon], json!({})))).await.unwrap_err();
    assert!(e.to_string().contains("Mastodon takes 500 characters, and this post is 523"), "{e}");
    let e = t.k.insert(&author, "SocialPost", obj(post(&"a".repeat(290), &[on_bluesky], json!({})))).await.unwrap_err();
    assert!(e.to_string().contains("Bluesky takes 300 characters"), "{e}");
    let png = t.k.store_file(&author, None, "cat.png", "image/png", PNG).await.unwrap();
    let mut pictured = post("The cat in the garden", &[on_linkedin], json!({"image": png["url"], "image_alt": "A cat"}));
    pictured["link"] = Value::Null;
    let e = t.k.insert(&author, "SocialPost", obj(pictured)).await.unwrap_err();
    assert!(e.to_string().contains("LinkedIn takes no pictures"), "an image a description cannot send is refused, not dropped: {e}");

    let go = |id: Uuid| {
        let (t, author, approver) = (&t, &author, &approver);
        async move {
            t.k.run_action(author, "SocialPost", id, "send_for_review", Value::Null).await.unwrap();
            t.k.decide_approval(approver, "SocialPost", id, true, None).await.unwrap();
        }
    };
    let later = Utc::now() + Duration::hours(3);

    // ───────────── a post waits, saying why, for an account that is not connected ─────────────
    let spring = uid(&t
        .insert(&author, "SocialPost", post("Spring is here", &[on_mastodon, on_bluesky], json!({"image": png["url"], "image_alt": "A cat in bloom"})))
        .await["id"]);
    go(spring).await;
    t.upkeep(later).await;
    let row = t.get("SocialPost", spring).await;
    assert_eq!(row["status"], "Approved", "not failed, not sent");
    assert!(
        row["waiting_for"].as_str().is_some_and(|w| w.contains("Northwind Mastodon: Not Connected") && !w.contains("Bluesky")),
        "it says which account and why: {row:?}"
    );
    assert!(
        t.net().statuses.is_empty() && t.net().records.is_empty() && t.net().media.is_empty(),
        "nothing went out to the account that could be reached either"
    );
    assert_eq!(t.count("select count(*) from e_social_publication where tenant_id = $1").await, 0);

    // ───────────── Mastodon-shaped: text, a picture as multipart, a key, Retry-After, the address ─────────────
    t.sign_in(&t.admin, mastodon, "CODE-north").await;
    assert_eq!(state(on_mastodon).await.0, "Not Connected", "the account shows what it last said until it is looked at");
    t.net().limit_posts = 1;
    t.upkeep(later).await;
    assert_eq!(state(on_mastodon).await.0, "Connected");
    let row = t.get("SocialPost", spring).await;
    assert_eq!(row["status"], "Published", "{row:?}");
    assert!(row["waiting_for"].is_null());
    {
        let n = t.net();
        assert_eq!(n.media.len(), 1, "the picture went once, as multipart");
        assert_eq!(n.media[0]["filename"], "cat.png");
        assert_eq!(n.media[0]["content_type"], "image/png");
        assert_eq!(n.media[0]["bytes"], String::from_utf8_lossy(PNG).as_ref(), "the stored file, byte for byte, as the part");
        assert_eq!(n.media[0]["description"], "A cat in bloom", "what it shows, as its description");
        assert_eq!(n.statuses.len(), 1, "a 429 with Retry-After, then one status");
        assert_eq!(n.statuses[0]["status"], "Spring is here https://northwind.example/spring");
        assert_eq!(n.statuses[0]["media_ids"], json!(["m1"]));
        assert!(n.statuses[0]["key"].as_str().unwrap().starts_with("publish:"), "under the key of its publication");
        assert!(n.statuses[0]["who"] == "north", "as the signed-in account");
        assert_eq!(n.seen.iter().filter(|s| s.starts_with("POST /api/v1/statuses")).count(), 2, "the refused try and the one that went");
    }
    let pubs = t.rows("SocialPublication", json!({"post": spring.to_string()})).await;
    let mp = pubs.iter().find(|p| uid(&p["account"]) == on_mastodon).unwrap();
    assert_eq!((mp["external_id"].as_str(), mp["permalink"].as_str()), (Some("s5000"), Some("https://fedi.test/@north/s5000")));

    // ───────────── Bluesky-shaped: a session, a picture as the raw body, a record ─────────────
    let bp = pubs.iter().find(|p| uid(&p["account"]) == on_bluesky).unwrap();
    assert_eq!(bp["external_id"], "at://did:plc:north/app.bsky.feed.post/3k1");
    {
        let n = t.net();
        assert_eq!((n.logins, n.refreshes), (1, 0), "signed in once for the picture and the post");
        assert_eq!(n.blobs.len(), 1);
        assert_eq!((n.blobs[0]["content_type"].as_str(), n.blobs[0]["same"].as_bool()), (Some("image/png"), Some(true)), "the file is the body, with its type");
        let record = &n.records[0]["record"];
        assert_eq!(record["text"], "Spring is here https://northwind.example/spring", "with a picture the link stays in the text");
        let image = &record["embed"]["images"][0];
        assert_eq!(
            (image["alt"].as_str(), image["image"]["ref"]["$link"].as_str(), image["image"]["mimeType"].as_str()),
            (Some("A cat in bloom"), Some("bafy-blob-1"), Some("image/png"))
        );
        assert_eq!(image["image"]["size"], PNG.len());
        assert!(record["createdAt"].as_str().unwrap().ends_with('Z'));
    }
    // Without a picture the link is a card, and a token the service has dropped is renewed by the description's refresh.
    t.net().access.clear();
    let card = uid(&t.insert(&author, "SocialPost", post("A short one", &[on_bluesky], json!({}))).await["id"]);
    go(card).await;
    t.upkeep(later).await;
    {
        let n = t.net();
        assert_eq!((n.logins, n.refreshes), (1, 1), "refreshed after a 401, not signed in again");
        let record = &n.records[1]["record"];
        assert_eq!(record["text"], "A short one");
        assert_eq!(record["embed"]["external"]["uri"], "https://northwind.example/spring");
    }

    // ───────────── LinkedIn-shaped: the id in a header; deleting by it ─────────────
    t.sign_in(&t.admin, linkedin, "CODE-north").await;
    let li = uid(&t.insert(&author, "SocialPost", post("Hello LinkedIn", &[on_linkedin], json!({}))).await["id"]);
    go(li).await;
    t.upkeep(later).await;
    let lp = t.rows("SocialPublication", json!({"post": li.to_string()})).await;
    assert_eq!(lp[0]["external_id"], "urn:li:share:1", "the post's id was in the x-restli-id header");
    assert_eq!(t.net().li_posts[0]["body"]["commentary"], "Hello LinkedIn https://northwind.example/spring");
    t.k.run_action(&manager, "SocialPublication", uid(&lp[0]["id"]), "delete_remote", Value::Null).await.unwrap();
    assert_eq!(t.net().li_deleted, ["urn%3Ali%3Ashare%3A1"], "deleted by the urn, percent-encoded as a path wants it");

    // ───────────── delete, on each ─────────────
    t.k.run_action(&manager, "SocialPublication", uid(&mp["id"]), "delete_remote", Value::Null).await.unwrap();
    t.k.run_action(&manager, "SocialPublication", uid(&bp["id"]), "delete_remote", Value::Null).await.unwrap();
    let gone = {
        let n = t.net();
        (n.deleted.clone(), n.bsky_deleted.clone())
    };
    assert_eq!(gone, (vec!["s5000".to_string()], vec!["3k1".to_string()]), "the record's key is the last part of its address");

    // ───────────── numbers: a sum in the description's expression, the account's followers ─────────────
    let fresh = uid(&t.insert(&author, "SocialPost", post("Numbers", &[on_mastodon, on_bluesky], json!({}))).await["id"]);
    go(fresh).await;
    t.upkeep(later).await;
    t.upkeep(later + Duration::hours(7)).await;
    let numbers = t.rows("SocialMetric", json!({})).await;
    let of = |account: Uuid, post: bool| numbers.iter().find(|m| uid(&m["account"]) == account && m["publication"].is_null() != post).cloned().unwrap();
    assert_eq!(of(on_mastodon, false)["followers"], 321);
    assert_eq!(of(on_bluesky, false)["followers"], 77);
    assert_eq!(of(on_mastodon, true)["engagements"], 8, "favourites, boosts and replies, added by the description: 5 + 2 + 1");
    assert_eq!(of(on_bluesky, true)["engagements"], 9, "likes, reposts, replies and quotes: 4 + 3 + 1 + 1");

    // ───────────── the inbox: Mastodon's Link header, forward; Retry-After on a pull; the account by its connection ─────────────
    let mentions = |ids: std::ops::RangeInclusive<i64>| -> Vec<Value> {
        ids.map(|i| match i {
            101 => notification(i, "bo", "public", "<p>Is it <a href=\"x\">vegan</a> &amp; local?</p>", Some("s5001")),
            102 => notification(i, "cy", "direct", "<p>Private question</p>", None),
            _ => notification(i, &format!("fan{i}"), "unlisted", &format!("<p>Hello {i}</p>"), None),
        })
        .collect()
    };
    t.net().notifs.insert("north".into(), mentions(101..=102));
    t.net().limit_notifs = 1;
    t.pull(mastodon, "mentions").await;
    let waits = t
        .count("select extract(epoch from run_at - now())::bigint from job where tenant_id = $1 and kind = 'platform.connection.pull' and status = 'queued'")
        .await;
    assert!((100..=120).contains(&waits), "a pull that was told to wait 120 s is asked again then: {waits}");
    t.net().notif_reads.clear();
    t.work().await;
    while t.k.run_jobs_once(4).await.unwrap() > 0 {}
    {
        let mut tx = t.k.db.system_tx().await.unwrap();
        sqlx::query("update job set run_at = now() where status = 'queued'").execute(&mut *tx).await.unwrap();
        tx.commit().await.unwrap();
    }
    while t.k.run_jobs_once(4).await.unwrap() > 0 {}
    let inbox = t.rows("SocialMessage", json!({})).await;
    assert_eq!(inbox.len(), 2, "read after the wait: {inbox:?}");
    let bo = inbox.iter().find(|m| m["author"] == "bo").unwrap();
    let cy = inbox.iter().find(|m| m["author"] == "cy").unwrap();
    assert_eq!((bo["kind"].as_str(), cy["kind"].as_str()), (Some("Mention"), Some("Direct")), "a private status is a direct message");
    assert_eq!(bo["text"], "Is it vegan & local?", "the markup of a server that sends HTML is read as the words it holds");
    assert_eq!(uid(&bo["account"]), on_mastodon, "found by the post it answers");
    assert_eq!(uid(&cy["account"]), on_mastodon, "found by the one account its connection reaches");
    assert_eq!(bo["thread"], "public", "the status's openness, kept for the answer");
    // Between runs more arrive than a page holds: the Link header's prev is followed forward, page after page.
    t.net().notifs.get_mut("north").unwrap().extend(mentions(103..=107));
    t.net().notif_reads.clear();
    t.pull(mastodon, "mentions").await;
    let reads: Vec<String> = t.net().notif_reads.iter().map(|r| r.1.clone()).collect();
    assert!(
        reads[0].contains("min_id=102") && reads[1].contains("min_id=104") && reads[2].contains("min_id=106"),
        "forward, each page from the address the last named: {reads:?}"
    );
    assert_eq!(t.rows("SocialMessage", json!({})).await.len(), 7, "five more, each once");
    t.pull(mastodon, "mentions").await;
    assert_eq!(t.rows("SocialMessage", json!({})).await.len(), 7, "read again, nothing twice");
    // Mapped writes are marked as fetched from outside: nothing was sent back because they arrived.
    assert_eq!(t.count("select count(*) from job where tenant_id = $1 and kind = 'social.reply'").await, 0, "no reply is queued by a message that was read");

    // Answered as the status was: never more open than what it answers; a direct message directly.
    let reply = |m: &Doc, text: &str| {
        let (t, admin, id) = (&t, &t.admin, uid(&m["id"]));
        let text = text.to_string();
        async move {
            t.k.run_action(admin, "SocialMessage", id, "reply", json!({"reply": text})).await.unwrap();
            t.work().await;
        }
    };
    reply(bo, "Yes, both").await;
    reply(cy, "Of course").await;
    {
        let n = t.net();
        let a = n.statuses.iter().find(|s| s["status"] == "@bo Yes, both").expect("naming its author, so the server delivers it");
        assert_eq!((a["in_reply_to_id"].as_str(), a["visibility"].as_str()), (Some("s101"), Some("public")));
        let d = n.statuses.iter().find(|s| s["status"] == "@cy Of course").unwrap();
        assert_eq!((d["in_reply_to_id"].as_str(), d["visibility"].as_str()), (Some("s102"), Some("direct")), "a direct message is answered directly");
    }

    // ───────────── a person's own account: its pull runs as her, and what it reads is hers ─────────────
    let own = t
        .connect("mastodon_mine", "mastodon", json!({"client_secret": "pia-client-secret"}), |d| {
            d["auth"]["owner"] = json!("person");
            d["roles"] = json!(["Social Author"]);
        })
        .await;
    let mut pia_account = account("Pia Mastodon", "pia", own);
    pia_account["connected_by"] = json!("pia@brands.test");
    let on_pia = uid(&t.insert(&manager, "SocialAccount", pia_account).await["id"]);
    t.net().notifs.insert("pia".into(), vec![notification(201, "dee", "public", "<p>For Pia</p>", None)]);
    t.net().notif_reads.clear();
    assert!(matches!(t.k.pull_now(&t.admin, own, "mentions").await, Err(Error::NotFound(_))), "there is nobody to pull as before anyone has signed in");
    assert!(t.net().notif_reads.is_empty());
    t.sign_in(&pia, own, "CODE-pia").await;
    t.pull(own, "mentions").await;
    assert_eq!(t.net().notif_reads.iter().map(|r| r.0.as_str()).collect::<HashSet<_>>(), HashSet::from(["pia"]), "with her sign-in, not the workspace's");
    let dee = t.rows("SocialMessage", json!({"author": "dee"})).await;
    assert_eq!(dee.len(), 1, "made as her (she holds the role that creates it)");
    t.pull(own, "mentions").await;
    assert_eq!(t.rows("SocialMessage", json!({"author": "dee"})).await.len(), 1, "read again, not made again");
    assert_eq!(dee[0]["external_id"], "s201", "its id kept, so that it is not made again");
    assert_eq!(uid(&dee[0]["account"]), on_pia, "to the account she connected, not the workspace's");

    // Her numbers are read as her too (the upkeep calls the account's connection as the person who connected it).
    t.upkeep(later + Duration::hours(14)).await;
    assert!(t.net().verified_as.iter().any(|w| w == "pia"), "{:?}", t.net().verified_as);
    assert!(t.rows("SocialMetric", json!({"account": on_pia.to_string()})).await.iter().any(|m| m["followers"] == 321));

    // ───────────── Bluesky-shaped inbox: only replies and mentions, a thread kept for the answer ─────────────
    let notif = |reason: &str, uri: &str, cid: &str, who: &str, text: &str, reply: Option<(&str, &str)>, subject: &str| {
        let mut record = json!({"$type": "app.bsky.feed.post", "text": text});
        if let Some((root, parent)) = reply {
            record["reply"] = json!({"root": {"uri": root, "cid": "bafy-root"}, "parent": {"uri": parent, "cid": "bafy-parent"}});
        }
        json!({"uri": uri, "cid": cid, "reason": reason, "reasonSubject": subject, "author": {"handle": who, "did": format!("did:plc:{who}")}, "record": record})
    };
    let ours = bp["external_id"].as_str().unwrap();
    t.net().bsky_notifs = vec![
        notif("like", "at://did:plc:fan/app.bsky.feed.like/1", "c0", "fan.test", "", None, ours),
        notif("reply", "at://did:plc:ann/app.bsky.feed.post/9", "bafy-ann", "ann.test", "Nice!", Some((ours, ours)), ours),
        notif("mention", "at://did:plc:ben/app.bsky.feed.post/8", "bafy-ben", "ben.test", "@north.bsky.test hi", None, ""),
    ];
    t.pull(bluesky, "notifications").await;
    let bsky_inbox = t.rows("SocialMessage", json!({"external_id": "at://did:plc:ann/app.bsky.feed.post/9"})).await;
    let ann = &bsky_inbox[0];
    assert_eq!((ann["kind"].as_str(), ann["author"].as_str(), ann["text"].as_str()), (Some("Comment"), Some("ann.test"), Some("Nice!")));
    assert_eq!(uid(&ann["account"]), on_bluesky);
    assert_eq!(
        t.rows("SocialMessage", json!({"author": "fan.test"})).await.len(),
        0,
        "a like is not a message: the service was asked for replies and mentions"
    );
    let ben = &t.rows("SocialMessage", json!({"author": "ben.test"})).await[0];
    assert_eq!((ben["kind"].as_str(), uid(&ben["account"])), (Some("Mention"), on_bluesky), "found by the one account its connection reaches");
    assert_eq!(ann["thread"], format!("{ours} bafy-root bafy-ann"), "the thread's root and the comment's own reference, for the answer");
    t.k.run_action(&t.admin, "SocialMessage", uid(&ann["id"]), "reply", json!({"reply": "Thank you!"})).await.unwrap();
    t.work().await;
    {
        let n = t.net();
        let answered = &n.records.last().unwrap()["record"];
        assert_eq!(answered["text"], "Thank you!");
        assert_eq!(
            answered["reply"],
            json!({"root": {"uri": ours, "cid": "bafy-root"}, "parent": {"uri": "at://did:plc:ann/app.bsky.feed.post/9", "cid": "bafy-ann"}})
        );
    }
}
