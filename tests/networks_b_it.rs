//! The network descriptions of docs/connectors (the second batch), each against a fake of the network's documented API.
//!
//! One real PostgreSQL (`FERRUM_SOCIAL_TEST_DATABASE_URL`, reset first), one booted kernel with the Social app, and for
//! each network a fake service on 127.0.0.1 that answers in the shapes of that network's public documentation and
//! CHECKS what it is sent (credentials, headers, bodies, the order of the steps). Each description is the file's own with
//! its hosts replaced by the fake's address. None of this runs against the real networks: a first connection to each is
//! the first time its real answers are read.
//!
//! `NETS=telegram,discord cargo test --test networks_b_it` runs some of them.

#![allow(dead_code)]

use chrono::{Duration, Utc};
use ferrum_kernel::connections::ConnectionInput;
use ferrum_kernel::secret::SecretKey;
use ferrum_kernel::tenant::NewTenant;
use ferrum_kernel::{Ctx, Db, DbConfig, Doc, HookRegistry, Kernel, KernelSettings, ListQuery};
use ferrum_meta::Catalog;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use uuid::Uuid;

const PNG: &[u8] = b"\x89PNG\r\n\x1a\nCAT-PICTURE";
const MP4: &[u8] = b"\x00\x00\x00\x18ftypmp42VIDEO-BYTES";

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

const NETWORKS: [&str; 19] = [
    "telegram",
    "discord",
    "slack",
    "wordpress",
    "medium",
    "devto",
    "hashnode",
    "tumblr",
    "lemmy",
    "vk",
    "twitch",
    "kick",
    "dribbble",
    "whop",
    "mewe",
    "listmonk",
    "moltbook",
    "tiktok_business",
    "farcaster",
];

#[test]
fn the_descriptions_are_ones_the_platform_takes() {
    for name in NETWORKS {
        let Ok(text) = std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("docs/connectors/{name}.json"))) else {
            continue;
        };
        let d: ferrum_kernel::connections::Description = serde_json::from_str(&text).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(d.problem(), None, "{name}");
        assert!(d.apps.iter().any(|a| a == "social") && d.operations.contains_key("publish"), "{name}");
        let social = &d.meta.as_ref().unwrap_or_else(|| panic!("{name} says what its network takes"))["social"];
        assert!(social["name"].is_string() && social["kinds"].is_array(), "{name}");
        // Social's own reading of the rules (which refuses a key it does not know) is the `rules` action, in each scenario.
    }
}

// ───────────── the fakes ─────────────

#[derive(Clone, Debug)]
struct Req {
    method: String,
    /// Path and query, as sent.
    target: String,
    headers: HashMap<String, String>,
    body: Vec<u8>,
}

impl Req {
    fn path(&self) -> &str {
        self.target.split('?').next().unwrap_or_default()
    }
    fn query(&self) -> HashMap<String, String> {
        form(self.target.split_once('?').map(|(_, q)| q).unwrap_or_default()).into_iter().collect()
    }
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).to_string()
    }
    fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or(Value::Null)
    }
    fn form(&self) -> HashMap<String, String> {
        form(&self.text()).into_iter().collect()
    }
    fn header(&self, name: &str) -> &str {
        self.headers.get(name).map(String::as_str).unwrap_or_default()
    }
    /// The parts of a multipart body: (name, filename, content type, content).
    fn parts(&self) -> Vec<Part> {
        let kind = self.header("content-type");
        let Some(boundary) = kind.split("boundary=").nth(1) else { return vec![] };
        let body = self.body.clone();
        let marker = format!("--{boundary}").into_bytes();
        let mut out = vec![];
        let mut at = 0;
        let mut starts = vec![];
        while let Some(p) = find(&body[at..], &marker) {
            starts.push(at + p);
            at += p + marker.len();
        }
        for w in starts.windows(2) {
            let chunk = &body[w[0] + marker.len()..w[1]];
            let Some(split) = find(chunk, b"\r\n\r\n") else { continue };
            let head = String::from_utf8_lossy(&chunk[..split]).to_string();
            let mut content = chunk[split + 4..].to_vec();
            if content.ends_with(b"\r\n") {
                content.truncate(content.len() - 2);
            }
            let field = |what: &str| head.split(what).nth(1).and_then(|r| r.split('"').next()).map(str::to_string);
            let kind = head.lines().find_map(|l| l.strip_prefix("Content-Type: ").or(l.strip_prefix("content-type: "))).map(str::to_string);
            out.push(Part { name: field("name=\"").unwrap_or_default(), filename: field("filename=\""), kind, content });
        }
        out
    }
    fn part(&self, name: &str) -> Option<Part> {
        self.parts().into_iter().find(|p| p.name == name)
    }
}

