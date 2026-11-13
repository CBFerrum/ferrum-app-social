//! The network descriptions under docs/connectors (X, LinkedIn, Facebook, Instagram, Threads, YouTube, TikTok,
//! Pinterest, Reddit, Mastodon, Bluesky, Google Business Profile), each against a fake of its network's documented
//! API on 127.0.0.1. Nothing here reaches a real network: the fakes answer in the documented shapes and check what
//! they are sent (headers, body shapes, the order of steps); a first connection to each network is the first time
//! its real answers are read.
//!
//! Each network's calls are made as Social makes them (`connections::call` as the app, under Social's keys, with the
//! inputs `src/posts.rs`, `src/metrics.rs`, `src/picker.rs` and `src/inbox.rs` build, filtered to what the operation
//! declares), the sign-in through the platform's OAuth (PKCE, the token address), the picker and choices through
//! Social's own account actions. The description is the file's, its hosts put on the fake, its waits made short.

use ferrum_kernel::connections::{self, Call, ConnectionInput, Description};
use ferrum_kernel::secret::SecretKey;
use ferrum_kernel::tenant::NewTenant;
use ferrum_kernel::{Ctx, Db, DbConfig, Doc, Error, HookRegistry, Kernel, KernelSettings};
use ferrum_meta::Catalog;
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use uuid::Uuid;

fn dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/connectors")
}

fn file(name: &str) -> String {
    std::fs::read_to_string(dir().join(format!("{name}.json"))).unwrap()
}

fn obj(v: Value) -> Doc {
    v.as_object().cloned().unwrap()
}

fn uid(v: &Value) -> Uuid {
    v.as_str().unwrap_or_else(|| panic!("not an id: {v}")).parse().unwrap()
}

// ───────────── shape ─────────────

/// The descriptions written here, each tested below.
const NETWORKS: &[&str] = &["x", "linkedin", "linkedin_page", "facebook", "instagram", "instagram_standalone"];

#[test]
fn every_network_is_a_description_with_its_rules() {
    for name in NETWORKS {
        let path = dir().join(format!("{name}.json"));
        let text = std::fs::read_to_string(&path).unwrap();
        let d: Description = serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        assert_eq!(d.problem(), None, "{}", path.display());
        assert!(d.apps.iter().any(|a| a == "social") && d.operations.contains_key("publish"), "{}", path.display());
        let raw: Value = serde_json::from_str(&text).unwrap();
        assert!(raw["meta"]["social"]["name"].is_string(), "{}: meta.social says what the network takes", path.display());
        assert!(raw["meta"].to_string().len() <= 16 << 10, "{}: meta within 16 KB", path.display());
        // Every operation Social or a lookup names is there.
        let s = &raw["meta"]["social"];
        let looked: Vec<String> = s["settings"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|x| x["choices"]["operation"].as_str().map(str::to_string))
            .chain(s["accounts"]["operation"].as_str().map(str::to_string))
            .chain(s["mentions"]["operation"].as_str().map(str::to_string))
            .collect();
        for op in looked {
            assert!(d.operations.contains_key(&op), "{}: {op}", path.display());
        }
        if s["follow_ups"]["as"] == "comment" || s["first_comment"] == true {
            assert!(d.operations.contains_key("reply"), "{}: comments go by reply", path.display());
        }
    }
}

// ───────────── the fake networks ─────────────

#[derive(Debug, Clone, Default)]
struct Req {
    method: String,
    path: String,
    query: BTreeMap<String, String>,
    headers: HashMap<String, String>,
    body: Vec<u8>,
}

impl Req {
    fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or(Value::Null)
    }
    fn form(&self) -> BTreeMap<String, String> {
        query_of(&String::from_utf8_lossy(&self.body))
    }
    fn bearer(&self) -> Option<String> {
        self.headers.get("authorization").and_then(|a| a.strip_prefix("Bearer ").map(str::to_string))
    }
    fn header(&self, h: &str) -> Option<&str> {
        self.headers.get(h).map(String::as_str)
    }
    /// The parts of a multipart body: name → (filename, content type, bytes).
    fn parts(&self) -> BTreeMap<String, (Option<String>, Option<String>, Vec<u8>)> {
        let kind = self.header("content-type").unwrap_or_default();
        let Some(boundary) = kind.split("boundary=").nth(1) else { return BTreeMap::new() };
        let sep = format!("--{boundary}");
        let mut out = BTreeMap::new();
        let body = &self.body;
        let mut at = 0;
        let find = |hay: &[u8], needle: &[u8], from: usize| hay[from..].windows(needle.len()).position(|w| w == needle).map(|p| p + from);
        while let Some(start) = find(body, sep.as_bytes(), at) {
            let head_from = start + sep.len();
            let Some(head_end) = find(body, b"\r\n\r\n", head_from) else { break };
            let next = find(body, sep.as_bytes(), head_end).unwrap_or(body.len());
            let head = String::from_utf8_lossy(&body[head_from..head_end]).to_string();
            let content = body[head_end + 4..next.saturating_sub(2).max(head_end + 4)].to_vec();
            let field = |what: &str| head.split(what).nth(1).and_then(|r| r.split('"').next()).map(str::to_string);
            if let Some(name) = field("name=\"") {
                let ct = head.lines().find_map(|l| l.strip_prefix("Content-Type: ").or_else(|| l.strip_prefix("content-type: "))).map(str::to_string);
                out.insert(name, (field("filename=\""), ct, content));
            }
            at = next;
            if next >= body.len() {
                break;
            }
        }
        out
    }
}

fn query_of(q: &str) -> BTreeMap<String, String> {
    reqwest::Url::parse(&format!("http://f/?{q}")).unwrap().query_pairs().map(|(k, v)| (k.into_owned(), v.into_owned())).collect()
}

struct Resp(u16, Vec<(String, String)>, Vec<u8>);

fn ok(v: Value) -> Resp {
    Resp(200, vec![], v.to_string().into_bytes())
}

fn status(code: u16, v: Value) -> Resp {
    Resp(code, vec![], v.to_string().into_bytes())
}

fn empty(code: u16) -> Resp {
    Resp(code, vec![], vec![])
}

#[derive(Default)]
struct Net {
    base: String,
    /// Every request, by network: (network, request).
    seen: Vec<(String, Req)>,
    /// "METHOD /net/path" taken, then its answer lost, once each.
    lose: HashSet<String>,
    /// "METHOD /net/path" answered with this, once each.
    fail: HashMap<String, (u16, Value)>,
    /// Anything a fake keeps, by name.
    kept: BTreeMap<String, Value>,
    /// Counters for ids.
    n: u64,
}

impl Net {
    fn next(&mut self) -> u64 {
        self.n += 1;
        self.n
    }
    fn list(&mut self, name: &str) -> &mut Vec<Value> {
        let v = self.kept.entry(name.to_string()).or_insert_with(|| json!([]));
        v.as_array_mut().unwrap()
    }
    fn get(&self, name: &str) -> Value {
        self.kept.get(name).cloned().unwrap_or(Value::Null)
    }
    fn set(&mut self, name: &str, v: Value) {
        self.kept.insert(name.to_string(), v);
    }
    /// The requests a network was sent, "METHOD path".
    fn asked(&self, net: &str) -> Vec<String> {
        self.seen.iter().filter(|(n, _)| n == net).map(|(_, r)| format!("{} {}", r.method, r.path)).collect()
    }
    fn last(&self, net: &str, method: &str, path: &str) -> Req {
        self.seen.iter().rev().find(|(n, r)| n == net && r.method == method && r.path == path).map(|(_, r)| r.clone()).unwrap_or_default()
    }
    fn tokens(&mut self, net: &str) -> &mut Vec<Value> {
        self.list(&format!("{net}:tokens"))
    }
    fn valid(&self, net: &str, r: &Req) -> bool {
        let t = r.bearer().or_else(|| r.query.get("access_token").cloned());
        t.is_some_and(|t| self.kept.get(&format!("{net}:tokens")).and_then(Value::as_array).is_some_and(|l| l.iter().any(|x| x == &json!(t))))
    }
}

type Shared = Arc<Mutex<Net>>;
type Handler = fn(&mut Net, &Req) -> Resp;

