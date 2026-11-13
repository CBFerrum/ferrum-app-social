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
const NETWORKS: &[&str] = &[
    "x",
    "linkedin",
    "linkedin_page",
    "facebook",
    "instagram",
    "instagram_standalone",
    "threads",
    "youtube",
    "tiktok",
    "pinterest",
    "reddit",
    "mastodon",
    "bluesky",
];

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
        "th" => fake_threads,
        "yt" | "yto" => fake_youtube,
        "tt" | "ttu" => fake_tiktok,
        "pi" | "pis3" => fake_pinterest,
        "rd" | "rdw" | "rds3" => fake_reddit,
        "ma" => fake_mastodon,
        "bs" => fake_bluesky,
        "gb" | "gba" | "gbi" | "gbo" => fake_google_business,
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
                let mut headers = headers;
                headers.insert("x-raw-query".into(), rest.split_once('?').map(|(_, q)| q.to_string()).unwrap_or_default());
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
        self.connect_from(name, name, net, hosts, secrets, edit).await
    }
    /// A connection called `name` from the file `from`.
    async fn connect_from(&self, name: &str, from: &str, net: &str, hosts: &[&str], secrets: Value, edit: impl FnOnce(&mut Value)) -> Uuid {
        let mut text = file(from);
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
        // (round four) The title is Social's own input, not a setting: what a test still writes as `settings.title`
        // is given as Social gives it.
        let mut inputs = inputs;
        if let Some(t) = inputs["settings"].as_object_mut().and_then(|s| s.remove("title").or_else(|| s.remove("subject"))) {
            inputs["title"] = t;
        }
        let inputs = match inputs {
            Value::Object(o) if !declared.is_empty() => Value::Object(o.into_iter().filter(|(k, _)| declared.contains(k)).collect()),
            other => other,
        };
        let c = Call { connection: conn.into(), operation: op.into(), inputs, key: key.map(str::to_string), entity: None, record: None };
        connections::call(&self.k, &self.admin, Some("social"), &c).await
    }
    /// A stored file, and what Social gives `publish_media` for it (src/posts.rs `media_inputs`: the medium's own `kind`,
    /// the post's as `post_kind`, round four).
    async fn medium(&self, name: &str, ct: &str, bytes: &[u8], post_kind: Option<&str>, settings: Value, account_id: Option<&str>) -> Value {
        let f = self.k.store_file(&self.admin, None, name, ct, bytes).await.unwrap();
        let own = if ct.starts_with("video/") {
            "video"
        } else if ct.starts_with("image/") {
            "image"
        } else {
            "document"
        };
        json!({"file": f["id"], "name": name, "content_type": ct, "alt": "What it shows", "size": bytes.len(), "kind": own, "post_kind": post_kind, "thumbnail": null, "url": null,
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
    threads(&t).await;
    youtube(&t).await;
    tiktok(&t).await;
    pinterest(&t).await;
    reddit(&t).await;
    mastodon(&t).await;
    bluesky(&t).await;
    google_business(&t).await;
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

// ───────────── Threads ─────────────

fn fake_threads(n: &mut Net, r: &Req) -> Resp {
    let err = |code: u16, m: &str, c: u32| status(code, json!({"error": {"message": m, "type": "THApiException", "code": c}}));
    if r.path == "/oauth/access_token" {
        let f = r.form();
        if f.get("client_id").map(String::as_str) != Some("th-client")
            || f.get("client_secret").map(String::as_str) != Some("th-secret")
            || f.get("code").map(String::as_str) != Some("CODE-th")
        {
            return err(400, "Invalid OAuth code", 190);
        }
        let i = n.next();
        n.list("th:short").push(json!(format!("THS-{i}")));
        return ok(json!({"access_token": format!("THS-{i}"), "user_id": "T1"}));
    }
    if r.path == "/access_token" || r.path == "/refresh_access_token" {
        let given = r.query.get("access_token").cloned().unwrap_or_default();
        let fine = if r.path == "/access_token" {
            r.query.get("grant_type").map(String::as_str) == Some("th_exchange_token")
                && r.query.get("client_secret").map(String::as_str) == Some("th-secret")
                && n.list("th:short").contains(&json!(given))
        } else {
            r.query.get("grant_type").map(String::as_str) == Some("th_refresh_token") && n.list("th:tokens").contains(&json!(given))
        };
        if !fine {
            return err(400, "Invalid OAuth access token", 190);
        }
        let i = n.next();
        n.tokens("th").push(json!(format!("THL-{i}")));
        return ok(json!({"access_token": format!("THL-{i}"), "token_type": "bearer", "expires_in": 5183944}));
    }
    if !n.valid("th", r) {
        return err(401, "Invalid OAuth access token.", 190);
    }
    let mut cs = n.get("th:containers");
    if cs.is_null() {
        cs = json!({});
    }
    let rest: Vec<&str> = r.path.trim_start_matches("/v1.0/").split('/').collect();
    let st = |c: &Value| {
        if c["published"] == true {
            "PUBLISHED"
        } else if c["polls"].as_u64() >= Some(1) {
            "FINISHED"
        } else {
            "IN_PROGRESS"
        }
    };
    match (r.method.as_str(), rest.as_slice()) {
        ("POST", ["me", "threads"]) => {
            let b = r.json();
            let kind = b["media_type"].as_str().unwrap_or_default();
            if b["text"].as_str().is_some_and(|t| t.chars().count() > 500) {
                return err(400, "Param text must be at most 500 characters long.", 100);
            }
            let fine = match kind {
                "TEXT" => b["text"].is_string(),
                "IMAGE" => b["image_url"].is_string(),
                "VIDEO" => b["video_url"].is_string(),
                "CAROUSEL" => b["children"].as_str().is_some_and(|c| c.split(',').count() >= 2),
                _ => false,
            };
            if !fine {
                return err(400, "The media type and its parameters do not match", 100);
            }
            if let Some(to) = b["reply_to_id"].as_str()
                && !n.list("th:posts").iter().any(|p| p["id"] == to)
            {
                return err(400, "The post you are replying to does not exist", 100);
            }
            let id = format!("TC{}", n.next());
            cs[&id] = json!({"body": b, "polls": 0, "published": false});
            n.set("th:containers", cs);
            ok(json!({"id": id}))
        }
        ("GET", [""]) => {
            let mut out = json!({});
            for id in r.query.get("ids").map(|i| i.split(',').map(str::to_string).collect::<Vec<_>>()).unwrap_or_default() {
                cs[&id]["polls"] = json!(cs[&id]["polls"].as_u64().unwrap_or(0) + 1);
                out[&id] = json!({"id": id, "status": st(&cs[&id])});
            }
            n.set("th:containers", cs);
            ok(out)
        }
        ("POST", ["me", "threads_publish"]) => {
            let id = r.json()["creation_id"].as_str().unwrap_or_default().to_string();
            if st(&cs[&id]) != "FINISHED" {
                return err(400, "The media is not ready to be published", 24);
            }
            cs[&id]["published"] = json!(true);
            let body = cs[&id]["body"].clone();
            n.set("th:containers", cs);
            let pid = format!("TP{}", n.next());
            n.list("th:posts").insert(0, json!({"id": pid, "text": body["text"], "body": body}));
            ok(json!({"id": pid}))
        }
        ("GET", ["me", "threads"]) => ok(json!({"data": n.list("th:posts").iter().map(|p| json!({"id": p["id"], "text": p["text"]})).collect::<Vec<_>>()})),
        ("GET", ["me", "mentions"]) => ok(
            json!({"data": [{"id": "TM1", "text": "@northwind hi", "username": "zoe"}, {"id": "TM2", "text": "nice", "username": "yan", "replied_to": {"id": "TP0"}}]}),
        ),
        ("GET", ["me", "threads_insights"]) => ok(json!({"data": [{"name": "followers_count", "total_value": {"value": 640}}]})),
        ("GET", ["me"]) => ok(json!({"id": "T1", "username": "northwind", "name": "Northwind", "threads_profile_picture_url": "https://cdn.test/t.jpg"})),
        ("GET", [id, "insights"]) => {
            let _ = id;
            let v = |name: &str, n: i64| json!({"name": name, "period": "lifetime", "values": [{"value": n}]});
            ok(json!({"data": [v("views", 300), v("likes", 5), v("replies", 2), v("reposts", 1), v("quotes", 1), v("shares", 1)]}))
        }
        ("POST", [id, "repost"]) => {
            n.list("th:reposts").push(json!(id));
            ok(json!({"id": format!("R{id}")}))
        }
        ("DELETE", [id]) => {
            let id = id.to_string();
            n.list("th:posts").retain(|p| p["id"] != json!(id));
            ok(json!({"success": true}))
        }
        ("GET", [id]) if id.starts_with("TC") => {
            cs[*id]["polls"] = json!(cs[*id]["polls"].as_u64().unwrap_or(0) + 1);
            let s = st(&cs[*id]);
            n.set("th:containers", cs);
            ok(json!({"id": id, "status": s}))
        }
        ("GET", [id]) => ok(json!({"id": id, "permalink": format!("https://www.threads.net/@northwind/post/{id}")})),
        _ => err(404, "Unsupported request", 100),
    }
}

async fn threads(t: &T) {
    let conn = t.connect("threads", "th", &["graph.threads.net"], json!({"client_secret": "th-secret"}), |_| {}).await;
    t.sign_in(conn, "CODE-th").await;
    assert!(t.net().last("th", "GET", "/access_token").query.get("access_token").is_some_and(|a| a.starts_with("THS-")), "exchanged for a long-lived token");
    let account = t.account("Northwind Threads", conn).await;
    assert_eq!(t.action(account, "rules", json!({})).await.unwrap()["rules"]["name"], "Threads");
    let key = format!("publish:{}", Uuid::now_v7());
    // Text with its link as an attachment, then a thread answering it.
    let mut first_in = post_in("Open today", Some("https://northwind.test"), &[], &[]);
    first_in["settings"] = json!({"reply_control": "everyone"});
    let first = t.call("threads", "publish", first_in, Some(&key)).await.unwrap();
    {
        let mut n = t.net();
        let b = n.list("th:posts")[0]["body"].clone();
        assert_eq!(
            (b["media_type"].as_str(), b["text"].as_str(), b["link_attachment"].as_str()),
            (Some("TEXT"), Some("Open today"), Some("https://northwind.test"))
        );
    }
    assert!(first["url"].as_str().unwrap().ends_with(first["id"].as_str().unwrap()));
    let second = follow_in(post_in("Till six", None, &[], &[]), 1, 2, first["id"].as_str().unwrap(), &kept(0, &first));
    t.call("threads", "publish", second, Some(&format!("{key}:1"))).await.unwrap();
    assert_eq!(t.net().list("th:posts")[0]["body"]["reply_to_id"], first["id"], "each item answers the one before");
    // A carousel of two pictures by their addresses (which Social does not give yet).
    let mut ids = vec![];
    for i in 0..2 {
        let mut m = t.medium(&format!("t{i}.jpg"), "image/jpeg", b"JPEG", None, json!({}), None).await;
        m["url"] = json!(format!("https://files.northwind.test/t{i}.jpg"));
        ids.push(t.call("threads", "publish_media", m, Some(&format!("media:{key}:2:{i}"))).await.unwrap()["id"].as_str().unwrap().to_string());
    }
    assert_eq!(ids[0], "image:https://files.northwind.test/t0.jpg");
    t.call("threads", "publish", post_in("Two views", None, &[("t0.jpg", "image/jpeg", 4), ("t1.jpg", "image/jpeg", 4)], &ids), Some(&format!("{key}:2")))
        .await
        .unwrap();
    assert_eq!(t.net().list("th:posts")[0]["body"]["media_type"], "CAROUSEL");
    assert!(t.net().asked("th").iter().any(|a| a == "GET /v1.0/"), "the children waited for together");
    // The first comment, a reply under the post.
    t.call("threads", "reply", json!({"text": "Ask away", "in_reply_to": first["id"], "root": first["id"]}), Some(&format!("first_comment:{key}")))
        .await
        .unwrap();
    assert_eq!(t.net().list("th:posts")[0]["body"]["reply_to_id"], first["id"]);
    // A lost answer on publishing: found among the account's own posts by its text.
    t.net().lose.insert("POST /th/v1.0/me/threads_publish".into());
    let lost = format!("publish:{}", Uuid::now_v7());
    let before = t.net().list("th:posts").len();
    assert!(t.call("threads", "publish", post_in("Exactly one", None, &[], &[]), Some(&lost)).await.is_err());
    let got = t.call("threads", "publish", post_in("Exactly one", None, &[], &[]), Some(&lost)).await.unwrap();
    assert_eq!(t.net().list("th:posts").len(), before + 1);
    assert_eq!(got["id"], t.net().list("th:posts")[0]["id"]);
    // A refusal in Threads' words.
    let e = t.call("threads", "publish", post_in(&"a".repeat(501), None, &[], &[]), Some(&format!("publish:{}", Uuid::now_v7()))).await.unwrap_err();
    assert!(e.to_string().contains("at most 500 characters"), "{e}");
    // Numbers, repost, delete, and the token renewed by th_refresh_token.
    let m = t.call("threads", "metrics", json!({"id": first["id"], "account_id": null, "handle": null}), None).await.unwrap();
    assert_eq!((m["impressions"].as_i64(), m["engagements"].as_i64()), (Some(300), Some(10)));
    t.call("threads", "repost", json!({"id": first["id"]}), Some("repost:th")).await.unwrap();
    t.call("threads", "delete", json!({"id": got["id"]}), Some(&format!("delete:{lost}"))).await.unwrap();
    t.expire(conn).await;
    let a = t.call("threads", "account", json!({"account_id": null, "handle": null}), None).await.unwrap();
    assert_eq!((a["followers"].as_i64(), a["handle"].as_str()), (Some(640), Some("northwind")));
    assert_eq!(t.net().last("th", "GET", "/refresh_access_token").query.get("grant_type").map(String::as_str), Some("th_refresh_token"));
    // Mentions pulled: one answering a post is a comment on it.
    t.pull(conn, "mentions").await;
    let tm2 = t.messages(json!({"external_id": "TM2"})).await;
    assert_eq!((tm2[0]["kind"].as_str(), tm2[0]["post_external_id"].as_str()), (Some("Comment"), Some("TP0")));
    assert_eq!(t.messages(json!({"external_id": "TM1"})).await[0]["kind"], "Mention");
}

// ───────────── YouTube ─────────────

fn fake_youtube(n: &mut Net, r: &Req) -> Resp {
    let err = |code: u16, m: &str, reason: &str| status(code, json!({"error": {"code": code, "message": m, "errors": [{"reason": reason, "message": m}]}}));
    let base = n.base.clone();
    if r.path == "/token" {
        let f = r.form();
        if f.get("client_id").map(String::as_str) != Some("yt-client") || f.get("client_secret").map(String::as_str) != Some("yt-secret") {
            return status(401, json!({"error": "invalid_client"}));
        }
        let fine = match f.get("grant_type").map(String::as_str) {
            Some("authorization_code") => f.contains_key("code_verifier") && f.get("code").map(String::as_str) == Some("CODE-yt"),
            Some("refresh_token") => f.get("refresh_token").map(String::as_str) == Some("YR-1"),
            _ => false,
        };
        if !fine {
            return status(400, json!({"error": "invalid_grant", "error_description": "Bad Request"}));
        }
        let i = n.next();
        n.tokens("yt").push(json!(format!("YA-{i}")));
        let mut out = json!({"access_token": format!("YA-{i}"), "expires_in": 3599, "token_type": "Bearer"});
        if f.get("grant_type").map(String::as_str) == Some("authorization_code") {
            out["refresh_token"] = json!("YR-1");
        }
        return ok(out);
    }
    if !n.valid("yt", r) {
        return err(401, "Request had invalid authentication credentials.", "authError");
    }
    match (r.method.as_str(), r.path.as_str()) {
        ("GET", "/youtube/v3/channels") => ok(
            json!({"items": [{"id": "UC1", "snippet": {"title": "Northwind TV", "customUrl": "@northwindtv", "thumbnails": {"default": {"url": "https://yt3.test/n.jpg"}}},
                                                           "statistics": {"subscriberCount": "2500", "viewCount": "100000"}}]}),
        ),
        ("GET", "/youtube/v3/videoCategories") => {
            ok(json!({"items": [{"id": "22", "snippet": {"title": "People & Blogs"}}, {"id": "26", "snippet": {"title": "Howto & Style"}}]}))
        }
        ("POST", "/upload/youtube/v3/videos") => {
            let b = r.json();
            if r.query.get("uploadType").map(String::as_str) != Some("resumable") || r.header("x-upload-content-length").is_none() {
                return err(400, "uploadType=resumable and the content length are required", "badRequest");
            }
            if b["snippet"]["title"].as_str().is_none_or(str::is_empty) {
                return err(400, "The request metadata specifies an invalid or empty video title.", "invalidTitle");
            }
            let i = n.next();
            let mut ses = n.get("yt:sessions");
            if ses.is_null() {
                ses = json!({});
            }
            ses[format!("U{i}")] = json!({"meta": b, "total": r.header("x-upload-content-length").unwrap().parse::<u64>().unwrap(), "held": 0, "done": null});
            n.set("yt:sessions", ses);
            Resp(200, vec![("location".into(), format!("{base}/yt/upload/youtube/v3/videos?uploadType=resumable&upload_id=U{i}"))], b"{}".to_vec())
        }
        ("PUT", "/upload/youtube/v3/videos") => {
            let id = r.query.get("upload_id").cloned().unwrap_or_default();
            let mut ses = n.get("yt:sessions");
            let s = &mut ses[&id];
            let range = r.header("content-range").unwrap_or_default().trim_start_matches("bytes ").to_string();
            let total = s["total"].as_u64().unwrap();
            let mut held = s["held"].as_u64().unwrap();
            if !range.starts_with('*') {
                let start: u64 = range.split('-').next().unwrap().parse().unwrap();
                if start != held || !range.ends_with(&format!("/{total}")) {
                    return err(400, "Wrong offset", "badRequest");
                }
                held += r.body.len() as u64;
                s["held"] = json!(held);
            }
            let out = if held == total {
                if s["done"].is_null() {
                    let vid = format!("vid{:08}", n.n + 1);
                    s["done"] = json!(vid);
                    let meta = s["meta"].clone();
                    n.list("yt:videos").push(json!({"id": vid, "meta": meta, "bytes": held}));
                }
                ok(json!({"kind": "youtube#video", "id": s["done"]}))
            } else if held == 0 {
                empty(308)
            } else {
                Resp(308, vec![("range".into(), format!("bytes=0-{}", held - 1))], vec![])
            };
            n.n += 1;
            n.set("yt:sessions", ses);
            out
        }
        ("POST", "/upload/youtube/v3/thumbnails/set") => {
            n.list("yt:thumbs").push(json!([r.query.get("videoId"), r.body.len()]));
            ok(json!({"items": [{"default": {"url": "https://i.ytimg.test/t.jpg"}}]}))
        }
        ("POST", "/youtube/v3/commentThreads") => {
            let b = r.json();
            let id = format!("Ug{}", n.next());
            n.list("yt:comments").push(json!({"id": id, "video": b["snippet"]["videoId"], "text": b["snippet"]["topLevelComment"]["snippet"]["textOriginal"]}));
            ok(json!({"id": id}))
        }
        ("POST", "/youtube/v3/comments") => {
            let id = format!("Uc{}", n.next());
            n.list("yt:comments").push(json!({"id": id, "parent": r.json()["snippet"]["parentId"]}));
            ok(json!({"id": id}))
        }
        ("GET", "/youtube/v3/commentThreads") => {
            if r.query.get("allThreadsRelatedToChannelId").map(String::as_str) != Some("UC1") {
                return err(400, "allThreadsRelatedToChannelId is required", "badRequest");
            }
            ok(
                json!({"items": [{"id": "Ug-fan", "snippet": {"videoId": "vidX", "topLevelComment": {"snippet": {"authorDisplayName": "Fan", "authorChannelId": {"value": "UCfan"}, "textOriginal": "Great video"}}}}]}),
            )
        }
        ("GET", "/youtube/v3/videos") => {
            ok(json!({"items": [{"id": r.query.get("id"), "statistics": {"viewCount": "1500", "likeCount": "40", "commentCount": "6"}}]}))
        }
        ("DELETE", "/youtube/v3/videos") => {
            let id = r.query.get("id").cloned().unwrap_or_default();
            n.list("yt:videos").retain(|v| v["id"] != json!(id));
            empty(204)
        }
        _ => err(404, "Not Found", "notFound"),
    }
}

async fn youtube(t: &T) {
    let conn = t
        .connect("youtube", "yt", &["www.googleapis.com"], json!({"client_secret": "yt-secret"}), |d| {
            let text = d.to_string().replace("https://oauth2.googleapis.com", &format!("{}/yto", t.base));
            *d = serde_json::from_str(&text).unwrap();
        })
        .await;
    let q = t.sign_in(conn, "CODE-yt").await;
    assert_eq!((q["access_type"].as_str(), q["prompt"].as_str()), ("offline", "consent"), "a refresh token is asked for");
    let account = t.account("Northwind YouTube", conn).await;
    assert_eq!(t.action(account, "rules", json!({})).await.unwrap()["rules"]["name"], "YouTube");
    t.action(account, "choose", json!({"picked": ["UC1"]})).await.unwrap();
    let cats = t.action(account, "choices", json!({"setting": "category"})).await.unwrap();
    assert_eq!(cats["choices"][0], json!({"value": "22", "label": "People & Blogs"}));
    // A video larger than a chunk: the session started with its title and settings, sent 8 MiB at a time, its
    // thumbnail set.
    let video: Vec<u8> = (0..(8388608 + 1000)).map(|i| (i % 97) as u8).collect();
    let mut m = t.medium("clip.mp4", "video/mp4", &video, Some("video"), json!({}), Some("UC1")).await;
    let thumb = t.k.store_file(&t.admin, None, "thumb.jpg", "image/jpeg", b"JPEG-THUMB").await.unwrap();
    m["thumbnail"] = thumb["id"].clone();
    let key = format!("publish:{}", Uuid::now_v7());
    let id = t.call("youtube", "publish_media", m, Some(&format!("media:{key}"))).await.unwrap()["id"].as_str().unwrap().to_string();
    let settings = json!({"title": "Baking day", "type": "unlisted", "tags": ["bread"], "category": "22", "made_for_kids": false});
    let mut v = post_in("How we bake", None, &[("clip.mp4", "video/mp4", video.len() as i64)], std::slice::from_ref(&id));
    v["kind"] = json!("video");
    v["settings"] = settings.clone();
    let out = t.call("youtube", "publish", v.clone(), Some(&key)).await.unwrap();
    {
        let mut n = t.net();
        let made = n.list("yt:videos")[0].clone();
        assert_eq!(
            (made["bytes"].as_u64(), made["meta"]["snippet"]["title"].as_str(), made["meta"]["status"]["privacyStatus"].as_str()),
            (Some(video.len() as u64), Some("Baking day"), Some("unlisted"))
        );
        assert_eq!((made["meta"]["snippet"]["categoryId"].as_str(), made["meta"]["status"]["selfDeclaredMadeForKids"].as_bool()), (Some("22"), Some(false)));
        assert_eq!(n.list("yt:thumbs")[0], json!([out["id"], 10]));
        assert_eq!(n.asked("yt").iter().filter(|a| a.starts_with("PUT /upload")).count(), 2, "two chunks");
    }
    assert_eq!(out["url"], format!("https://www.youtube.com/watch?v={}", out["id"].as_str().unwrap()));
    // The last chunk's answer lost: the session is asked what it holds, and it is complete; one video.
    t.net().lose.insert("PUT /yt/upload/youtube/v3/videos".into());
    let lost = format!("publish:{}", Uuid::now_v7());
    let small = t.medium("short.mp4", "video/mp4", b"SHORT-VIDEO", Some("short"), json!({}), Some("UC1")).await;
    let sid = t.call("youtube", "publish_media", small, Some(&format!("media:{lost}"))).await.unwrap()["id"].as_str().unwrap().to_string();
    let mut s = post_in("A short", None, &[("short.mp4", "video/mp4", 11)], &[sid]);
    s["kind"] = json!("short");
    s["settings"] = json!({"title": "Short", "type": "public"});
    let before = t.net().list("yt:videos").len();
    assert!(t.call("youtube", "publish", s.clone(), Some(&lost)).await.is_err());
    let got = t.call("youtube", "publish", s, Some(&lost)).await.unwrap();
    assert_eq!(t.net().list("yt:videos").len(), before + 1, "one video");
    assert!(got["url"].as_str().unwrap().contains("/shorts/"));
    // A refusal in YouTube's words.
    let mut bad = v.clone();
    bad["settings"]["title"] = json!("");
    let e = t.call("youtube", "publish", bad, Some(&format!("publish:{}", Uuid::now_v7()))).await.unwrap_err();
    assert!(e.to_string().contains("invalid or empty video title"), "{e}");
    // The first comment opens a thread under the video; an answer in the inbox answers a comment.
    t.call("youtube", "reply", json!({"text": "Recipe below", "in_reply_to": out["id"], "root": out["id"]}), Some(&format!("first_comment:{key}")))
        .await
        .unwrap();
    t.call("youtube", "reply", json!({"text": "Thanks!", "in_reply_to": "Ug-fan-thread", "author": "Fan"}), Some("reply:yt1")).await.unwrap();
    {
        let mut n = t.net();
        let c = n.list("yt:comments").clone();
        assert_eq!((c[0]["video"].clone(), c[1]["parent"].clone()), (out["id"].clone(), json!("Ug-fan-thread")));
    }
    // Comments pulled from the channel kept at sign-in.
    t.pull(conn, "comments").await;
    let got = t.messages(json!({"external_id": "Ug-fan"})).await;
    assert_eq!((got[0]["kind"].as_str(), got[0]["post_external_id"].as_str()), (Some("Comment"), Some("vidX")));
    // Numbers after a refresh, the channel, delete.
    t.expire(conn).await;
    let m = t.call("youtube", "metrics", json!({"id": out["id"], "account_id": "UC1", "handle": null}), None).await.unwrap();
    assert_eq!((m["impressions"].as_i64(), m["engagements"].as_i64()), (Some(1500), Some(46)));
    assert_eq!(t.net().last("yto", "POST", "/token").form()["grant_type"], "refresh_token");
    let a = t.call("youtube", "account", json!({"account_id": "UC1", "handle": null}), None).await.unwrap();
    assert_eq!((a["followers"].as_i64(), a["handle"].as_str()), (Some(2500), Some("@northwindtv")));
    t.call("youtube", "delete", json!({"id": out["id"]}), Some(&format!("delete:{key}"))).await.unwrap();
    assert!(!t.net().list("yt:videos").iter().any(|v| v["id"] == out["id"]));
}

// ───────────── TikTok ─────────────

fn fake_tiktok(n: &mut Net, r: &Req) -> Resp {
    let err = |code: u16, c: &str, m: &str| status(code, json!({"error": {"code": c, "message": m, "log_id": "L1"}}));
    let base = n.base.clone();
    if r.path == "/v2/oauth/token/" {
        // TikTok names the client `client_key`, in the form.
        let f = r.form();
        if !f.contains_key("client_key") {
            return status(400, json!({"error": "invalid_request", "error_description": "client_key is required"}));
        }
        return ok(json!({"access_token": "TTA", "expires_in": 86400, "open_id": "O1", "refresh_token": "TTR", "scope": "video.publish"}));
    }
    if r.bearer().as_deref() != Some("tt-token") && !r.path.starts_with("/upload/") {
        return err(401, "access_token_invalid", "The access token is invalid or not found in the request.");
    }
    let mut pubs = n.get("tt:publishes");
    if pubs.is_null() {
        pubs = json!({});
    }
    match (r.method.as_str(), r.path.as_str()) {
        ("POST", "/v2/post/publish/creator_info/query/") => {
            ok(json!({"data": {"creator_username": "northwind", "privacy_level_options": ["PUBLIC_TO_EVERYONE", "SELF_ONLY"],
                                                                   "comment_disabled": false, "duet_disabled": false, "stitch_disabled": true, "max_video_post_duration_sec": 600},
                                                          "error": {"code": "ok", "message": ""}}))
        }
        ("POST", "/v2/post/publish/video/init/") => {
            let b = r.json();
            if n.get("tt:limit") == true {
                return err(403, "spam_risk_too_many_posts", "The daily post cap from the API is reached for the current user.");
            }
            let si = &b["source_info"];
            let (size, chunk, count) =
                (si["video_size"].as_u64().unwrap_or(0), si["chunk_size"].as_u64().unwrap_or(0), si["total_chunk_count"].as_u64().unwrap_or(0));
            if si["source"] != "FILE_UPLOAD" || chunk == 0 || count != (size / chunk).max(1) || b["post_info"]["privacy_level"].as_str().is_none() {
                return err(400, "invalid_params", "chunk_size and total_chunk_count do not match the video's size");
            }
            let id = format!("v_pub_file~v2-1.{}", n.next());
            pubs[&id] = json!({"size": size, "held": 0, "info": b["post_info"], "polls": 0});
            n.set("tt:publishes", pubs);
            ok(json!({"data": {"publish_id": id, "upload_url": format!("{base}/ttu/upload/?upload_id={id}")}, "error": {"code": "ok", "message": ""}}))
        }
        ("PUT", "/upload/") => {
            let id = r.query.get("upload_id").cloned().unwrap_or_default();
            let p = &mut pubs[&id];
            let range = r.header("content-range").unwrap_or_default().trim_start_matches("bytes ").to_string();
            let start: u64 = range.split('-').next().and_then(|s| s.parse().ok()).unwrap_or(u64::MAX);
            if p.is_null() || start != p["held"].as_u64().unwrap() || r.header("content-type") != Some("video/mp4") {
                return err(400, "invalid_params", "Content-Range does not follow the bytes already uploaded");
            }
            let held = start + r.body.len() as u64;
            p["held"] = json!(held);
            let done = held == p["size"].as_u64().unwrap();
            if done {
                let info = p["info"].clone();
                n.list("tt:posts").push(json!({"publish_id": id, "info": info}));
            }
            n.set("tt:publishes", pubs);
            empty(if done { 201 } else { 206 })
        }
        ("POST", "/v2/post/publish/content/init/") => {
            let b = r.json();
            if b["media_type"] != "PHOTO" || b["source_info"]["photo_images"].as_array().is_none_or(Vec::is_empty) {
                return err(400, "invalid_params", "photo_images is required");
            }
            let id = format!("p_pub_url~v2.{}", n.next());
            pubs[&id] = json!({"size": 0, "held": 0, "info": b["post_info"], "polls": 0, "photos": b["source_info"]["photo_images"]});
            n.set("tt:publishes", pubs);
            n.list("tt:posts").push(json!({"publish_id": id, "info": b["post_info"], "photos": b["source_info"]["photo_images"]}));
            ok(json!({"data": {"publish_id": id}, "error": {"code": "ok", "message": ""}}))
        }
        ("POST", "/v2/post/publish/status/fetch/") => {
            let id = r.json()["publish_id"].as_str().unwrap_or_default().to_string();
            let p = &mut pubs[&id];
            p["polls"] = json!(p["polls"].as_u64().unwrap_or(0) + 1);
            let complete = p["held"] == p["size"] && p["polls"].as_u64() >= Some(2);
            let out = if complete {
                json!({"data": {"status": "PUBLISH_COMPLETE", "publicaly_available_post_id": [7300000000u64 + p["polls"].as_u64().unwrap()], "uploaded_bytes": p["held"]}, "error": {"code": "ok"}})
            } else {
                json!({"data": {"status": "PROCESSING_UPLOAD", "uploaded_bytes": p["held"]}, "error": {"code": "ok"}})
            };
            n.set("tt:publishes", pubs);
            ok(out)
        }
        ("POST", "/v2/video/query/") => ok(
            json!({"data": {"videos": [{"id": r.json()["filters"]["video_ids"][0], "view_count": 5000, "like_count": 300, "comment_count": 20, "share_count": 10}]},
                                                 "error": {"code": "ok"}}),
        ),
        ("GET", "/v2/user/info/") => ok(
            json!({"data": {"user": {"open_id": "O1", "display_name": "Northwind", "username": "northwind", "follower_count": 8800, "avatar_url": "https://p16.test/a.jpg"}},
                                               "error": {"code": "ok"}}),
        ),
        _ => err(404, "not_found", "Not found"),
    }
}

async fn tiktok(t: &T) {
    // As the file is: TikTok's token address wants the client as `client_key`, which the platform cannot name yet.
    let conn = t.connect("tiktok", "tt", &["open.tiktokapis.com", "open-upload.tiktokapis.com"], json!({"client_secret": "tt-secret"}), |_| {}).await;
    let session = Uuid::now_v7();
    let url = t.k.connect_start(&t.admin, conn, session).await.unwrap();
    let q = query_of(reqwest::Url::parse(&url).unwrap().query().unwrap_or_default());
    assert_eq!(q.get("client_key").map(String::as_str), Some("tt-client"), "the authorisation address names the client as TikTok wants");
    let e = t.k.connect_finish(&t.admin, session, &q["state"], Some("CODE-tt"), None).await.unwrap_err();
    assert!(e.to_string().contains("did not give tokens: 400: invalid_request"), "{e}");
    assert!(t.net().last("tt", "POST", "/v2/oauth/token/").form().contains_key("client_id"), "the platform names it client_id");
    // The rest, on the same description signed in with a token as a secret.
    let tested = t
        .connect_from("tiktok_tested", "tiktok", "tt", &["open.tiktokapis.com", "open-upload.tiktokapis.com"], json!({"token": "tt-token"}), |d| {
            d["auth"] = json!({"type": "bearer", "secret": "token"});
        })
        .await;
    let account = t.account("Northwind TikTok", tested).await;
    assert_eq!(t.action(account, "rules", json!({})).await.unwrap()["rules"]["name"], "TikTok");
    let privacy = t.action(account, "choices", json!({"setting": "privacy_level"})).await.unwrap();
    assert_eq!(privacy["choices"].as_array().unwrap().len(), 2, "the creator's own options: {privacy}");
    // A video in two chunks (the remainder in the last), its publishing waited for.
    let video: Vec<u8> = (0..21_000_000u32).map(|i| (i % 251) as u8).collect();
    let settings =
        json!({"privacy_level": "SELF_ONLY", "comment": true, "duet": false, "stitch": false, "brand_content_toggle": false, "brand_organic_toggle": false});
    let key = format!("publish:{}", Uuid::now_v7());
    let m = t.medium("clip.mp4", "video/mp4", &video, Some("video"), settings.clone(), None).await;
    let id = t.call("tiktok_tested", "publish_media", m, Some(&format!("media:{key}"))).await.unwrap()["id"].as_str().unwrap().to_string();
    let mut v = post_in("Bake with us #bread", None, &[("clip.mp4", "video/mp4", video.len() as i64)], &[id]);
    v["kind"] = json!("video");
    v["settings"] = settings.clone();
    let out = t.call("tiktok_tested", "publish", v.clone(), Some(&key)).await.unwrap();
    assert!(out["id"].as_str().unwrap().starts_with("73"), "the public post's id: {out}");
    {
        let mut n = t.net();
        let p = n.list("tt:posts")[0].clone();
        assert_eq!(
            (p["info"]["title"].as_str(), p["info"]["privacy_level"].as_str(), p["info"]["disable_duet"].as_bool()),
            (Some("Bake with us #bread"), Some("SELF_ONLY"), Some(true))
        );
        let puts: Vec<String> = n
            .seen
            .iter()
            .filter(|(net, r)| net == "ttu" && r.method == "PUT")
            .map(|(_, r)| r.header("content-range").unwrap_or_default().to_string())
            .collect();
        assert_eq!(puts, ["bytes 0-9999999/21000000", "bytes 10000000-20999999/21000000"]);
    }
    // Photos, pulled by their addresses.
    let mut p = t.medium("a.jpg", "image/jpeg", b"JPEG", Some("photo"), json!({}), None).await;
    p["url"] = json!("https://files.northwind.test/a.jpg");
    let pid = t.call("tiktok_tested", "publish_media", p, Some(&format!("media:{key}:p"))).await.unwrap()["id"].as_str().unwrap().to_string();
    let mut ph = post_in("Fresh loaves", None, &[("a.jpg", "image/jpeg", 4)], &[pid]);
    ph["kind"] = json!("photo");
    ph["settings"] = json!({"privacy_level": "PUBLIC_TO_EVERYONE", "comment": true, "title": "Loaves"});
    t.call("tiktok_tested", "publish", ph, Some(&format!("{key}:p"))).await.unwrap();
    assert_eq!(t.net().list("tt:posts").last().unwrap()["photos"], json!(["https://files.northwind.test/a.jpg"]));
    // The last chunk's answer lost: TikTok publishes an upload once it is whole and has no way to ask what it holds,
    // so the call stops rather than start again; there is one post.
    let small: Vec<u8> = vec![b's'; 1000];
    let m = t.medium("s.mp4", "video/mp4", &small, Some("video"), settings.clone(), None).await;
    let lost = format!("publish:{}", Uuid::now_v7());
    let sid = t.call("tiktok_tested", "publish_media", m, Some(&format!("media:{lost}"))).await.unwrap()["id"].as_str().unwrap().to_string();
    let mut s = post_in("Short and once", None, &[("s.mp4", "video/mp4", 1000)], &[sid]);
    s["kind"] = json!("video");
    s["settings"] = settings.clone();
    let before = t.net().list("tt:posts").len();
    t.net().lose.insert("PUT /ttu/upload/".into());
    assert!(t.call("tiktok_tested", "publish", s.clone(), Some(&lost)).await.is_err());
    let e = t.call("tiktok_tested", "publish", s, Some(&lost)).await.unwrap_err();
    assert!(e.to_string().to_lowercase().contains("sent") || e.to_string().contains("lost"), "{e}");
    assert_eq!(t.net().list("tt:posts").len(), before + 1, "one post");
    // A refusal in TikTok's words.
    t.net().set("tt:limit", json!(true));
    let e = t.call("tiktok_tested", "publish", v, Some(&format!("publish:{}", Uuid::now_v7()))).await.unwrap_err();
    assert!(e.to_string().contains("daily post cap"), "{e}");
    let m = t.call("tiktok_tested", "metrics", json!({"id": out["id"], "account_id": null, "handle": null}), None).await.unwrap();
    assert_eq!((m["impressions"].as_i64(), m["engagements"].as_i64()), (Some(5000), Some(330)));
    let a = t.call("tiktok_tested", "account", json!({"account_id": null, "handle": null}), None).await.unwrap();
    assert_eq!((a["followers"].as_i64(), a["handle"].as_str()), (Some(8800), Some("northwind")));
}

// ───────────── Pinterest ─────────────

fn fake_pinterest(n: &mut Net, r: &Req) -> Resp {
    let err = |code: u16, c: u32, m: &str| status(code, json!({"code": c, "message": m}));
    let base = n.base.clone();
    if r.path == "/v5/oauth/token" {
        if r.header("authorization") != Some(basic("pi-client", "pi-secret").as_str()) {
            return err(401, 2, "Authentication failed.");
        }
        let f = r.form();
        let fine = match f.get("grant_type").map(String::as_str) {
            Some("authorization_code") => f.get("code").map(String::as_str) == Some("CODE-pi") && f.contains_key("redirect_uri"),
            Some("refresh_token") => f.get("refresh_token").map(String::as_str) == Some("PR-1"),
            _ => false,
        };
        if !fine {
            return err(400, 1, "Invalid request.");
        }
        let i = n.next();
        n.tokens("pi").push(json!(format!("PA-{i}")));
        return ok(json!({"access_token": format!("PA-{i}"), "refresh_token": "PR-1", "response_type": "authorization_code", "token_type": "bearer",
                         "expires_in": 2592000, "refresh_token_expires_in": 31536000, "scope": "boards:read pins:read pins:write user_accounts:read"}));
    }
    if r.path == "/upload" {
        // S3: a form whose fields the register answer named, then the file; no credentials of the connection's.
        let parts = r.parts();
        if r.header("authorization").is_some() || parts.get("key").map(|p| p.2.as_slice()) != Some(b"uploads/v1.mp4".as_slice()) || !parts.contains_key("file")
        {
            return Resp(403, vec![("content-type".into(), "application/xml".into())], b"<Error><Code>AccessDenied</Code></Error>".to_vec());
        }
        let names: Vec<String> = parts.keys().cloned().collect();
        n.set("pi:s3", json!({"fields": names, "bytes": parts["file"].2.len()}));
        return empty(204);
    }
    if !n.valid("pi", r) {
        return err(401, 2, "Authentication failed.");
    }
    let seg: Vec<&str> = r.path.trim_start_matches("/v5/").split('/').collect();
    match (r.method.as_str(), seg.as_slice()) {
        ("GET", ["boards"]) => ok(json!({"items": [{"id": "B1", "name": "Breads"}, {"id": "B2", "name": "Cakes"}], "bookmark": null})),
        ("POST", ["media"]) => {
            let id = format!("{}", 9000 + n.next());
            ok(json!({"media_id": id, "media_type": "video", "upload_url": format!("{base}/pis3/upload"),
                      "upload_parameters": {"key": "uploads/v1.mp4", "x-amz-signature": "sig", "policy": "pol", "Content-Type": "multipart/form-data"}}))
        }
        ("GET", ["media", id]) => {
            let polls = n.next();
            let _ = id;
            ok(
                json!({"media_id": id, "media_type": "video", "status": if n.get("pi:s3").is_null() { "registered" } else if polls.is_multiple_of(2) { "succeeded" } else { "processing" }}),
            )
        }
        ("GET", ["user_account"]) => {
            ok(json!({"username": "northwind", "business_name": "Northwind", "follower_count": 210, "profile_image": "https://i.pinimg.test/n.jpg"}))
        }
        ("POST", ["pins"]) => {
            let b = r.json();
            if !["B1", "B2"].contains(&b["board_id"].as_str().unwrap_or_default()) {
                return err(404, 2, "Board not found.");
            }
            let src = &b["media_source"];
            let fine = match src["source_type"].as_str() {
                Some("image_url") => src["url"].is_string(),
                Some("multiple_image_urls") => src["items"].as_array().is_some_and(|i| i.len() >= 2),
                Some("video_id") => src["media_id"].is_string() && src["cover_image_key_frame_time"].is_number(),
                _ => false,
            };
            if !fine {
                return err(400, 1, "Invalid media_source.");
            }
            let id = format!("{}", 1_000_000 + n.next());
            n.list("pi:pins").insert(0, json!({"id": id, "board_id": b["board_id"], "title": b["title"], "description": b["description"], "link": b["link"], "alt_text": b["alt_text"], "media_source": src}));
            status(201, json!({"id": id}))
        }
        ("GET", ["boards", board, "pins"]) => {
            let mine: Vec<Value> = n.list("pi:pins").iter().filter(|p| p["board_id"] == json!(board)).cloned().collect();
            ok(json!({"items": mine, "bookmark": null}))
        }
        ("GET", ["pins", id]) => {
            let _ = id;
            ok(
                json!({"id": id, "pin_metrics": {"lifetime_metrics": {"impression": 700, "save": 12, "pin_click": 30, "outbound_click": 9, "reaction": 4, "comment": 1}}}),
            )
        }
        ("DELETE", ["pins", id]) => {
            let id = id.to_string();
            n.list("pi:pins").retain(|p| p["id"] != json!(id));
            empty(204)
        }
        _ => err(404, 404, "Not found"),
    }
}

async fn pinterest(t: &T) {
    let conn = t
        .connect("pinterest", "pi", &["api.pinterest.com", "pinterest-media-upload.s3-accelerate.amazonaws.com"], json!({"client_secret": "pi-secret"}), |_| {})
        .await;
    t.sign_in(conn, "CODE-pi").await;
    let account = t.account("Northwind Pinterest", conn).await;
    assert_eq!(t.action(account, "rules", json!({})).await.unwrap()["rules"]["name"], "Pinterest");
    let boards = t.action(account, "choices", json!({"setting": "board"})).await.unwrap();
    assert_eq!(boards["choices"][0], json!({"value": "B1", "label": "Breads"}));
    let pin = |text: &str, ids: &[String], settings: Value| {
        let media: Vec<(&str, &str, i64)> = ids.iter().map(|_| ("x", "image/jpeg", 4)).collect();
        let mut v = post_in(text, Some("https://northwind.test/bread"), &media, ids);
        v["settings"] = settings;
        v
    };
    let key = format!("publish:{}", Uuid::now_v7());
    let mut ids = vec![];
    for i in 0..2 {
        let mut m = t.medium(&format!("{i}.jpg"), "image/jpeg", b"JPEG", None, json!({}), None).await;
        m["url"] = json!(format!("https://files.northwind.test/{i}.jpg"));
        ids.push(t.call("pinterest", "publish_media", m, Some(&format!("media:{key}:0:{i}"))).await.unwrap()["id"].as_str().unwrap().to_string());
    }
    // One picture, then a carousel, each on the board the setting names.
    let one = t.call("pinterest", "publish", pin("Sourdough", &ids[..1], json!({"board": "B1", "title": "Sourdough"})), Some(&key)).await.unwrap();
    assert_eq!(one["url"], format!("https://www.pinterest.com/pin/{}/", one["id"].as_str().unwrap()));
    {
        let mut n = t.net();
        let p = n.list("pi:pins")[0].clone();
        assert_eq!(
            (p["media_source"]["source_type"].as_str(), p["link"].as_str(), p["alt_text"].as_str()),
            (Some("image_url"), Some("https://northwind.test/bread"), Some("What it shows"))
        );
    }
    t.call("pinterest", "publish", pin("Two loaves", &ids, json!({"board": "B1"})), Some(&format!("{key}:c"))).await.unwrap();
    assert_eq!(t.net().list("pi:pins")[0]["media_source"]["items"].as_array().unwrap().len(), 2);
    // A video: registered, sent to the storage address with the fields it named, waited for, pinned.
    let m = t.medium("v.mp4", "video/mp4", b"VIDEO-BYTES", None, json!({}), None).await;
    let vid = t.call("pinterest", "publish_media", m, Some(&format!("media:{key}:v"))).await.unwrap()["id"].as_str().unwrap().to_string();
    assert!(vid.starts_with("video:"));
    assert_eq!(t.net().get("pi:s3")["bytes"], 11);
    assert!(t.net().get("pi:s3")["fields"].as_array().unwrap().contains(&json!("x-amz-signature")));
    let mut v = post_in("Watch the crumb", None, &[("v.mp4", "video/mp4", 11)], &[vid]);
    v["settings"] = json!({"board": "B2"});
    t.call("pinterest", "publish", v, Some(&format!("{key}:v"))).await.unwrap();
    assert_eq!(t.net().list("pi:pins")[0]["media_source"]["source_type"], "video_id");
    // A lost answer: found on its board by its description.
    t.net().lose.insert("POST /pi/v5/pins".into());
    let lost = format!("publish:{}", Uuid::now_v7());
    let before = t.net().list("pi:pins").len();
    assert!(t.call("pinterest", "publish", pin("Only once", &ids[..1], json!({"board": "B1"})), Some(&lost)).await.is_err());
    let got = t.call("pinterest", "publish", pin("Only once", &ids[..1], json!({"board": "B1"})), Some(&lost)).await.unwrap();
    assert_eq!(t.net().list("pi:pins").len(), before + 1);
    assert_eq!(got["id"], t.net().list("pi:pins")[0]["id"]);
    // A refusal in Pinterest's words.
    let e = t.call("pinterest", "publish", pin("Nowhere", &ids[..1], json!({"board": "B9"})), Some(&format!("publish:{}", Uuid::now_v7()))).await.unwrap_err();
    assert!(e.to_string().contains("Board not found"), "{e}");
    // Refreshed, numbers, the account, delete.
    t.expire(conn).await;
    let m = t.call("pinterest", "metrics", json!({"id": one["id"], "account_id": null, "handle": null}), None).await.unwrap();
    assert_eq!((m["impressions"].as_i64(), m["clicks"].as_i64(), m["engagements"].as_i64()), (Some(700), Some(39), Some(17)));
    assert_eq!(t.net().last("pi", "POST", "/v5/oauth/token").form()["grant_type"], "refresh_token");
    let a = t.call("pinterest", "account", json!({"account_id": null, "handle": null}), None).await.unwrap();
    assert_eq!(a["followers"], 210);
    t.call("pinterest", "delete", json!({"id": one["id"]}), Some(&format!("delete:{key}"))).await.unwrap();
    assert!(!t.net().list("pi:pins").iter().any(|p| p["id"] == one["id"]));
}

// ───────────── Reddit ─────────────

fn fake_reddit(n: &mut Net, r: &Req) -> Resp {
    let base = n.base.clone();
    let refused = |code: &str, words: &str, field: &str| ok(json!({"json": {"errors": [[code, words, field]]}}));
    if r.path == "/api/v1/access_token" {
        if r.header("authorization") != Some(basic("rd-client", "rd-secret").as_str()) {
            return status(401, json!({"message": "Unauthorized", "error": 401}));
        }
        let f = r.form();
        let fine = match f.get("grant_type").map(String::as_str) {
            Some("authorization_code") => f.get("code").map(String::as_str) == Some("CODE-rd"),
            Some("refresh_token") => f.get("refresh_token").map(String::as_str) == Some("RDR"),
            _ => false,
        };
        if !fine {
            return status(400, json!({"error": "invalid_grant"}));
        }
        let i = n.next();
        n.tokens("rd").push(json!(format!("RDA-{i}")));
        return ok(
            json!({"access_token": format!("RDA-{i}"), "token_type": "bearer", "expires_in": 86400, "refresh_token": "RDR", "scope": "identity submit"}),
        );
    }
    if r.path == "/upload" {
        let parts = r.parts();
        if r.header("authorization").is_some()
            || parts.get("key").map(|p| p.2.as_slice()) != Some(b"rte_images/abc.png".as_slice())
            || !parts.contains_key("file")
        {
            return Resp(
                403,
                vec![("content-type".into(), "application/xml".into())],
                b"<Error><Code>AccessDenied</Code><Message>Bad form</Message></Error>".to_vec(),
            );
        }
        let first = parts.keys().next().cloned();
        n.set("rd:s3", json!({"first": first, "bytes": parts["file"].2.len()}));
        return Resp(
            201,
            vec![("content-type".into(), "application/xml".into())],
            format!("<PostResponse><Location>{base}/rds3/rte_images%2Fabc.png</Location><Key>rte_images/abc.png</Key></PostResponse>").into_bytes(),
        );
    }
    if !n.valid("rd", r) {
        return status(401, json!({"message": "Unauthorized", "error": 401}));
    }
    if !r.header("user-agent").is_some_and(|u| u.starts_with("web:ferrum-social:")) {
        return status(429, json!({"message": "Too Many Requests", "error": 429}));
    }
    match (r.method.as_str(), r.path.as_str()) {
        ("GET", "/api/v1/me") => {
            ok(json!({"name": "northwind_bakery", "icon_img": "https://styles.redditmedia.test/n.png", "subreddit": {"title": "Northwind", "subscribers": 77}}))
        }
        ("GET", "/api/subreddit_autocomplete_v2") => {
            ok(json!({"data": {"children": [{"data": {"display_name": "Breadit", "display_name_prefixed": "r/Breadit"}}]}}))
        }
        ("GET", "/r/Breadit/api/link_flair_v2") => ok(json!([{"id": "f-1", "text": "Sourdough"}, {"id": "f-2", "text": "Question"}])),
        ("POST", "/api/media/asset.json") => {
            let f = r.form();
            if f.get("mimetype").map(String::as_str) != Some("image/png") {
                return status(400, json!({"message": "mimetype is required"}));
            }
            ok(
                json!({"args": {"action": format!("{base}/rds3/upload"), "fields": [{"name": "acl", "value": "private"}, {"name": "key", "value": "rte_images/abc.png"},
                                                                                     {"name": "x-amz-signature", "value": "sig"}]},
                      "asset": {"asset_id": "ast1", "websocket_url": "wss://ws.test/x"}}),
            )
        }
        ("POST", "/api/submit") => {
            let f = r.form();
            if f.get("sr").map(String::as_str) != Some("Breadit") {
                return refused("SUBREDDIT_NOEXIST", "that subreddit doesn't exist", "sr");
            }
            if n.list("rd:posts").iter().any(|p| p["title"] == json!(f.get("title"))) {
                return refused("ALREADY_SUB", "that link has already been submitted", "url");
            }
            let id = format!("p{}", n.next());
            n.list("rd:posts").insert(
                0,
                json!({"name": format!("t3_{id}"), "title": f.get("title"), "kind": f.get("kind"), "url": f.get("url"), "text": f.get("text"),
                                               "flair": f.get("flair_id"), "permalink": format!("/r/Breadit/comments/{id}/")}),
            );
            ok(
                json!({"json": {"errors": [], "data": {"id": id, "name": format!("t3_{id}"), "url": format!("https://www.reddit.com/r/Breadit/comments/{id}/")}}}),
            )
        }
        ("POST", "/api/submit_gallery_post.json") => {
            let b = r.json();
            let id = format!("g{}", n.next());
            n.list("rd:posts").insert(0, json!({"name": format!("t3_{id}"), "title": b["title"], "kind": "gallery", "items": b["items"], "permalink": format!("/r/Breadit/comments/{id}/")}));
            ok(json!({"json": {"errors": [], "data": {"id": id, "url": format!("https://www.reddit.com/gallery/{id}")}}}))
        }
        ("GET", "/user/northwind_bakery/submitted") => {
            let children: Vec<Value> = n.list("rd:posts").iter().map(|p| json!({"kind": "t3", "data": p})).collect();
            ok(json!({"data": {"children": children}}))
        }
        ("POST", "/api/comment") => {
            let f = r.form();
            let id = format!("t1_c{}", n.next());
            n.list("rd:comments").push(json!({"name": id, "parent": f.get("thing_id"), "text": f.get("text")}));
            ok(json!({"json": {"errors": [], "data": {"things": [{"kind": "t1", "data": {"name": id}}]}}}))
        }
        ("POST", "/api/del") => {
            let id = r.form().get("id").cloned().unwrap_or_default();
            n.list("rd:posts").retain(|p| p["name"] != json!(id));
            ok(json!({}))
        }
        ("GET", "/api/info") => ok(json!({"data": {"children": [{"data": {"name": r.query.get("id"), "score": 42, "num_comments": 8}}]}})),
        ("GET", "/message/inbox") => ok(json!({"data": {"children": [
            {"kind": "t1", "data": {"name": "t1_x1", "author": "crumb", "body": "Recipe?", "subject": "post reply", "was_comment": true, "link_id": "t3_p9"}},
            {"kind": "t1", "data": {"name": "t1_x2", "author": "yeast", "body": "u/northwind_bakery look", "subject": "username mention", "was_comment": true, "link_id": "t3_q1"}},
            {"kind": "t4", "data": {"name": "t4_m1", "author": "flour", "body": "Hello", "subject": "hi", "was_comment": false}}]}})),
        _ => status(404, json!({"message": "Not Found", "error": 404})),
    }
}

async fn reddit(t: &T) {
    let conn = t
        .connect("reddit", "rd", &["oauth.reddit.com", "reddit-uploaded-media.s3-accelerate.amazonaws.com"], json!({"client_secret": "rd-secret"}), |d| {
            let text = d.to_string().replace("https://www.reddit.com/api/v1/access_token", &format!("{}/rdw/api/v1/access_token", t.base));
            *d = serde_json::from_str(&text).unwrap();
        })
        .await;
    let q = t.sign_in(conn, "CODE-rd").await;
    assert_eq!(q.get("duration").map(String::as_str), Some("permanent"), "a refresh token is asked for");
    let account = t.account("Northwind Reddit", conn).await;
    assert_eq!(t.action(account, "rules", json!({})).await.unwrap()["rules"]["name"], "Reddit");
    let subs = t.action(account, "choices", json!({"setting": "subreddit", "query": "brea"})).await.unwrap();
    assert_eq!(subs["choices"][0], json!({"value": "Breadit", "label": "r/Breadit"}));
    let flairs = t.action(account, "choices", json!({"setting": "flair", "settings": {"subreddit": "Breadit"}})).await.unwrap();
    assert_eq!(flairs["choices"][0], json!({"value": "f-1", "label": "Sourdough"}), "the flairs of the subreddit picked");
    let settings = |title: &str| json!({"subreddit": "Breadit", "title": title, "flair": "f-1", "nsfw": false, "spoiler": false});
    let post = |text: &str, kind: &str, title: &str, ids: &[String]| {
        let media: Vec<(&str, &str, i64)> = ids.iter().map(|_| ("x.png", "image/png", 4)).collect();
        let mut v = post_in(text, Some("https://northwind.test/rye"), &media, ids);
        v["kind"] = json!(kind);
        v["settings"] = settings(title);
        v
    };
    let key = format!("publish:{}", Uuid::now_v7());
    // A text post, a link post.
    let selfp = t.call("reddit", "publish", post("Our rye method", "self", "Rye, step by step", &[]), Some(&key)).await.unwrap();
    assert_eq!(selfp["id"].as_str().map(|i| i.starts_with("t3_")), Some(true));
    t.call("reddit", "publish", post("", "link", "Rye recipe", &[]), Some(&format!("{key}:l"))).await.unwrap();
    {
        let mut n = t.net();
        let p = n.list("rd:posts").clone();
        assert_eq!((p[1]["kind"].as_str(), p[1]["text"].as_str(), p[1]["flair"].as_str()), (Some("self"), Some("Our rye method"), Some("f-1")));
        assert_eq!((p[0]["kind"].as_str(), p[0]["url"].as_str()), (Some("link"), Some("https://northwind.test/rye")));
    }
    // A picture: an upload lease, the file to the storage address with its fields, then the post by its address.
    let m = t.medium("abc.png", "image/png", b"PNG-RYE", Some("media"), json!({}), None).await;
    let pic = t.call("reddit", "publish_media", m, Some(&format!("media:{key}"))).await.unwrap()["id"].as_str().unwrap().to_string();
    assert_eq!(t.net().get("rd:s3"), json!({"first": "acl", "bytes": 7}), "the lease's fields went with the file");
    t.call("reddit", "publish", post("", "media", "Fresh rye", std::slice::from_ref(&pic)), Some(&format!("{key}:m"))).await.unwrap();
    assert!(t.net().list("rd:posts")[0]["url"].as_str().unwrap().ends_with("/rds3/upload/rte_images/abc.png"));
    // Two pictures: a gallery of their assets.
    t.call("reddit", "publish", post("", "media", "Rye gallery", &[pic.clone(), pic.clone()]), Some(&format!("{key}:g"))).await.unwrap();
    assert_eq!(t.net().list("rd:posts")[0]["items"][0]["media_id"], "ast1");
    // The first comment, under the post.
    t.call("reddit", "reply", json!({"text": "Questions welcome", "in_reply_to": selfp["id"], "root": selfp["id"]}), Some(&format!("first_comment:{key}")))
        .await
        .unwrap();
    assert_eq!(t.net().list("rd:comments")[0]["parent"], selfp["id"]);
    // A lost answer: found among the account's submissions by its title.
    t.net().lose.insert("POST /rd/api/submit".into());
    let lost = format!("publish:{}", Uuid::now_v7());
    let before = t.net().list("rd:posts").len();
    assert!(t.call("reddit", "publish", post("Once", "self", "Only once", &[]), Some(&lost)).await.is_err());
    let got = t.call("reddit", "publish", post("Once", "self", "Only once", &[]), Some(&lost)).await.unwrap();
    assert_eq!(t.net().list("rd:posts").len(), before + 1);
    assert_eq!(got["id"], t.net().list("rd:posts")[0]["name"]);
    // A refusal: Reddit answers 200 with its reasons in a list, which the platform cannot read as a refusal yet; the
    // call fails because the post has no id.
    let mut nowhere = post("x", "self", "Nowhere", &[]);
    nowhere["settings"]["subreddit"] = json!("NoSuchPlace");
    let e = t.call("reddit", "publish", nowhere, Some(&format!("publish:{}", Uuid::now_v7()))).await.unwrap_err();
    assert!(!e.to_string().contains("doesn't exist"), "Reddit's words are not read yet: {e}");
    // The inbox: a reply to a post is a comment on it, a mention a mention, a private message a direct message.
    t.pull(conn, "inbox").await;
    let kinds: BTreeMap<String, String> = t
        .messages(json!({}))
        .await
        .iter()
        .filter(|m| m["external_id"].as_str().is_some_and(|i| i.starts_with("t1_x") || i.starts_with("t4_")))
        .map(|m| (m["external_id"].as_str().unwrap().to_string(), m["kind"].as_str().unwrap().to_string()))
        .collect();
    assert_eq!(kinds, BTreeMap::from([("t1_x1".into(), "Comment".into()), ("t1_x2".into(), "Mention".into()), ("t4_m1".into(), "Direct".into())]));
    // Refreshed; numbers; the account; delete.
    t.expire(conn).await;
    let m = t.call("reddit", "metrics", json!({"id": selfp["id"], "account_id": null, "handle": null}), None).await.unwrap();
    assert_eq!(m["engagements"], 50);
    assert_eq!(t.net().last("rdw", "POST", "/api/v1/access_token").form()["grant_type"], "refresh_token");
    let a = t.call("reddit", "account", json!({"account_id": null, "handle": null}), None).await.unwrap();
    assert_eq!((a["followers"].as_i64(), a["handle"].as_str()), (Some(77), Some("northwind_bakery")));
    t.call("reddit", "delete", json!({"id": selfp["id"]}), Some(&format!("delete:{key}"))).await.unwrap();
    assert!(!t.net().list("rd:posts").iter().any(|p| p["name"] == selfp["id"]));
}

// ───────────── Mastodon (any server) ─────────────

fn fake_mastodon(n: &mut Net, r: &Req) -> Resp {
    let err = |code: u16, m: &str| status(code, json!({"error": m}));
    match (r.method.as_str(), r.path.as_str()) {
        ("POST", "/api/v1/apps") => {
            let f = r.form();
            if !f.get("redirect_uris").is_some_and(|u| u.ends_with("/api/connections/callback")) || f.get("scopes").map(String::as_str) != Some("read write") {
                return err(422, "Validation failed: Redirect URI must be an absolute URI.");
            }
            n.list("ma:apps").push(json!(f.get("client_name")));
            return ok(
                json!({"id": "1", "name": f.get("client_name"), "client_id": "MA-CID", "client_secret": "MA-CS", "redirect_uri": f.get("redirect_uris")}),
            );
        }
        ("POST", "/oauth/token") => {
            let f = r.form();
            // Doorkeeper takes the client in the body or as basic authentication.
            let in_body = f.get("client_id").map(String::as_str) == Some("MA-CID") && f.get("client_secret").map(String::as_str) == Some("MA-CS");
            if !(in_body || r.header("authorization") == Some(basic("MA-CID", "MA-CS").as_str())) || !f.contains_key("code_verifier") {
                return status(401, json!({"error": "invalid_client", "error_description": "Client authentication failed"}));
            }
            n.tokens("ma").push(json!("MAT-1"));
            return ok(json!({"access_token": "MAT-1", "token_type": "Bearer", "scope": "read write", "created_at": 1}));
        }
        _ => {}
    }
    if !n.valid("ma", r) {
        return err(401, "The access token is invalid");
    }
    let seg: Vec<&str> = r.path.trim_start_matches("/api/").split('/').collect();
    match (r.method.as_str(), seg.as_slice()) {
        ("GET", ["v1", "accounts", "verify_credentials"]) => {
            ok(json!({"id": "109", "acct": "north", "display_name": "Northwind", "avatar": "https://fedi.test/a.png", "followers_count": 321}))
        }
        ("POST", ["v2", "media"]) => {
            let parts = r.parts();
            let Some(file) = parts.get("file") else { return err(422, "Validation failed: File can't be blank") };
            let id = format!("m{}", n.next());
            let video = file.1.as_deref().is_some_and(|c| c.starts_with("video/"));
            n.list("ma:media").push(json!({"id": id, "description": String::from_utf8_lossy(&parts.get("description").map(|d| d.2.clone()).unwrap_or_default()), "polls": 0, "video": video}));
            if video { status(202, json!({"id": id, "url": null})) } else { ok(json!({"id": id, "url": format!("https://fedi.test/m/{id}")})) }
        }
        ("GET", ["v1", "media", id]) => {
            let id = id.to_string();
            let mut list = n.list("ma:media").clone();
            let m = list.iter_mut().find(|m| m["id"] == json!(id)).unwrap();
            m["polls"] = json!(m["polls"].as_u64().unwrap() + 1);
            let done = m["polls"].as_u64() >= Some(2);
            n.set("ma:media", Value::Array(list));
            if done { ok(json!({"id": id, "url": format!("https://fedi.test/m/{id}")})) } else { status(206, json!({"id": id, "url": null})) }
        }
        ("POST", ["v1", "statuses"]) => {
            let b = r.json();
            let key = r.header("idempotency-key").unwrap_or_default().to_string();
            if let Some(s) = n.list("ma:statuses").iter().find(|s| s["key"] == json!(key)).cloned() {
                return ok(json!({"id": s["id"], "url": format!("https://fedi.test/@north/{}", s["id"].as_str().unwrap())}));
            }
            if b["status"].as_str().is_some_and(|t| t.chars().count() > 500) {
                return err(422, "Validation failed: Text character limit of 500 exceeded");
            }
            if b["poll"].is_object() && b["media_ids"].is_array() {
                return err(422, "Validation failed: Cannot attach files to a post that has a poll");
            }
            let id = format!("{}", 110_000 + n.next());
            n.list("ma:statuses").push(json!({"id": id, "key": key, "body": b}));
            ok(json!({"id": id, "url": format!("https://fedi.test/@north/{id}")}))
        }
        ("POST", ["v1", "statuses", id, what @ ("favourite" | "reblog")]) => {
            n.list(&format!("ma:{what}")).push(json!(id));
            ok(json!({"id": id}))
        }
        ("DELETE", ["v1", "statuses", id]) => {
            let id = id.to_string();
            n.list("ma:statuses").retain(|s| s["id"] != json!(id));
            ok(json!({"id": id}))
        }
        ("GET", ["v1", "statuses", id]) => ok(json!({"id": id, "favourites_count": 5, "reblogs_count": 2, "replies_count": 1})),
        ("GET", ["v2", "search"]) => ok(
            json!({"accounts": [{"id": "7", "acct": format!("{}@fedi.test", r.query.get("q").cloned().unwrap_or_default()), "display_name": "Ana", "avatar": "https://fedi.test/ana.png"}],
                                             "statuses": [], "hashtags": []}),
        ),
        ("GET", ["v1", "notifications"]) => {
            if r.query.contains_key("min_id") {
                return ok(json!([]));
            }
            let mention = |id: &str, acct: &str, visibility: &str, reply_to: Option<(&str, &str)>| {
                json!({"id": id, "type": "mention", "account": {"id": format!("a-{acct}"), "acct": acct},
                       "status": {"id": format!("s{id}"), "visibility": visibility, "content": format!("<p>Hi from {acct}</p>"),
                                  "in_reply_to_id": reply_to.map(|r| r.0), "in_reply_to_account_id": reply_to.map(|r| r.1)}})
            };
            let page = json!([
                mention("903", "cy", "direct", None),
                mention("902", "bo", "public", Some(("110001", "109"))),
                mention("901", "al", "public", Some(("999", "555")))
            ]);
            let base = n.base.clone();
            Resp(
                200,
                vec![(
                    "link".into(),
                    format!("<{base}/ma/api/v1/notifications?max_id=901>; rel=\"next\", <{base}/ma/api/v1/notifications?min_id=903>; rel=\"prev\""),
                )],
                page.to_string().into_bytes(),
            )
        }
        _ => err(404, "Record not found"),
    }
}

async fn mastodon(t: &T) {
    let conn = t.connect("mastodon", "ma", &["mastodon.example"], json!({}), |_| {}).await;
    // The platform's client registered at this server by its own request, once; its token never expires.
    let q = t.sign_in(conn, "CODE-ma").await;
    assert_eq!(q["client_id"], "MA-CID");
    assert_eq!(t.net().list("ma:apps").len(), 1);
    let row = t.k.connection_defs(&t.admin).await.unwrap().into_iter().find(|c| c["name"] == "mastodon").unwrap();
    assert!(row["account"]["expires_at"].is_null(), "never expires: {row}");
    let account = t.account("Northwind Mastodon", conn).await;
    assert_eq!(t.action(account, "rules", json!({})).await.unwrap()["rules"]["name"], "Mastodon");
    let found = t.action(account, "mentions", json!({"query": "ana"})).await.unwrap();
    assert_eq!(found["mentions"][0]["text"], "@ana@fedi.test");
    let key = format!("publish:{}", Uuid::now_v7());
    // A post with its settings, then a thread answering it.
    let mut v = post_in("Spring menu", Some("https://northwind.test"), &[], &[]);
    v["settings"] = json!({"visibility": "unlisted", "sensitive": false, "spoiler": "Food", "language": "en"});
    let first = t.call("mastodon", "publish", v, Some(&key)).await.unwrap();
    {
        let mut n = t.net();
        let b = n.list("ma:statuses")[0]["body"].clone();
        assert_eq!((b["visibility"].as_str(), b["spoiler_text"].as_str(), b["language"].as_str()), (Some("unlisted"), Some("Food"), Some("en")));
        assert_eq!(n.list("ma:statuses")[0]["key"], json!(key), "under its key, which Mastodon honours");
    }
    let mut second = follow_in(post_in("Then summer", None, &[], &[]), 1, 2, first["id"].as_str().unwrap(), &kept(0, &first));
    second["settings"] = json!({"visibility": "unlisted"});
    t.call("mastodon", "publish", second, Some(&format!("{key}:1"))).await.unwrap();
    assert_eq!(t.net().list("ma:statuses")[1]["body"]["in_reply_to_id"], first["id"]);
    // A poll.
    let mut poll = post_in("Which bread?", None, &[], &[]);
    poll["settings"] = json!({"poll_options": ["Rye", "Spelt"], "poll_expires_in": "3600", "poll_multiple": true});
    t.call("mastodon", "publish", poll, Some(&format!("{key}:poll"))).await.unwrap();
    assert_eq!(t.net().list("ma:statuses").last().unwrap()["body"]["poll"], json!({"options": ["Rye", "Spelt"], "expires_in": 3600, "multiple": true}));
    // A video, processed before it is used.
    let m = t.medium("clip.mp4", "video/mp4", b"VIDEO", None, json!({}), None).await;
    let id = t.call("mastodon", "publish_media", m, Some(&format!("media:{key}"))).await.unwrap()["id"].as_str().unwrap().to_string();
    assert!(
        t.net().asked("ma").iter().filter(|a| a.starts_with("GET /api/v1/media/")).count() >= 2,
        "waited for while processing: {:?}",
        t.net().get("ma:media")
    );
    assert_eq!(t.net().list("ma:media")[0]["description"], "What it shows");
    t.call("mastodon", "publish", post_in("Watch", None, &[("clip.mp4", "video/mp4", 5)], &[id]), Some(&format!("{key}:v"))).await.unwrap();
    // A lost answer: sent again under the same key, which Mastodon answers with the status it made.
    t.net().lose.insert("POST /ma/api/v1/statuses".into());
    let lost = format!("publish:{}", Uuid::now_v7());
    let before = t.net().list("ma:statuses").len();
    assert!(t.call("mastodon", "publish", post_in("Once", None, &[], &[]), Some(&lost)).await.is_err());
    let got = t.call("mastodon", "publish", post_in("Once", None, &[], &[]), Some(&lost)).await.unwrap();
    assert_eq!(t.net().list("ma:statuses").len(), before + 1);
    assert_eq!(got["id"], t.net().list("ma:statuses").last().unwrap()["id"]);
    // A refusal in the server's words.
    let e = t.call("mastodon", "publish", post_in(&"x".repeat(501), None, &[], &[]), Some(&format!("publish:{}", Uuid::now_v7()))).await.unwrap_err();
    assert!(e.to_string().contains("character limit of 500"), "{e}");
    // Mentions: a reply to the account's own status is a comment, one to someone else's a mention, a private one direct.
    t.pull(conn, "mentions").await;
    let kinds: BTreeMap<String, String> = t
        .messages(json!({}))
        .await
        .iter()
        .filter(|m| ["s901", "s902", "s903"].contains(&m["external_id"].as_str().unwrap_or_default()))
        .map(|m| (m["external_id"].as_str().unwrap().to_string(), m["kind"].as_str().unwrap().to_string()))
        .collect();
    assert_eq!(kinds, BTreeMap::from([("s901".into(), "Mention".into()), ("s902".into(), "Comment".into()), ("s903".into(), "Direct".into())]));
    // Favourite, boost, numbers, the account, delete.
    t.call("mastodon", "like", json!({"id": first["id"]}), Some("like:ma")).await.unwrap();
    t.call("mastodon", "repost", json!({"id": first["id"]}), Some("repost:ma")).await.unwrap();
    let m = t.call("mastodon", "metrics", json!({"id": first["id"], "account_id": null, "handle": "north"}), None).await.unwrap();
    assert_eq!(m["engagements"], 8);
    let a = t.call("mastodon", "account", json!({"account_id": null, "handle": "north"}), None).await.unwrap();
    assert_eq!((a["followers"].as_i64(), a["handle"].as_str()), (Some(321), Some("north")));
    t.call("mastodon", "delete", json!({"id": first["id"]}), Some(&format!("delete:{key}"))).await.unwrap();
    assert!(!t.net().list("ma:statuses").iter().any(|s| s["id"] == first["id"]));
}

// ───────────── Bluesky ─────────────

fn fake_bluesky(n: &mut Net, r: &Req) -> Resp {
    let err = |code: u16, e: &str, m: &str| status(code, json!({"error": e, "message": m}));
    let call = r.path.trim_start_matches("/xrpc/").to_string();
    let b = r.json();
    if call == "com.atproto.server.createSession" {
        if b["identifier"] != "north.bsky.test" || b["password"] != "app-pass-1234" {
            return err(401, "AuthenticationRequired", "Invalid identifier or password");
        }
        let i = n.next();
        n.tokens("bs").push(json!(format!("BA-{i}")));
        n.list("bs:refresh").push(json!(format!("BR-{i}")));
        n.list("bs:logins").push(json!(i));
        return ok(json!({"accessJwt": format!("BA-{i}"), "refreshJwt": format!("BR-{i}"), "handle": "north.bsky.test", "did": "did:plc:north"}));
    }
    if call == "com.atproto.server.refreshSession" {
        if !r.bearer().is_some_and(|t| n.list("bs:refresh").contains(&json!(t))) {
            return err(400, "ExpiredToken", "Token has been revoked");
        }
        let i = n.next();
        n.tokens("bs").push(json!(format!("BA-{i}")));
        n.list("bs:refresh").push(json!(format!("BR-{i}")));
        n.list("bs:refreshes").push(json!(i));
        return ok(json!({"accessJwt": format!("BA-{i}"), "refreshJwt": format!("BR-{i}"), "handle": "north.bsky.test", "did": "did:plc:north"}));
    }
    if !n.valid("bs", r) {
        return err(401, "ExpiredToken", "Token has expired");
    }
    let post = |n: &mut Net, uri: &str| n.list("bs:records").iter().find(|x| x["uri"] == json!(uri)).cloned();
    match (r.method.as_str(), call.as_str()) {
        ("GET", "app.bsky.actor.getProfiles") => {
            let wanted: Vec<String> = r.target_query_all("actors");
            let known = ["ana.bsky.test"];
            ok(
                json!({"profiles": wanted.iter().filter(|h| known.contains(&h.as_str())).map(|h| json!({"did": format!("did:plc:{}", h.split('.').next().unwrap()), "handle": h})).collect::<Vec<_>>()}),
            )
        }
        ("POST", "com.atproto.repo.uploadBlob") => {
            let cid = format!("bafy-blob-{}", n.next());
            n.list("bs:blobs").push(json!({"cid": cid, "type": r.header("content-type"), "bytes": r.body.len()}));
            ok(json!({"blob": {"$type": "blob", "ref": {"$link": cid}, "mimeType": r.header("content-type"), "size": r.body.len()}}))
        }
        ("POST", "com.atproto.repo.createRecord") => {
            let rec = &b["record"];
            if b["repo"] != "north.bsky.test"
                || !rec["createdAt"].as_str().is_some_and(|t| t.ends_with('Z'))
                || b["rkey"].as_str().is_none_or(|k| k.len() != 13)
            {
                return err(400, "InvalidRequest", "Invalid record");
            }
            if rec["text"].as_str().is_some_and(|t| t.chars().count() > 300) {
                return err(400, "InvalidRequest", "Invalid app.bsky.feed.post record: Record/text must not be longer than 300 graphemes");
            }
            let collection = b["collection"].as_str().unwrap_or_default().to_string();
            let uri = format!("at://did:plc:north/{collection}/{}", b["rkey"].as_str().unwrap());
            if n.list("bs:records").iter().any(|x| x["uri"] == json!(uri)) {
                return err(400, "InvalidRequest", "Record already exists");
            }
            for key in ["root", "parent"] {
                if let Some(u) = rec["reply"][key]["uri"].as_str() {
                    let target = post(n, u);
                    if target.as_ref().map(|t| &t["cid"]) != Some(&rec["reply"][key]["cid"]) {
                        return err(400, "InvalidRequest", &format!("reply {key} does not match a post"));
                    }
                }
            }
            let cid = format!("bafy-rec-{}", n.next());
            n.list("bs:records").push(json!({"uri": uri, "cid": cid, "collection": collection, "record": rec}));
            ok(json!({"uri": uri, "cid": cid}))
        }
        ("GET", "com.atproto.repo.getRecord") => {
            let uri =
                format!("at://did:plc:north/{}/{}", r.query.get("collection").cloned().unwrap_or_default(), r.query.get("rkey").cloned().unwrap_or_default());
            match post(n, &uri) {
                Some(p) => ok(json!({"uri": uri, "cid": p["cid"], "value": p["record"]})),
                None => err(400, "RecordNotFound", "Could not locate record"),
            }
        }
        ("GET", "app.bsky.feed.getPosts") => {
            let uri = r.query.get("uris").cloned().unwrap_or_default();
            match post(n, &uri) {
                Some(p) => ok(
                    json!({"posts": [{"uri": uri, "cid": p["cid"], "record": p["record"], "likeCount": 4, "repostCount": 3, "replyCount": 1, "quoteCount": 1}]}),
                ),
                None => ok(json!({"posts": []})),
            }
        }
        ("POST", "com.atproto.repo.deleteRecord") => {
            let rkey = b["rkey"].as_str().unwrap_or_default().to_string();
            n.list("bs:records").retain(|x| !x["uri"].as_str().unwrap_or_default().ends_with(&format!("/{rkey}")));
            ok(json!({}))
        }
        ("GET", "app.bsky.actor.getProfile") => {
            ok(json!({"handle": "north.bsky.test", "displayName": "Northwind", "followersCount": 77, "avatar": "https://cdn.bsky.test/a.jpg"}))
        }
        ("GET", "app.bsky.actor.searchActorsTypeahead") => ok(json!({"actors": [{"did": "did:plc:ana", "handle": "ana.bsky.test", "displayName": "Ana"}]})),
        ("GET", "chat.bsky.convo.getConvoForMembers") if r.header("atproto-proxy") == Some("did:web:api.bsky.chat#bsky_chat") => {
            ok(json!({"convo": {"id": format!("convo-{}", r.query.get("members").cloned().unwrap_or_default())}}))
        }
        ("POST", "chat.bsky.convo.sendMessage") if r.header("atproto-proxy") == Some("did:web:api.bsky.chat#bsky_chat") => {
            n.list("bs:chats").push(json!({"convo": b["convoId"], "text": b["message"]["text"]}));
            ok(json!({"id": "msg1", "text": b["message"]["text"]}))
        }
        _ => err(404, "MethodNotImplemented", &format!("No {call}")),
    }
}

impl Req {
    /// Every value of a query parameter given more than once.
    fn target_query_all(&self, name: &str) -> Vec<String> {
        self.headers
            .get("x-raw-query")
            .map(|q| reqwest::Url::parse(&format!("http://f/?{q}")).unwrap().query_pairs().filter(|(k, _)| k == name).map(|(_, v)| v.into_owned()).collect())
            .unwrap_or_default()
    }
}

async fn bluesky(t: &T) {
    let conn = t.connect("bluesky", "bs", &["bsky.social"], json!({"handle": "north.bsky.test", "app_password": "app-pass-1234"}), |_| {}).await;
    let account = t.account("north.bsky.test", conn).await;
    assert_eq!(t.action(account, "rules", json!({})).await.unwrap()["rules"]["name"], "Bluesky");
    let key = format!("publish:{}", Uuid::now_v7());
    // Text with a link (a card), a tag and a mention, each a facet at its byte offsets.
    let text = "Bake 🍞 with @ana.bsky.test and @nobody.test #bread https://northwind.test/a";
    let mut v = post_in(text, Some("https://northwind.test/card"), &[], &[]);
    v["settings"] = json!({"language": "en"});
    let first = t.call("bluesky", "publish", v, Some(&key)).await.unwrap();
    let rec = t.net().list("bs:records")[0]["record"].clone();
    let at = |f: &Value| &text.as_bytes()[f["index"]["byteStart"].as_u64().unwrap() as usize..f["index"]["byteEnd"].as_u64().unwrap() as usize];
    let facets = rec["facets"].as_array().unwrap().clone();
    assert_eq!(facets.len(), 3, "a link, a tag and the one mention that resolves: {facets:?}");
    for f in &facets {
        let feature = &f["features"][0];
        match feature["$type"].as_str().unwrap() {
            "app.bsky.richtext.facet#link" => assert_eq!(at(f), b"https://northwind.test/a"),
            "app.bsky.richtext.facet#tag" => assert_eq!((at(f), feature["tag"].as_str()), (b"#bread".as_slice(), Some("bread"))),
            _ => assert_eq!((at(f), feature["did"].as_str()), (b"@ana.bsky.test".as_slice(), Some("did:plc:ana"))),
        }
    }
    assert_eq!((rec["embed"]["external"]["uri"].as_str(), rec["langs"].clone()), (Some("https://northwind.test/card"), json!(["en"])));
    assert!(first["url"].is_null(), "no permalink: its handle is the connection's secret, masked in answers: {first}");
    // A thread: each item answers the one before, with the root's reference carried along.
    let one = kept(0, &first);
    let second = t
        .call("bluesky", "publish", follow_in(post_in("Two", None, &[], &[]), 1, 3, first["id"].as_str().unwrap(), &one), Some(&format!("{key}:1")))
        .await
        .unwrap();
    let two = kept(1, &second);
    t.call("bluesky", "publish", follow_in(post_in("Three", None, &[], &[]), 2, 3, first["id"].as_str().unwrap(), &two), Some(&format!("{key}:2")))
        .await
        .unwrap();
    {
        let mut n = t.net();
        let third = n.list("bs:records")[2]["record"]["reply"].clone();
        assert_eq!((third["root"]["uri"].clone(), third["root"]["cid"].clone()), (first["id"].clone(), first["cid"].clone()));
        assert_eq!((third["parent"]["uri"].clone(), third["parent"]["cid"].clone()), (second["id"].clone(), second["cid"].clone()));
    }
    // Two pictures with their descriptions and shapes, and a video.
    let mut ids = vec![];
    for i in 0..2 {
        let m = t.medium(&format!("{i}.png"), "image/png", b"PNG-BYTES", None, json!({}), None).await;
        ids.push(t.call("bluesky", "publish_media", m, Some(&format!("media:{key}:3:{i}"))).await.unwrap()["id"].as_str().unwrap().to_string());
    }
    t.call("bluesky", "publish", post_in("Two pictures", None, &[("0.png", "image/png", 9), ("1.png", "image/png", 9)], &ids), Some(&format!("{key}:3")))
        .await
        .unwrap();
    let pics = t.net().list("bs:records").last().unwrap()["record"]["embed"].clone();
    assert_eq!((pics["images"].as_array().unwrap().len(), pics["images"][0]["aspectRatio"]["width"].as_u64()), (2, Some(1080)));
    assert_eq!(pics["images"][1]["image"]["ref"]["$link"], ids[1]);
    let m = t.medium("v.mp4", "video/mp4", b"VIDEO", None, json!({}), None).await;
    let vid = t.call("bluesky", "publish_media", m, Some(&format!("media:{key}:4:0"))).await.unwrap()["id"].as_str().unwrap().to_string();
    t.call("bluesky", "publish", post_in("A video", None, &[("v.mp4", "video/mp4", 5)], std::slice::from_ref(&vid)), Some(&format!("{key}:4"))).await.unwrap();
    let r = t.net().list("bs:records").last().unwrap()["record"].clone();
    assert_eq!(r["embed"]["video"]["ref"]["$link"], json!(vid), "{r}");
    // The first comment: the post's reference read, then the reply.
    t.call(
        "bluesky",
        "reply",
        json!({"text": "More soon", "in_reply_to": first["id"], "root": first["id"], "created_at": "2026-11-13T10:00:00.000Z"}),
        Some(&format!("first_comment:{key}")),
    )
    .await
    .unwrap();
    assert_eq!(t.net().list("bs:records").last().unwrap()["record"]["reply"]["parent"]["cid"], first["cid"]);
    // A lost answer: the record key comes from the call's key, so the record is found where it would be; one post.
    t.net().lose.insert("POST /bs/xrpc/com.atproto.repo.createRecord".into());
    let lost = format!("publish:{}", Uuid::now_v7());
    let before = t.net().list("bs:records").len();
    assert!(t.call("bluesky", "publish", post_in("Exactly once", None, &[], &[]), Some(&lost)).await.is_err());
    let got = t.call("bluesky", "publish", post_in("Exactly once", None, &[], &[]), Some(&lost)).await.unwrap();
    assert_eq!(t.net().list("bs:records").len(), before + 1);
    assert_eq!(got["id"], t.net().list("bs:records").last().unwrap()["uri"]);
    // A refusal in Bluesky's words.
    let e = t.call("bluesky", "publish", post_in(&"a".repeat(301), None, &[], &[]), Some(&format!("publish:{}", Uuid::now_v7()))).await.unwrap_err();
    assert!(e.to_string().contains("longer than 300 graphemes"), "{e}");
    // A dropped token is renewed by the session's refresh, not a new login.
    let logins = t.net().list("bs:logins").len();
    t.net().kept.insert("bs:tokens".into(), json!([]));
    t.call("bluesky", "like", json!({"id": first["id"]}), Some("like:bs")).await.unwrap();
    {
        let mut n = t.net();
        assert_eq!((n.list("bs:logins").len(), n.list("bs:refreshes").len()), (logins, 1), "refreshed, not signed in again");
    }
    t.call("bluesky", "repost", json!({"id": first["id"]}), Some("repost:bs")).await.unwrap();
    assert!(t.net().list("bs:records").iter().any(|x| x["collection"] == "app.bsky.feed.like" && x["record"]["subject"]["cid"] == first["cid"]));
    // A direct message through the chat service.
    t.call("bluesky", "send_message", json!({"to": "did:plc:ana", "text": "Hello", "author": "ana.bsky.test"}), Some("reply:bs-dm")).await.unwrap();
    assert_eq!(t.net().list("bs:chats")[0]["convo"], "convo-did:plc:ana");
    let m = t.call("bluesky", "metrics", json!({"id": first["id"], "account_id": null, "handle": null}), None).await.unwrap();
    assert_eq!(m["engagements"], 9);
    let a = t.call("bluesky", "account", json!({"account_id": null, "handle": null}), None).await.unwrap();
    assert_eq!(a["followers"], 77);
    t.call("bluesky", "delete", json!({"id": got["id"]}), Some(&format!("delete:{lost}"))).await.unwrap();
    assert!(!t.net().list("bs:records").iter().any(|x| x["uri"] == got["id"]));
}

// ───────────── Google Business Profile ─────────────

fn fake_google_business(n: &mut Net, r: &Req) -> Resp {
    let err = |code: u16, m: &str, st: &str| status(code, json!({"error": {"code": code, "message": m, "status": st}}));
    if r.path == "/token" {
        let f = r.form();
        if f.get("client_id").map(String::as_str) != Some("gb-client") || f.get("client_secret").map(String::as_str) != Some("gb-secret") {
            return status(401, json!({"error": "invalid_client"}));
        }
        let fine = match f.get("grant_type").map(String::as_str) {
            Some("authorization_code") => f.get("code").map(String::as_str) == Some("CODE-gb"),
            Some("refresh_token") => f.get("refresh_token").map(String::as_str) == Some("GR"),
            _ => false,
        };
        if !fine {
            return status(400, json!({"error": "invalid_grant"}));
        }
        let i = n.next();
        n.tokens("gb").push(json!(format!("GA-{i}")));
        let mut out = json!({"access_token": format!("GA-{i}"), "expires_in": 3599});
        if f.get("grant_type").map(String::as_str) == Some("authorization_code") {
            out["refresh_token"] = json!("GR");
        }
        return ok(out);
    }
    if !n.valid("gb", r) {
        return err(401, "Request had invalid authentication credentials.", "UNAUTHENTICATED");
    }
    let path = r.path.clone();
    match (r.method.as_str(), path.as_str()) {
        ("GET", "/v1/accounts") => ok(json!({"accounts": [{"name": "accounts/1", "accountName": "Northwind", "type": "PERSONAL"}]})),
        ("GET", "/v1/accounts/1/locations") => {
            if r.query.get("readMask").is_none_or(|m| !m.contains("title")) {
                return err(400, "readMask is required", "INVALID_ARGUMENT");
            }
            ok(json!({"locations": [{"name": "locations/2", "title": "Northwind Cafe", "storefrontAddress": {"locality": "Leeds"}}]}))
        }
        ("GET", "/v1/locations/2") => ok(json!({"name": "locations/2", "title": "Northwind Cafe", "storefrontAddress": {"locality": "Leeds"}})),
        ("POST", "/v4/accounts/1/locations/2/localPosts") => {
            let b = r.json();
            if let Some(cta) = b["callToAction"].as_object()
                && cta["actionType"] != "CALL"
                && cta.get("url").and_then(Value::as_str).is_none_or(str::is_empty)
            {
                return err(400, "Request contains an invalid argument.", "INVALID_ARGUMENT");
            }
            if ["EVENT", "OFFER"].contains(&b["topicType"].as_str().unwrap_or_default()) && !b["event"]["schedule"]["startDate"]["year"].is_number() {
                return err(400, "An event needs its schedule.", "INVALID_ARGUMENT");
            }
            let name = format!("accounts/1/locations/2/localPosts/{}", n.next());
            n.list("gb:posts").insert(0, json!({"name": name, "summary": b["summary"], "body": b, "searchUrl": format!("https://local.google.test/{name}")}));
            ok(json!({"name": name, "summary": b["summary"], "state": "LIVE", "searchUrl": format!("https://local.google.test/{name}")}))
        }
        ("GET", "/v4/accounts/1/locations/2/localPosts") => ok(json!({"localPosts": n.list("gb:posts").clone()})),
        ("POST", "/v4/accounts/1/locations/2/localPosts:reportInsights") => {
            let b = r.json();
            ok(json!({"localPostMetrics": [{"localPostName": b["localPostNames"][0], "metricValues": [
                {"metric": "LOCAL_POST_VIEWS_SEARCH", "totalValue": {"metricOption": "AGGREGATED_TOTAL", "value": "640"}},
                {"metric": "LOCAL_POST_ACTIONS_CALL_TO_ACTION", "totalValue": {"metricOption": "AGGREGATED_TOTAL", "value": "12"}}]}]}))
        }
        ("DELETE", p) if p.starts_with("/v4/accounts/1/locations/2/localPosts/") => {
            let name = p.trim_start_matches("/v4/").to_string();
            n.list("gb:posts").retain(|x| x["name"] != json!(name));
            ok(json!({}))
        }
        _ => err(404, "Requested entity was not found.", "NOT_FOUND"),
    }
}

async fn google_business(t: &T) {
    let conn = t
        .connect("google_business", "gb", &["mybusiness.googleapis.com", "oauth2.googleapis.com"], json!({"client_secret": "gb-secret"}), |d| {
            let text = d
                .to_string()
                .replace("https://mybusinessaccountmanagement.googleapis.com", &format!("{}/gba", t.base))
                .replace("https://mybusinessbusinessinformation.googleapis.com", &format!("{}/gbi", t.base))
                .replace(&format!("{}/gb/token", t.base), &format!("{}/gbo/token", t.base));
            *d = serde_json::from_str(&text).unwrap();
        })
        .await;
    t.sign_in(conn, "CODE-gb").await;
    let account = t.account("Northwind on Google", conn).await;
    assert_eq!(t.action(account, "rules", json!({})).await.unwrap()["rules"]["name"], "Google Business Profile");
    let reach = t.action(account, "reachable", json!({})).await.unwrap();
    assert_eq!((reach["accounts"][0]["id"].as_str(), reach["accounts"][0]["handle"].as_str()), (Some("accounts/1/locations/2"), Some("Leeds")), "{reach}");
    t.action(account, "choose", json!({"picked": ["accounts/1/locations/2"]})).await.unwrap();
    let on = |text: &str, kind: &str, settings: Value, media: &[String]| {
        let m: Vec<(&str, &str, i64)> = media.iter().map(|_| ("p.jpg", "image/jpeg", 4)).collect();
        let mut v = post_in(text, None, &m, media);
        v["account_id"] = json!("accounts/1/locations/2");
        v["kind"] = json!(kind);
        v["settings"] = settings;
        v
    };
    let key = format!("publish:{}", Uuid::now_v7());
    // An update with a button and a picture by its address.
    let mut m = t.medium("p.jpg", "image/jpeg", b"JPEG", Some("standard"), json!({}), Some("accounts/1/locations/2")).await;
    m["url"] = json!("https://files.northwind.test/p.jpg");
    let photo = t.call("google_business", "publish_media", m, Some(&format!("media:{key}"))).await.unwrap()["id"].as_str().unwrap().to_string();
    let first = t
        .call(
            "google_business",
            "publish",
            on("Fresh today", "standard", json!({"cta": "LEARN_MORE", "cta_url": "https://northwind.test", "language": "en"}), &[photo]),
            Some(&key),
        )
        .await
        .unwrap();
    {
        let mut n = t.net();
        let b = n.list("gb:posts")[0]["body"].clone();
        assert_eq!(
            (b["topicType"].as_str(), b["callToAction"]["actionType"].as_str(), b["media"][0]["sourceUrl"].as_str()),
            (Some("STANDARD"), Some("LEARN_MORE"), Some("https://files.northwind.test/p.jpg"))
        );
    }
    assert!(first["url"].as_str().unwrap().contains("localPosts"));
    // An event and an offer, their dates as Google's date objects.
    let event = json!({"cta": "NONE", "event_title": "Bread class", "event_start": "2026-12-01", "event_end": "2026-12-02"});
    t.call("google_business", "publish", on("Join us", "event", event.clone(), &[]), Some(&format!("{key}:e"))).await.unwrap();
    assert_eq!(t.net().list("gb:posts")[0]["body"]["event"]["schedule"]["startDate"], json!({"year": 2026, "month": 12, "day": 1}));
    let mut offer = event.clone();
    offer["coupon"] = json!("BREAD10");
    t.call("google_business", "publish", on("10% off", "offer", offer, &[]), Some(&format!("{key}:o"))).await.unwrap();
    assert_eq!(t.net().list("gb:posts")[0]["body"]["offer"]["couponCode"], "BREAD10");
    // A lost answer: found among the location's posts by its words.
    t.net().lose.insert("POST /gb/v4/accounts/1/locations/2/localPosts".into());
    let lost = format!("publish:{}", Uuid::now_v7());
    let before = t.net().list("gb:posts").len();
    assert!(t.call("google_business", "publish", on("Once only", "standard", json!({"cta": "NONE"}), &[]), Some(&lost)).await.is_err());
    let got = t.call("google_business", "publish", on("Once only", "standard", json!({"cta": "NONE"}), &[]), Some(&lost)).await.unwrap();
    assert_eq!(t.net().list("gb:posts").len(), before + 1);
    assert_eq!(got["id"], t.net().list("gb:posts")[0]["name"]);
    // A refusal in Google's words.
    let e = t
        .call("google_business", "publish", on("No address", "standard", json!({"cta": "BOOK"}), &[]), Some(&format!("publish:{}", Uuid::now_v7())))
        .await
        .unwrap_err();
    assert!(e.to_string().contains("invalid argument"), "{e}");
    // Refreshed; numbers; the location; delete.
    t.expire(conn).await;
    let m = t.call("google_business", "metrics", json!({"id": first["id"], "account_id": "accounts/1/locations/2", "handle": null}), None).await.unwrap();
    assert_eq!((m["impressions"].as_i64(), m["clicks"].as_i64()), (Some(640), Some(12)));
    assert_eq!(t.net().last("gbo", "POST", "/token").form()["grant_type"], "refresh_token");
    let a = t.call("google_business", "account", json!({"account_id": "accounts/1/locations/2", "handle": null}), None).await.unwrap();
    assert_eq!(a["name"], "Northwind Cafe");
    t.call("google_business", "delete", json!({"id": first["id"]}), Some(&format!("delete:{key}"))).await.unwrap();
    assert!(!t.net().list("gb:posts").iter().any(|p| p["name"] == first["id"]));
}