#[derive(Clone, Debug)]
struct Part {
    name: String,
    filename: Option<String>,
    kind: Option<String>,
    content: Vec<u8>,
}

impl Part {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.content).to_string()
    }
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

fn form(q: &str) -> Vec<(String, String)> {
    reqwest::Url::parse(&format!("http://f/?{q}")).unwrap().query_pairs().map(|(k, v)| (k.into_owned(), v.into_owned())).collect()
}

enum Resp {
    Answer(u16, Vec<(String, String)>, String),
    /// The connection is closed without an answer: the request was received, and the sender never learns what came of it.
    Lost,
}

fn ok(v: Value) -> Resp {
    Resp::Answer(200, vec![], v.to_string())
}

fn status(code: u16, v: Value) -> Resp {
    Resp::Answer(code, vec![], v.to_string())
}

fn with_headers(code: u16, headers: &[(&str, &str)], body: &str) -> Resp {
    Resp::Answer(code, headers.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(), body.to_string())
}

/// A fake service: every request it was sent, in order, and the address it answers on.
#[derive(Clone)]
struct Fake {
    base: String,
    log: Arc<Mutex<Vec<Req>>>,
}

impl Fake {
    fn seen(&self) -> Vec<Req> {
        self.log.lock().unwrap().clone()
    }
    /// The requests whose path starts with `prefix` (after any `strip`), oldest first.
    fn to(&self, method: &str, contains: &str) -> Vec<Req> {
        self.seen().into_iter().filter(|r| r.method == method && r.path().contains(contains)).collect()
    }
    fn clear(&self) {
        self.log.lock().unwrap().clear();
    }
    /// "METHOD path" of every request, in order.
    fn order(&self) -> Vec<String> {
        self.seen().iter().map(|r| format!("{} {}", r.method, r.path())).collect()
    }
}

async fn fake(mut handler: impl FnMut(&Req) -> Resp + Send + 'static) -> Fake {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let log: Arc<Mutex<Vec<Req>>> = Arc::default();
    let handler: Arc<Mutex<Box<dyn FnMut(&Req) -> Resp + Send>>> = Arc::new(Mutex::new(Box::new(move |r| handler(r))));
    let logged = log.clone();
    tokio::spawn(async move {
        loop {
            let Ok((mut s, _)) = listener.accept().await else { return };
            let (handler, log) = (handler.clone(), logged.clone());
            tokio::spawn(async move {
                let mut buf = Vec::new();
                let mut chunk = [0u8; 16384];
                loop {
                    let n = s.read(&mut chunk).await.unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    buf.extend_from_slice(&chunk[..n]);
                    if let Some(end) = find(&buf, b"\r\n\r\n") {
                        let head = String::from_utf8_lossy(&buf[..end]).to_ascii_lowercase();
                        let len = head.lines().find_map(|l| l.strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0))).unwrap_or(0);
                        if buf.len() >= end + 4 + len {
                            break;
                        }
                    }
                }
                let end = find(&buf, b"\r\n\r\n").unwrap_or(buf.len());
                let head = String::from_utf8_lossy(&buf[..end]).to_string();
                let body = buf.get(end + 4..).unwrap_or_default().to_vec();
                let mut lines = head.lines();
                let mut first = lines.next().unwrap_or_default().split_whitespace();
                let (method, target) = (first.next().unwrap_or_default().to_string(), first.next().unwrap_or("/").to_string());
                let headers: HashMap<String, String> =
                    lines.filter_map(|l| l.split_once(':').map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string()))).collect();
                let req = Req { method, target, headers, body };
                log.lock().unwrap().push(req.clone());
                let answer = (handler.lock().unwrap())(&req);
                match answer {
                    Resp::Lost => {
                        let _ = s.shutdown().await;
                    }
                    Resp::Answer(code, extra, reply) => {
                        let mut out =
                            format!("HTTP/1.1 {code} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n", reply.len());
                        for (k, v) in extra {
                            out.push_str(&format!("{k}: {v}\r\n"));
                        }
                        out.push_str(&format!("\r\n{reply}"));
                        let _ = s.write_all(out.as_bytes()).await;
                    }
                }
            });
        }
    });
    Fake { base: format!("http://127.0.0.1:{port}"), log }
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
        self.0.lock().unwrap().get(key).cloned().ok_or_else(|| ferrum_kernel::Error::NotFound("object".into()))
    }
    async fn get_range(&self, key: &str, range: std::ops::Range<u64>) -> ferrum_kernel::Result<Vec<u8>> {
        let all = self.get(key).await?;
        Ok(all[(range.start as usize).min(all.len())..(range.end as usize).min(all.len())].to_vec())
    }
    async fn delete(&self, key: &str) -> ferrum_kernel::Result<()> {
        self.0.lock().unwrap().remove(key);
        Ok(())
    }
}