/// The networks' fakes, by the first segment of the path.
fn handler(net: &str) -> Option<Handler> {
    Some(match net {
        "x" => fake_x,
        "li" | "liw" => fake_linkedin,
        "fb" | "fbv" | "fbr" => fake_facebook,
        "igf" | "igs" | "igt" | "igr" => fake_instagram,
        _ => return None,
    })
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
                let mut chunk = [0u8; 65536];
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
                let mut segs = target.trim_start_matches('/').splitn(2, '/');
                let name = segs.next().unwrap_or_default().to_string();
                let rest = format!("/{}", segs.next().unwrap_or_default());
                let path = rest.split('?').next().unwrap_or_default().to_string();
                let query = rest.split_once('?').map(|(_, q)| query_of(q)).unwrap_or_default();
                let req = Req { method: method.clone(), path: path.clone(), query, headers, body };
                let (resp, lost) = {
                    let mut n = net.lock().unwrap();
                    n.seen.push((name.clone(), req.clone()));
                    let route = format!("{method} /{name}{path}");
                    let lost = n.lose.remove(&route);
                    let resp = match n.fail.remove(&route) {
                        Some((code, v)) => status(code, v),
                        None => match handler(&name) {
                            Some(h) => h(&mut n, &req),
                            None => status(404, json!({"error": format!("no network {name}")})),
                        },
                    };
                    (resp, lost)
                };
                if lost {
                    // Taken, and its answer lost: the connection gives up waiting.
                    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                    return;
                }
                let Resp(code, extra, reply) = resp;
                let mut out = format!("HTTP/1.1 {code} X\r\ncontent-length: {}\r\nconnection: close\r\n", reply.len());
                if !extra.iter().any(|(k, _)| k == "content-type") {
                    out.push_str("content-type: application/json\r\n");
                }
                for (k, v) in extra {
                    out.push_str(&format!("{k}: {v}\r\n"));
                }
                out.push_str("\r\n");
                let mut bytes = out.into_bytes();
                bytes.extend(reply);
                let _ = s.write_all(&bytes).await;
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

// ───────────── Social's side ─────────────

struct T {
    k: Arc<Kernel>,
    tenant: Uuid,
    admin: Ctx,
    net: Shared,
    base: String,
    brand: Uuid,
    /// Each connection's description as saved, by name.
    descs: Mutex<HashMap<String, Value>>,
}

/// What Social gives `publish` for item `n` (src/posts.rs), before it is fitted to what the operation takes.
fn post_in(text: &str, link: Option<&str>, media: &[(&str, &str, i64)], ids: &[String]) -> Value {
    let described: Vec<Value> = media
        .iter()
        .zip(ids)
        .map(|((_, ct, size), id)| json!({"id": id, "kind": if ct.starts_with("video/") { "video" } else if ct.starts_with("image/") { "image" } else { "document" }, "alt": "What it shows", "content_type": ct, "size": size, "width": 1080, "height": 1080, "seconds": if ct.starts_with("video/") { json!(12.0) } else { Value::Null }}))
        .collect();
    json!({"text": text, "link": link, "text_with_link": link.map_or_else(|| text.to_string(), |l| format!("{text} {l}")),
           "media": ids, "media_id": ids.first(), "media_items": described, "alt": if media.is_empty() { Value::Null } else { json!("What it shows") },
           "created_at": "2026-11-13T10:00:00.000Z", "picture_type": media.first().map(|m| m.1), "picture_size": media.first().map(|m| m.2),
           "reply_to": null, "root": null, "previous": null, "in_reply_to": null,
           "kind": null, "settings": {}, "account_id": null, "handle": null, "item": 0, "items": 1})
}

/// Item `n` after `before` (what Social keeps of the item before: `{item, id, url, media, sent_at, answer}`).
fn follow_in(mut v: Value, n: usize, items: usize, root: &str, before: &Value) -> Value {
    v["item"] = json!(n);
    v["items"] = json!(items);
    v["reply_to"] = before["id"].clone();
    v["root"] = json!(root);
    v["in_reply_to"] = json!(root);
    v["previous"] = before.clone();
    v
}

/// What Social keeps of an item it sent, from the call's result.
fn kept(n: usize, out: &Value) -> Value {
    let mut answer = out.as_object().cloned().unwrap_or_default();
    answer.remove("id");
    answer.remove("url");
    json!({"item": n, "id": out["id"], "url": out["url"], "media": [], "sent_at": "2026-11-13T10:00:00Z", "answer": answer})
}

impl T {
    /// A connection from the file `name`, its hosts on the fake under `/<net>`, its client id `<net>-client`, its waits
    /// a second apart and a second's patience for an answer.
    async fn connect(&self, name: &str, net: &str, hosts: &[&str], secrets: Value, edit: impl FnOnce(&mut Value)) -> Uuid {
        let mut text = file(name);
        for h in hosts {
            text = text.replace(&format!("https://{h}"), &format!("{}/{net}", self.base));
        }
        text = text.replace("REPLACE-WITH-THE-APPS-CLIENT-ID", &format!("{net}-client"));
        let mut d: Value = serde_json::from_str(&text).unwrap();
        d["timeout_ms"] = json!(1000);
        fn quick(v: &mut Value) {
            match v {
                Value::Object(o) => {
                    if let Some(e) = o.get_mut("every_secs") {
                        *e = json!(1);
                    }
                    o.values_mut().for_each(quick);
                }
                Value::Array(a) => a.iter_mut().for_each(quick),
                _ => {}
            }
        }
        quick(&mut d);
        edit(&mut d);
        self.descs.lock().unwrap().insert(name.to_string(), d.clone());
        let input: ConnectionInput =
            serde_json::from_value(json!({"name": name, "label": name, "description": d, "secrets": secrets, "enabled": true})).unwrap();
        self.k.save_connection_def(&self.admin, None, &input).await.unwrap_or_else(|e| panic!("{name}: {e:?}"))
    }
    /// Signs the workspace in through the platform's OAuth: the authorisation address, then the code exchanged.
    async fn sign_in(&self, conn: Uuid, code: &str) -> BTreeMap<String, String> {
        let session = Uuid::now_v7();
        let url = self.k.connect_start(&self.admin, conn, session).await.unwrap();
        let q = query_of(reqwest::Url::parse(&url).unwrap().query().unwrap_or_default());
        self.k.connect_finish(&self.admin, session, &q["state"], Some(code), None).await.unwrap_or_else(|e| panic!("sign-in: {e:?}"));
        q
    }
    /// Makes the sign-in's token due, so the next call refreshes it.
    async fn expire(&self, conn: Uuid) {
        let mut tx = self.k.db.tenant_tx(self.tenant).await.unwrap();
        sqlx::query("update conn_token set expires_at = now() - interval '1 minute' where tenant_id = $1 and conn_id = $2")
            .bind(self.tenant)
            .bind(conn)
            .execute(&mut *tx)
            .await
            .unwrap();
        tx.commit().await.unwrap();
    }
    /// Calls `op` as Social does: as the app, the inputs fitted to what the operation declares.
    async fn call(&self, conn: &str, op: &str, inputs: Value, key: Option<&str>) -> ferrum_kernel::Result<Value> {
        let declared: Vec<String> = self.descs.lock().unwrap()[conn]["operations"][op]["inputs"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|i| i.as_str().map(str::to_string))
            .collect();
        let inputs = match inputs {
            Value::Object(o) if !declared.is_empty() => Value::Object(o.into_iter().filter(|(k, _)| declared.contains(k)).collect()),
            other => other,
        };
        let c = Call { connection: conn.into(), operation: op.into(), inputs, key: key.map(str::to_string), entity: None, record: None };
        connections::call(&self.k, &self.admin, Some("social"), &c).await
    }
    /// A stored file, and what Social gives `publish_media` for it (src/posts.rs `media_inputs`; the post's `kind`
    /// replaces the medium's, as Social's common inputs do).
    async fn medium(&self, name: &str, ct: &str, bytes: &[u8], post_kind: Option<&str>, settings: Value, account_id: Option<&str>) -> Value {
        let f = self.k.store_file(&self.admin, None, name, ct, bytes).await.unwrap();
        json!({"file": f["id"], "name": name, "content_type": ct, "alt": "What it shows", "size": bytes.len(), "kind": post_kind, "thumbnail": null,
               "width": 1080, "height": 1080, "seconds": if ct.starts_with("video/") { json!(12.0) } else { Value::Null }, "item": 0, "index": 0,
               "settings": settings, "account_id": account_id, "handle": null, "items": 1})
    }
    /// A Social account on the connection.
    async fn account(&self, name: &str, conn: Uuid) -> Uuid {
        let d = self
            .k
            .insert(
                &self.admin,
                "SocialAccount",
                obj(json!({"account_name": name, "brand": self.brand.to_string(), "handle": name, "connection": conn.to_string()})),
            )
            .await
            .unwrap();
        uid(&d["id"])
    }
    async fn action(&self, account: Uuid, name: &str, params: Value) -> ferrum_kernel::Result<Value> {
        self.k.run_action(&self.admin, "SocialAccount", account, name, params).await
    }
    async fn pull(&self, conn: Uuid, name: &str) {
        self.k.pull_now(&self.admin, conn, name).await.unwrap();
        for _ in 0..3 {
            let mut tx = self.k.db.system_tx().await.unwrap();
            sqlx::query("update job set run_at = now() where status = 'queued'").execute(&mut *tx).await.unwrap();
            tx.commit().await.unwrap();
            while self.k.run_jobs_once(4).await.unwrap() > 0 {}
        }
    }
    async fn messages(&self, filters: Value) -> Vec<Doc> {
        let q = ferrum_kernel::ListQuery { filters: ferrum_kernel::query::parse_filters(&filters).unwrap(), limit: Some(200), ..Default::default() };
        self.k.list(&self.admin, "SocialMessage", &q).await.unwrap().rows
    }
    fn net(&self) -> std::sync::MutexGuard<'_, Net> {
        self.net.lock().unwrap()
    }
}

/// A lost answer on a step that may not be sent twice: the call fails, the next attempt refuses to send again.
fn refuses_again(e: &Error) -> bool {
    e.to_string().contains("may have been sent")
}

#[test]
fn the_networks_against_their_fakes() {
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
    let port = serve(net.clone()).await;
    let brand = uid(&k.insert(&admin, "SocialBrand", obj(json!({"brand_name": "Northwind"}))).await.unwrap()["id"]);
    let t = T { k: k.clone(), tenant, admin, net, base: format!("http://127.0.0.1:{port}"), brand, descs: Mutex::default() };
    x(&t).await;
    linkedin(&t).await;
    linkedin_page(&t).await;
    facebook(&t).await;
    instagram(&t).await;
    instagram_standalone(&t).await;
}

/// The tables of the workspace's database any of `secrets` rests in, as text.
async fn rests(k: &Kernel, secrets: &[&str]) -> Vec<String> {
    let like: Vec<String> = secrets.iter().map(|s| format!("%{s}%")).collect();
    let mut tx = k.db.pool.begin().await.unwrap();
    sqlx::query("select set_config('ferrum.system', 'on', true)").execute(&mut *tx).await.unwrap();
    let tables: Vec<String> =
        sqlx::query_scalar("select table_name::text from information_schema.tables where table_schema = 'public' and table_type = 'BASE TABLE' order by 1")
            .fetch_all(&mut *tx)
            .await
            .unwrap();
    let mut out = Vec::new();
    for table in tables {
        let hits: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!("select count(*) from \"{table}\" x where x::text ilike any($1)")))
            .bind(&like)
            .fetch_one(&mut *tx)
            .await
            .unwrap();
        if hits > 0 {
            out.push(format!("{table}: {hits}"));
        }
    }
    out
}

/// A path's segment as it was meant, percent-decoded.
fn decoded(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && let Some(v) = s.get(i + 1..i + 3).and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(v);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).to_string()
}

/// Basic client authentication as the token address must see it.
fn basic(id: &str, secret: &str) -> String {
    format!("Basic {}", b64(format!("{id}:{secret}").as_bytes()))
}

fn b64(data: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for c in data.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            out.push(if i <= c.len() { A[(n >> (18 - 6 * i) & 63) as usize] as char } else { '=' });
        }
    }
    out
}

// ───────────── X ─────────────

