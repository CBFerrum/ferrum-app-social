//! Social's Bluesky connector against a real PostgreSQL (`FERRUM_BLUESKY_TEST_DATABASE_URL`, default
//! `postgres://ferrum:ferrum@localhost:5432/ferrum_bluesky_test`; reset first) and a stand-in AT
//! Protocol server speaking the parts of its XRPC API the connector uses, over HTTP: the app password
//! checked, refused when wrong; one session kept, and opened again when the server stops taking it;
//! a post sent as a record with its link as a link facet, and once only when the first answer is lost
//! (its record key made from the delivery's); what it did read back as engagements, reposts not the
//! brand's; a reply, a mention and a quote read from the account's notifications and routed to it,
//! likes left out; and a reply sent in the thread of what it answers.

use chrono::{Duration, Utc};
use ferrum_kernel::connect::ConnectionInput;
use ferrum_kernel::secret::SecretKey;
use ferrum_kernel::tenant::NewTenant;
use ferrum_kernel::{Ctx, Db, DbConfig, Doc, Error, HookRegistry, Kernel, KernelSettings, ListQuery};
use ferrum_meta::Catalog;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use uuid::Uuid;

const APP_PASSWORD: &str = "abcd-efgh-ijkl-mnop";
const DID: &str = "did:plc:northwind";
const HANDLE: &str = "northwind.bsky.social";

fn obj(v: Value) -> Doc {
    v.as_object().cloned().unwrap()
}

fn uid(v: &Value) -> Uuid {
    v.as_str().unwrap_or_else(|| panic!("not an id: {v}")).parse().unwrap()
}

/// A query string's text, `%XX` and `+` read.
fn decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 3 <= b.len()
            && let Ok(v) = u8::from_str_radix(std::str::from_utf8(&b[i + 1..i + 3]).unwrap_or(""), 16)
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(if b[i] == b'+' { b' ' } else { b[i] });
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// What the stand-in server holds and was sent.
#[derive(Default)]
struct Server {
    /// The account's posts, oldest first, as Bluesky shows a post.
    posts: Vec<Value>,
    /// Other people's posts, by what the account's notifications name.
    others: Vec<Value>,
    notifications: Vec<Value>,
    /// The access token the server takes now, and how many sessions it opened.
    jwt: String,
    opened: usize,
    /// How many times to drop the answer to a new record after making it (a lost reply).
    lose_answers: usize,
    /// Every request: method and route.
    seen: Vec<(String, String)>,
    /// Blobs uploaded: (content type, bytes).
    blobs: Vec<(String, usize)>,
}