// ───────────── the workspace ─────────────

struct T {
    k: Arc<Kernel>,
    tenant: Uuid,
    admin: Ctx,
    manager: Ctx,
    author: Ctx,
    approver: Ctx,
    brand: Uuid,
    png: Value,
    mp4: Value,
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
    async fn upkeep(&self) {
        let now = Utc::now() + Duration::hours(3);
        ferrum_app_social::maintenance_at(&self.k, now).await.expect("upkeep");
        self.work().await;
        ferrum_app_social::maintenance_at(&self.k, now).await.expect("upkeep settles");
    }
    async fn act(&self, who: &Ctx, entity: &str, id: Uuid, action: &str, params: Value) -> ferrum_kernel::Result<Value> {
        self.k.run_action(who, entity, id, action, params).await
    }
    async fn sign_in(&self, conn: Uuid, code: &str) {
        let session = Uuid::now_v7();
        let url = self.k.connect_start(&self.admin, conn, session).await.unwrap();
        let state = reqwest::Url::parse(&url).unwrap().query_pairs().find(|(k, _)| k == "state").unwrap().1.to_string();
        self.k.connect_finish(&self.admin, session, &state, Some(code), None).await.unwrap();
    }
    /// A connection from the file `file`, its hosts (`from` → the fake's address) put where the real ones are.
    async fn connect(&self, name: &str, file: &str, hosts: &[(&str, &str)], secrets: Value, edit: impl FnOnce(&mut Value)) -> Uuid {
        let mut text = example(file);
        for (from, to) in hosts {
            text = text.replace(from, to);
        }
        let mut d: Value = serde_json::from_str(&text).unwrap();
        edit(&mut d);
        let input: ConnectionInput =
            serde_json::from_value(json!({"name": name, "label": name, "description": d, "secrets": secrets, "enabled": true})).unwrap();
        self.k.save_connection_def(&self.admin, None, &input).await.unwrap_or_else(|e| panic!("{name}: {e:?}"))
    }
    /// An account on the connection, its id on the network picked from what the sign-in reaches (`reachable`, `choose`).
    async fn account(&self, name: &str, handle: &str, conn: Uuid, network_id: &str) -> Uuid {
        let a = uid(&self
            .insert(
                &self.manager,
                "SocialAccount",
                json!({"account_name": name, "brand": self.brand.to_string(), "handle": handle, "connection": conn.to_string()}),
            )
            .await["id"]);
        let reached = self.act(&self.manager, "SocialAccount", a, "reachable", Value::Null).await.unwrap_or_else(|e| panic!("{name}: reachable: {e:?}"));
        assert!(reached["accounts"].as_array().unwrap().iter().any(|x| x["id"] == network_id), "{name}: the sign-in reaches {network_id}: {reached}");
        self.act(&self.manager, "SocialAccount", a, "choose", json!({"picked": [network_id]})).await.unwrap_or_else(|e| panic!("{name}: choose: {e:?}"));
        let row = self.get("SocialAccount", a).await;
        assert_eq!(row["network_id"], network_id, "{name}");
        a
    }
    /// A post to `targets` (each `{account, kind?, settings?}`), saved, sent for review and approved.
    async fn post(&self, text: &str, targets: Vec<Value>, extra: Value) -> Uuid {
        let mut p = json!({"headline": "Spring launch", "brand": self.brand.to_string(), "text": text,
                           "scheduled_at": (Utc::now() + Duration::hours(1)).to_rfc3339(), "targets": targets});
        for (k, v) in extra.as_object().unwrap() {
            p[k] = v.clone();
        }
        let id = uid(&self.insert(&self.author, "SocialPost", p).await["id"]);
        self.act(&self.author, "SocialPost", id, "send_for_review", Value::Null).await.unwrap_or_else(|e| panic!("send_for_review: {e:?}"));
        self.k.decide_approval(&self.approver, "SocialPost", id, true, None).await.unwrap();
        id
    }
    /// The same, refused at review: what it said.
    async fn refused(&self, text: &str, targets: Vec<Value>, extra: Value) -> String {
        let mut p = json!({"headline": "Spring launch", "brand": self.brand.to_string(), "text": text,
                           "scheduled_at": (Utc::now() + Duration::hours(1)).to_rfc3339(), "targets": targets});
        for (k, v) in extra.as_object().unwrap() {
            p[k] = v.clone();
        }
        let id = uid(&self.insert(&self.author, "SocialPost", p).await["id"]);
        self.act(&self.author, "SocialPost", id, "send_for_review", Value::Null).await.unwrap_err().to_string()
    }
    /// Sends it and gives its publication on the account.
    async fn out(&self, post: Uuid, account: Uuid) -> Doc {
        self.upkeep().await;
        self.publication(post, account).await
    }
    async fn publication(&self, post: Uuid, account: Uuid) -> Doc {
        self.rows("SocialPublication", json!({"post": post.to_string()})).await.into_iter().find(|p| uid(&p["account"]) == account).expect("a publication")
    }
    /// What the account's network takes and asks, as Social reads the description's `meta.social`.
    async fn rules(&self, account: Uuid) -> Value {
        let r = self.act(&self.author, "SocialAccount", account, "rules", Value::Null).await.unwrap_or_else(|e| panic!("rules: {e:?}"));
        r["rules"].clone()
    }
    async fn delete_remote(&self, publication: &Doc) {
        self.act(&self.manager, "SocialPublication", uid(&publication["id"]), "delete_remote", Value::Null).await.unwrap_or_else(|e| panic!("delete: {e:?}"));
    }
    fn media(&self, which: &Value, alt: &str) -> Value {
        json!({"item": 1, "file": which["url"], "alt": alt})
    }
}