fn fake_x(n: &mut Net, r: &Req) -> Resp {
    let err = |code: u16, detail: &str| status(code, json!({"title": "Problem", "detail": detail, "status": code}));
    if r.path == "/2/oauth2/token" {
        if r.header("authorization") != Some(basic("x-client", "x-secret").as_str()) {
            return err(401, "Unauthorized client");
        }
        let f = r.form();
        let fine = match f.get("grant_type").map(String::as_str) {
            Some("authorization_code") => f.get("code_verifier").is_some_and(|v| v.len() >= 43) && f.get("code").is_some_and(|c| c == "CODE-x"),
            Some("refresh_token") => n.list("x:refresh").contains(&json!(f.get("refresh_token"))),
            _ => false,
        };
        if !fine {
            return status(400, json!({"error": "invalid_request", "error_description": "Value passed for the token was invalid."}));
        }
        let i = n.next();
        let (a, rt) = (format!("XA-{i}"), format!("XR-{i}"));
        n.tokens("x").push(json!(a));
        // Refresh tokens are single use: the one used goes.
        n.list("x:refresh").retain(|t| Some(t) != f.get("refresh_token").map(|s| json!(s)).as_ref());
        n.list("x:refresh").push(json!(rt));
        return ok(json!({"token_type": "bearer", "access_token": a, "refresh_token": rt, "expires_in": 7200, "scope": "tweet.read tweet.write"}));
    }
    if !n.valid("x", r) {
        return err(401, "Unauthorized");
    }
    let user = json!({"id": "u1", "username": "northwind", "name": "Northwind", "profile_image_url": "https://pbs.twimg.test/n.jpg", "public_metrics": {"followers_count": 42}});
    let seg: Vec<&str> = r.path.trim_start_matches('/').split('/').collect();
    match (r.method.as_str(), seg.as_slice()) {
        ("GET", ["2", "users", "me"]) => ok(json!({"data": user})),
        ("POST", ["2", "tweets"]) => {
            let b = r.json();
            let Some(text) = b["text"].as_str() else { return err(400, "text is required") };
            if n.list("x:tweets").iter().any(|t| t["text"] == text) {
                return err(403, "You are not allowed to create a Tweet with duplicate content.");
            }
            for m in b["media"]["media_ids"].as_array().into_iter().flatten() {
                let media = n.get("x:media");
                let state = &media[m.as_str().unwrap_or_default()];
                if state["finalized"] != true || state["state"] != "succeeded" {
                    return err(400, "Your media IDs are invalid.");
                }
            }
            if let Some(to) = b["reply"]["in_reply_to_tweet_id"].as_str()
                && !n.list("x:tweets").iter().any(|t| t["id"] == to)
            {
                return err(400, "The Tweet you are replying to does not exist.");
            }
            let id = format!("17{:05}", n.next());
            n.list("x:tweets").push(json!({"id": id, "text": text, "media": b["media"]["media_ids"], "reply_to": b["reply"]["in_reply_to_tweet_id"]}));
            status(201, json!({"data": {"id": id, "text": text}}))
        }
        ("DELETE", ["2", "tweets", id]) => {
            let id = id.to_string();
            n.list("x:tweets").retain(|t| t["id"] != json!(id));
            ok(json!({"data": {"deleted": true}}))
        }
        ("GET", ["2", "tweets", id]) if r.query.get("tweet.fields").is_some_and(|f| f.contains("public_metrics")) => ok(
            json!({"data": {"id": id, "public_metrics": {"impression_count": 900, "like_count": 7, "retweet_count": 2, "reply_count": 1, "quote_count": 1, "bookmark_count": 3}}}),
        ),
        ("POST", ["2", "users", "u1", what @ ("retweets" | "likes")]) => {
            let id = r.json()["tweet_id"].clone();
            n.list(&format!("x:{what}")).push(id);
            ok(json!({"data": {"retweeted": true, "liked": true}}))
        }
        ("POST", ["2", "media", "upload", "initialize"]) => {
            let b = r.json();
            let video = b["media_type"].as_str().unwrap_or_default().starts_with("video/");
            let want = if video { "tweet_video" } else { "tweet_image" };
            if b["media_category"] != want || !b["total_bytes"].is_u64() {
                return err(400, "media_category and total_bytes are required");
            }
            let id = format!("M{}", n.next());
            let mut media = n.get("x:media");
            if media.is_null() {
                media = json!({});
            }
            media[&id] = json!({"total": b["total_bytes"], "bytes": 0, "segments": [], "video": video, "finalized": false, "polls": 0, "state": "succeeded"});
            n.set("x:media", media);
            ok(json!({"data": {"id": id, "media_key": format!("7_{id}"), "expires_after_secs": 86400}}))
        }
        ("POST", ["2", "media", "upload", id, "append"]) => {
            let parts = r.parts();
            let (Some(seg), Some(media)) = (parts.get("segment_index"), parts.get("media")) else { return err(400, "segment_index and media are required") };
            let mut all = n.get("x:media");
            let m = &mut all[*id];
            if m.is_null() {
                return err(400, "unknown media");
            }
            m["segments"].as_array_mut().unwrap().push(json!(String::from_utf8_lossy(&seg.2).to_string()));
            m["bytes"] = json!(m["bytes"].as_u64().unwrap() + media.2.len() as u64);
            n.set("x:media", all);
            empty(204)
        }
        ("POST", ["2", "media", "upload", id, "finalize"]) => {
            let mut all = n.get("x:media");
            let m = &mut all[*id];
            if m["bytes"] != m["total"] {
                return err(400, "Segments do not add up to provided total file size.");
            }
            m["finalized"] = json!(true);
            let video = m["video"] == true;
            if video {
                m["state"] = json!("pending");
            }
            n.set("x:media", all);
            let info = if video { json!({"state": "pending", "check_after_secs": 1}) } else { Value::Null };
            let mut data = json!({"id": id, "media_key": format!("7_{id}"), "size": 1});
            if !info.is_null() {
                data["processing_info"] = info;
            }
            ok(json!({"data": data}))
        }
        ("GET", ["2", "media", "upload"]) if r.query.get("command").map(String::as_str) == Some("STATUS") => {
            let id = r.query.get("media_id").cloned().unwrap_or_default();
            let mut all = n.get("x:media");
            let m = &mut all[&id];
            if m["finalized"] != true {
                return err(400, "media is not finalized");
            }
            m["polls"] = json!(m["polls"].as_u64().unwrap_or(0) + 1);
            if m["polls"].as_u64() >= Some(2) {
                m["state"] = json!("succeeded");
            }
            let state = m["state"].clone();
            n.set("x:media", all);
            ok(json!({"data": {"id": id, "processing_info": {"state": state, "check_after_secs": 1, "progress_percent": 50}}}))
        }
        ("POST", ["2", "media", "metadata"]) => {
            let b = r.json();
            n.list("x:alts").push(json!([b["id"], b["metadata"]["alt_text"]["text"]]));
            ok(json!({"data": {"id": b["id"]}}))
        }
        ("POST", ["2", "dm_conversations", "with", to, "messages"]) => {
            let id = format!("DM{}", n.next());
            n.list("x:dms").push(json!({"to": to, "text": r.json()["text"]}));
            status(201, json!({"data": {"dm_conversation_id": "c1", "dm_event_id": id}}))
        }
        ("GET", ["2", "users", "by"]) => {
            let names: Vec<&str> = r.query.get("usernames").map(|u| u.split(',').collect()).unwrap_or_default();
            ok(
                json!({"data": names.iter().map(|u| json!({"id": format!("id-{u}"), "username": u, "name": u.to_uppercase(), "profile_image_url": "https://pbs.twimg.test/u.jpg"})).collect::<Vec<_>>()}),
            )
        }
        ("GET", ["2", "users", "u1", "mentions"]) => {
            if r.query.contains_key("since_id") {
                return ok(json!({"meta": {"result_count": 0}}));
            }
            ok(json!({"data": [
                {"id": "901", "text": "@northwind lovely", "author_id": "a9", "conversation_id": "17001", "referenced_tweets": [{"type": "replied_to", "id": "17001"}]},
                {"id": "902", "text": "hey @northwind", "author_id": "a8", "conversation_id": "902"}
            ], "meta": {"newest_id": "902", "oldest_id": "901", "result_count": 2}}))
        }
        _ => err(404, "Not Found"),
    }
}

async fn x(t: &T) {
    let conn = t.connect("x", "x", &["api.x.com"], json!({"client_secret": "x-secret"}), |_| {}).await;
    let q = t.sign_in(conn, "CODE-x").await;
    assert_eq!((q.get("code_challenge_method").map(String::as_str), q.get("client_id").map(String::as_str)), (Some("S256"), Some("x-client")), "PKCE: {q:?}");
    assert!(q["scope"].split(' ').any(|s| s == "offline.access") && q["scope"].contains("media.write"), "{q:?}");
    assert_eq!(t.net().last("x", "GET", "/2/users/me").bearer().as_deref(), Some("XA-1"), "the account read after the sign-in, with its new token");

    // Text, then a thread answering it.
    let key = format!("publish:{}", Uuid::now_v7());
    let first = t.call("x", "publish", post_in("Spring is here", Some("https://northwind.test/spring"), &[], &[]), Some(&key)).await.unwrap();
    assert_eq!(first["url"], format!("https://x.com/i/web/status/{}", first["id"].as_str().unwrap()));
    assert_eq!(t.net().list("x:tweets")[0]["text"], "Spring is here https://northwind.test/spring");
    let second = follow_in(post_in("And then summer", None, &[], &[]), 1, 2, first["id"].as_str().unwrap(), &kept(0, &first));
    let out = t.call("x", "publish", second, Some(&format!("{key}:1"))).await.unwrap();
    assert_eq!(t.net().list("x:tweets")[1]["reply_to"], first["id"], "a thread: each item answers the one before");

    // Two pictures and a video: initialised, appended in segments, finalised, the video waited for, alt text set.
    let video: Vec<u8> = (0..(4194304 + 10)).map(|i| (i % 251) as u8).collect();
    let mut ids = vec![];
    for (i, (name, ct, bytes)) in [("a.png", "image/png", b"PNG-ONE".to_vec()), ("clip.mp4", "video/mp4", video.clone())].into_iter().enumerate() {
        let inputs = t.medium(name, ct, &bytes, None, json!({}), None).await;
        let r = t.call("x", "publish_media", inputs, Some(&format!("media:{key}:2:{i}"))).await.unwrap();
        ids.push(r["id"].as_str().unwrap().to_string());
    }
    {
        let n = t.net();
        let media = n.get("x:media");
        assert_eq!(media[&ids[1]]["segments"], json!(["0", "1"]), "the video in two segments, in order");
        assert_eq!(media[&ids[1]]["bytes"], video.len());
        assert_eq!(media[&ids[1]]["state"], "succeeded", "waited for until processed");
        let asked = n.asked("x");
        let at = |p: &str| asked.iter().position(|a| a == p).unwrap_or_else(|| panic!("{p}: {asked:?}"));
        assert!(at(&format!("POST /2/media/upload/{}/finalize", ids[1])) < at("GET /2/media/upload"), "status asked after finalising");
        assert_eq!(n.kept["x:alts"].as_array().unwrap().len(), 2, "each medium's description set");
    }
    // A video is not posted until processed: here it is. The picture and the video together are refused by the rules,
    // so they go as two posts.
    let pic = t.call("x", "publish", post_in("With a picture", None, &[("a.png", "image/png", 7)], &ids[..1]), Some(&format!("{key}:p"))).await.unwrap();
    assert!(pic["id"].is_string());
    t.call("x", "publish", post_in("With a video", None, &[("clip.mp4", "video/mp4", video.len() as i64)], &ids[1..]), Some(&format!("{key}:v")))
        .await
        .unwrap();

    // A lost answer on the post: one post, and the call says it may have been sent rather than sending again.
    let lost = format!("publish:{}", Uuid::now_v7());
    t.net().lose.insert("POST /x/2/tweets".into());
    let before = t.net().list("x:tweets").len();
    assert!(t.call("x", "publish", post_in("Only once", None, &[], &[]), Some(&lost)).await.is_err());
    let e = t.call("x", "publish", post_in("Only once", None, &[], &[]), Some(&lost)).await.unwrap_err();
    assert!(refuses_again(&e), "{e}");
    assert_eq!(t.net().list("x:tweets").len(), before + 1, "exactly one post");

    // A refusal, in X's own words.
    let e = t
        .call("x", "publish", post_in("Spring is here https://northwind.test/spring", None, &[], &[]), Some(&format!("publish:{}", Uuid::now_v7())))
        .await
        .unwrap_err();
    assert!(e.to_string().contains("duplicate content"), "{e}");

    // First comment, repost, like, delete, a direct message.
    let c = t
        .call("x", "reply", json!({"text": "More soon", "in_reply_to": first["id"], "root": first["id"]}), Some(&format!("first_comment:{key}")))
        .await
        .unwrap();
    assert_eq!(t.net().list("x:tweets").iter().find(|x| x["id"] == c["id"]).unwrap()["reply_to"], first["id"]);
    t.call("x", "repost", json!({"id": first["id"]}), Some("repost:1")).await.unwrap();
    t.call("x", "like", json!({"id": first["id"]}), Some("like:1")).await.unwrap();
    {
        let mut n = t.net();
        assert_eq!((n.list("x:retweets").clone(), n.list("x:likes").clone()), (vec![first["id"].clone()], vec![first["id"].clone()]));
    }
    t.call("x", "delete", json!({"id": out["id"], "item": 1, "follow_up": "thread", "root": first["id"]}), Some(&format!("delete:{key}:1"))).await.unwrap();
    assert!(!t.net().list("x:tweets").iter().any(|x| x["id"] == out["id"]));
    t.call("x", "send_message", json!({"to": "a9", "text": "Thanks!", "author": "a9"}), Some("reply:dm1")).await.unwrap();
    assert_eq!(t.net().list("x:dms")[0]["to"], "a9");

    // A refreshed token: the refresh token, with the client's basic authentication, then the call with the new token.
    t.expire(conn).await;
    let m = t.call("x", "metrics", json!({"id": first["id"], "account_id": null, "handle": "northwind"}), None).await.unwrap();
    assert_eq!((m["impressions"].as_i64(), m["engagements"].as_i64()), (Some(900), Some(11)), "{m}");
    let refreshed = t.net().last("x", "POST", "/2/oauth2/token").form();
    assert_eq!((refreshed["grant_type"].as_str(), refreshed["refresh_token"].as_str()), ("refresh_token", "XR-1"));
    assert!(t.net().last("x", "GET", &format!("/2/tweets/{}", first["id"].as_str().unwrap())).bearer().is_some_and(|b| b != "XA-1"));
    let a = t.call("x", "account", json!({"account_id": null, "handle": "northwind"}), None).await.unwrap();
    assert_eq!((a["followers"].as_i64(), a["handle"].as_str()), (Some(42), Some("northwind")));

    // Mentions to the composer, and the inbox pulled: a reply is a comment on the post it answers.
    let account = t.account("Northwind X", conn).await;
    assert_eq!(t.action(account, "rules", json!({})).await.unwrap()["rules"]["name"], "X", "Social reads the description's rules");
    let found = t.action(account, "mentions", json!({"query": "ana"})).await.unwrap();
    assert_eq!(found["mentions"][0]["text"], "@ana");
    t.pull(conn, "mentions").await;
    let got = t.messages(json!({})).await;
    let c = got.iter().find(|m| m["external_id"] == "901").expect("the reply was read");
    assert_eq!((c["kind"].as_str(), c["post_external_id"].as_str()), (Some("Comment"), Some("17001")));
    assert_eq!(got.iter().find(|m| m["external_id"] == "902").unwrap()["kind"], "Mention");
    assert_eq!(uid(&c["account"]), account);
}

