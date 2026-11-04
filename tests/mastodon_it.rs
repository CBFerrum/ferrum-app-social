//! Social's Mastodon connector against a real PostgreSQL (`FERRUM_MASTODON_TEST_DATABASE_URL`, default
//! `postgres://ferrum:ferrum@localhost:5432/ferrum_mastodon_test`; reset first) and a stand-in
//! Mastodon server speaking the parts of its REST API the connector uses, over HTTP: the token
//! checked, refused when wrong; a post sent as a status with its link, and once only when the
//! first answer is lost (Mastodon's `Idempotency-Key`); what it did read back as engagements; a
//! comment, a mention and a direct message read from the account's notifications and routed to it;
//! and a reply sent as a status answering the comment.

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

const TOKEN: &str = "a-test-access-token";

fn obj(v: Value) -> Doc {
    v.as_object().cloned().unwrap()
}

fn uid(v: &Value) -> Uuid {
    v.as_str().unwrap_or_else(|| panic!("not an id: {v}")).parse().unwrap()
}

/// What the stand-in server holds and was sent.
#[derive(Default)]
struct Server {
    /// Statuses by id, in the order made, with their counts.
    statuses: Vec<Value>,
    /// Idempotency keys already used, and the status each made.
    keys: HashMap<String, String>,
    /// How many times to drop the answer to a new status after making it (a lost reply).
    lose_answers: usize,
    notifications: Vec<Value>,
    /// Every request: method, path, idempotency key.
    seen: Vec<(String, String, Option<String>)>,
}

/// The parts of Mastodon's API the connector uses.
fn answer(s: &mut Server, method: &str, path: &str, auth: Option<&str>, key: Option<&str>, body: &str) -> (u16, String) {
    s.seen.push((method.into(), path.into(), key.map(str::to_string)));
    if auth != Some(&format!("Bearer {TOKEN}")) {
        return (401, json!({"error": "The access token is invalid"}).to_string());
    }
    let route = path.split('?').next().unwrap_or_default();
    match (method, route) {
        ("GET", "/api/v1/accounts/verify_credentials") => (200, json!({"id": "1", "username": "northwind", "acct": "northwind"}).to_string()),
        ("GET", "/api/v1/accounts/1/statuses") => {
            let mut page: Vec<Value> = s.statuses.iter().rev().cloned().collect();
            // A boost of someone else's status, which is not the brand's post.
            if !path.contains("exclude_reblogs=true") {
                page.push(json!({"id": "99", "content": "", "reblog": {"id": "7", "content": "<p>Someone else's</p>"}, "url": null}));
            }
            (200, Value::Array(page).to_string())
        }
        ("GET", "/api/v1/notifications") => (200, Value::Array(s.notifications.clone()).to_string()),
        ("POST", "/api/v1/statuses") => {
            if let Some(made) = key.and_then(|k| s.keys.get(k)) {
                let status = s.statuses.iter().find(|x| x["id"] == made.as_str()).cloned().unwrap();
                return (200, status.to_string());
            }
            let b: Value = serde_json::from_str(body).unwrap_or_default();
            let id = (100 + s.statuses.len()).to_string();
            let status = json!({"id": id, "content": format!("<p>{}</p>", b["status"].as_str().unwrap_or_default()), "url": format!("https://social.example/@northwind/{id}"),
                                "in_reply_to_id": b["in_reply_to_id"], "favourites_count": 0, "reblogs_count": 0, "replies_count": 0, "edited_at": null});
            s.statuses.push(status.clone());
            if let Some(k) = key {
                s.keys.insert(k.to_string(), id);
            }
            if s.lose_answers > 0 {
                s.lose_answers -= 1;
                return (503, json!({"error": "upstream timed out"}).to_string());
            }
            (200, status.to_string())
        }
        _ => (404, json!({"error": "Record not found"}).to_string()),
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
                    answer(&mut state.lock().unwrap(), &method, &path, header("authorization").as_deref(), header("idempotency-key").as_deref(), &body);
                let resp = format!("HTTP/1.1 {code} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{text}", text.len());
                let _ = sock.write_all(resp.as_bytes()).await;
            });
        }
    });
    format!("http://127.0.0.1:{port}")
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
fn a_mastodon_account() {
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().thread_stack_size(16 << 20).build().unwrap();
    rt.block_on(async { tokio::spawn(scenario()).await.unwrap() });
}