fn target(account: Uuid) -> Value {
    json!({"account": account.to_string()})
}

fn target_with(account: Uuid, kind: &str, settings: Value) -> Value {
    json!({"account": account.to_string(), "kind": kind, "settings": settings})
}

fn wanted(name: &str) -> bool {
    std::env::var("NETS").map(|n| n.split(',').any(|x| x.trim() == name)).unwrap_or(true)
}

#[test]
fn the_networks_end_to_end() {
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
    let mut t = T {
        k: k.clone(),
        tenant,
        admin: admin.clone(),
        manager: admin.clone(),
        author: admin.clone(),
        approver: admin,
        brand: Uuid::nil(),
        png: Value::Null,
        mp4: Value::Null,
    };
    t.author = t.user("author@brands.test", &["Social Author"]).await;
    t.approver = t.user("approver@brands.test", &["Social Approver"]).await;
    t.manager = t.user("manager@brands.test", &["Social Manager"]).await;
    t.brand = uid(&t.insert(&t.manager, "SocialBrand", json!({"brand_name": "Northwind"})).await["id"]);
    t.png = t.k.store_file(&t.author, None, "cat.png", "image/png", PNG).await.unwrap();
    t.mp4 = t.k.store_file(&t.author, None, "clip.mp4", "video/mp4", MP4).await.unwrap();

    if wanted("telegram") {
        telegram(&t).await;
    }
}

// ───────────── Telegram ─────────────

const TG_TOKEN: &str = "123456:ABC-def_token";