// ───────────── LinkedIn (a member, and an organisation's page) ─────────────

fn fake_linkedin(n: &mut Net, r: &Req) -> Resp {
    let err = |code: u16, m: &str| status(code, json!({"message": m, "status": code}));
    let base = n.base.clone();
    if r.path == "/oauth/v2/accessToken" {
        let f = r.form();
        let client = f.get("client_id").map(String::as_str) == Some("li-client") && f.get("client_secret").map(String::as_str) == Some("li-secret");
        let fine = match f.get("grant_type").map(String::as_str) {
            Some("authorization_code") => {
                f.get("redirect_uri").is_some_and(|u| u.ends_with("/api/connections/callback")) && f.get("code").is_some_and(|c| c.starts_with("CODE-li"))
            }
            Some("refresh_token") => n.list("li:refresh").contains(&json!(f.get("refresh_token"))),
            _ => false,
        };
        if !client || !fine {
            return status(400, json!({"error": "invalid_request", "error_description": "A required parameter is missing or wrong"}));
        }
        let i = n.next();
        n.tokens("li").push(json!(format!("LA-{i}")));
        n.list("li:refresh").push(json!(format!("LR-{i}")));
        return ok(
            json!({"access_token": format!("LA-{i}"), "expires_in": 5184000, "refresh_token": format!("LR-{i}"), "refresh_token_expires_in": 31536000, "scope": f.get("scope")}),
        );
    }
    if r.path.starts_with("/dms-uploads/") {
        if !n.valid("li", r) {
            return err(401, "Empty or invalid access token");
        }
        let key = r.path.trim_start_matches("/dms-uploads/").to_string();
        let mut up = n.get("li:uploads");
        if up.is_null() {
            up = json!({});
        }
        up[&key] = json!(String::from_utf8_lossy(&r.body).to_string());
        n.set("li:uploads", up);
        return Resp(201, vec![("etag".into(), format!("etag-{key}"))], vec![]);
    }
    if !n.valid("li", r) {
        return err(401, "Empty or invalid access token");
    }
    if r.path == "/v2/userinfo" {
        return ok(json!({"sub": "p1", "name": "Ana Person", "picture": "https://media.licdn.test/ana.jpg"}));
    }
    if r.path == "/v2/organizationalEntityAcls" {
        return ok(json!({"elements": [{"role": "ADMINISTRATOR", "organizationalTarget": "urn:li:organization:777",
                                        "organizationalTarget~": {"id": 777, "localizedName": "Northwind Ltd", "vanityName": "northwind"}}]}));
    }
    if r.header("linkedin-version").is_none() || r.header("x-restli-protocol-version") != Some("2.0.0") {
        return err(400, "A Linkedin-Version header is required");
    }
    let seg: Vec<String> = r.path.trim_start_matches("/rest/").split('/').map(decoded).collect();
    let seg: Vec<&str> = seg.iter().map(String::as_str).collect();
    match (r.method.as_str(), seg.as_slice()) {
        ("POST", [what @ ("images" | "documents" | "videos")]) if r.query.get("action").map(String::as_str) == Some("initializeUpload") => {
            let b = r.json();
            let owner = b["initializeUploadRequest"]["owner"].as_str().unwrap_or_default().to_string();
            if !owner.starts_with("urn:li:person:p1") && owner != "urn:li:organization:777" {
                return err(403, "Not enough permissions to upload for this owner");
            }
            let i = n.next();
            let kind = &what[..what.len() - 1];
            let urn = format!("urn:li:{kind}:{i}");
            let mut media = n.get("li:media");
            if media.is_null() {
                media = json!({});
            }
            media[&urn] = json!({"polls": 0, "owner": owner});
            n.set("li:media", media);
            if *what == "videos" {
                let size = b["initializeUploadRequest"]["fileSizeBytes"].as_u64().unwrap_or(0);
                let half = size / 2;
                return ok(json!({"value": {"video": urn, "uploadToken": "", "uploadInstructions": [
                    {"uploadUrl": format!("{base}/liw/dms-uploads/{i}-0"), "firstByte": 0, "lastByte": half - 1},
                    {"uploadUrl": format!("{base}/liw/dms-uploads/{i}-1"), "firstByte": half, "lastByte": size - 1}]}}));
            }
            ok(json!({"value": {"uploadUrl": format!("{base}/liw/dms-uploads/{i}"), kind: urn}}))
        }
        ("POST", ["videos"]) if r.query.get("action").map(String::as_str) == Some("finalizeUpload") => {
            let b = &r.json()["finalizeUploadRequest"];
            let i = b["video"].as_str().unwrap_or_default().rsplit(':').next().unwrap_or_default().to_string();
            if b["uploadedPartIds"] != json!([format!("etag-{i}-0"), format!("etag-{i}-1")]) {
                return err(400, "uploadedPartIds do not match the parts");
            }
            n.list("li:finalized").push(b["video"].clone());
            empty(200)
        }
        ("GET", [what @ ("videos" | "images" | "documents"), urn]) => {
            let mut media = n.get("li:media");
            let finalized = n.list("li:finalized").contains(&json!(urn));
            let m = &mut media[*urn];
            if m.is_null() {
                return err(404, "Not found");
            }
            if m["owner"].as_str().is_some_and(|o| o.starts_with("urn:li:person")) && *what != "videos" {
                return err(403, "Not enough permissions to access: images.GET");
            }
            m["polls"] = json!(m["polls"].as_u64().unwrap_or(0) + 1);
            let status = match (*what, finalized, m["polls"].as_u64().unwrap_or(0)) {
                ("videos", false, _) => "WAITING_UPLOAD",
                ("videos", true, p) if p < 3 => "PROCESSING",
                _ => "AVAILABLE",
            };
            n.set("li:media", media);
            ok(json!({"id": urn, "status": status}))
        }
        ("POST", ["posts"]) => {
            let b = r.json();
            let text = b["commentary"].as_str().unwrap_or_default().to_string();
            if b["author"] != "urn:li:person:p1" && b["author"] != "urn:li:organization:777" {
                return err(403, "Not enough permissions to post as this author");
            }
            if n.list("li:posts").iter().any(|p| p["commentary"] == text && p["author"] == b["author"]) {
                return err(422, "Content is a duplicate of an existing post");
            }
            let id = format!("urn:li:share:{}", n.next());
            n.list("li:posts")
                .push(json!({"id": id, "author": b["author"], "commentary": text, "content": b["content"], "lifecycleState": b["lifecycleState"]}));
            Resp(201, vec![("x-restli-id".into(), id)], vec![])
        }
        ("GET", ["posts"]) if r.query.get("q").map(String::as_str) == Some("author") => {
            let author = r.query.get("author").cloned().unwrap_or_default();
            let mine: Vec<Value> = n.list("li:posts").iter().filter(|p| p["author"] == json!(author)).cloned().collect();
            ok(json!({"elements": mine}))
        }
        ("GET", ["posts", urn]) => match n.list("li:posts").iter().find(|p| p["id"] == json!(urn)).cloned() {
            Some(p) => ok(p),
            None => err(404, "Not found"),
        },
        ("DELETE", ["posts", urn]) => {
            let urn = urn.to_string();
            n.list("li:posts").retain(|p| p["id"] != json!(urn));
            empty(204)
        }
        ("POST", ["socialActions", post, "comments"]) => {
            let b = r.json();
            if b["object"] != json!(post) {
                return err(400, "object must be the post the comment is under");
            }
            let id = format!("{}", 7000 + n.next());
            n.list("li:comments").push(json!({"id": id, "post": post, "actor": b["actor"], "text": b["message"]["text"]}));
            Resp(201, vec![("x-restli-id".into(), format!("urn:li:comment:({post},{id})"))], json!({"id": id, "object": post}).to_string().into_bytes())
        }
        ("DELETE", ["socialActions", _post, "comments", id]) => {
            let id = id.to_string();
            n.list("li:comments").retain(|c| c["id"] != json!(id));
            empty(204)
        }
        ("GET", ["organizationalEntityShareStatistics"]) => {
            if r.query.get("organizationalEntity").map(String::as_str) != Some("urn:li:organization:777") || !r.query.contains_key("shares") {
                return err(400, "organizationalEntity and shares are required");
            }
            ok(
                json!({"elements": [{"totalShareStatistics": {"impressionCount": 500, "uniqueImpressionsCount": 300, "clickCount": 20, "likeCount": 9, "commentCount": 3, "shareCount": 2, "engagement": 0.05}}]}),
            )
        }
        ("GET", ["networkSizes", urn]) if *urn == "urn:li:organization:777" => ok(json!({"firstDegreeSize": 1234})),
        ("GET", ["organizations", "777"]) => ok(json!({"id": 777, "localizedName": "Northwind Ltd", "vanityName": "northwind"})),
        _ => err(404, "Not found"),
    }
}

