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
const NETWORKS: &[&str] = &["x"];

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