async fn telegram(t: &T) {
    // Bot API shapes (core.telegram.org/bots/api): every method under /bot<token>/, JSON or multipart in, `{ok, result}` or
    // `{ok: false, error_code, description}` out.
    let sent: Arc<Mutex<Vec<Value>>> = Arc::default();
    let lose_next: Arc<Mutex<Option<String>>> = Arc::default();
    let (s2, l2) = (sent.clone(), lose_next.clone());
    let net = fake(move |r| {
        let Some(rest) = r.path().strip_prefix(&format!("/bot{TG_TOKEN}/")) else {
            return status(401, json!({"ok": false, "error_code": 401, "description": "Unauthorized"}));
        };
        let mut sent = s2.lock().unwrap();
        let mut message = |chat: &str, extra: Value| -> Resp {
            if chat == "-999" {
                return status(400, json!({"ok": false, "error_code": 400, "description": "Bad Request: chat not found"}));
            }
            let id = 100 + sent.len() as i64;
            let mut m = json!({"message_id": id, "chat": chat});
            m.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
            sent.push(m);
            if l2.lock().unwrap().as_deref() == Some(extra["text"].as_str().unwrap_or("")) {
                *l2.lock().unwrap() = None;
                return Resp::Lost;
            }
            ok(json!({"ok": true, "result": {"message_id": id, "chat": {"id": chat}}}))
        };
        match (r.method.as_str(), rest) {
            ("GET", "getMe") => ok(json!({"ok": true, "result": {"id": 123456, "is_bot": true, "username": "northwind_bot"}})),
            ("GET", "getUpdates") => ok(json!({"ok": true, "result": [
                {"update_id": 1, "my_chat_member": {"chat": {"id": -100123, "title": "Northwind News", "type": "channel"}}},
                {"update_id": 2, "message": {"text": "hello", "chat": {"id": 7}}},
                {"update_id": 3, "my_chat_member": {"chat": {"id": -999, "title": "Gone chat", "type": "group"}}}]})),
            ("GET", "getChat") => ok(json!({"ok": true, "result": {"id": -100123, "title": "Northwind News", "username": "northwind_news"}})),
            ("GET", "getChatMemberCount") => ok(json!({"ok": true, "result": 4321})),
            ("POST", "sendMessage") => {
                let b = r.json();
                if r.header("content-type") != "application/json" {
                    return status(400, json!({"ok": false, "error_code": 400, "description": "Bad Request: not json"}));
                }
                message(b["chat_id"].as_str().unwrap_or_default(), json!({"text": b["text"], "parse_mode": b["parse_mode"], "reply_to": b["reply_parameters"]["message_id"]}))
            }
            ("POST", m @ ("sendPhoto" | "sendVideo" | "sendDocument")) => {
                let field = m.trim_start_matches("send").to_lowercase();
                let Some(file) = r.part(&field) else { return status(400, json!({"ok": false, "error_code": 400, "description": "Bad Request: there is no media in the request"})) };
                let chat = r.part("chat_id").map(|p| p.text()).unwrap_or_default();
                message(&chat, json!({"text": r.part("caption").map(|p| p.text()), "file": file.text().len(), "bytes": String::from_utf8_lossy(&file.content), "filename": file.filename, "type": file.kind, "via": m}))
            }
            ("POST", "deleteMessages") => {
                let b = r.json();
                let ids: Vec<i64> = b["message_ids"].as_array().unwrap().iter().map(|v| v.as_i64().expect("integers")).collect();
                sent.retain(|m| !(b["chat_id"] == m["chat"] && ids.contains(&m["message_id"].as_i64().unwrap())));
                ok(json!({"ok": true, "result": true}))
            }
            _ => status(404, json!({"ok": false, "error_code": 404, "description": "Not Found"})),
        }
    })
    .await;
    let conn = t.connect("telegram_north", "telegram", &[("https://api.telegram.org", &net.base)], json!({"bot_token": TG_TOKEN}), |_| {}).await;
    // Connecting: the chats the bot was added to are what the sign-in reaches.
    let news = t.account("Northwind News", "northwind_news", conn, "-100123").await;
    let rules = t.rules(news).await;
    assert_eq!((rules["name"].as_str(), rules["text"]["format"].as_str(), rules["text"]["max"].as_u64()), (Some("Telegram"), Some("html"), Some(4096)));
    assert!(net.seen().iter().all(|r| r.path().starts_with(&format!("/bot{TG_TOKEN}/"))), "the token is in the path, as Telegram wants it, colon and all");

    // A text post: HTML, the link after the words, to the chat.
    let p = t.post("<b>Spring</b> is here", vec![target(news)], json!({"link": "https://northwind.example/spring"})).await;
    let publication = t.out(p, news).await;
    assert_eq!(publication["external_id"], "-100123:100", "the chat and the message, which is what deleting needs");
    {
        let m = sent.lock().unwrap();
        assert_eq!((m[0]["text"].as_str(), m[0]["parse_mode"].as_str()), (Some("<b>Spring</b> is here https://northwind.example/spring"), Some("HTML")));
        assert_eq!(m[0]["chat"], "-100123");
    }
    assert_eq!(net.to("POST", "sendMessage").len(), 1);

    // Two pictures: the file as the part, the words as the first one's caption, none on the second.
    net.clear();
    let p = t.post("Two views", vec![target_with(news, "media", json!({}))], json!({"media": [t.media(&t.png, "a"), t.media(&t.png, "b")]})).await;
    let pics = t.out(p, news).await;
    assert_eq!(pics["external_id"], "-100123:101,102");
    let photos = net.to("POST", "sendPhoto");
    assert_eq!(photos.len(), 2);
    let (a, b) = (&photos[0], &photos[1]);
    assert!(a.header("content-type").starts_with("multipart/form-data"));
    assert_eq!(a.part("photo").unwrap().content, PNG, "the stored file, byte for byte");
    assert_eq!((a.part("caption").unwrap().text(), b.part("caption").unwrap().text()), ("Two views".to_string(), String::new()));
    assert_eq!(a.part("chat_id").unwrap().text(), "-100123");

    // A video, then a follow-up that answers the first message.
    net.clear();
    let p = t
        .post(
            "Clip",
            vec![target_with(news, "media", json!({}))],
            json!({"media": [json!({"item": 1, "file": t.mp4["url"]})], "items": [{"text": "More soon"}]}),
        )
        .await;
    let clip = t.out(p, news).await;
    assert_eq!(net.to("POST", "sendVideo").len(), 1);
    let reply = net.to("POST", "sendMessage")[0].json();
    assert_eq!(
        (reply["text"].as_str(), reply["reply_parameters"]["message_id"].as_i64()),
        (Some("More soon"), Some(103)),
        "answering the message before it: {reply}"
    );
    assert_eq!(clip["sent_items"].as_array().unwrap().len(), 2);

    // A refusal, in the words Telegram gives.
    let gone = t.account("Gone chat", "gone", conn, "-999").await;
    let p = t.post("Anyone there?", vec![target(gone)], json!({})).await;
    t.upkeep().await;
    let post = t.get("SocialPost", p).await;
    assert_eq!(post["status"], "Failed");
    assert!(post["failure"].as_str().unwrap().contains("Bad Request: chat not found"), "{post:?}");

    // A lost answer: the message may exist, so it is not sent again and the call says so.
    *lose_next.lock().unwrap() = Some("Lost one".into());
    net.clear();
    let p = t.post("Lost one", vec![target(news)], json!({})).await;
    t.upkeep().await;
    assert_eq!(net.to("POST", "sendMessage").len(), 1, "one try; its answer never came; no second send");
    assert_eq!(sent.lock().unwrap().iter().filter(|m| m["text"] == "Lost one").count(), 1);
    let post = t.get("SocialPost", p).await;
    assert_eq!(post["status"], "Failed");
    assert!(post["failure"].as_str().unwrap().contains("may have been sent"), "{post:?}");

    // Delete: every message the item made, in one call.
    net.clear();
    t.delete_remote(&pics).await;
    let del = net.to("POST", "deleteMessages")[0].json();
    assert_eq!(del, json!({"chat_id": "-100123", "message_ids": [101, 102]}));

    // Numbers: the members, and the chat's name.
    net.clear();
    let checked = t.act(&t.manager, "SocialAccount", news, "check", Value::Null).await.unwrap();
    assert_eq!(checked["read"]["display_name"], "Northwind News", "{checked}");
}