async fn linkedin(t: &T) {
    let conn = t
        .connect("linkedin", "li", &[], json!({"client_secret": "li-secret"}), |d| {
            let text =
                d.to_string().replace("https://api.linkedin.com", &format!("{}/li", t.base)).replace("https://www.linkedin.com", &format!("{}/liw", t.base));
            *d = serde_json::from_str(&text).unwrap();
        })
        .await;
    let q = t.sign_in(conn, "CODE-li-1").await;
    assert!(q["scope"].contains("w_member_social"), "{q:?}");
    // Words LinkedIn's text format reserves are escaped, so they are read as written.
    let key = format!("publish:{}", Uuid::now_v7());
    let first = t.call("linkedin", "publish", post_in("Hello (world) #news_today @me", None, &[], &[]), Some(&key)).await.unwrap();
    {
        let mut n = t.net();
        let p = n.list("li:posts")[0].clone();
        assert_eq!(p["commentary"], r"Hello \(world\) \#news\_today \@me");
        assert_eq!((p["author"].as_str(), p["lifecycleState"].as_str()), (Some("urn:li:person:p1"), Some("PUBLISHED")), "as the member the sign-in read");
    }
    assert!(first["url"].as_str().unwrap().ends_with(&format!("/feed/update/{}", first["id"].as_str().unwrap())), "{first}");
    // A picture, registered, put to the address given, attached with its description.
    let pic = t.medium("cat.png", "image/png", b"PNG-CAT", None, json!({}), None).await;
    let up = t.call("linkedin", "publish_media", pic, Some(&format!("media:{key}:9:0"))).await.unwrap();
    assert!(up["id"].as_str().unwrap().starts_with("urn:li:image:"));
    let ids = vec![up["id"].as_str().unwrap().to_string()];
    t.call("linkedin", "publish", post_in("With a cat", None, &[("cat.png", "image/png", 7)], &ids), Some(&format!("{key}:p"))).await.unwrap();
    {
        let mut n = t.net();
        let p = n.list("li:posts").last().cloned().unwrap();
        assert_eq!(p["content"]["media"], json!({"id": ids[0], "altText": "What it shows"}));
        assert!(n.get("li:uploads").as_object().unwrap().values().any(|v| v == "PNG-CAT"), "the bytes went to the upload address");
    }
    // A video: cut into the parts LinkedIn names, finalised with their etags, waited for.
    let video: Vec<u8> = (0..1000).map(|i| (i % 200) as u8 + 32).collect();
    let v = t.medium("clip.mp4", "video/mp4", &video, None, json!({}), None).await;
    let vid = t.call("linkedin", "publish_media", v, Some(&format!("media:{key}:9:1"))).await.unwrap();
    assert_eq!(t.net().list("li:finalized").clone(), vec![vid["id"].clone()]);
    let asked = t.net().asked("li");
    assert!(asked.iter().filter(|a| a.starts_with("GET /rest/videos/")).count() >= 2, "{asked:?}");
    // A document carousel, its title from the settings.
    let doc = t.medium("deck.pdf", "application/pdf", b"%PDF-1.7 deck", Some("document"), json!({"document_title": "Our deck"}), None).await;
    let d = t.call("linkedin", "publish_media", doc, Some(&format!("media:{key}:9:2"))).await.unwrap();
    let mut inputs = post_in("The deck", None, &[("deck.pdf", "application/pdf", 13)], &[d["id"].as_str().unwrap().to_string()]);
    inputs["kind"] = json!("document");
    inputs["settings"] = json!({"document_title": "Our deck"});
    t.call("linkedin", "publish", inputs, Some(&format!("{key}:d"))).await.unwrap();
    assert_eq!(t.net().list("li:posts").last().unwrap()["content"]["media"]["title"], "Our deck");
    // A follow-up as a comment, and the first comment, as the member.
    let follow = follow_in(post_in("More in the comments", None, &[], &[]), 1, 2, first["id"].as_str().unwrap(), &kept(0, &first));
    let c = t.call("linkedin", "reply", follow, Some(&format!("{key}:1"))).await.unwrap();
    {
        let mut n = t.net();
        let made = n.list("li:comments")[0].clone();
        assert_eq!((made["actor"].as_str(), made["post"].as_str(), made["id"].as_str()), (Some("urn:li:person:p1"), first["id"].as_str(), c["id"].as_str()));
    }
    // Deleting the comment under its post, then the post.
    t.call("linkedin", "delete", json!({"id": c["id"], "item": 1, "follow_up": "comment", "root": first["id"]}), Some(&format!("delete:{key}:1")))
        .await
        .unwrap();
    assert!(t.net().list("li:comments").is_empty());
    t.call("linkedin", "delete", json!({"id": first["id"], "item": 0, "follow_up": null, "root": first["id"]}), Some(&format!("delete:{key}"))).await.unwrap();
    assert!(!t.net().list("li:posts").iter().any(|p| p["id"] == first["id"]));
    // A lost answer: a member's posts cannot be read back, so the call says it may have been sent, and there is one.
    t.net().lose.insert("POST /li/rest/posts".into());
    let lost = format!("publish:{}", Uuid::now_v7());
    let before = t.net().list("li:posts").len();
    assert!(t.call("linkedin", "publish", post_in("Once only", None, &[], &[]), Some(&lost)).await.is_err());
    assert!(refuses_again(&t.call("linkedin", "publish", post_in("Once only", None, &[], &[]), Some(&lost)).await.unwrap_err()));
    assert_eq!(t.net().list("li:posts").len(), before + 1);
    // A refusal in LinkedIn's words.
    let e = t.call("linkedin", "publish", post_in("Once only", None, &[], &[]), Some(&format!("publish:{}", Uuid::now_v7()))).await.unwrap_err();
    assert!(e.to_string().contains("duplicate of an existing post"), "{e}");
    let account = t.account("Ana on LinkedIn", conn).await;
    assert_eq!(t.action(account, "rules", json!({})).await.unwrap()["rules"]["name"], "LinkedIn");
    let a = t.call("linkedin", "account", json!({"account_id": null, "handle": null}), None).await.unwrap();
    assert_eq!(a["name"], "Ana Person");
}

async fn linkedin_page(t: &T) {
    let conn = t
        .connect("linkedin_page", "li", &[], json!({"client_secret": "li-secret"}), |d| {
            let text =
                d.to_string().replace("https://api.linkedin.com", &format!("{}/li", t.base)).replace("https://www.linkedin.com", &format!("{}/liw", t.base));
            *d = serde_json::from_str(&text).unwrap();
        })
        .await;
    let q = t.sign_in(conn, "CODE-li-2").await;
    assert!(q["scope"].contains("w_organization_social") && q["scope"].contains("rw_organization_admin"), "{q:?}");
    // The page picked from what the sign-in administers.
    let account = t.account("Northwind on LinkedIn", conn).await;
    let reach = t.action(account, "reachable", json!({})).await.unwrap();
    assert_eq!((reach["accounts"][0]["id"].as_str(), reach["accounts"][0]["name"].as_str()), (Some("777"), Some("Northwind Ltd")), "{reach}");
    t.action(account, "choose", json!({"picked": ["777"]})).await.unwrap();
    let row = t.k.get(&t.admin, "SocialAccount", account, false).await.unwrap();
    assert_eq!((row["network_id"].as_str(), row["handle"].as_str()), (Some("777"), Some("northwind")));
    let page = |text: &str| {
        let mut v = post_in(text, Some("https://northwind.test"), &[], &[]);
        v["account_id"] = json!("777");
        v
    };
    // A picture as the organisation, waited for until available, then the post.
    let key = format!("publish:{}", Uuid::now_v7());
    let pic = t.medium("cat.png", "image/png", b"PNG-PAGE", None, json!({}), Some("777")).await;
    let up = t.call("linkedin_page", "publish_media", pic, Some(&format!("media:{key}"))).await.unwrap();
    assert!(t.net().asked("li").iter().any(|a| a.starts_with("GET /rest/images/")), "a page's picture is waited for");
    let mut v = page("Spring at Northwind");
    v["media"] = json!([up["id"]]);
    v["media_items"] = json!([{"id": up["id"], "alt": "A cat", "content_type": "image/png"}]);
    let first = t.call("linkedin_page", "publish", v, Some(&key)).await.unwrap();
    assert_eq!(t.net().list("li:posts").last().unwrap()["author"], "urn:li:organization:777");
    // A lost answer on a page's post: read back among the page's posts, not sent again.
    t.net().lose.insert("POST /li/rest/posts".into());
    let lost = format!("publish:{}", Uuid::now_v7());
    let before = t.net().list("li:posts").len();
    assert!(t.call("linkedin_page", "publish", page("Exactly once"), Some(&lost)).await.is_err());
    let found = t.call("linkedin_page", "publish", page("Exactly once"), Some(&lost)).await.unwrap();
    assert_eq!(t.net().list("li:posts").len(), before + 1, "one post");
    assert_eq!(found["id"], t.net().list("li:posts").last().unwrap()["id"], "the one read back");
    // The first comment, as the organisation that wrote the post (Social gives a first comment no account).
    t.call(
        "linkedin_page",
        "reply",
        json!({"text": "Ask us anything", "in_reply_to": first["id"], "root": first["id"]}),
        Some(&format!("first_comment:{key}")),
    )
    .await
    .unwrap();
    assert_eq!(t.net().list("li:comments").last().unwrap()["actor"], "urn:li:organization:777");
    // Numbers, after the token is refreshed.
    t.expire(conn).await;
    let m = t.call("linkedin_page", "metrics", json!({"id": first["id"], "account_id": "777", "handle": "northwind"}), None).await.unwrap();
    assert_eq!(
        (m["impressions"].as_i64(), m["reach"].as_i64(), m["clicks"].as_i64(), m["engagements"].as_i64()),
        (Some(500), Some(300), Some(20), Some(14)),
        "{m}"
    );
    assert_eq!(t.net().last("liw", "POST", "/oauth/v2/accessToken").form()["grant_type"], "refresh_token");
    let a = t.call("linkedin_page", "account", json!({"account_id": "777", "handle": "northwind"}), None).await.unwrap();
    assert_eq!((a["followers"].as_i64(), a["name"].as_str()), (Some(1234), Some("Northwind Ltd")));
}

// ───────────── Facebook Page ─────────────

const PAGE_TOKEN: &str = "PAGE-TOKEN-Qq7";