async fn scenario() {
    let url = std::env::var("FERRUM_MASTODON_TEST_DATABASE_URL").unwrap_or_else(|_| "postgres://ferrum:ferrum@localhost:5432/ferrum_mastodon_test".into());
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
    let instance = serve(server.clone()).await;

    // ───────────── the connection ─────────────
    let listed = k.connectors(&admin).await.unwrap();
    let info = listed.iter().find(|c| c.provider == "mastodon").expect("compiled in with Social");
    assert_eq!(info.capabilities, ["publish_text", "reply", "read_messages", "read_engagement"]);
    let mapping = json!({
        "posts": {"entity": "SocialPublication", "fields": {"text": "text", "link": "link", "impressions": "impressions", "engagements": "engagements", "permalink": "permalink"},
                  "conflict": "source"},
        "messages": {"entity": "SocialMessage", "fields": {"account": "account_handle", "kind": "kind", "author": "author", "text": "text", "post": "post_external_id"},
                     "values": {"kind": {"comment": "Comment", "mention": "Mention", "direct": "Direct"}}},
        "replies": {"entity": "SocialReply", "fields": {"text": "text", "in_reply_to": "in_reply_to"}},
    });
    let input = |token: &str| -> ConnectionInput {
        serde_json::from_value(json!({"provider": "mastodon", "name": "Northwind on Mastodon", "config": {"instance": instance}, "credentials": {"token": token}, "mapping": mapping})).unwrap()
    };
    let wrong = k.save_connection(&admin, None, input("not-the-token")).await.unwrap();
    assert!(matches!(k.test_connection(&admin, wrong).await, Err(Error::Unauthorized(m)) if m.contains("The access token is invalid")));
    let conn = k.save_connection(&admin, None, input(TOKEN)).await.unwrap();
    assert_eq!(k.test_connection(&admin, conn).await.unwrap(), "@northwind");

    // ───────────── a post, sent once though its first answer is lost ─────────────
    let brand = uid(&k.insert(&admin, "SocialBrand", obj(json!({"brand_name": "Northwind"}))).await.unwrap()["id"]);
    let account =
        json!({"account_name": "Northwind Mastodon", "brand": brand.to_string(), "network": "Mastodon", "handle": "northwind", "connection": conn.to_string()});
    let account = uid(&k.insert(&admin, "SocialAccount", obj(account)).await.unwrap()["id"]);
    let author = {
        let id = k.create_user(&admin, "author@brand.test", "Author", Some("Correct-Horse-9"), &["Social Author".to_string()], false).await.unwrap();
        k.load_ctx(tenant, id, None, "it".into()).await.unwrap()
    };
    let at = Utc::now() + Duration::minutes(5);
    let post = json!({"headline": "Spring", "brand": brand.to_string(), "text": "Spring is here.", "link": "https://northwind.example/spring", "scheduled_at": at.to_rfc3339(),
                      "targets": [{"account": account.to_string()}]});
    let post = uid(&k.insert(&author, "SocialPost", obj(post)).await.unwrap()["id"]);
    k.run_action(&author, "SocialPost", post, "send_for_review", Value::Null).await.unwrap();
    k.run_action(&admin, "SocialPost", post, "approve", Value::Null).await.expect("approved by an administrator, not its author");
    server.lock().unwrap().lose_answers = 1;
    ferrum_app_social::maintenance_at(&k, at + Duration::minutes(1)).await.unwrap();
    t.work().await;
    let (statuses, posts_sent): (Vec<Value>, Vec<Option<String>>) = {
        let s = server.lock().unwrap();
        (s.statuses.clone(), s.seen.iter().filter(|(m, p, _)| m == "POST" && p == "/api/v1/statuses").map(|(_, _, k)| k.clone()).collect())
    };
    assert_eq!(statuses.len(), 1, "one status, though it was sent twice");
    assert_eq!(posts_sent.len(), 2, "the first answer was lost, so it was sent again");
    assert!(posts_sent[0].is_some() && posts_sent[0] == posts_sent[1], "with the same Idempotency-Key both times: {posts_sent:?}");
    assert_eq!(statuses[0]["content"], "<p>Spring is here. https://northwind.example/spring</p>");
    ferrum_app_social::maintenance_at(&k, at + Duration::minutes(2)).await.unwrap();
    assert_eq!(k.get(&admin, "SocialPost", post, false).await.unwrap()["status"], "Published");
    let publication = t.rows("SocialPublication", json!([["post", "=", post.to_string()]])).await.remove(0);
    assert_eq!(publication["external_id"], "100");

    // ───────────── what it did ─────────────
    {
        let mut s = server.lock().unwrap();
        s.statuses[0]["favourites_count"] = json!(12);
        s.statuses[0]["reblogs_count"] = json!(3);
        s.statuses[0]["replies_count"] = json!(2);
    }
    k.sync_connection(&admin, conn, false).await.unwrap();
    t.work().await;
    let read = t.rows("SocialPublication", json!([["external_id", "=", "100"]])).await.remove(0);
    assert_eq!((read["engagements"].as_i64(), read["permalink"].as_str()), (Some(17), Some("https://social.example/@northwind/100")));
    assert_eq!(uid(&read["post"]), post, "the same publication");
    assert_eq!(t.rows("SocialPublication", json!([])).await.len(), 1, "boosts are not the brand's posts");

    // ───────────── sent again after Mastodon forgot its key: still one status ─────────────
    server.lock().unwrap().keys.clear();
    let (id, key) = (uid(&publication["id"]), format!("{conn}:{}:posts", publication["id"].as_str().unwrap()));
    let mut tx = k.db.tenant_tx(tenant).await.unwrap();
    let again =
        ferrum_kernel::jobs::NewJob::new(ferrum_kernel::connect::DELIVER_KIND, json!({"connection": conn, "stream": "posts", "record": id, "key": key}));
    ferrum_kernel::jobs::enqueue(&mut tx, tenant, None, again).await.unwrap();
    tx.commit().await.unwrap();
    t.work().await;
    assert_eq!(server.lock().unwrap().statuses.len(), 1, "what is out stays as it went");

    // ───────────── the inbox, and a reply ─────────────
    server.lock().unwrap().notifications = vec![
        json!({"id": "n1", "type": "mention", "account": {"acct": "pat@other.example"},
               "status": {"id": "200", "content": "<p>@northwind is sage in stock?</p>", "in_reply_to_id": "100", "in_reply_to_account_id": "1", "visibility": "public"}}),
        json!({"id": "n2", "type": "mention", "account": {"acct": "sam@other.example"},
               "status": {"id": "201", "content": "<p>Loving @northwind</p>", "in_reply_to_id": null, "in_reply_to_account_id": null, "visibility": "public"}}),
        json!({"id": "n3", "type": "mention", "account": {"acct": "lee@other.example"},
               "status": {"id": "202", "content": "<p>@northwind can we talk?</p>", "in_reply_to_id": null, "in_reply_to_account_id": null, "visibility": "direct"}}),
    ];
    k.sync_connection(&admin, conn, false).await.unwrap();
    t.work().await;
    let inbox: Vec<(String, String, String)> = t
        .rows("SocialMessage", json!([]))
        .await
        .iter()
        .map(|m| (m["author"].as_str().unwrap().to_string(), m["kind"].as_str().unwrap().to_string(), m["text"].as_str().unwrap().to_string()))
        .collect::<Vec<_>>()
        .into_iter()
        .collect();
    let mut kinds: Vec<(String, String)> = inbox.iter().map(|(a, k, _)| (a.clone(), k.clone())).collect();
    kinds.sort();
    assert_eq!(
        kinds,
        [
            ("lee@other.example".to_string(), "Direct".to_string()),
            ("pat@other.example".to_string(), "Comment".to_string()),
            ("sam@other.example".to_string(), "Mention".to_string())
        ]
    );
    assert!(inbox.iter().any(|(_, _, text)| text == "@northwind is sage in stock?"), "statuses read as plain text");
    let comment = t.rows("SocialMessage", json!([["author", "=", "pat@other.example"]])).await.remove(0);
    assert_eq!((uid(&comment["account"]), comment["post_external_id"].as_str()), (account, Some("100")));
    let reply = k.insert(&author, "SocialReply", obj(json!({"message": comment["id"], "text": "Yes, in every size."}))).await.unwrap();
    assert_eq!(reply["in_reply_to"], "200");
    t.work().await;
    let answered = server.lock().unwrap().statuses.iter().find(|s| s["in_reply_to_id"] == "200").cloned().expect("a status answering the comment");
    assert_eq!(answered["content"], "<p>Yes, in every size.</p>");
    ferrum_app_social::maintenance_at(&k, Utc::now()).await.unwrap();
    assert_eq!(k.get(&admin, "SocialReply", uid(&reply["id"]), false).await.unwrap()["external_id"], answered["id"]);
}