/// The parts of the XRPC API the connector uses.
fn answer(s: &mut Server, method: &str, path: &str, auth: Option<&str>, ctype: Option<&str>, body: &str) -> (u16, String) {
    let (route, query) = path.split_once('?').unwrap_or((path, ""));
    let q: HashMap<String, String> = query.split('&').filter_map(|p| p.split_once('=')).map(|(k, v)| (decode(k), decode(v))).collect();
    s.seen.push((method.into(), route.into()));
    let b: Value = serde_json::from_str(body).unwrap_or_default();
    if (method, route) == ("POST", "/xrpc/com.atproto.server.createSession") {
        if b["password"] != APP_PASSWORD {
            return (401, json!({"error": "AuthenticationRequired", "message": "Invalid identifier or password"}).to_string());
        }
        s.opened += 1;
        s.jwt = format!("jwt-{}", s.opened);
        return (200, json!({"accessJwt": s.jwt, "refreshJwt": "refresh", "did": DID, "handle": HANDLE}).to_string());
    }
    if auth != Some(&format!("Bearer {}", s.jwt)) {
        return (400, json!({"error": "ExpiredToken", "message": "Token has expired"}).to_string());
    }
    let find = |list: &[Value], uri: &str| list.iter().find(|p| p["uri"] == uri).cloned();
    match (method, route) {
        ("GET", "/xrpc/com.atproto.repo.getRecord") => {
            let uri = format!("at://{DID}/app.bsky.feed.post/{}", q.get("rkey").cloned().unwrap_or_default());
            match find(&s.posts, &uri) {
                Some(p) => (200, json!({"uri": p["uri"], "cid": p["cid"], "value": p["record"]}).to_string()),
                None => (400, json!({"error": "RecordNotFound", "message": "Could not locate record"}).to_string()),
            }
        }
        ("POST", "/xrpc/com.atproto.repo.createRecord") => {
            let uri = format!("at://{DID}/app.bsky.feed.post/{}", b["rkey"].as_str().unwrap_or_default());
            if find(&s.posts, &uri).is_some() {
                return (400, json!({"error": "InvalidRequest", "message": "Record already exists"}).to_string());
            }
            let cid = format!("cid-{}", s.posts.len());
            s.posts.push(json!({"uri": uri, "cid": cid, "author": {"did": DID, "handle": HANDLE}, "record": b["record"],
                                "likeCount": 0, "repostCount": 0, "replyCount": 0, "quoteCount": 0, "indexedAt": Utc::now().to_rfc3339()}));
            if s.lose_answers > 0 {
                s.lose_answers -= 1;
                return (503, json!({"error": "InternalServerError", "message": "upstream timed out"}).to_string());
            }
            (200, json!({"uri": uri, "cid": cid}).to_string())
        }
        ("POST", "/xrpc/com.atproto.repo.uploadBlob") => {
            let ctype = ctype.unwrap_or_default().to_string();
            s.blobs.push((ctype.clone(), body.len()));
            (200, json!({"blob": {"$type": "blob", "ref": {"$link": format!("bafkrei-{}", s.blobs.len())}, "mimeType": ctype, "size": body.len()}}).to_string())
        }
        ("GET", "/xrpc/app.bsky.feed.getAuthorFeed") => {
            let mut feed: Vec<Value> = s.posts.iter().rev().filter(|p| p["record"]["reply"].is_null()).map(|p| json!({"post": p})).collect();
            // A repost of someone else's post, which is not the brand's.
            feed.push(json!({"post": {"uri": "at://did:plc:pat/app.bsky.feed.post/r9", "cid": "c9", "record": {"text": "Someone else's"}},
                             "reason": {"$type": "app.bsky.feed.defs#reasonRepost"}}));
            (200, json!({"feed": feed, "cursor": null}).to_string())
        }
        ("GET", "/xrpc/app.bsky.notification.listNotifications") => (200, json!({"notifications": s.notifications, "cursor": null}).to_string()),
        ("GET", "/xrpc/app.bsky.feed.getPosts") => {
            let uri = q.get("uris").cloned().unwrap_or_default();
            let found = find(&s.others, &uri).or_else(|| find(&s.posts, &uri));
            (200, json!({"posts": found.into_iter().collect::<Vec<_>>()}).to_string())
        }
        _ => (404, json!({"error": "MethodNotImplemented"}).to_string()),
    }
}

async fn serve(state: Arc<Mutex<Server>>) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        while let Ok((mut sock, _)) = listener.accept().await {
            let state = state.clone();
            tokio::spawn(async move {
                let (mut buf, mut n) = (vec![0u8; 65536], 0);
                let head_end = loop {
                    match sock.read(&mut buf[n..]).await {
                        Ok(0) | Err(_) => return,
                        Ok(m) => n += m,
                    }
                    if let Some(i) = buf[..n].windows(4).position(|w| w == b"\r\n\r\n") {
                        break i + 4;
                    }
                };
                let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
                let header = |name: &str| {
                    head.lines().find_map(|l| {
                        let (k, v) = l.split_once(':')?;
                        k.eq_ignore_ascii_case(name).then(|| v.trim().to_string())
                    })
                };
                let length: usize = header("content-length").and_then(|v| v.parse().ok()).unwrap_or(0);
                while n < head_end + length {
                    match sock.read(&mut buf[n..]).await {
                        Ok(0) | Err(_) => break,
                        Ok(m) => n += m,
                    }
                }
                let body = String::from_utf8_lossy(&buf[head_end..n]).to_string();
                let mut line = head.lines().next().unwrap_or_default().split_whitespace();
                let (method, path) = (line.next().unwrap_or_default().to_string(), line.next().unwrap_or_default().to_string());
                let (code, text) =
                    answer(&mut state.lock().unwrap(), &method, &path, header("authorization").as_deref(), header("content-type").as_deref(), &body);
                let resp = format!("HTTP/1.1 {code} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{text}", text.len());
                let _ = sock.write_all(resp.as_bytes()).await;
            });
        }
    });
    format!("http://127.0.0.1:{port}")
}

/// An in-memory object store standing in for S3.
#[derive(Default)]
struct MemFiles(std::sync::Mutex<HashMap<String, Vec<u8>>>);

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

struct T {
    k: Arc<Kernel>,
    admin: Ctx,
}