fn fake_facebook(n: &mut Net, r: &Req) -> Resp {
    let err = |code: u16, m: &str, c: u32| status(code, json!({"error": {"message": m, "type": "OAuthException", "code": c}}));
    let base = n.base.clone();
    let seg: Vec<&str> = r.path.trim_start_matches('/').split('/').collect();
    if seg.get(1) == Some(&"oauth") {
        if r.query.get("grant_type").map(String::as_str) == Some("fb_exchange_token") {
            if r.query.get("client_secret").map(String::as_str) != Some("fb-secret") || !n.list("fb:user").contains(&json!(r.query.get("fb_exchange_token"))) {
                return err(400, "Error validating access token", 190);
            }
            let long = format!("LONG-{}", n.next());
            n.list("fb:user").push(json!(long));
            return ok(json!({"access_token": long, "token_type": "bearer", "expires_in": 5183944}));
        }
        let f = r.form();
        if f.get("client_id").map(String::as_str) != Some("fb-client")
            || f.get("client_secret").map(String::as_str) != Some("fb-secret")
            || f.get("code").map(String::as_str) != Some("CODE-fb")
        {
            return err(400, "Invalid verification code format.", 100);
        }
        let short = format!("SHORT-{}", n.next());
        n.list("fb:user").push(json!(short));
        return ok(json!({"access_token": short, "token_type": "bearer", "expires_in": 3600}));
    }
    let token = r.bearer().or_else(|| r.header("authorization").and_then(|a| a.strip_prefix("OAuth ")).map(str::to_string)).unwrap_or_default();
    let user = n.list("fb:user").contains(&json!(token));
    let page = token == PAGE_TOKEN;
    let rest: Vec<&str> = seg[1..].to_vec();
    if rest.first() == Some(&"me") {
        if !user {
            return err(401, "Invalid OAuth access token.", 190);
        }
        return ok(
            json!({"data": [{"id": "P1", "name": "Northwind Cafe", "username": "northwindcafe", "access_token": PAGE_TOKEN, "tasks": ["CREATE_CONTENT"],
                                    "picture": {"data": {"url": "https://scontent.test/p1.jpg"}}}]}),
        );
    }
    if !page {
        return err(403, "(#200) This endpoint requires the page's own access token", 200);
    }
    if seg[0] == "video-upload" {
        let vid = seg.last().unwrap().to_string();
        if r.header("offset") != Some("0") || r.header("file_size").map(str::to_string) != Some(r.body.len().to_string()) {
            return err(400, "offset and file_size headers are required", 100);
        }
        let mut v = n.get("fb:videos");
        v[&vid]["bytes"] = json!(r.body.len());
        n.set("fb:videos", v);
        return ok(json!({"success": true}));
    }
    match (r.method.as_str(), rest.as_slice()) {
        ("POST", ["P1", "photos"]) => {
            let parts = r.parts();
            if parts.get("published").map(|p| p.2.as_slice()) != Some(b"false".as_slice()) || !parts.contains_key("source") {
                return err(400, "(#100) source and published are required", 100);
            }
            let id = format!("PH{}", n.next());
            n.list("fb:photos").push(json!({"id": id, "bytes": parts["source"].2.len(), "temporary": String::from_utf8_lossy(&parts["temporary"].2)}));
            ok(json!({"id": id}))
        }
        ("POST", ["P1", edge @ ("video_reels" | "video_stories")]) => {
            let b = r.json();
            match b["upload_phase"].as_str() {
                Some("start") => {
                    let vid = format!("V{}", n.next());
                    let mut v = n.get("fb:videos");
                    if v.is_null() {
                        v = json!({});
                    }
                    v[&vid] = json!({"edge": edge, "bytes": 0, "polls": 0, "published": false});
                    n.set("fb:videos", v);
                    ok(json!({"video_id": vid, "upload_url": format!("{base}/fbr/video-upload/v25.0/{vid}")}))
                }
                Some("finish") => {
                    let vid = b["video_id"].as_str().unwrap_or_default().to_string();
                    let mut v = n.get("fb:videos");
                    if v[&vid]["bytes"] == 0 {
                        return err(400, "The video has not been uploaded", 6000);
                    }
                    v[&vid]["published"] = json!(true);
                    v[&vid]["description"] = b["description"].clone();
                    n.set("fb:videos", v);
                    if *edge == "video_stories" { ok(json!({"success": true, "post_id": format!("S{vid}")})) } else { ok(json!({"success": true})) }
                }
                _ => err(400, "upload_phase is start or finish", 100),
            }
        }
        ("POST", ["P1", "feed"]) => {
            let b = r.json();
            for m in b["attached_media"].as_array().into_iter().flatten() {
                if !n.list("fb:photos").iter().any(|p| p["id"] == m["media_fbid"]) {
                    return err(400, "(#100) Invalid attached_media", 100);
                }
            }
            if n.list("fb:posts").iter().any(|p| p["message"] == b["message"]) {
                return err(400, "(#506) Duplicate status message", 506);
            }
            let id = format!("P1_{}", n.next());
            n.list("fb:posts").push(json!({"id": id, "message": b["message"], "link": b["link"], "attached_media": b["attached_media"]}));
            ok(json!({"id": id}))
        }
        ("GET", ["P1", "feed"]) => {
            let mut all = n.list("fb:posts").clone();
            all.reverse();
            ok(json!({"data": all}))
        }
        ("POST", ["P1", "videos"]) if seg[0] == "v25.0" => {
            let parts = r.parts();
            let id = format!("VID{}", n.next());
            n.list("fb:posts").push(json!({"id": id, "message": String::from_utf8_lossy(&parts["description"].2), "video_bytes": parts["source"].2.len()}));
            ok(json!({"id": id}))
        }
        ("POST", ["P1", "photo_stories"]) => {
            let photo = r.json()["photo_id"].clone();
            if !n.list("fb:photos").iter().any(|p| p["id"] == photo && p["temporary"] == "true") {
                return err(400, "(#100) A story's photo is uploaded as temporary", 100);
            }
            let id = format!("ST{}", n.next());
            n.list("fb:stories").push(json!({"id": id, "photo": photo}));
            ok(json!({"success": true, "post_id": id}))
        }
        ("POST", [id, "comments"]) => {
            let cid = format!("{id}_c{}", n.next());
            n.list("fb:comments").push(json!({"id": cid, "on": id, "message": r.json()["message"]}));
            ok(json!({"id": cid}))
        }
        ("DELETE", [id]) => {
            let id = id.to_string();
            n.list("fb:posts").retain(|p| p["id"] != json!(id));
            ok(json!({"success": true}))
        }
        ("GET", [id, "insights"]) => {
            let _ = id;
            ok(json!({"data": [{"name": "post_media_view", "values": [{"value": 800}]}, {"name": "post_total_media_view_unique", "values": [{"value": 600}]},
                               {"name": "post_clicks", "values": [{"value": 30}]}]}))
        }
        ("GET", ["P1"]) => ok(
            json!({"id": "P1", "name": "Northwind Cafe", "username": "northwindcafe", "followers_count": 4321, "picture": {"data": {"url": "https://scontent.test/p1.jpg"}}}),
        ),
        ("GET", [id]) if r.query.get("fields").is_some_and(|f| f == "status") => {
            let mut v = n.get("fb:videos");
            let m = &mut v[*id];
            if m.is_null() {
                return err(404, "Unsupported get request", 100);
            }
            m["polls"] = json!(m["polls"].as_u64().unwrap_or(0) + 1);
            let uploaded = if m["bytes"] != 0 && m["polls"].as_u64() >= Some(2) { "complete" } else { "in_progress" };
            let publishing = if m["published"] == true { "complete" } else { "not_started" };
            let out = json!({"status": {"video_status": "upload_complete", "uploading_phase": {"status": uploaded}, "processing_phase": {"status": "not_started"},
                                        "publishing_phase": {"status": publishing}}});
            n.set("fb:videos", v);
            ok(out)
        }
        ("GET", [_id]) => ok(
            json!({"reactions": {"data": [], "summary": {"total_count": 12}}, "comments": {"data": [], "summary": {"total_count": 4}}, "shares": {"count": 2}}),
        ),
        _ => err(404, "Unknown path components", 2500),
    }
}

async fn facebook(t: &T) {
    let conn = t
        .connect("facebook", "fb", &["graph.facebook.com", "graph-video.facebook.com", "rupload.facebook.com"], json!({"client_secret": "fb-secret"}), |d| {
            let text = d.to_string().replace(
                &format!("{}/fb/v25.0/{{{{ inputs.account_id }}}}/videos", t.base),
                &format!("{}/fbv/v25.0/{{{{ inputs.account_id }}}}/videos", t.base),
            );
            *d = serde_json::from_str(&text).unwrap();
        })
        .await;
    // The code exchanged, the short token exchanged for a long one, and the pages' own tokens kept sealed with it.
    t.sign_in(conn, "CODE-fb").await;
    assert!(t.net().last("fb", "GET", "/v25.0/me/accounts").bearer().is_some_and(|b| b.starts_with("LONG-")), "the pages read with the long-lived token");
    assert!(rests(&t.k, &[PAGE_TOKEN]).await.is_empty(), "a page's token rests nowhere in the clear: {:?}", rests(&t.k, &[PAGE_TOKEN]).await);
    let account = t.account("Northwind Cafe", conn).await;
    assert_eq!(t.action(account, "rules", json!({})).await.unwrap()["rules"]["name"], "Facebook");
    let reach = t.action(account, "reachable", json!({})).await.unwrap();
    assert_eq!(reach["accounts"][0]["id"], "P1");
    t.action(account, "choose", json!({"picked": ["P1"]})).await.unwrap();
    let on = |mut v: Value, kind: &str| {
        v["account_id"] = json!("P1");
        v["kind"] = json!(kind);
        v
    };
    // Text with its link as the link's card.
    let key = format!("publish:{}", Uuid::now_v7());
    let first = t.call("facebook", "publish", on(post_in("Fresh bread today", Some("https://northwind.test"), &[], &[]), "post"), Some(&key)).await.unwrap();
    assert_eq!(t.net().list("fb:posts")[0]["link"], "https://northwind.test");
    assert_eq!(first["url"], format!("https://www.facebook.com/{}", first["id"].as_str().unwrap()));
    assert!(t.net().last("fb", "POST", "/v25.0/P1/feed").bearer().as_deref() == Some(PAGE_TOKEN), "as the page, with its own token");
    // Two photos, uploaded unpublished, then attached to one post.
    let mut ids = vec![];
    for i in 0..2 {
        let m = t.medium(&format!("p{i}.jpg"), "image/jpeg", b"JPEG-DATA", Some("post"), json!({}), Some("P1")).await;
        ids.push(t.call("facebook", "publish_media", m, Some(&format!("media:{key}:1:{i}"))).await.unwrap()["id"].as_str().unwrap().to_string());
    }
    t.call(
        "facebook",
        "publish",
        on(post_in("Two loaves", None, &[("p0.jpg", "image/jpeg", 9), ("p1.jpg", "image/jpeg", 9)], &ids), "post"),
        Some(&format!("{key}:1")),
    )
    .await
    .unwrap();
    assert_eq!(t.net().list("fb:posts").last().unwrap()["attached_media"].as_array().unwrap().len(), 2);
    // A reel: started, its bytes sent to the upload host, waited for, then finished with its description.
    let video = vec![b'v'; 3000];
    let m = t.medium("reel.mp4", "video/mp4", &video, Some("reel"), json!({}), Some("P1")).await;
    let reel = t.call("facebook", "publish_media", m, Some(&format!("media:{key}:2:0"))).await.unwrap();
    let r = t
        .call(
            "facebook",
            "publish",
            on(post_in("Watch us bake", None, &[("reel.mp4", "video/mp4", 3000)], &[reel["id"].as_str().unwrap().to_string()]), "reel"),
            Some(&format!("{key}:2")),
        )
        .await
        .unwrap();
    {
        let n = t.net();
        let v = &n.get("fb:videos")[r["id"].as_str().unwrap()];
        assert_eq!((v["bytes"].as_u64(), v["published"].as_bool(), v["description"].as_str()), (Some(3000), Some(true), Some("Watch us bake")));
        assert_eq!(
            n.last("fbr", "POST", &format!("/video-upload/v25.0/{}", r["id"].as_str().unwrap())).header("authorization"),
            Some(format!("OAuth {PAGE_TOKEN}").as_str())
        );
    }
    assert!(r["url"].as_str().unwrap().contains("/reel/"));
    // A story of a photo.
    let m = t.medium("s.jpg", "image/jpeg", b"JPEG-STORY", Some("story"), json!({}), Some("P1")).await;
    let s = t.call("facebook", "publish_media", m, Some(&format!("media:{key}:3:0"))).await.unwrap();
    t.call(
        "facebook",
        "publish",
        on(post_in("", None, &[("s.jpg", "image/jpeg", 10)], &[s["id"].as_str().unwrap().to_string()]), "story"),
        Some(&format!("{key}:3")),
    )
    .await
    .unwrap();
    assert_eq!(t.net().list("fb:stories").len(), 1);
    // A video post: its bytes sent with its description to the video host.
    let m = t.medium("long.mp4", "video/mp4", &video, Some("post"), json!({}), Some("P1")).await;
    let v = t.call("facebook", "publish_media", m, Some(&format!("media:{key}:4:0"))).await.unwrap();
    assert!(v["id"].as_str().unwrap().starts_with("file:"));
    t.call(
        "facebook",
        "publish",
        on(post_in("The long one", None, &[("long.mp4", "video/mp4", 3000)], &[v["id"].as_str().unwrap().to_string()]), "post"),
        Some(&format!("{key}:4")),
    )
    .await
    .unwrap();
    assert_eq!(t.net().list("fb:posts").last().unwrap()["video_bytes"], 3000);
    // The first comment, as the page.
    let mut c = json!({"text": "Open till six", "in_reply_to": first["id"], "root": first["id"]});
    c["account_id"] = json!("P1");
    t.call("facebook", "reply", c, Some(&format!("first_comment:{key}"))).await.unwrap();
    assert_eq!(t.net().list("fb:comments")[0]["on"], first["id"]);
    // A lost answer: read back among the page's posts, not posted again.
    t.net().lose.insert("POST /fb/v25.0/P1/feed".into());
    let lost = format!("publish:{}", Uuid::now_v7());
    let before = t.net().list("fb:posts").len();
    assert!(t.call("facebook", "publish", on(post_in("Just once", None, &[], &[]), "post"), Some(&lost)).await.is_err());
    let again = t.call("facebook", "publish", on(post_in("Just once", None, &[], &[]), "post"), Some(&lost)).await.unwrap();
    assert_eq!(t.net().list("fb:posts").len(), before + 1);
    assert_eq!(again["id"], t.net().list("fb:posts").last().unwrap()["id"]);
    // A refusal in Facebook's words.
    let e = t.call("facebook", "publish", on(post_in("Just once", None, &[], &[]), "post"), Some(&format!("publish:{}", Uuid::now_v7()))).await.unwrap_err();
    assert!(e.to_string().contains("Duplicate status message"), "{e}");
    // The long-lived token renewed by exchanging it again, when the user's own token is next used (the picker); the
    // pages' tokens kept.
    t.expire(conn).await;
    t.action(account, "reachable", json!({})).await.unwrap();
    let m = t.call("facebook", "metrics", json!({"id": first["id"], "account_id": "P1", "handle": null}), None).await.unwrap();
    assert_eq!(
        (m["impressions"].as_i64(), m["reach"].as_i64(), m["clicks"].as_i64(), m["engagements"].as_i64()),
        (Some(800), Some(600), Some(30), Some(18)),
        "{m}"
    );
    assert!(t.net().last("fb", "GET", "/v25.0/oauth/access_token").query.get("fb_exchange_token").is_some_and(|t| t.starts_with("LONG-")));
    let a = t.call("facebook", "account", json!({"account_id": "P1", "handle": null}), None).await.unwrap();
    assert_eq!(a["followers"], 4321);
    t.call("facebook", "delete", json!({"id": first["id"], "account_id": "P1"}), Some(&format!("delete:{key}"))).await.unwrap();
    assert!(!t.net().list("fb:posts").iter().any(|p| p["id"] == first["id"]));
    assert!(rests(&t.k, &[PAGE_TOKEN]).await.is_empty(), "{:?}", rests(&t.k, &[PAGE_TOKEN]).await);
}

