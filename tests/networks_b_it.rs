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
    // On a thread with the stack the server's workers have: the larger descriptions are deep for the default 2 MB.
    std::thread::Builder::new().stack_size(16 << 20).spawn(shapes).unwrap().join().unwrap();
}

fn shapes() {
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
    /// The upkeep seven hours on: what reads numbers has come due.
    async fn later_upkeep(&self) {
        self.upkeep().await;
        ferrum_app_social::maintenance_at(&self.k, Utc::now() + Duration::hours(12)).await.expect("upkeep");
        self.work().await;
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
    /// Signs in as `code` stands for at the fake provider; gives the address the browser was sent to.
    async fn sign_in(&self, conn: Uuid, code: &str) -> reqwest::Url {
        let session = Uuid::now_v7();
        let url = reqwest::Url::parse(&self.k.connect_start(&self.admin, conn, session).await.unwrap()).unwrap();
        let state = url.query_pairs().find(|(k, _)| k == "state").unwrap().1.to_string();
        self.k.connect_finish(&self.admin, session, &state, Some(code), None).await.unwrap();
        url
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
        let p = self.publication(post, account).await;
        assert!(p["sent_at"].is_string(), "it did not go out: {:?} / {:?}", p["failure"], self.get("SocialPost", post).await["failure"]);
        p
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

/// Standard base64, for checking a basic-auth header.
fn b64(bytes: &[u8]) -> String {
    const A: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for c in bytes.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            if i <= c.len() {
                out.push(A[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
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
    if wanted("discord") {
        discord(&t).await;
    }
    if wanted("slack") {
        slack(&t).await;
    }
    if wanted("wordpress") {
        wordpress(&t).await;
    }
    if wanted("medium") {
        medium(&t).await;
    }
    if wanted("devto") {
        devto(&t).await;
    }
    if wanted("hashnode") {
        hashnode(&t).await;
    }
    if wanted("tumblr") {
        tumblr(&t).await;
    }
    if wanted("lemmy") {
        lemmy(&t).await;
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

// ───────────── Discord ─────────────

const DC_TOKEN: &str = "MTA.bot.token";

async fn discord(t: &T) {
    // REST v10 (discord.com/developers/docs): `Authorization: Bot <token>`, a User-Agent, JSON or multipart (a
    // `payload_json` part and `files[n]`), a `nonce` with `enforce_nonce` to make a message once, a thread of a message
    // that has the message's own id.
    let messages: Arc<Mutex<Vec<Value>>> = Arc::default();
    let threads: Arc<Mutex<Vec<String>>> = Arc::default();
    let lose: Arc<Mutex<Option<String>>> = Arc::default();
    let (m2, th2, l2) = (messages.clone(), threads.clone(), lose.clone());
    let net = fake(move |r| {
        if r.header("authorization") != format!("Bot {DC_TOKEN}") || !r.header("user-agent").starts_with("DiscordBot (") {
            return status(401, json!({"message": "401: Unauthorized", "code": 0}));
        }
        let Some(path) = r.path().strip_prefix("/api/v10") else { return status(404, json!({"message": "404: Not Found", "code": 0})) };
        let seg: Vec<&str> = path.trim_matches('/').split('/').collect();
        let mut messages = m2.lock().unwrap();
        match (r.method.as_str(), seg.as_slice()) {
            ("GET", ["users", "@me"]) => ok(json!({"id": "B1", "username": "northwind-bot", "bot": true})),
            ("GET", ["users", "@me", "guilds"]) => {
                ok(json!([{"id": "G1", "name": "Northwind HQ", "icon": "abc123"}, {"id": "G2", "name": "Elsewhere", "icon": null}]))
            }
            ("GET", ["guilds", "G1"]) => ok(json!({"id": "G1", "name": "Northwind HQ", "approximate_member_count": 250})),
            ("GET", ["guilds", "G1", "channels"]) => ok(json!([
                {"id": "C0", "type": 4, "name": "Info"}, {"id": "C1", "type": 0, "name": "announcements", "parent_id": "C0"},
                {"id": "C2", "type": 2, "name": "Voice"}, {"id": "C3", "type": 5, "name": "news"}])),
            ("GET", ["guilds", "G1", "members", "search"]) => {
                assert_eq!(r.query().get("query").map(String::as_str), Some("an"));
                ok(json!([{"user": {"id": "U9", "username": "ana", "global_name": "Ana Lima"}, "nick": null}]))
            }
            ("GET", ["channels", id]) => {
                if th2.lock().unwrap().iter().any(|x| x == id) {
                    ok(json!({"id": id, "type": 11}))
                } else {
                    status(404, json!({"message": "Unknown Channel", "code": 10003}))
                }
            }
            ("POST", ["channels", c, "messages", m, "threads"]) => {
                let mut th = th2.lock().unwrap();
                if th.iter().any(|x| x == m) {
                    return status(400, json!({"message": "A thread has already been created for this message", "code": 160004}));
                }
                assert!(messages.iter().any(|x| x["id"] == *m && x["channel_id"] == *c), "a thread is made from a message that is there");
                assert_eq!(r.json()["auto_archive_duration"], 1440);
                th.push(m.to_string());
                ok(json!({"id": m, "type": 11}))
            }
            ("POST", ["channels", c, "messages"]) => {
                let allowed = ["C1", "C2"].contains(c) || th2.lock().unwrap().iter().any(|x| x == c);
                if !allowed {
                    return status(403, json!({"message": "Missing Permissions", "code": 50013}));
                }
                let (body, files) = if r.header("content-type").starts_with("multipart/form-data") {
                    let payload: Value = serde_json::from_str(&r.part("payload_json").expect("payload_json").text()).unwrap();
                    let files: Vec<Value> = (0..10)
                        .filter_map(|i| r.part(&format!("files[{i}]")))
                        .map(|p| json!({"filename": p.filename, "type": p.kind, "same": p.content == PNG}))
                        .collect();
                    (payload, files)
                } else {
                    assert_eq!(r.header("content-type"), "application/json");
                    (r.json(), vec![])
                };
                let nonce = body["nonce"].as_str().unwrap_or_default().to_string();
                assert!(!nonce.is_empty() && nonce.len() <= 25 && body["enforce_nonce"] == true, "{body}");
                if let Some(old) = messages.iter().find(|x| x["nonce"] == nonce.as_str() && x["channel_id"] == *c) {
                    return ok(old.clone());
                }
                let id = format!("M{}", messages.len() + 1);
                let msg = json!({"id": id, "channel_id": c, "content": body["content"], "nonce": nonce, "attachments": body["attachments"], "files": files,
                                 "reference": body["message_reference"]["message_id"], "mentions": body["allowed_mentions"]});
                messages.push(msg.clone());
                if l2.lock().unwrap().as_deref() == body["content"].as_str() {
                    *l2.lock().unwrap() = None;
                    return Resp::Lost;
                }
                ok(msg)
            }
            ("DELETE", ["channels", c, "messages", m]) => {
                messages.retain(|x| !(x["id"] == *m && x["channel_id"] == *c));
                Resp::Answer(204, vec![], String::new())
            }
            _ => status(404, json!({"message": "404: Not Found", "code": 0})),
        }
    })
    .await;
    let conn = t.connect("discord_north", "discord", &[("https://discord.com", &net.base)], json!({"bot_token": DC_TOKEN}), |_| {}).await;
    // Connecting: the servers the bot is in; the server's channels are the choices (text and announcement ones only).
    let hq = t.account("Northwind Discord", "northwind", conn, "G1").await;
    let rules = t.rules(hq).await;
    assert_eq!((rules["name"].as_str(), rules["text"]["format"].as_str(), rules["text"]["max"].as_u64()), (Some("Discord"), Some("markdown"), Some(2000)));
    let reached = t.act(&t.manager, "SocialAccount", hq, "reachable", Value::Null).await.unwrap();
    assert_eq!(reached["accounts"][0]["picture"], "https://cdn.discordapp.com/icons/G1/abc123.png");
    assert_eq!(reached["accounts"][1]["picture"], Value::Null);
    let channels = t.act(&t.author, "SocialAccount", hq, "choices", json!({"setting": "channel"})).await.unwrap();
    assert_eq!(channels["choices"], json!([{"value": "C1", "label": "announcements"}, {"value": "C3", "label": "news"}]), "no category and no voice channel");
    let found = t.act(&t.author, "SocialAccount", hq, "mentions", json!({"query": "an"})).await.unwrap();
    assert_eq!(found["mentions"][0]["text"], "<@U9>");
    let on = |channel: &str| target_with(hq, "message", json!({"channel": channel}));
    let e = t.refused("No channel", vec![target(hq)], json!({})).await;
    assert!(e.contains("Channel is needed"), "{e}");

    // A text post: markdown as it is, the link after the words, one message once (the nonce).
    let p = t.post("**Spring** is here", vec![on("C1")], json!({"link": "https://northwind.example/spring"})).await;
    let first = t.out(p, hq).await;
    assert_eq!(first["external_id"], "C1:M1");
    {
        let m = messages.lock().unwrap();
        assert_eq!(m[0]["content"], "**Spring** is here https://northwind.example/spring");
        assert_eq!(m[0]["mentions"], json!({"parse": ["users"]}), "never @everyone");
    }

    // Pictures: one request, a payload_json and a files[n] part for each, the description of each.
    net.clear();
    let p = t.post("Two views", vec![on("C1")], json!({"media": [t.media(&t.png, "front"), t.media(&t.png, "back")]})).await;
    let pics = t.out(p, hq).await;
    let sends: Vec<Req> = net.to("POST", "/messages");
    assert_eq!(sends.len(), 1, "both files in the one message: {:?}", net.order());
    assert!(sends[0].part("files[0]").unwrap().content == PNG && sends[0].part("files[1]").unwrap().content == PNG);
    let payload: Value = serde_json::from_str(&sends[0].part("payload_json").unwrap().text()).unwrap();
    assert_eq!(payload["attachments"], json!([{"id": 0, "description": "front"}, {"id": 1, "description": "back"}]));
    assert_eq!(payload["content"], "Two views");
    assert_eq!(pics["external_id"], "C1:M2");

    // Replies as a thread of the first message: made once, then each answer posted in it.
    net.clear();
    let p = t.post("Launch", vec![on("C1")], json!({"items": [{"text": "Details"}, {"text": "More"}]})).await;
    let threaded = t.out(p, hq).await;
    let m = messages.lock().unwrap().clone();
    let root = m.iter().find(|x| x["content"] == "Launch").unwrap()["id"].as_str().unwrap().to_string();
    assert_eq!(net.to("POST", "/threads").len(), 1, "one thread: {:?}", net.order());
    let in_thread: Vec<&Value> = m.iter().filter(|x| x["channel_id"] == root.as_str()).collect();
    assert_eq!(in_thread.iter().map(|x| x["content"].as_str().unwrap()).collect::<Vec<_>>(), ["Details", "More"]);
    assert_eq!(threaded["sent_items"].as_array().unwrap().len(), 3);

    // A lost answer: the message was made, and sent again under the same nonce it is the same message.
    *lose.lock().unwrap() = Some("Lost one".into());
    net.clear();
    let p = t.post("Lost one", vec![on("C1")], json!({})).await;
    let kept = t.out(p, hq).await;
    assert!(kept["sent_at"].is_string(), "{kept:?}");
    assert_eq!(messages.lock().unwrap().iter().filter(|x| x["content"] == "Lost one").count(), 1, "one message");
    let tries = net.to("POST", "/messages");
    assert_eq!(tries.len(), 2, "sent again after the answer was lost");
    assert_eq!(tries[0].json()["nonce"], tries[1].json()["nonce"]);

    // A refusal, in Discord's words.
    let p = t.post("Not here", vec![on("C3")], json!({})).await;
    t.upkeep().await;
    let post = t.get("SocialPost", p).await;
    assert!(post["failure"].as_str().unwrap().contains("Missing Permissions"), "{post:?}");

    // A first comment: an answer in the channel to the first message.
    let p = t.post("Question?", vec![on("C1")], json!({"first_comment": "Answer here"})).await;
    t.out(p, hq).await;
    let m = messages.lock().unwrap().clone();
    let asked = m.iter().find(|x| x["content"] == "Question?").unwrap();
    let answer = m.iter().find(|x| x["content"] == "Answer here").unwrap();
    assert_eq!((answer["channel_id"].clone(), answer["reference"].clone()), (asked["channel_id"].clone(), asked["id"].clone()));

    // Delete: by channel and message.
    net.clear();
    t.delete_remote(&first).await;
    assert_eq!(net.order(), ["DELETE /api/v10/channels/C1/messages/M1"]);
    let checked = t.act(&t.manager, "SocialAccount", hq, "check", Value::Null).await.unwrap();
    assert_eq!(checked["read"]["display_name"], "Northwind HQ", "{checked}");
    let n = t.rows("SocialMetric", json!({})).await;
    let _ = n;
}

// ───────────── Slack ─────────────

async fn slack(t: &T) {
    // Web API (api.slack.com/methods): always 200 with `ok`, a bearer token, OAuth v2, uploads
    // by the external-upload flow (a slot, the bytes, the completion), a message's `metadata` read back by
    // conversations.history.
    let messages: Arc<Mutex<Vec<Value>>> = Arc::default();
    let files: Arc<Mutex<HashMap<String, Value>>> = Arc::default();
    let lose: Arc<Mutex<Option<String>>> = Arc::default();
    let (m2, f2, l2) = (messages.clone(), files.clone(), lose.clone());
    let base: Arc<Mutex<String>> = Arc::default();
    let b2 = base.clone();
    let net = fake(move |r| {
        let no = |e: &str| ok(json!({"ok": false, "error": e}));
        let mut messages = m2.lock().unwrap();
        let mut files = f2.lock().unwrap();
        if r.path() == "/api/oauth.v2.access" {
            let f = r.form();
            if f.get("code").map(String::as_str) != Some("CODE-north") || f.get("client_secret").map(String::as_str) != Some("slack-client-secret") || !f.contains_key("client_id") {
                return no("invalid_code");
            }
            return ok(json!({"ok": true, "access_token": "xoxb-1", "token_type": "bot", "scope": "chat:write", "bot_user_id": "UB",
                             "team": {"id": "T1", "name": "Northwind"}, "authed_user": {"id": "U1"}}));
        }
        if let Some(id) = r.path().strip_prefix("/upload/") {
            assert!(r.header("authorization").is_empty(), "the upload address is signed already: no credentials");
            assert_eq!(r.body, PNG, "the bytes are the body");
            files.get_mut(id).expect("a slot").as_object_mut().unwrap().insert("bytes".into(), json!(r.body.len()));
            return Resp::Answer(200, vec![], format!("OK - {}", r.body.len()));
        }
        if r.header("authorization") != "Bearer xoxb-1" {
            return no("invalid_auth");
        }
        let q = r.query();
        match (r.method.as_str(), r.path()) {
            ("GET", "/api/auth.test") => ok(json!({"ok": true, "team_id": "T1", "team": "Northwind", "user": "northwind_bot"})),
            ("GET", "/api/conversations.list") => ok(json!({"ok": true, "channels": [{"id": "C1", "name": "general", "is_member": true}, {"id": "C2", "name": "launches"}]})),
            ("GET", "/api/users.list") => ok(json!({"ok": true, "members": [
                {"id": "U9", "name": "ana", "real_name": "Ana Lima", "deleted": false}, {"id": "U8", "name": "bot", "real_name": "Robot", "is_bot": true, "deleted": false}]})),
            ("GET", "/api/files.getUploadURLExternal") => {
                let id = format!("F{}", files.len() + 1);
                files.insert(id.clone(), json!({"name": q.get("filename"), "length": q.get("length")}));
                ok(json!({"ok": true, "upload_url": format!("{}/upload/{id}", b2.lock().unwrap()), "file_id": id}))
            }
            ("POST", "/api/files.completeUploadExternal") => {
                let b = r.json();
                let id = b["files"][0]["id"].as_str().unwrap().to_string();
                let f = files.get_mut(&id).expect("a slot");
                assert!(f["bytes"].is_number(), "the bytes came before the completion");
                f.as_object_mut().unwrap().insert("title".into(), b["files"][0]["title"].clone());
                ok(json!({"ok": true, "files": [{"id": id, "title": b["files"][0]["title"]}]}))
            }
            ("GET", "/api/files.info") => match files.get(q.get("file").map(String::as_str).unwrap_or_default()) {
                Some(f) if f["title"].is_string() => ok(json!({"ok": true, "file": {"id": q["file"], "title": f["title"]}})),
                _ => no("file_not_found"),
            },
            ("POST", "/api/chat.postMessage") => {
                assert_eq!(r.header("content-type"), "application/json");
                let b = r.json();
                if !["C1", "C2"].contains(&b["channel"].as_str().unwrap_or_default()) {
                    return no("channel_not_found");
                }
                let ts = format!("17000000{:02}.000100", messages.len() + 10);
                let m = json!({"channel": b["channel"], "ts": ts, "text": b["text"], "thread_ts": b["thread_ts"], "blocks": b["blocks"], "metadata": b["metadata"]});
                messages.push(m);
                if l2.lock().unwrap().as_deref() == b["text"].as_str() {
                    *l2.lock().unwrap() = None;
                    return Resp::Lost;
                }
                ok(json!({"ok": true, "channel": b["channel"], "ts": ts}))
            }
            ("GET", "/api/conversations.history") => {
                assert_eq!(q.get("include_all_metadata").map(String::as_str), Some("true"));
                let mine: Vec<Value> = messages.iter().filter(|m| m["channel"] == q["channel"].as_str()).rev().cloned().collect();
                ok(json!({"ok": true, "messages": mine}))
            }
            ("GET", "/api/chat.getPermalink") => ok(json!({"ok": true, "permalink": format!("https://northwind.slack.test/archives/{}/p{}", q["channel"], q["message_ts"].replace('.', ""))})),
            ("POST", "/api/chat.delete") => {
                let b = r.json();
                let before = messages.len();
                messages.retain(|m| !(m["channel"] == b["channel"] && m["ts"] == b["ts"]));
                if messages.len() == before { no("message_not_found") } else { ok(json!({"ok": true})) }
            }
            _ => no("unknown_method"),
        }
    })
    .await;
    *base.lock().unwrap() = net.base.clone();
    let conn = t
        .connect(
            "slack_north",
            "slack",
            &[("https://slack.com", &net.base), ("https://files.slack.com", &net.base)],
            json!({"client_secret": "slack-client-secret"}),
            |d| {
                d["auth"]["provider"]["client_id"] = json!("slack-client-id");
            },
        )
        .await;
    // Connecting: the browser is sent to the authorize address with the scopes, the code is exchanged
    // for the bot token, the workspace is the account.
    let sent_to = t.sign_in(conn, "CODE-north").await;
    let q: HashMap<String, String> = sent_to.query_pairs().into_owned().collect();
    assert_eq!(sent_to.path(), "/oauth/v2/authorize");
    assert!(q["scope"].starts_with("chat:write chat:write.public channels:read") && q["client_id"] == "slack-client-id", "{sent_to}");
    let ws = t.account("Northwind Slack", "northwind_bot", conn, "T1").await;
    let rules = t.rules(ws).await;
    assert_eq!((rules["name"].as_str(), rules["text"]["max"].as_u64(), rules["settings"][0]["key"].as_str()), (Some("Slack"), Some(4000), Some("channel")));
    let channels = t.act(&t.author, "SocialAccount", ws, "choices", json!({"setting": "channel"})).await.unwrap();
    assert_eq!(channels["choices"], json!([{"value": "C1", "label": "general"}, {"value": "C2", "label": "launches"}]));
    let found = t.act(&t.author, "SocialAccount", ws, "mentions", json!({"query": "ana"})).await.unwrap();
    assert_eq!(found["mentions"], json!([{"value": "U9", "label": "Ana Lima", "picture": null, "text": "<@U9>"}]));
    let on = |kind: &str, channel: &str| target_with(ws, kind, json!({"channel": channel}));

    // A text post with a threaded follow-up, a permalink.
    let p = t
        .post("Spring is here", vec![on("message", "C1")], json!({"link": "https://northwind.example/spring", "items": [{"text": "More in the thread"}]}))
        .await;
    let first = t.out(p, ws).await;
    let (head, reply) = {
        let m = messages.lock().unwrap();
        (m[0].clone(), m[1].clone())
    };
    assert_eq!(first["external_id"], format!("C1:{}", head["ts"].as_str().unwrap()));
    assert_eq!((head["text"].as_str(), head["blocks"].is_null()), (Some("Spring is here https://northwind.example/spring"), true));
    assert!(head["metadata"]["event_payload"]["ref"].as_str().unwrap().starts_with("publish:"), "the call's key, to read it back by");
    assert_eq!((reply["text"].as_str(), reply["thread_ts"].clone()), (Some("More in the thread"), head["ts"].clone()), "in the thread of the first");
    assert!(first["permalink"].as_str().unwrap().starts_with("https://northwind.slack.test/archives/C1/p"), "{first:?}");

    // Pictures: for each, a slot, the bytes, the completion (in that order), then image blocks that name the files.
    net.clear();
    let p = t.post("Two views", vec![on("images", "C2")], json!({"media": [t.media(&t.png, "front"), t.media(&t.png, "back")]})).await;
    let pics = t.out(p, ws).await;
    let order: Vec<String> = net.order().into_iter().filter(|o| !o.contains("getPermalink") && !o.contains("auth.test")).collect();
    assert_eq!(
        order,
        [
            "GET /api/files.getUploadURLExternal",
            "POST /upload/F1",
            "POST /api/files.completeUploadExternal",
            "GET /api/files.getUploadURLExternal",
            "POST /upload/F2",
            "POST /api/files.completeUploadExternal",
            "POST /api/chat.postMessage"
        ]
    );
    let q = net.to("GET", "getUploadURLExternal")[0].query();
    assert_eq!((q["filename"].as_str(), q["length"].parse::<usize>().unwrap()), ("cat.png", PNG.len()));
    let blocks = messages.lock().unwrap().last().unwrap()["blocks"].clone();
    assert_eq!(blocks[0]["text"]["text"], "Two views");
    assert_eq!(
        (blocks[1]["slack_file"]["id"].clone(), blocks[1]["alt_text"].clone(), blocks[2]["slack_file"]["id"].clone(), blocks[2]["alt_text"].clone()),
        (json!("F1"), json!("front"), json!("F2"), json!("back"))
    );

    // A lost answer: the message exists; the call reads it back by the key in its metadata and does not send again.
    *lose.lock().unwrap() = Some("Lost one".into());
    net.clear();
    let p = t.post("Lost one", vec![on("message", "C1")], json!({})).await;
    let kept = t.out(p, ws).await;
    assert!(kept["sent_at"].is_string(), "{kept:?}");
    assert_eq!(net.to("POST", "chat.postMessage").len(), 1, "sent once");
    assert_eq!(net.to("GET", "conversations.history").len(), 1, "and read back");
    assert_eq!(messages.lock().unwrap().iter().filter(|m| m["text"] == "Lost one").count(), 1);
    assert_eq!(kept["external_id"].as_str().unwrap().split(':').next(), Some("C1"));

    // A refusal: Slack answers 200 with ok false. The call fails, once (the words are the platform's limit; see the notes).
    net.clear();
    let p = t.post("Nowhere", vec![on("message", "C9")], json!({})).await;
    t.upkeep().await;
    let post = t.get("SocialPost", p).await;
    assert_eq!(post["status"], "Failed", "{post:?}");
    assert_eq!(net.to("POST", "chat.postMessage").len(), 1, "a refusal is not sent again");

    // Delete: by channel and timestamp.
    net.clear();
    t.delete_remote(&pics).await;
    let del = net.to("POST", "chat.delete")[0].json();
    assert_eq!(del["channel"], "C2");
    assert!(!messages.lock().unwrap().iter().any(|m| m["text"] == "Two views"));
    let checked = t.act(&t.manager, "SocialAccount", ws, "check", Value::Null).await.unwrap();
    assert_eq!(checked["read"]["display_name"], "Northwind", "{checked}");
}

// ───────────── WordPress ─────────────

async fn wordpress(t: &T) {
    // REST API (developer.wordpress.org/rest-api): basic auth with an application password, /types, /categories, /tags,
    // /media (the bytes as the body, the name in Content-Disposition), /posts (title, html content, status, term ids,
    // featured_media), `context=edit` for the raw title.
    let posts: Arc<Mutex<Vec<Value>>> = Arc::default();
    let media: Arc<Mutex<Vec<Value>>> = Arc::default();
    let lose: Arc<Mutex<Option<String>>> = Arc::default();
    let base: Arc<Mutex<String>> = Arc::default();
    let (p2, m2, l2, b2) = (posts.clone(), media.clone(), lose.clone(), base.clone());
    let net = fake(move |r| {
        let want = format!("Basic {}", b64(b"editor:abcd efgh ijkl mnop"));
        let err = |code: u16, c: &str, m: &str| status(code, json!({"code": c, "message": m, "data": {"status": code}}));
        if r.header("authorization") != want {
            return err(401, "rest_not_logged_in", "You are not currently logged in.");
        }
        let site = b2.lock().unwrap().clone();
        let mut posts = p2.lock().unwrap();
        let mut media = m2.lock().unwrap();
        let q = r.query();
        let path = r.path().trim_start_matches("/wp-json").to_string();
        match (r.method.as_str(), path.as_str()) {
            ("GET", "/") => ok(json!({"name": "Northwind Blog", "url": site, "home": site})),
            ("GET", "/wp/v2/types") => ok(json!({
                "post": {"name": "Posts", "rest_base": "posts"}, "page": {"name": "Pages", "rest_base": "pages"},
                "attachment": {"name": "Media", "rest_base": "media"}, "wp_block": {"name": "Patterns", "rest_base": "blocks"}})),
            ("GET", "/wp/v2/categories") => ok(json!([{"id": 3, "name": "News"}, {"id": 4, "name": "Launches"}])),
            ("GET", "/wp/v2/tags") => {
                assert_eq!(q.get("search").map(String::as_str), Some("sp"));
                ok(json!([{"id": 7, "name": "spring"}]))
            }
            ("POST", "/wp/v2/media") => {
                let name = r.header("content-disposition").to_string();
                assert!(name.starts_with("attachment; filename=\"cat.png\""), "{name}");
                assert_eq!((r.header("content-type"), r.body.as_slice()), ("image/png", PNG));
                let id = 50 + media.len() as i64;
                media.push(json!({"id": id, "alt": null}));
                Resp::Answer(201, vec![], json!({"id": id, "source_url": format!("{site}/wp-content/cat.png")}).to_string())
            }
            ("POST", m) if m.starts_with("/wp/v2/media/") => {
                let id: i64 = m.rsplit('/').next().unwrap().parse().unwrap();
                media.iter_mut().find(|x| x["id"] == id).expect("a medium")["alt"] = r.json()["alt_text"].clone();
                ok(json!({"id": id}))
            }
            ("POST", p @ ("/wp/v2/posts" | "/wp/v2/pages")) => {
                let b = r.json();
                if b["title"] == "Forbidden" {
                    return err(401, "rest_cannot_create", "Sorry, you are not allowed to create posts as this user.");
                }
                let id = 100 + posts.len() as i64;
                let post = json!({"id": id, "type": p, "title": {"raw": b["title"]}, "content": b["content"], "status": b["status"], "categories": b["categories"], "tags": b["tags"],
                                  "featured_media": b["featured_media"], "slug": b["slug"], "link": format!("{site}/?p={id}"), "date": Utc::now().to_rfc3339()});
                posts.push(post);
                if l2.lock().unwrap().as_deref() == b["title"].as_str() {
                    *l2.lock().unwrap() = None;
                    return Resp::Lost;
                }
                Resp::Answer(201, vec![], json!({"id": id, "link": format!("{site}/?p={id}")}).to_string())
            }
            ("GET", "/wp/v2/posts") => {
                assert_eq!((q.get("status").map(String::as_str), q.get("context").map(String::as_str)), (Some("any"), Some("edit")));
                assert!(q.contains_key("after"));
                let found: Vec<Value> = posts.iter().filter(|x| x["title"]["raw"] == q["search"].as_str()).cloned().collect();
                ok(Value::Array(found))
            }
            ("DELETE", d) if d.starts_with("/wp/v2/posts/") => {
                let id: i64 = d.rsplit('/').next().unwrap().parse().unwrap();
                posts.retain(|x| x["id"] != id);
                ok(json!({"id": id, "deleted": false}))
            }
            _ => err(404, "rest_no_route", "No route was found matching the URL and request method."),
        }
    })
    .await;
    *base.lock().unwrap() = net.base.clone();
    let conn = t
        .connect("wordpress_north", "wordpress", &[("https://wordpress.example", &net.base)], json!({"application_password": "abcd efgh ijkl mnop"}), |d| {
            d["auth"]["username"] = json!("editor");
        })
        .await;
    let blog = t.account("Northwind Blog", "blog", conn, &net.base).await;
    let rules = t.rules(blog).await;
    assert_eq!((rules["name"].as_str(), rules["text"]["format"].as_str()), (Some("WordPress"), Some("html")));
    let types = t.act(&t.author, "SocialAccount", blog, "choices", json!({"setting": "post_type"})).await.unwrap();
    let mut kinds = types["choices"].as_array().unwrap().clone();
    kinds.sort_by_key(|k| k["value"].to_string());
    assert_eq!(kinds, [json!({"value": "pages", "label": "Pages"}), json!({"value": "posts", "label": "Posts"})], "no media, no patterns");
    let cats = t.act(&t.author, "SocialAccount", blog, "choices", json!({"setting": "categories"})).await.unwrap();
    assert_eq!(cats["choices"], json!([{"value": "3", "label": "News"}, {"value": "4", "label": "Launches"}]));
    let tags = t.act(&t.author, "SocialAccount", blog, "choices", json!({"setting": "tags", "query": "sp"})).await.unwrap();
    assert_eq!(tags["choices"], json!([{"value": "7", "label": "spring"}]));
    let article = |settings: Value| target_with(blog, "article", settings);
    let e = t.refused("<p>No title</p>", vec![article(json!({}))], json!({})).await;
    assert!(e.contains("Title is needed"), "{e}");

    // An article with a cover picture: the picture first (its bytes as the body, then its description), then the post.
    net.clear();
    let p = t
        .post(
            "<p>Spring is here</p>",
            vec![article(json!({"title": "Spring", "status": "draft", "categories": ["3"], "tags": ["7"], "slug": "spring-2027"}))],
            json!({"media": [t.media(&t.png, "A cat in bloom")]}),
        )
        .await;
    let out = t.out(p, blog).await;
    assert_eq!(out["external_id"], "posts:100");
    assert_eq!(out["permalink"], format!("{}/?p=100", net.base));
    assert_eq!(
        net.order().into_iter().filter(|o| o != "GET /wp-json/").collect::<Vec<_>>(),
        ["POST /wp-json/wp/v2/media", "POST /wp-json/wp/v2/media/50", "POST /wp-json/wp/v2/posts"]
    );
    {
        let m = media.lock().unwrap();
        assert_eq!(m[0]["alt"], "A cat in bloom");
        let post = posts.lock().unwrap()[0].clone();
        assert_eq!(
            (post["title"]["raw"].as_str(), post["content"].as_str(), post["status"].as_str(), post["featured_media"].as_i64(), post["slug"].as_str()),
            (Some("Spring"), Some("<p>Spring is here</p>"), Some("draft"), Some(50), Some("spring-2027"))
        );
        assert_eq!((post["categories"].clone(), post["tags"].clone()), (json!([3]), json!([7])), "term ids as numbers");
    }

    // A page, by the kind of content the site has.
    let p = t.post("<p>About us</p>", vec![article(json!({"title": "About", "post_type": "pages"}))], json!({})).await;
    let page = t.out(p, blog).await;
    assert_eq!(page["external_id"], "pages:101");
    assert_eq!(posts.lock().unwrap()[1]["type"], "/wp/v2/pages");

    // A lost answer: the post was made; the call finds it by its title since the publication began and does not send again.
    *lose.lock().unwrap() = Some("Lost one".into());
    net.clear();
    let p = t.post("<p>Lost</p>", vec![article(json!({"title": "Lost one"}))], json!({})).await;
    let kept = t.out(p, blog).await;
    assert!(kept["sent_at"].is_string(), "{kept:?}");
    assert_eq!(net.to("POST", "/wp/v2/posts").len(), 1, "sent once");
    assert_eq!(net.to("GET", "/wp/v2/posts").len(), 1, "read back once");
    assert_eq!(posts.lock().unwrap().iter().filter(|x| x["title"]["raw"] == "Lost one").count(), 1);
    assert_eq!(kept["external_id"], "posts:102");

    // A refusal, in WordPress's words.
    let p = t.post("<p>No</p>", vec![article(json!({"title": "Forbidden"}))], json!({})).await;
    t.upkeep().await;
    let post = t.get("SocialPost", p).await;
    assert!(post["failure"].as_str().unwrap().contains("not allowed to create posts as this user"), "{post:?}");

    // Delete: to the trash, by the kind of content and the id.
    net.clear();
    t.delete_remote(&out).await;
    assert_eq!(net.order(), ["DELETE /wp-json/wp/v2/posts/100"]);
    assert!(net.to("DELETE", "posts")[0].target.find("force").is_none(), "the trash, not for good");
    let checked = t.act(&t.manager, "SocialAccount", blog, "check", Value::Null).await.unwrap();
    assert_eq!(checked["read"]["display_name"], "Northwind Blog", "{checked}");
}

// ───────────── Medium ─────────────

async fn medium(t: &T) {
    // API (github.com/Medium/medium-api-docs): an integration token as a bearer, /me, /users/{id}/posts and
    // /publications/{id}/posts with `contentFormat`, `publishStatus`, `tags` (three at most), `canonicalUrl`, /images.
    let articles: Arc<Mutex<Vec<Value>>> = Arc::default();
    let lose: Arc<Mutex<Option<String>>> = Arc::default();
    let (a2, l2) = (articles.clone(), lose.clone());
    let net = fake(move |r| {
        let err = |code: u16, c: i64, m: &str| status(code, json!({"errors": [{"message": m, "code": c}]}));
        if r.header("authorization") != "Bearer medium-token" {
            return err(401, 6003, "Token was invalid.");
        }
        let mut articles = a2.lock().unwrap();
        match (r.method.as_str(), r.path()) {
            ("GET", "/v1/me") => ok(json!({"data": {"id": "u1", "username": "northwind", "name": "Northwind", "imageUrl": "https://miro.medium.test/u1.png"}})),
            ("GET", "/v1/users/u1/publications") => ok(json!({"data": [{"id": "p1", "name": "Northwind Blog"}]})),
            ("POST", "/v1/images") => {
                let f = r.part("image").expect("an image part");
                assert_eq!((f.content.as_slice(), f.kind.as_deref()), (PNG, Some("image/png")));
                ok(json!({"data": {"url": "https://cdn-images-1.medium.test/max/cat.png", "md5": "x"}}))
            }
            ("POST", p @ ("/v1/users/u1/posts" | "/v1/publications/p1/posts")) => {
                let b = r.json();
                if b["title"] == "Forbidden" {
                    return err(403, 2002, "You do not have permission to publish in this publication.");
                }
                assert_eq!(b["contentFormat"], "markdown");
                assert!(b["tags"].as_array().is_none_or(|t| t.len() <= 3));
                let id = format!("a{}", articles.len() + 1);
                articles.push(json!({"id": id, "via": p, "body": b}));
                if l2.lock().unwrap().as_deref() == b["title"].as_str() {
                    *l2.lock().unwrap() = None;
                    return Resp::Lost;
                }
                Resp::Answer(201, vec![], json!({"data": {"id": id, "title": b["title"], "url": format!("https://medium.test/@northwind/{id}"), "publishStatus": b["publishStatus"]}}).to_string())
            }
            _ => err(404, 404, "Not found"),
        }
    })
    .await;
    let conn = t.connect("medium_north", "medium", &[("https://api.medium.com", &net.base)], json!({"integration_token": "medium-token"}), |_| {}).await;
    let me = t.account("Northwind Medium", "northwind", conn, "u1").await;
    let rules = t.rules(me).await;
    assert_eq!((rules["name"].as_str(), rules["text"]["format"].as_str()), (Some("Medium"), Some("markdown")));
    let pubs = t.act(&t.author, "SocialAccount", me, "choices", json!({"setting": "publication"})).await.unwrap();
    assert_eq!(pubs["choices"], json!([{"value": "p1", "label": "Northwind Blog"}]));
    let article = |settings: Value| target_with(me, "article", settings);

    // A story in markdown with a picture on top, tags, the original address and a visibility.
    let p = t
        .post(
            "# Spring\n\nIt is here.",
            vec![article(json!({"title": "Spring", "tags": "spring, launch ,, news", "canonical": "https://northwind.example/spring", "status": "draft"}))],
            json!({"link": "https://northwind.example/spring", "media": [t.media(&t.png, "A cat")]}),
        )
        .await;
    let out = t.out(p, me).await;
    assert_eq!((out["external_id"].as_str(), out["permalink"].as_str()), (Some("a1"), Some("https://medium.test/@northwind/a1")));
    {
        let a = articles.lock().unwrap();
        assert_eq!(a[0]["via"], "/v1/users/u1/posts");
        let b = &a[0]["body"];
        assert_eq!(b["content"], "![A cat](https://cdn-images-1.medium.test/max/cat.png)\n\n# Spring\n\nIt is here. https://northwind.example/spring");
        assert_eq!(
            (b["tags"].clone(), b["canonicalUrl"].as_str(), b["publishStatus"].as_str()),
            (json!(["spring", "launch", "news"]), Some("https://northwind.example/spring"), Some("draft"))
        );
    }
    // In a publication, by the publication's own address.
    let p = t.post("In the blog", vec![article(json!({"title": "Blog", "publication": "p1"}))], json!({})).await;
    t.out(p, me).await;
    assert_eq!(articles.lock().unwrap()[1]["via"], "/v1/publications/p1/posts");

    // A lost answer: nothing can show whether Medium made it (no list of one's stories), so it is not sent again.
    *lose.lock().unwrap() = Some("Lost one".into());
    let p = t.post("Lost", vec![article(json!({"title": "Lost one"}))], json!({})).await;
    t.upkeep().await;
    let post = t.get("SocialPost", p).await;
    assert_eq!(post["status"], "Failed");
    assert!(post["failure"].as_str().unwrap().contains("may have been sent"), "{post:?}");
    assert_eq!(articles.lock().unwrap().iter().filter(|a| a["body"]["title"] == "Lost one").count(), 1, "one story");

    // A refusal, in Medium's words.
    let p = t.post("No", vec![article(json!({"title": "Forbidden"}))], json!({})).await;
    t.upkeep().await;
    let post = t.get("SocialPost", p).await;
    assert!(post["failure"].as_str().unwrap().contains("do not have permission to publish"), "{post:?}");

    // No delete: the API has none.
    let e = t.act(&t.manager, "SocialPublication", uid(&out["id"]), "delete_remote", Value::Null).await.unwrap_err().to_string();
    assert!(e.contains("does not offer to delete"), "{e}");
    let checked = t.act(&t.manager, "SocialAccount", me, "check", Value::Null).await.unwrap();
    assert_eq!(checked["read"]["display_name"], "Northwind", "{checked}");
}

// ───────────── DEV Community ─────────────

async fn devto(t: &T) {
    // Forem API v1 (developers.forem.com/api/v1): an `api-key` header and the v1 Accept type, /users/me, /tags,
    // /articles (an `article` object), /articles/me/all, /organizations/{username}, PUT /articles/{id}.
    let articles: Arc<Mutex<Vec<Value>>> = Arc::default();
    let lose: Arc<Mutex<Option<String>>> = Arc::default();
    let (a2, l2) = (articles.clone(), lose.clone());
    let net = fake(move |r| {
        let err = |code: u16, m: &str| status(code, json!({"error": m, "status": code}));
        if r.header("api-key") != "devto-key" || r.header("accept") != "application/vnd.forem.api-v1+json" {
            return err(401, "unauthorized");
        }
        let mut articles = a2.lock().unwrap();
        match (r.method.as_str(), r.path()) {
            ("GET", "/api/users/me") => ok(json!({"id": 77, "name": "Northwind", "username": "northwind", "profile_image": "https://dev.to/p.png"})),
            ("GET", "/api/tags") => ok(json!([{"id": 1, "name": "webdev"}, {"id": 2, "name": "rust"}])),
            ("GET", "/api/articles/me/all") => {
                let list: Vec<Value> = articles
                    .iter()
                    .rev()
                    .map(|a: &Value| json!({"id": a["id"], "title": a["article"]["title"], "url": a["url"], "published_timestamp": a["at"],
                                            "organization": if a["article"]["organization_id"].is_null() { Value::Null } else { json!({"username": "northwindorg"}) }}))
                    .collect();
                let mut list = list;
                list.push(json!({"id": 1, "title": "Old", "url": "https://dev.to/old", "published_timestamp": "2020-01-01T00:00:00Z", "organization": {"username": "northwindorg"}}));
                list.push(json!({"id": 2, "title": "Older", "url": "https://dev.to/older", "published_timestamp": "2020-01-01T00:00:00Z", "organization": {"username": "northwindorg"}}));
                ok(Value::Array(list))
            }
            ("GET", "/api/organizations/northwindorg") => ok(json!({"id": 4242, "name": "Northwind Org", "username": "northwindorg"})),
            ("POST", "/api/articles") => {
                let b = r.json();
                let a = &b["article"];
                if a["title"] == "Forbidden" {
                    return err(422, "Validation failed: Title has already been taken");
                }
                assert!(a["tags"].as_array().is_none_or(|t| t.len() <= 4));
                let id = 900 + articles.len() as i64;
                articles.push(json!({"id": id, "article": a, "url": format!("https://dev.to/northwind/a-{id}"), "at": Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string()}));
                if l2.lock().unwrap().as_deref() == a["title"].as_str() {
                    *l2.lock().unwrap() = None;
                    return Resp::Lost;
                }
                Resp::Answer(201, vec![], json!({"id": id, "url": format!("https://dev.to/northwind/a-{id}")}).to_string())
            }
            ("PUT", p) if p.starts_with("/api/articles/") => {
                let id: i64 = p.rsplit('/').next().unwrap().parse().unwrap();
                let a = articles.iter_mut().find(|a| a["id"] == id).expect("an article");
                a["unpublished"] = json!(r.json()["article"]["published"] == false);
                ok(json!({"id": id}))
            }
            _ => err(404, "not found"),
        }
    })
    .await;
    let conn = t.connect("devto_north", "devto", &[("https://dev.to", &net.base)], json!({"api_key": "devto-key"}), |_| {}).await;
    let me = t.account("Northwind DEV", "northwind", conn, "77").await;
    let rules = t.rules(me).await;
    assert_eq!((rules["name"].as_str(), rules["text"]["format"].as_str()), (Some("DEV Community"), Some("markdown")));
    let tags = t.act(&t.author, "SocialAccount", me, "choices", json!({"setting": "tags"})).await.unwrap();
    assert_eq!(tags["choices"], json!([{"value": "webdev", "label": "webdev"}, {"value": "rust", "label": "rust"}]));
    let orgs = t.act(&t.author, "SocialAccount", me, "choices", json!({"setting": "organization"})).await.unwrap();
    assert_eq!(orgs["choices"], json!([{"value": "4242", "label": "Northwind Org"}]), "each organisation once");
    assert_eq!(net.to("GET", "/organizations/").len(), 1);
    let article = |settings: Value| target_with(me, "article", settings);
    let e = t.refused("Body", vec![article(json!({"title": "T", "tags": ["a", "b", "c", "d", "e"]}))], json!({})).await;
    assert!(e.to_lowercase().contains("tags"), "five tags are one too many: {e}");

    // An article: markdown, tags, cover address, original address, an organisation, published.
    let p = t
        .post(
            "# Spring\n\nIt is here.",
            vec![article(json!({"title": "Spring", "tags": ["rust", "webdev"], "cover": "https://northwind.example/cover.png", "canonical": "https://northwind.example/spring", "organization": "4242"}))],
            json!({"link": "https://northwind.example/spring"}),
        )
        .await;
    let out = t.out(p, me).await;
    assert_eq!((out["external_id"].as_str(), out["permalink"].as_str()), (Some("900"), Some("https://dev.to/northwind/a-900")));
    {
        let a = articles.lock().unwrap()[0]["article"].clone();
        assert_eq!(a["body_markdown"], "# Spring\n\nIt is here. https://northwind.example/spring");
        assert_eq!(
            (a["tags"].clone(), a["main_image"].as_str(), a["canonical_url"].as_str(), a["organization_id"].as_i64(), a["published"].as_bool()),
            (json!(["rust", "webdev"]), Some("https://northwind.example/cover.png"), Some("https://northwind.example/spring"), Some(4242), Some(true))
        );
    }

    // A lost answer: the article was made; the call finds it in the author's own list and does not send again.
    *lose.lock().unwrap() = Some("Lost one".into());
    net.clear();
    let p = t.post("Lost", vec![article(json!({"title": "Lost one"}))], json!({})).await;
    let kept = t.out(p, me).await;
    assert!(kept["sent_at"].is_string(), "{kept:?}");
    assert_eq!(net.to("POST", "/articles").len(), 1, "sent once");
    assert_eq!(articles.lock().unwrap().iter().filter(|a| a["article"]["title"] == "Lost one").count(), 1);
    assert_eq!(kept["external_id"], "901");

    // A refusal, in DEV's words.
    let p = t.post("No", vec![article(json!({"title": "Forbidden"}))], json!({})).await;
    t.upkeep().await;
    let post = t.get("SocialPost", p).await;
    assert!(post["failure"].as_str().unwrap().contains("Title has already been taken"), "{post:?}");

    // "Delete" unpublishes: the API has no delete.
    net.clear();
    t.delete_remote(&out).await;
    assert_eq!(net.order(), ["PUT /api/articles/900"]);
    assert_eq!(articles.lock().unwrap()[0]["unpublished"], true);
    let checked = t.act(&t.manager, "SocialAccount", me, "check", Value::Null).await.unwrap();
    assert_eq!(checked["read"]["display_name"], "Northwind", "{checked}");
}

// ───────────── Hashnode ─────────────

async fn hashnode(t: &T) {
    // GraphQL (apidocs.hashnode.com): one POST address, the personal access token as the Authorization header with no
    // prefix, `me`, `publishPost(input)`, `publication(id).posts`, `removePost`; failures are `errors` in a 200 answer.
    let posts: Arc<Mutex<Vec<Value>>> = Arc::default();
    let lose: Arc<Mutex<Option<String>>> = Arc::default();
    let (p2, l2) = (posts.clone(), lose.clone());
    let net = fake(move |r| {
        if r.header("authorization") != "hn-token" {
            return ok(json!({"errors": [{"message": "You are not authenticated"}], "data": null}));
        }
        assert_eq!((r.method.as_str(), r.path()), ("POST", "/"));
        let b = r.json();
        let q = b["query"].as_str().unwrap_or_default();
        let mut posts = p2.lock().unwrap();
        if q.contains("publishPost") {
            let i = &b["variables"]["input"];
            if i["title"] == "Forbidden" {
                return ok(json!({"errors": [{"message": "You do not have permission to publish to this publication"}], "data": null}));
            }
            let id = format!("hn{}", posts.len() + 1);
            posts.push(json!({"id": id, "input": i, "url": format!("https://northwind.hashnode.dev/{id}"), "at": Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()}));
            if l2.lock().unwrap().as_deref() == i["title"].as_str() {
                *l2.lock().unwrap() = None;
                return Resp::Lost;
            }
            return ok(json!({"data": {"publishPost": {"post": {"id": id, "url": format!("https://northwind.hashnode.dev/{id}")}}}}));
        }
        if q.contains("removePost") {
            let id = b["variables"]["input"]["id"].clone();
            posts.retain(|p| p["id"] != id);
            return ok(json!({"data": {"removePost": {"post": {"id": id}}}}));
        }
        if q.contains("publication(id") {
            assert_eq!(b["variables"]["id"], "pub1");
            let edges: Vec<Value> = posts.iter().rev().map(|p| json!({"node": {"id": p["id"], "url": p["url"], "title": p["input"]["title"], "publishedAt": p["at"]}})).collect();
            return ok(json!({"data": {"publication": {"posts": {"edges": edges}}}}));
        }
        if q.contains("publications(first") {
            return ok(json!({"data": {"me": {"publications": {"edges": [{"node": {"id": "pub1", "title": "Northwind Engineering"}}]}}}}));
        }
        if q.contains("me {") {
            return ok(json!({"data": {"me": {"id": "user1", "username": "northwind", "name": "Northwind", "profilePicture": "https://cdn.hashnode.test/me.png"}}}));
        }
        ok(json!({"errors": [{"message": "unknown query"}]}))
    })
    .await;
    let conn = t.connect("hashnode_north", "hashnode", &[("https://gql.hashnode.com", &net.base)], json!({"access_token": "hn-token"}), |_| {}).await;
    let me = t.account("Northwind Hashnode", "northwind", conn, "user1").await;
    let rules = t.rules(me).await;
    assert_eq!((rules["name"].as_str(), rules["settings"][0]["key"].as_str()), (Some("Hashnode"), Some("publication")));
    let pubs = t.act(&t.author, "SocialAccount", me, "choices", json!({"setting": "publication"})).await.unwrap();
    assert_eq!(pubs["choices"], json!([{"value": "pub1", "label": "Northwind Engineering"}]));
    let article = |settings: Value| target_with(me, "article", settings);
    let e = t.refused("Body", vec![article(json!({"title": "T"}))], json!({})).await;
    assert!(e.contains("Publication is needed"), "{e}");

    let p = t
        .post(
            "# Spring\n\nIt is here.",
            vec![article(json!({"publication": "pub1", "title": "Spring", "subtitle": "At last", "tags": "Rust, web dev,, ", "canonical": "https://northwind.example/spring", "cover": "https://northwind.example/cover.png"}))],
            json!({}),
        )
        .await;
    let out = t.out(p, me).await;
    assert_eq!((out["external_id"].as_str(), out["permalink"].as_str()), (Some("hn1"), Some("https://northwind.hashnode.dev/hn1")));
    {
        let i = posts.lock().unwrap()[0]["input"].clone();
        assert_eq!(
            (i["publicationId"].as_str(), i["contentMarkdown"].as_str(), i["subtitle"].as_str()),
            (Some("pub1"), Some("# Spring\n\nIt is here."), Some("At last"))
        );
        assert_eq!(i["tags"], json!([{"slug": "rust", "name": "Rust"}, {"slug": "web-dev", "name": "web dev"}]));
        assert_eq!(
            (i["originalArticleURL"].as_str(), i["coverImageOptions"]["coverImageURL"].as_str()),
            (Some("https://northwind.example/spring"), Some("https://northwind.example/cover.png"))
        );
    }

    // A lost answer: the post was made; the call finds it in the publication's latest posts and does not send again.
    *lose.lock().unwrap() = Some("Lost one".into());
    net.clear();
    let p = t.post("Lost", vec![article(json!({"publication": "pub1", "title": "Lost one"}))], json!({})).await;
    let kept = t.out(p, me).await;
    assert!(kept["sent_at"].is_string(), "{kept:?}");
    assert_eq!(net.seen().iter().filter(|r| r.text().contains("publishPost")).count(), 1, "sent once");
    assert_eq!(posts.lock().unwrap().iter().filter(|p| p["input"]["title"] == "Lost one").count(), 1);
    assert_eq!(kept["external_id"], "hn2");

    // A refusal arrives as `errors` in a 200 answer: the call fails, once (the words are the platform's limit; see the notes).
    net.clear();
    let p = t.post("No", vec![article(json!({"publication": "pub1", "title": "Forbidden"}))], json!({})).await;
    t.upkeep().await;
    let post = t.get("SocialPost", p).await;
    assert_eq!(post["status"], "Failed", "{post:?}");
    assert_eq!(net.seen().iter().filter(|r| r.text().contains("publishPost")).count(), 1, "not sent again");

    net.clear();
    t.delete_remote(&out).await;
    assert!(net.seen()[0].text().contains("removePost") && posts.lock().unwrap().iter().all(|p| p["id"] != "hn1"));
    let checked = t.act(&t.manager, "SocialAccount", me, "check", Value::Null).await.unwrap();
    assert_eq!(checked["read"]["display_name"], "Northwind", "{checked}");
}

// ───────────── Tumblr ─────────────

async fn tumblr(t: &T) {
    // API v2 (tumblr.com/docs/en/api/v2): OAuth 2, /user/info, NPF posts (`content` blocks; with files a multipart request
    // whose `json` part names each file by an `identifier`), /blog/{id}/posts?npf=true, post/delete.
    let posts: Arc<Mutex<Vec<Value>>> = Arc::default();
    let lose: Arc<Mutex<Option<String>>> = Arc::default();
    let (p2, l2) = (posts.clone(), lose.clone());
    let net = fake(move |r| {
        let err = |code: u16, title: &str, detail: &str| status(code, json!({"meta": {"status": code, "msg": title}, "response": [], "errors": [{"title": title, "code": 6000, "detail": detail}]}));
        if r.path() == "/v2/oauth2/token" {
            let f = r.form();
            if f.get("code").map(String::as_str) != Some("CODE-north") || f.get("client_secret").map(String::as_str) != Some("tumblr-secret") || f.get("grant_type").map(String::as_str) != Some("authorization_code") {
                return status(400, json!({"error": "invalid_grant"}));
            }
            return ok(json!({"access_token": "tb-token", "token_type": "bearer", "expires_in": 2520, "refresh_token": "tb-refresh"}));
        }
        if r.header("authorization") != "Bearer tb-token" {
            return err(401, "Unauthorized", "Token invalid");
        }
        let mut posts = p2.lock().unwrap();
        let q = r.query();
        match (r.method.as_str(), r.path()) {
            ("GET", "/v2/user/info") => ok(json!({"meta": {"status": 200}, "response": {"user": {"name": "northwind", "blogs": [
                {"name": "northwind", "title": "Northwind Notes", "uuid": "t:1"}, {"name": "side", "title": "", "uuid": "t:2"}]}}})),
            ("GET", "/v2/blog/northwind/info") => ok(json!({"response": {"blog": {"title": "Northwind Notes", "name": "northwind", "followers": 88}}})),
            ("POST", "/v2/blog/northwind/posts") => {
                let (body, files) = if r.header("content-type").starts_with("multipart/form-data") {
                    let body: Value = serde_json::from_str(&r.part("json").expect("a json part").text()).unwrap();
                    let files: Vec<Value> = r.parts().iter().filter(|p| p.name != "json").map(|p| json!({"name": p.name, "same": p.content == PNG || p.content == MP4, "type": p.kind})).collect();
                    (body, files)
                } else {
                    (r.json(), vec![])
                };
                let blocks = body["content"].as_array().expect("blocks").clone();
                if blocks.iter().any(|b| b["text"] == "Forbidden") {
                    return err(403, "Forbidden", "You do not have permission to post to this blog.");
                }
                for b in &blocks {
                    let media = if b["type"] == "image" { b["media"][0].clone() } else { b["media"].clone() };
                    if b["type"] != "text" {
                        assert!(files.iter().any(|f| f["name"] == media["identifier"]), "every block names a part: {body}");
                    }
                }
                let id = format!("7000{}", posts.len() + 1);
                posts.push(json!({"id_string": id, "timestamp": Utc::now().timestamp(), "content": blocks, "tags": body["tags"], "state": body["state"], "files": files, "note_count": 12}));
                if l2.lock().unwrap().as_deref() == blocks[0]["text"].as_str() {
                    *l2.lock().unwrap() = None;
                    return Resp::Lost;
                }
                Resp::Answer(201, vec![], json!({"meta": {"status": 201, "msg": "Created"}, "response": {"id": 7000 + posts.len(), "id_string": id, "state": "published"}}).to_string())
            }
            ("GET", "/v2/blog/northwind/posts") => {
                assert_eq!(q.get("npf").map(String::as_str), Some("true"));
                let list: Vec<Value> = posts.iter().rev().filter(|p| q.get("id").is_none_or(|i| p["id_string"] == i.as_str())).cloned().collect();
                ok(json!({"response": {"posts": list}}))
            }
            ("POST", "/v2/blog/northwind/post/delete") => {
                let id = r.form()["id"].clone();
                posts.retain(|p| p["id_string"] != id.as_str());
                ok(json!({"meta": {"status": 200}, "response": {"id": id}}))
            }
            _ => err(404, "Not Found", "no such route"),
        }
    })
    .await;
    let conn = t
        .connect(
            "tumblr_north",
            "tumblr",
            &[("https://api.tumblr.com", &net.base), ("https://www.tumblr.com", &net.base)],
            json!({"client_secret": "tumblr-secret"}),
            |d| {
                d["auth"]["provider"]["client_id"] = json!("tumblr-id");
            },
        )
        .await;
    t.sign_in(conn, "CODE-north").await;
    // Connecting: the blogs the sign-in reaches are the accounts to pick from.
    let blog = t.account("Northwind Notes", "northwind", conn, "northwind").await;
    let rules = t.rules(blog).await;
    assert_eq!((rules["name"].as_str(), rules["text"]["format"].as_str()), (Some("Tumblr"), Some("plain")));
    let reached = t.act(&t.manager, "SocialAccount", blog, "reachable", Value::Null).await.unwrap();
    assert_eq!(reached["accounts"][1]["name"], "side", "a blog without a title is named by its address");

    // Text: a block for each paragraph, tags as given, the link at the end of the last.
    let p = t
        .post(
            "First paragraph.\n\nSecond paragraph.",
            vec![target_with(blog, "post", json!({"tags": "spring, launch"}))],
            json!({"link": "https://northwind.example/spring"}),
        )
        .await;
    let out = t.out(p, blog).await;
    assert_eq!((out["external_id"].as_str(), out["permalink"].as_str()), (Some("northwind:70001"), Some("https://northwind.tumblr.com/post/70001")));
    {
        let post = posts.lock().unwrap()[0].clone();
        assert_eq!(
            post["content"],
            json!([{"type": "text", "text": "First paragraph."}, {"type": "text", "text": "Second paragraph. https://northwind.example/spring"}])
        );
        assert_eq!((post["tags"].as_str(), post["state"].as_str()), (Some("spring, launch"), Some("published")));
    }

    // Two pictures and a clip in one post: the blocks name the parts, the parts are the files.
    net.clear();
    let p = t
        .post("Gallery", vec![target(blog)], json!({"media": [t.media(&t.png, "front"), t.media(&t.png, "back"), json!({"item": 1, "file": t.mp4["url"]})]}))
        .await;
    let gallery = t.out(p, blog).await;
    let sent = net.to("POST", "/posts");
    assert_eq!(sent.len(), 1, "one request: {:?}", net.order());
    {
        let post = posts.lock().unwrap().last().unwrap().clone();
        let c = post["content"].as_array().unwrap();
        assert_eq!(
            (c[0]["text"].as_str(), c[1]["type"].as_str(), c[1]["alt_text"].as_str(), c[2]["alt_text"].as_str(), c[3]["type"].as_str()),
            (Some("Gallery"), Some("image"), Some("front"), Some("back"), Some("video"))
        );
        assert_eq!(
            (c[1]["media"][0]["identifier"].as_str(), c[1]["media"][0]["type"].as_str(), c[3]["media"]["identifier"].as_str(), c[3]["media"]["type"].as_str()),
            (Some("f0"), Some("image/png"), Some("f2"), Some("video/mp4"))
        );
        assert_eq!(post["files"].as_array().unwrap().iter().map(|f| f["name"].as_str().unwrap()).collect::<Vec<_>>(), ["f0", "f1", "f2"]);
        assert!(post["files"].as_array().unwrap().iter().all(|f| f["same"] == true), "the stored files, byte for byte");
    }

    // A lost answer: the post was made; the call finds it in the blog's latest posts by its first paragraph.
    *lose.lock().unwrap() = Some("Lost one".into());
    net.clear();
    let p = t.post("Lost one", vec![target(blog)], json!({})).await;
    let kept = t.out(p, blog).await;
    assert!(kept["sent_at"].is_string(), "{kept:?}");
    assert_eq!(net.to("POST", "/posts").len(), 1, "sent once");
    assert_eq!(posts.lock().unwrap().iter().filter(|p| p["content"][0]["text"] == "Lost one").count(), 1);

    // A refusal, in Tumblr's words.
    let p = t.post("Forbidden", vec![target(blog)], json!({})).await;
    t.upkeep().await;
    let post = t.get("SocialPost", p).await;
    assert!(post["failure"].as_str().unwrap().contains("do not have permission to post to this blog"), "{post:?}");

    // Numbers: the notes of a post.
    t.later_upkeep().await;
    let numbers = t.rows("SocialMetric", json!({})).await;
    assert!(numbers.iter().any(|m| m["engagements"] == 12), "{numbers:?}");

    net.clear();
    t.delete_remote(&gallery).await;
    assert_eq!(net.order(), ["POST /v2/blog/northwind/post/delete"]);
    assert_eq!(net.seen()[0].form()["id"], gallery["external_id"].as_str().unwrap().split(':').nth(1).unwrap());
    let checked = t.act(&t.manager, "SocialAccount", blog, "check", Value::Null).await.unwrap();
    assert_eq!(checked["read"]["display_name"], "Northwind Notes", "{checked}");
}

// ───────────── Lemmy ─────────────

async fn lemmy(t: &T) {
    // HTTP API v3 (join-lemmy.org/api): /user/login for a jwt, /site, /community/list, /search, /pictrs/image (field
    // `images[]`), /post, /post/list, /comment, /comment/list, /post/delete, /comment/delete.
    let posts: Arc<Mutex<Vec<Value>>> = Arc::default();
    let comments: Arc<Mutex<Vec<Value>>> = Arc::default();
    let logins: Arc<Mutex<usize>> = Arc::default();
    let lose: Arc<Mutex<Option<String>>> = Arc::default();
    let (p2, c2, g2, l2) = (posts.clone(), comments.clone(), logins.clone(), lose.clone());
    let net = fake(move |r| {
        let err = |code: u16, e: &str| status(code, json!({"error": e}));
        if r.path() == "/api/v3/user/login" {
            let b = r.json();
            if b["username_or_email"] != "northwind" || b["password"] != "lemmy-pass" {
                return err(400, "incorrect_login");
            }
            *g2.lock().unwrap() += 1;
            return ok(json!({"jwt": "JWT-1", "registration_created": false, "verify_email_sent": false}));
        }
        if r.header("authorization") != "Bearer JWT-1" {
            return err(400, "not_logged_in");
        }
        let mut posts = p2.lock().unwrap();
        let mut comments = c2.lock().unwrap();
        let q = r.query();
        let community = |id: i64, title: &str| json!({"community": {"id": id, "name": title.to_lowercase(), "title": title}});
        match (r.method.as_str(), r.path()) {
            ("GET", "/api/v3/site") => ok(json!({"my_user": {"local_user_view": {"person": {"id": 5, "name": "northwind", "display_name": "Northwind", "avatar": "https://lemmy.test/pictrs/image/me.png"}}}})),
            ("GET", "/api/v3/community/list") => ok(json!({"communities": [community(11, "Linux"), community(12, "Rust")]})),
            ("GET", "/api/v3/search") => {
                assert_eq!((q["q"].as_str(), q["type_"].as_str()), ("ru", "Communities"));
                ok(json!({"communities": [community(12, "Rust")]}))
            }
            ("POST", "/pictrs/image") => {
                let f = r.part("images[]").expect("an images[] part");
                assert_eq!(f.content, PNG);
                ok(json!({"msg": "ok", "files": [{"file": "abc123.png", "delete_token": "d"}]}))
            }
            ("POST", "/api/v3/post") => {
                let b = r.json();
                if b["name"] == "Forbidden" {
                    return err(400, "language_not_allowed");
                }
                let id = 300 + posts.len() as i64;
                posts.push(json!({"post": {"id": id, "name": b["name"], "ap_id": format!("https://lemmy.test/post/{id}"), "published": Utc::now().format("%Y-%m-%dT%H:%M:%S%.6fZ").to_string()}, "input": b}));
                if l2.lock().unwrap().as_deref() == b["name"].as_str() {
                    *l2.lock().unwrap() = None;
                    return Resp::Lost;
                }
                ok(json!({"post_view": {"post": {"id": id, "ap_id": format!("https://lemmy.test/post/{id}")}}}))
            }
            ("GET", "/api/v3/post/list") => {
                assert_eq!(q.get("sort").map(String::as_str), Some("New"));
                ok(json!({"posts": posts.iter().rev().map(|p| json!({"post": p["post"]})).collect::<Vec<_>>()}))
            }
            ("GET", "/api/v3/post") => ok(json!({"post_view": {"counts": {"upvotes": 7, "downvotes": 1, "comments": 2, "score": 6}}})),
            ("POST", "/api/v3/comment") => {
                let b = r.json();
                let id = 500 + comments.len() as i64;
                comments.push(json!({"comment": {"id": id, "content": b["content"], "published": Utc::now().format("%Y-%m-%dT%H:%M:%S%.6fZ").to_string()}, "post_id": b["post_id"]}));
                if l2.lock().unwrap().as_deref() == b["content"].as_str() {
                    *l2.lock().unwrap() = None;
                    return Resp::Lost;
                }
                ok(json!({"comment_view": {"comment": {"id": id}}}))
            }
            ("GET", "/api/v3/comment/list") => ok(json!({"comments": comments.iter().rev().filter(|c| c["post_id"].as_i64().map(|n| n.to_string()).as_deref() == Some(q["post_id"].as_str())).map(|c| json!({"comment": c["comment"]})).collect::<Vec<_>>()})),
            ("POST", "/api/v3/post/delete") => {
                let id = r.json()["post_id"].as_i64().unwrap();
                posts.retain(|p| p["post"]["id"] != id);
                ok(json!({}))
            }
            ("POST", "/api/v3/comment/delete") => {
                let id = r.json()["comment_id"].as_i64().unwrap();
                comments.retain(|c| c["comment"]["id"] != id);
                ok(json!({}))
            }
            _ => err(404, "not_found"),
        }
    })
    .await;
    let conn =
        t.connect("lemmy_north", "lemmy", &[("https://lemmy.example", &net.base)], json!({"username": "northwind", "password": "lemmy-pass"}), |_| {}).await;
    // Connecting: a session from the account's user name and password, on first use; the user is the account.
    let me = t.account("Northwind Lemmy", "northwind", conn, "5").await;
    assert_eq!(*logins.lock().unwrap(), 1, "signed in once, and the session kept");
    let rules = t.rules(me).await;
    assert_eq!((rules["name"].as_str(), rules["follow_ups"]["as"].as_str()), (Some("Lemmy"), Some("comment")));
    let all = t.act(&t.author, "SocialAccount", me, "choices", json!({"setting": "community"})).await.unwrap();
    assert_eq!(all["choices"], json!([{"value": "11", "label": "Linux"}, {"value": "12", "label": "Rust"}]));
    let found = t.act(&t.author, "SocialAccount", me, "choices", json!({"setting": "community", "query": "ru"})).await.unwrap();
    assert_eq!(found["choices"], json!([{"value": "12", "label": "Rust"}]), "searched when something was typed");
    let post_in = |settings: Value| target_with(me, "post", settings);

    // A link post with a picture as its thumbnail, and a comment as a follow-up.
    let p = t
        .post(
            "We shipped **Spring**.",
            vec![post_in(json!({"community": "12", "title": "Spring is here"}))],
            json!({"link": "https://northwind.example/spring", "media": [t.media(&t.png, "A cat")], "items": [{"text": "More details below"}]}),
        )
        .await;
    let out = t.out(p, me).await;
    assert_eq!((out["external_id"].as_str(), out["permalink"].as_str()), (Some("300"), Some("https://lemmy.test/post/300")));
    {
        let i = posts.lock().unwrap()[0]["input"].clone();
        assert_eq!((i["name"].as_str(), i["community_id"].as_i64(), i["body"].as_str()), (Some("Spring is here"), Some(12), Some("We shipped **Spring**.")));
        assert_eq!((i["url"].as_str(), i["alt_text"].as_str()), (Some("https://northwind.example/spring"), Some("A cat")));
        assert_eq!(i["custom_thumbnail"], format!("{}/pictrs/image/abc123.png", net.base), "the uploaded picture, as the thumbnail of the link");
        let c = comments.lock().unwrap()[0].clone();
        assert_eq!((c["comment"]["content"].as_str(), c["post_id"].as_i64()), (Some("More details below"), Some(300)));
    }
    assert!(net.order().iter().position(|o| o == "POST /pictrs/image") < net.order().iter().position(|o| o == "POST /api/v3/post"), "the picture first");
    // A picture and no link: the picture is the post.
    let p = t.post("Just a picture", vec![post_in(json!({"community": "12", "title": "Photo"}))], json!({"media": [t.media(&t.png, "")]})).await;
    t.out(p, me).await;
    assert_eq!(posts.lock().unwrap()[1]["input"]["url"], format!("{}/pictrs/image/abc123.png", net.base));

    // A lost answer on the post, and on a comment: both found again by what they said, since they began.
    *lose.lock().unwrap() = Some("Lost one".into());
    net.clear();
    let p = t.post("Lost", vec![post_in(json!({"community": "12", "title": "Lost one"}))], json!({})).await;
    let kept = t.out(p, me).await;
    assert!(kept["sent_at"].is_string() && net.to("POST", "/api/v3/post").len() == 1, "sent once");
    *lose.lock().unwrap() = Some("Lost comment".into());
    let p = t.post("Body", vec![post_in(json!({"community": "12", "title": "With comment"}))], json!({"items": [{"text": "Lost comment"}]})).await;
    t.out(p, me).await;
    assert_eq!(comments.lock().unwrap().iter().filter(|c| c["comment"]["content"] == "Lost comment").count(), 1, "one comment");

    // A refusal, in Lemmy's error code.
    let p = t.post("No", vec![post_in(json!({"community": "12", "title": "Forbidden"}))], json!({})).await;
    t.upkeep().await;
    let post = t.get("SocialPost", p).await;
    assert!(post["failure"].as_str().unwrap().contains("language_not_allowed"), "{post:?}");

    t.later_upkeep().await;
    assert!(t.rows("SocialMetric", json!({})).await.iter().any(|m| m["engagements"] == 10), "7 + 1 + 2");
    net.clear();
    t.delete_remote(&out).await;
    assert_eq!(net.order(), ["POST /api/v3/comment/delete", "POST /api/v3/post/delete"], "the comment first, then the post");
    assert_eq!(net.seen()[0].json(), json!({"comment_id": 500, "deleted": true}));
    let checked = t.act(&t.manager, "SocialAccount", me, "check", Value::Null).await.unwrap();
    assert_eq!(checked["read"]["display_name"], "Northwind", "{checked}");
    assert_eq!(*logins.lock().unwrap(), 1, "still one session");
}