impl T {
    async fn rows(&self, entity: &str, filters: Value) -> Vec<Doc> {
        let q = ListQuery { filters: ferrum_kernel::query::parse_filters(&filters).unwrap(), limit: Some(100), ..Default::default() };
        self.k.list(&self.admin, entity, &q).await.unwrap().rows
    }
    async fn work(&self) {
        loop {
            let mut tx = self.k.db.system_tx().await.unwrap();
            sqlx::query("update job set run_at = now() where status = 'queued'").execute(&mut *tx).await.unwrap();
            sqlx::query("update connection set waiting_until = now() where waiting_until is not null").execute(&mut *tx).await.unwrap();
            tx.commit().await.unwrap();
            while self.k.dispatch_outbox_once().await.unwrap() > 0 {}
            if self.k.run_jobs_once(4).await.unwrap() == 0 {
                break;
            }
        }
    }
}

#[test]
fn a_bluesky_account() {
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().thread_stack_size(16 << 20).build().unwrap();
    rt.block_on(async { tokio::spawn(scenario()).await.unwrap() });
}

async fn scenario() {
    let url = std::env::var("FERRUM_BLUESKY_TEST_DATABASE_URL").unwrap_or_else(|_| "postgres://ferrum:ferrum@localhost:5432/ferrum_bluesky_test".into());
    let db = Db::connect(DbConfig { url, max_connections: 16, ..Default::default() }).await.expect("connect");
    sqlx::raw_sql("drop schema if exists public cascade; create schema public;").execute(&db.pool).await.expect("reset");
    let mut packages = ferrum_domain::packages().expect("platform packages");
    packages.extend(ferrum_app_social::packages().expect("app packages"));
    let catalog = Catalog::from_packages(packages).expect("catalog");
    let mut hooks = HookRegistry::default();
    ferrum_domain::register(&mut hooks);
    ferrum_app_social::register(&mut hooks);
    // The stand-in server is on this machine.
    let settings = KernelSettings { secret_key: Some(SecretKey::new([7u8; 32])), webhooks_allow_private: true, ..Default::default() };
    let k = Kernel::boot(db, catalog, hooks, settings, None).await.expect("boot");
    k.set_file_store(Arc::new(MemFiles::default()));
    ferrum_domain::maintenance(&k).await.expect("partitions");
    let tenant = k
        .create_tenant(NewTenant {
            slug: "brand".into(),
            name: "Brand".into(),
            admin_email: "admin@brand.test".into(),
            admin_password: "Correct-Horse-9".into(),
            admin_name: None,
            packages: vec!["social".into()],
            settings: Some(json!({"timezone": "UTC"})),
        })
        .await
        .expect("tenant");
    let session = k.login("brand", "admin@brand.test", "Correct-Horse-9").await.expect("login");
    let admin = k.load_ctx(tenant, session.user, Some(session.token_version), "it".into()).await.expect("ctx");
    let t = T { k: k.clone(), admin: admin.clone() };
    let server = Arc::new(Mutex::new(Server::default()));
    let service = serve(server.clone()).await;

    // ───────────── the connection ─────────────
    let listed = k.connectors(&admin).await.unwrap();
    let info = listed.iter().find(|c| c.provider == "bluesky").expect("compiled in with Social");
    assert_eq!(info.capabilities, ["publish_text", "publish_image", "reply", "read_messages", "read_engagement"]);
    let mapping = json!({
        "posts": {"entity": "SocialPublication", "fields": {"text": "text", "link": "link", "image": "image", "image_alt": "image_alt", "impressions": "impressions",
                                                             "engagements": "engagements", "permalink": "permalink"},
                  "conflict": "source"},
        "messages": {"entity": "SocialMessage", "fields": {"account": "account_handle", "kind": "kind", "author": "author", "text": "text", "post": "post_external_id"},
                     "values": {"kind": {"comment": "Comment", "mention": "Mention", "direct": "Direct"}}},
        "replies": {"entity": "SocialReply", "fields": {"text": "text", "in_reply_to": "in_reply_to"}},
    });
    let input = |name: &str, password: &str| -> ConnectionInput {
        serde_json::from_value(json!({"provider": "bluesky", "name": name, "config": {"service": service},
                                      "credentials": {"identifier": HANDLE, "app_password": password}, "mapping": mapping}))
        .unwrap()
    };
    let wrong = k.save_connection(&admin, None, input("Northwind, wrong password", "not-the-password")).await.unwrap();
    assert!(matches!(k.test_connection(&admin, wrong).await, Err(Error::Unauthorized(m)) if m.contains("Invalid identifier or password")));
    let conn = k.save_connection(&admin, None, input("Northwind on Bluesky", APP_PASSWORD)).await.unwrap();
    assert_eq!(k.test_connection(&admin, conn).await.unwrap(), format!("@{HANDLE}"));

    // ───────────── a post, sent once though its first answer is lost ─────────────
    let brand = uid(&k.insert(&admin, "SocialBrand", obj(json!({"brand_name": "Northwind"}))).await.unwrap()["id"]);
    let made = k
        .insert(
            &admin,
            "SocialAccount",
            obj(json!({"account_name": "Northwind Bluesky", "brand": brand.to_string(), "network": "Bluesky", "handle": HANDLE,
                                                   "connection": "Northwind on Bluesky"})),
        )
        .await
        .unwrap();
    let account = uid(&made["id"]);
    let author = {
        let id = k.create_user(&admin, "author@brand.test", "Author", Some("Correct-Horse-9"), &["Social Author".to_string()], false).await.unwrap();
        k.load_ctx(tenant, id, None, "it".into()).await.unwrap()
    };
    let at = Utc::now() + Duration::minutes(5);
    let post = json!({"headline": "Spring", "brand": brand.to_string(), "text": "Spring is here.", "link": "https://northwind.example/spring", "scheduled_at": at.to_rfc3339(),
                      "targets": [{"account": account.to_string()}]});
    let post = uid(&k.insert(&author, "SocialPost", obj(post)).await.unwrap()["id"]);
    k.run_action(&author, "SocialPost", post, "send_for_review", Value::Null).await.unwrap();
    k.run_action(&admin, "SocialPost", post, "approve", Value::Null).await.expect("approved");
    server.lock().unwrap().lose_answers = 1;
    ferrum_app_social::maintenance_at(&k, at + Duration::minutes(1)).await.unwrap();
    t.work().await;
    let (posts, creates) = {
        let s = server.lock().unwrap();
        (s.posts.clone(), s.seen.iter().filter(|(m, r)| m == "POST" && r == "/xrpc/com.atproto.repo.createRecord").count())
    };
    assert_eq!((posts.len(), creates), (1, 1), "one post: the second try found the first's record under the same key");
    let record = &posts[0]["record"];
    assert_eq!(record["text"], "Spring is here. https://northwind.example/spring");
    assert_eq!(
        (record["facets"][0]["index"]["byteStart"].as_u64(), record["facets"][0]["features"][0]["uri"].as_str()),
        (Some(16), Some("https://northwind.example/spring")),
        "the link a link"
    );
    ferrum_app_social::maintenance_at(&k, at + Duration::minutes(2)).await.unwrap();
    assert_eq!(k.get(&admin, "SocialPost", post, false).await.unwrap()["status"], "Published");
    let publication = t.rows("SocialPublication", json!([["post", "=", post.to_string()]])).await.remove(0);
    let uri = posts[0]["uri"].as_str().unwrap().to_string();
    assert_eq!(publication["external_id"], uri.as_str());

    // ───────────── what it did ─────────────
    {
        let mut s = server.lock().unwrap();
        s.posts[0]["likeCount"] = json!(12);
        s.posts[0]["repostCount"] = json!(3);
        s.posts[0]["replyCount"] = json!(2);
        s.posts[0]["quoteCount"] = json!(1);
    }
    k.sync_connection(&admin, conn, false).await.unwrap();
    t.work().await;
    let read = t.rows("SocialPublication", json!([["external_id", "=", uri.as_str()]])).await.remove(0);
    let rkey = uri.rsplit('/').next().unwrap();
    assert_eq!((read["engagements"].as_i64(), read["permalink"].as_str()), (Some(18), Some(format!("https://bsky.app/profile/{HANDLE}/post/{rkey}").as_str())));
    assert_eq!(t.rows("SocialPublication", json!([])).await.len(), 1, "reposts are not the brand's posts");
    assert_eq!(server.lock().unwrap().opened, 1, "one session, kept");

    // ───────────── a session the server stops taking is opened again ─────────────
    server.lock().unwrap().jwt = "revoked".into();
    k.sync_connection(&admin, conn, false).await.unwrap();
    t.work().await;
    assert_eq!(server.lock().unwrap().opened, 2);

    // ───────────── a picture, embedded with what it shows ─────────────
    let png = k.store_file(&author, None, "sage.png", "image/png", b"\x89PNG\r\n\x1a\nsage in bloom").await.unwrap();
    let at = Utc::now() + Duration::minutes(5);
    let pictured = json!({"headline": "Sage", "brand": brand.to_string(), "text": "Sage, in bloom.", "image": png["url"], "image_alt": "Sage flowers in a field",
                          "scheduled_at": at.to_rfc3339(), "targets": [{"account": account.to_string()}]});
    let pictured = uid(&k.insert(&author, "SocialPost", obj(pictured)).await.unwrap()["id"]);
    k.run_action(&author, "SocialPost", pictured, "send_for_review", Value::Null).await.unwrap();
    k.run_action(&admin, "SocialPost", pictured, "approve", Value::Null).await.unwrap();
    ferrum_app_social::maintenance_at(&k, at + Duration::minutes(1)).await.unwrap();
    t.work().await;
    let (blobs, post) = {
        let s = server.lock().unwrap();
        (s.blobs.clone(), s.posts.iter().find(|p| p["record"]["text"] == "Sage, in bloom.").cloned().expect("the post"))
    };
    assert_eq!((blobs.len(), blobs[0].0.as_str()), (1, "image/png"), "uploaded once, as what it is");
    let image = &post["record"]["embed"]["images"][0];
    assert_eq!(
        (post["record"]["embed"]["$type"].as_str(), image["alt"].as_str(), image["image"]["ref"]["$link"].as_str()),
        (Some("app.bsky.embed.images"), Some("Sage flowers in a field"), Some("bafkrei-1")),
        "embedded, saying what it shows"
    );

    // ───────────── the inbox, and a reply in its thread ─────────────
    let pat_reply = json!({"uri": "at://did:plc:pat/app.bsky.feed.post/r1", "cid": "c-r1", "author": {"did": "did:plc:pat", "handle": "pat.bsky.social"},
                           "record": {"text": "Is sage in stock?", "reply": {"root": {"uri": uri, "cid": "cid-0"}, "parent": {"uri": uri, "cid": "cid-0"}}}});
    {
        let mut s = server.lock().unwrap();
        s.others.push(pat_reply.clone());
        s.notifications = vec![
            json!({"uri": pat_reply["uri"], "cid": "c-r1", "author": pat_reply["author"], "reason": "reply", "record": pat_reply["record"]}),
            json!({"uri": "at://did:plc:sam/app.bsky.feed.post/m1", "cid": "c-m1", "author": {"did": "did:plc:sam", "handle": "sam.bsky.social"}, "reason": "mention",
                   "record": {"text": "Loving @northwind.bsky.social"}}),
            json!({"uri": "at://did:plc:kim/app.bsky.feed.post/q1", "cid": "c-q1", "author": {"did": "did:plc:kim", "handle": "kim.bsky.social"}, "reason": "quote",
                   "reasonSubject": uri, "record": {"text": "This!"}}),
            json!({"uri": "at://did:plc:lee/app.bsky.feed.like/l1", "cid": "c-l1", "author": {"did": "did:plc:lee", "handle": "lee.bsky.social"}, "reason": "like",
                   "record": {}}),
        ];
    }
    k.sync_connection(&admin, conn, false).await.unwrap();
    t.work().await;
    let mut kinds: Vec<(String, String)> =
        t.rows("SocialMessage", json!([])).await.iter().map(|m| (m["author"].as_str().unwrap().to_string(), m["kind"].as_str().unwrap().to_string())).collect();
    kinds.sort();
    assert_eq!(
        kinds,
        [
            ("kim.bsky.social".to_string(), "Mention".to_string()),
            ("pat.bsky.social".to_string(), "Comment".to_string()),
            ("sam.bsky.social".to_string(), "Mention".to_string())
        ],
        "likes are not messages"
    );
    let comment = t.rows("SocialMessage", json!([["author", "=", "pat.bsky.social"]])).await.remove(0);
    assert_eq!((uid(&comment["account"]), comment["post_external_id"].as_str()), (account, Some(uri.as_str())));
    k.run_action(&author, "SocialMessage", uid(&comment["id"]), "reply", json!({"reply": "Yes, in every size."})).await.unwrap();
    t.work().await;
    let answered = server.lock().unwrap().posts.iter().find(|p| !p["record"]["reply"].is_null()).cloned().expect("a post answering the comment");
    assert_eq!(answered["record"]["text"], "Yes, in every size.");
    assert_eq!(
        (answered["record"]["reply"]["parent"]["uri"].as_str(), answered["record"]["reply"]["root"]["uri"].as_str()),
        (Some("at://did:plc:pat/app.bsky.feed.post/r1"), Some(uri.as_str())),
        "answering the comment, in the thread of the brand's post"
    );
    ferrum_app_social::maintenance_at(&k, Utc::now()).await.unwrap();
    let replies = t.rows("SocialReply", json!([])).await;
    assert_eq!(replies[0]["external_id"], answered["uri"]);
}