// ───────────── Instagram (through a Facebook page, and with Instagram's own sign-in) ─────────────

fn fake_instagram(n: &mut Net, r: &Req) -> Resp {
    let err = |code: u16, m: &str, c: u32| status(code, json!({"error": {"message": m, "type": "OAuthException", "code": c}}));
    let base = n.base.clone();
    let seg: Vec<&str> = r.path.trim_start_matches('/').split('/').collect();
    // Sign-in: Facebook's (a code, then a long-lived token), or Instagram's own (a code at api.instagram.com, then
    // ig_exchange_token, renewed by ig_refresh_token).
    let grant = r.query.get("grant_type").map(String::as_str);
    if seg.get(1) == Some(&"oauth") || r.path == "/oauth/access_token" {
        let f = r.form();
        let client =
            f.get("client_id").is_some_and(|c| c == "igf-client" || c == "igs-client") && f.get("client_secret").map(String::as_str) == Some("ig-secret");
        if grant != Some("fb_exchange_token") && (!client || !f.contains_key("code")) {
            return err(400, "Invalid platform app", 101);
        }
        let i = n.next();
        if grant == Some("fb_exchange_token") {
            n.tokens("ig").push(json!(format!("IGL-{i}")));
            return ok(json!({"access_token": format!("IGL-{i}"), "expires_in": 5183944}));
        }
        n.list("ig:short").push(json!(format!("IGS-{i}")));
        if r.path == "/oauth/access_token" {
            return ok(json!({"access_token": format!("IGS-{i}"), "user_id": 17841400, "permissions": "instagram_business_basic"}));
        }
        n.tokens("ig").push(json!(format!("IGS-{i}")));
        return ok(json!({"access_token": format!("IGS-{i}"), "expires_in": 3600}));
    }
    if matches!(grant, Some("ig_exchange_token") | Some("ig_refresh_token")) {
        let given = r.query.get("access_token").cloned().unwrap_or_default();
        let known = if grant == Some("ig_exchange_token") { n.list("ig:short").contains(&json!(given)) } else { n.list("ig:tokens").contains(&json!(given)) };
        if !known || (grant == Some("ig_exchange_token") && r.query.get("client_secret").map(String::as_str) != Some("ig-secret")) {
            return err(400, "Invalid OAuth access token", 190);
        }
        let i = n.next();
        n.tokens("ig").push(json!(format!("IGLL-{i}")));
        return ok(json!({"access_token": format!("IGLL-{i}"), "token_type": "bearer", "expires_in": 5183944}));
    }
    if !n.valid("ig", r) {
        return err(401, "Invalid OAuth access token.", 190);
    }
    let mut cs = n.get("ig:containers");
    if cs.is_null() {
        cs = json!({});
    }
    if seg[0] == "ig-api-upload" {
        let cid = seg.last().unwrap().to_string();
        if r.header("offset") != Some("0") || r.header("file_size").map(str::to_string) != Some(r.body.len().to_string()) || cs[&cid].is_null() {
            return err(400, "offset and file_size are required", 100);
        }
        cs[&cid]["bytes"] = json!(r.body.len());
        n.set("ig:containers", cs);
        return ok(json!({"success": true, "message": "Upload successful"}));
    }
    let rest: Vec<&str> = seg[1..].to_vec();
    let user = |u: &str| u == "IG1" || u == "me";
    let status_of = |c: &Value, all: &Value| -> &'static str {
        if c["published"] == true {
            return "PUBLISHED";
        }
        if c["media_type"] == "CAROUSEL" {
            let kids = c["children"].as_array().cloned().unwrap_or_default();
            return if kids.iter().all(|k| all[k.as_str().unwrap_or_default()]["polls"].as_u64() >= Some(1)) { "FINISHED" } else { "IN_PROGRESS" };
        }
        let video = c["upload"] == true;
        match (video, c["bytes"].as_u64().unwrap_or(0) > 0, c["polls"].as_u64().unwrap_or(0)) {
            (true, false, _) => "IN_PROGRESS",
            (_, _, p) if p >= 1 => "FINISHED",
            _ => "IN_PROGRESS",
        }
    };
    match (r.method.as_str(), rest.as_slice()) {
        ("GET", ["me", "accounts"]) => ok(json!({"data": [
            {"name": "Northwind", "instagram_business_account": {"id": "IG1", "username": "northwind.ig", "name": "Northwind", "profile_picture_url": "https://cdn.test/ig.jpg"}},
            {"name": "A page without Instagram"}]})),
        ("POST", [u, "media"]) if user(u) => {
            let b = r.json();
            let video = b["upload_type"] == "resumable";
            if b["media_type"].is_null() && b["image_url"].as_str().is_none_or(str::is_empty) {
                return err(400, "The parameter image_url is required.", 100);
            }
            if b["media_type"] == "CAROUSEL" && b["children"].as_array().is_none_or(|c| c.len() < 2) {
                return err(400, "A carousel has at least two children", 100);
            }
            let cid = format!("C{}", n.next());
            cs[&cid] = json!({"media_type": b["media_type"], "image_url": b["image_url"], "upload": video, "carousel_item": b["is_carousel_item"], "children": b["children"],
                              "caption": b["caption"], "collaborators": b["collaborators"], "trial": b["trial_params"], "bytes": 0, "polls": 0, "published": false});
            n.set("ig:containers", cs);
            let mut out = json!({"id": cid});
            if video {
                out["uri"] = json!(format!("{base}/igr/ig-api-upload/v25.0/{cid}"));
            }
            ok(out)
        }
        ("GET", [""]) => {
            let ids: Vec<String> = r.query.get("ids").map(|i| i.split(',').map(str::to_string).collect()).unwrap_or_default();
            let mut out = json!({});
            for id in &ids {
                let polls = cs[id]["polls"].as_u64().unwrap_or(0) + 1;
                cs[id]["polls"] = json!(polls);
                out[id] = json!({"id": id, "status_code": status_of(&cs[id], &cs)});
            }
            n.set("ig:containers", cs);
            ok(out)
        }
        ("POST", [u, "media_publish"]) if user(u) => {
            let id = r.json()["creation_id"].as_str().unwrap_or_default().to_string();
            match status_of(&cs[&id], &cs) {
                "FINISHED" => {}
                "PUBLISHED" => return err(400, "The media has already been published", 9007),
                _ => return err(400, "Media ID is not available", 9007),
            }
            cs[&id]["published"] = json!(true);
            let mid = format!("M{}", n.next());
            let caption = cs[&id]["caption"].clone();
            n.set("ig:containers", cs);
            n.list("ig:media").insert(0, json!({"id": mid, "caption": caption, "container": id}));
            ok(json!({"id": mid}))
        }
        ("GET", [u, "media"]) if user(u) => ok(json!({"data": n.list("ig:media").clone()})),
        ("POST", [m, "comments"]) => {
            let id = format!("{}", 1800 + n.next());
            n.list("ig:comments").push(json!({"id": id, "on": m, "message": r.json()["message"]}));
            ok(json!({"id": id}))
        }
        ("GET", [_m, "insights"]) => ok(json!({"data": [{"name": "reach", "values": [{"value": 70}]}, {"name": "views", "values": [{"value": 120}]},
                                                        {"name": "total_interactions", "values": [{"value": 15}]}, {"name": "likes", "values": [{"value": 10}]}]})),
        ("GET", [u]) if user(u) => {
            ok(json!({"id": "IG1", "followers_count": 999, "username": "northwind.ig", "name": "Northwind", "profile_picture_url": "https://cdn.test/ig.jpg"}))
        }
        ("GET", [id]) if id.starts_with('M') => ok(json!({"id": id, "permalink": format!("https://www.instagram.com/p/{id}/")})),
        ("GET", [id]) if id.starts_with('C') => {
            let polls = cs[*id]["polls"].as_u64().unwrap_or(0) + 1;
            cs[*id]["polls"] = json!(polls);
            let st = status_of(&cs[*id], &cs);
            n.set("ig:containers", cs);
            ok(json!({"id": id, "status_code": st, "status": "Finished: Media has been uploaded and it is ready to be published."}))
        }
        _ => err(404, "Unsupported request", 100),
    }
}

/// What Social would give `publish_media` with the public address it does not give yet (see notes-a.md).
async fn ig_medium(t: &T, name: &str, ct: &str, bytes: &[u8], kind: &str, account: Option<&str>) -> Value {
    let mut m = t.medium(name, ct, bytes, Some(kind), json!({}), account).await;
    m["url"] = json!(format!("https://files.northwind.test/{name}"));
    m
}

async fn instagram(t: &T) {
    let conn = t
        .connect("instagram", "igf", &["graph.facebook.com", "rupload.facebook.com"], json!({"client_secret": "ig-secret"}), |d| {
            let text = d.to_string().replace(&format!("{}/igf/ig-api-upload", t.base), &format!("{}/igr/ig-api-upload", t.base));
            *d = serde_json::from_str(&text).unwrap();
        })
        .await;
    t.sign_in(conn, "CODE-ig").await;
    let account = t.account("Northwind Instagram", conn).await;
    assert_eq!(t.action(account, "rules", json!({})).await.unwrap()["rules"]["name"], "Instagram");
    let reach = t.action(account, "reachable", json!({})).await.unwrap();
    assert_eq!(reach["accounts"].as_array().unwrap().len(), 1, "a page without Instagram is not offered: {reach}");
    t.action(account, "choose", json!({"picked": ["IG1"]})).await.unwrap();
    let on = |mut v: Value, kind: &str, settings: Value| {
        v["account_id"] = json!("IG1");
        v["kind"] = json!(kind);
        v["settings"] = settings;
        v
    };
    let key = format!("publish:{}", Uuid::now_v7());
    // One picture: a container from its address, waited for, published, its permalink read.
    let m = ig_medium(t, "a.jpg", "image/jpeg", b"JPEG-A", "post", Some("IG1")).await;
    let a = t.call("instagram", "publish_media", m, Some(&format!("media:{key}"))).await.unwrap();
    assert_eq!(a["id"], "url:https://files.northwind.test/a.jpg");
    let ids = vec![a["id"].as_str().unwrap().to_string()];
    let first = t
        .call(
            "instagram",
            "publish",
            on(post_in("Morning light", None, &[("a.jpg", "image/jpeg", 6)], &ids), "post", json!({"collaborators": ["ana"]})),
            Some(&key),
        )
        .await
        .unwrap();
    assert_eq!(first["url"], format!("https://www.instagram.com/p/{}/", first["id"].as_str().unwrap()));
    {
        let n = t.net();
        let c = n.get("ig:containers")["C".to_string() + &n.get("ig:media")[0]["container"].as_str().unwrap()[1..]].clone();
        assert_eq!(
            (c["image_url"].as_str(), c["caption"].as_str(), c["collaborators"].clone()),
            (Some("https://files.northwind.test/a.jpg"), Some("Morning light"), json!(["ana"]))
        );
    }
    // A carousel of a picture and a video: children made, the video's bytes sent to the upload host, all waited for,
    // then the carousel, waited for, published.
    let video = vec![b'v'; 2048];
    let p = ig_medium(t, "b.jpg", "image/jpeg", b"JPEG-B", "post", Some("IG1")).await;
    let v = ig_medium(t, "c.mp4", "video/mp4", &video, "post", Some("IG1")).await;
    let pi = t.call("instagram", "publish_media", p, Some(&format!("media:{key}:1:0"))).await.unwrap()["id"].as_str().unwrap().to_string();
    let vi = t.call("instagram", "publish_media", v, Some(&format!("media:{key}:1:1"))).await.unwrap()["id"].as_str().unwrap().to_string();
    assert!(vi.starts_with("file:"), "a video goes as bytes, not by address");
    let car = t
        .call(
            "instagram",
            "publish",
            on(post_in("Two views", None, &[("b.jpg", "image/jpeg", 6), ("c.mp4", "video/mp4", 2048)], &[pi, vi]), "post", json!({})),
            Some(&format!("{key}:1")),
        )
        .await
        .unwrap();
    {
        let n = t.net();
        let cs = n.get("ig:containers");
        let carousel = cs.as_object().unwrap().values().find(|c| c["media_type"] == "CAROUSEL").unwrap();
        assert_eq!(carousel["children"].as_array().unwrap().len(), 2);
        let kids: Vec<&Value> = carousel["children"].as_array().unwrap().iter().map(|k| &cs[k.as_str().unwrap()]).collect();
        assert!(kids.iter().all(|k| k["carousel_item"] == true) && kids[1]["bytes"] == 2048, "{kids:?}");
        assert!(n.asked("igf").iter().any(|a| a == "GET /v25.0/"), "the children waited for together");
    }
    assert!(car["id"].is_string());
    // A reel as a trial, and a story.
    let r = ig_medium(t, "r.mp4", "video/mp4", &video, "reel", Some("IG1")).await;
    let ri = t.call("instagram", "publish_media", r, Some(&format!("media:{key}:2:0"))).await.unwrap()["id"].as_str().unwrap().to_string();
    t.call(
        "instagram",
        "publish",
        on(post_in("Reel time", None, &[("r.mp4", "video/mp4", 2048)], &[ri]), "reel", json!({"trial_reel": true})),
        Some(&format!("{key}:2")),
    )
    .await
    .unwrap();
    let s = ig_medium(t, "s.jpg", "image/jpeg", b"JPEG-S", "story", Some("IG1")).await;
    let si = t.call("instagram", "publish_media", s, Some(&format!("media:{key}:3:0"))).await.unwrap()["id"].as_str().unwrap().to_string();
    t.call("instagram", "publish", on(post_in("", None, &[("s.jpg", "image/jpeg", 6)], &[si]), "story", json!({})), Some(&format!("{key}:3"))).await.unwrap();
    {
        let n = t.net();
        let cs = n.get("ig:containers");
        let all: Vec<&Value> = cs.as_object().unwrap().values().collect();
        assert!(all.iter().any(|c| c["media_type"] == "REELS" && c["trial"]["graduation_strategy"] == "MANUAL"));
        assert!(all.iter().any(|c| c["media_type"] == "STORIES" && c["caption"].is_null()));
    }
    // The first comment.
    let mut c = json!({"text": "Tag a friend", "in_reply_to": first["id"], "root": first["id"]});
    c["account_id"] = json!("IG1");
    t.call("instagram", "reply", c, Some(&format!("first_comment:{key}"))).await.unwrap();
    assert_eq!(t.net().list("ig:comments")[0]["on"], first["id"]);
    // A lost answer on publishing: read back among the account's media by its caption, not published twice.
    t.net().lose.insert("POST /igf/v25.0/IG1/media_publish".into());
    let lost = format!("publish:{}", Uuid::now_v7());
    let m = ig_medium(t, "d.jpg", "image/jpeg", b"JPEG-D", "post", Some("IG1")).await;
    let di = t.call("instagram", "publish_media", m, Some(&format!("media:{lost}"))).await.unwrap()["id"].as_str().unwrap().to_string();
    let once = on(post_in("Only one of these", None, &[("d.jpg", "image/jpeg", 6)], std::slice::from_ref(&di)), "post", json!({}));
    let before = t.net().list("ig:media").len();
    assert!(t.call("instagram", "publish", once.clone(), Some(&lost)).await.is_err());
    let got = t.call("instagram", "publish", once, Some(&lost)).await.unwrap();
    assert_eq!(t.net().list("ig:media").len(), before + 1);
    assert_eq!(got["id"], t.net().list("ig:media")[0]["id"]);
    // A refusal in Instagram's words: a picture with no public address.
    let mut bare = t.medium("e.jpg", "image/jpeg", b"JPEG-E", Some("post"), json!({}), Some("IG1")).await;
    bare["url"] = Value::Null;
    let ei = t.call("instagram", "publish_media", bare, Some("media:bare")).await.unwrap()["id"].as_str().unwrap().to_string();
    let e = t
        .call("instagram", "publish", on(post_in("No address", None, &[("e.jpg", "image/jpeg", 6)], &[ei]), "post", json!({})), Some("publish:bare"))
        .await
        .unwrap_err();
    assert!(e.to_string().contains("image_url is required"), "{e}");
    // Numbers, the account, and the long-lived token renewed.
    let m = t.call("instagram", "metrics", json!({"id": first["id"], "account_id": "IG1", "handle": null}), None).await.unwrap();
    assert_eq!((m["impressions"].as_i64(), m["reach"].as_i64(), m["engagements"].as_i64()), (Some(120), Some(70), Some(15)));
    t.expire(conn).await;
    let a = t.call("instagram", "account", json!({"account_id": "IG1", "handle": null}), None).await.unwrap();
    assert_eq!((a["followers"].as_i64(), a["handle"].as_str()), (Some(999), Some("northwind.ig")));
    assert!(t.net().last("igf", "GET", "/v25.0/oauth/access_token").query.get("fb_exchange_token").is_some_and(|t| t.starts_with("IGL-")));
}

async fn instagram_standalone(t: &T) {
    let conn = t
        .connect("instagram_standalone", "igs", &["graph.instagram.com", "rupload.facebook.com"], json!({"client_secret": "ig-secret"}), |d| {
            let text = d
                .to_string()
                .replace("https://api.instagram.com", &format!("{}/igt", t.base))
                .replace(&format!("{}/igs/ig-api-upload", t.base), &format!("{}/igr/ig-api-upload", t.base));
            *d = serde_json::from_str(&text).unwrap();
        })
        .await;
    t.sign_in(conn, "CODE-igs").await;
    assert!(t.net().last("igs", "GET", "/access_token").query.get("access_token").is_some_and(|a| a.starts_with("IGS-")), "the short token exchanged");
    let key = format!("publish:{}", Uuid::now_v7());
    let m = ig_medium(t, "z.jpg", "image/jpeg", b"JPEG-Z", "post", None).await;
    let zi = t.call("instagram_standalone", "publish_media", m, Some(&format!("media:{key}"))).await.unwrap()["id"].as_str().unwrap().to_string();
    let mut v = post_in("From Instagram's own sign-in", None, &[("z.jpg", "image/jpeg", 6)], &[zi]);
    v["kind"] = json!("post");
    t.call("instagram_standalone", "publish", v, Some(&key)).await.unwrap();
    assert!(t.net().asked("igs").iter().any(|a| a == "POST /v25.0/me/media_publish"), "as the signed-in account: /me");
    // Renewed by ig_refresh_token with the long-lived token itself.
    t.expire(conn).await;
    let a = t.call("instagram_standalone", "account", json!({"account_id": null, "handle": null}), None).await.unwrap();
    assert_eq!(a["followers"], 999);
    assert_eq!(t.net().last("igs", "GET", "/refresh_access_token").query.get("grant_type").map(String::as_str), Some("ig_refresh_token"));
}
