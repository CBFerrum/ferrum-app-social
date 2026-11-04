//! Social media management (FERRUM-ECOSYSTEM-005) against a real PostgreSQL
//! (`FERRUM_SOCIAL_TEST_DATABASE_URL`, default `postgres://ferrum:ferrum@localhost:5432/ferrum_social_test`;
//! reset first), through the platform's connections to a stand-in network: a brand's accounts on
//! two connections and a third that can only read; posts refused where a network would not take
//! them; review by someone other than the author; a changed post approved again; sending at its
//! time, each publication through its own account's connection; what posts did read back, and a
//! post made on the network itself; the inbox routed by handle, assigned and answered back
//! through the right connection; and who may do what.

use async_trait::async_trait;
use chrono::{Duration, Utc};
use ferrum_kernel::connect::{Connection, ConnectionInput, Connector, ConnectorInfo, Incoming, Outgoing, Pulled};
use ferrum_kernel::secret::SecretKey;
use ferrum_kernel::tenant::NewTenant;
use ferrum_kernel::{Ctx, Db, DbConfig, Doc, Error, HookRegistry, Kernel, KernelSettings, ListQuery, Result};
use ferrum_meta::Catalog;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use uuid::Uuid;

fn obj(v: Value) -> Doc {
    v.as_object().cloned().unwrap()
}

fn uid(v: &Value) -> Uuid {
    v.as_str().unwrap_or_else(|| panic!("not an id: {v}")).parse().unwrap()
}

/// What the stand-in network holds and was sent.
#[derive(Default)]
struct World {
    /// The network's posts, with what each did: (id, text, impressions, engagements).
    posts: Vec<(String, String, i64, i64)>,
    /// Messages to accounts: (id, account handle, kind, author, text, post).
    messages: Vec<(String, String, String, String, String, Option<String>)>,
    /// What was sent, through which connection: (connection, stream, fields).
    sent: Vec<(Uuid, String, serde_json::Map<String, Value>)>,
}

struct Network {
    provider: &'static str,
    capabilities: Vec<String>,
    world: Arc<Mutex<World>>,
}

#[async_trait]
impl Connector for Network {
    fn info(&self) -> ConnectorInfo {
        ConnectorInfo {
            provider: self.provider.into(),
            label: "A network".into(),
            family: "social".into(),
            api_version: "2026-01".into(),
            streams: vec!["posts".into(), "messages".into(), "replies".into()],
            writes: vec!["SocialPublication".into(), "SocialReply".into()],
            capabilities: self.capabilities.clone(),
            ..Default::default()
        }
    }

    async fn check(&self, _k: &Kernel, _c: &Connection) -> Result<String> {
        Ok("account 7".into())
    }

    async fn pull(&self, _k: &Kernel, _c: &Connection, stream: &str, _cursor: Option<&str>) -> Result<Pulled> {
        let w = self.world.lock().unwrap();
        let records = match stream {
            "posts" => w
                .posts
                .iter()
                .map(|(id, text, seen, engaged)| Incoming {
                    external_id: id.clone(),
                    version: Some(format!("{seen}-{engaged}")),
                    deleted: false,
                    fields: obj(json!({"text": text, "impressions": seen, "engagements": engaged, "permalink": format!("https://network.example/p/{id}")})),
                })
                .collect(),
            "messages" => w
                .messages
                .iter()
                .map(|(id, to, kind, author, text, post)| Incoming {
                    external_id: id.clone(),
                    version: None,
                    deleted: false,
                    fields: obj(json!({"account": to, "kind": kind, "author": author, "text": text, "post": post})),
                })
                .collect(),
            _ => vec![],
        };
        Ok(Pulled { records, next: None })
    }

    async fn push(&self, _k: &Kernel, c: &Connection, change: &Outgoing) -> Result<String> {
        let mut w = self.world.lock().unwrap();
        let stream = if change.entity == "SocialReply" { "replies" } else { "posts" };
        w.sent.push((c.id, stream.into(), change.fields.clone()));
        Ok(format!("{stream}-{}", w.sent.len()))
    }
}

struct T {
    k: Arc<Kernel>,
    tenant: Uuid,
    admin: Ctx,
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
    async fn act(&self, ctx: &Ctx, id: Uuid, action: &str, params: Value) -> std::result::Result<Value, Error> {
        self.k.run_action(ctx, "SocialPost", id, action, params).await
    }
    async fn rows(&self, entity: &str, filters: Value) -> Vec<Doc> {
        let q = ListQuery { filters: ferrum_kernel::query::parse_filters(&filters).unwrap(), limit: Some(100), ..Default::default() };
        self.k.list(&self.admin, entity, &q).await.unwrap().rows
    }
    /// Jobs run, time passing between attempts, until none is left.
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
    async fn connect(&self, provider: &str, name: &str) -> Uuid {
        let mapping = json!({
            "posts": {"entity": "SocialPublication", "fields": {"text": "text", "link": "link", "impressions": "impressions", "engagements": "engagements", "permalink": "permalink"}, "conflict": "source"},
            "messages": {"entity": "SocialMessage", "fields": {"account": "account_handle", "kind": "kind", "author": "author", "text": "text", "post": "post_external_id"},
                         "values": {"kind": {"comment": "Comment", "mention": "Mention", "direct": "Direct"}}},
            "replies": {"entity": "SocialReply", "fields": {"text": "text", "in_reply_to": "in_reply_to"}},
        });
        let input: ConnectionInput =
            serde_json::from_value(json!({"provider": provider, "name": name, "credentials": {"token": "t"}, "mapping": mapping})).unwrap();
        let id = self.k.save_connection(&self.admin, None, input).await.expect("connection");
        self.k.test_connection(&self.admin, id).await.expect("it works");
        id
    }
}

fn refused<V: std::fmt::Debug>(r: std::result::Result<V, Error>, kind: &str, says: &str) {
    let msg = format!("{r:?}");
    let ok = matches!((&r, kind), (Err(Error::Invalid(_) | Error::Validation(_)), "invalid") | (Err(Error::Forbidden(_)), "forbidden"));
    assert!(ok && msg.contains(says), "expected {kind} saying {says:?}, got {msg}");
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
    let world = Arc::new(Mutex::new(World::default()));
    let reader = Arc::new(Mutex::new(World::default()));
    let mut hooks = HookRegistry::default();
    ferrum_domain::register(&mut hooks);
    ferrum_app_social::register(&mut hooks);
    let all = ["publish_text", "reply", "read_messages", "read_insights"].map(String::from).to_vec();
    hooks.add_connector(Arc::new(Network { provider: "network", capabilities: all, world: world.clone() }));
    hooks.add_connector(Arc::new(Network { provider: "reader", capabilities: vec!["read_messages".into()], world: reader.clone() }));
    let settings = KernelSettings { secret_key: Some(SecretKey::new([7u8; 32])), ..Default::default() };
    let k = Kernel::boot(db, catalog, hooks, settings, None).await.expect("boot");
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
    let t = T { k: k.clone(), tenant, admin };
    let author = t.user("author@brands.test", &["Social Author"]).await;
    let approver = t.user("approver@brands.test", &["Social Approver"]).await;
    let manager = t.user("manager@brands.test", &["Social Manager"]).await;
    let on_x = t.connect("network", "Northwind on X").await;
    let on_masto = t.connect("network", "Northwind on Mastodon").await;
    let reading = t.connect("reader", "Northwind on LinkedIn").await;

    // ───────────── brands and their accounts ─────────────
    let northwind = uid(&t.insert(&manager, "SocialBrand", json!({"brand_name": "Northwind"})).await["id"]);
    let contoso = uid(&t.insert(&manager, "SocialBrand", json!({"brand_name": "Contoso"})).await["id"]);
    let account = |name: &str, brand: Uuid, network: &str, handle: &str, conn: Uuid| json!({"account_name": name, "brand": brand.to_string(), "network": network, "handle": handle, "connection": conn.to_string()});
    refused(t.k.insert(&manager, "SocialAccount", obj(account("Lost", northwind, "X", "lost", Uuid::new_v4()))).await, "invalid", "no connection by that id");
    let nw_x = uid(&t.insert(&manager, "SocialAccount", account("Northwind X", northwind, "X", "@northwind", on_x)).await["id"]);
    assert_eq!(t.get("SocialAccount", nw_x).await["handle"], "northwind");
    let nw_masto =
        uid(&t.insert(&manager, "SocialAccount", account("Northwind Mastodon", northwind, "Mastodon", "northwind@social.example", on_masto)).await["id"]);
    let nw_li = uid(&t.insert(&manager, "SocialAccount", account("Northwind LinkedIn", northwind, "LinkedIn", "northwind-li", reading)).await["id"]);
    let nw_insta = uid(&t.insert(&manager, "SocialAccount", account("Northwind Instagram", northwind, "Instagram", "northwind.pics", on_x)).await["id"]);
    let mut paused = account("Northwind Bluesky", northwind, "Bluesky", "northwind.bsky", on_x);
    paused["active"] = json!(false);
    let nw_bsky = uid(&t.insert(&manager, "SocialAccount", paused).await["id"]);
    let co_x = uid(&t.insert(&manager, "SocialAccount", account("Contoso X", contoso, "X", "contoso", on_x)).await["id"]);
    t.insert(&manager, "SocialAccount", account("Contoso Mastodon", contoso, "Mastodon", "contoso", on_masto)).await;
    refused(t.k.insert(&author, "SocialAccount", obj(account("Mine", northwind, "X", "mine", on_x))).await, "forbidden", "");

    // ───────────── what each network takes ─────────────
    let at = Utc::now() + Duration::hours(1);
    let post = |text: &str, targets: &[Uuid]| {
        json!({"headline": "Spring launch", "brand": northwind.to_string(), "text": text, "link": "https://northwind.example/spring",
               "scheduled_at": at.to_rfc3339(), "targets": targets.iter().map(|a| json!({"account": a.to_string()})).collect::<Vec<_>>()})
    };
    refused(t.k.insert(&author, "SocialPost", obj(post(&"a".repeat(260), &[nw_x]))).await, "invalid", "for Northwind X: X takes 280 characters");
    refused(t.k.insert(&author, "SocialPost", obj(post("Spring is here", &[co_x]))).await, "invalid", "Contoso X is another brand's");
    refused(t.k.insert(&author, "SocialPost", obj(post("Spring is here", &[nw_li]))).await, "invalid", "Northwind LinkedIn's connection cannot publish posts");
    refused(t.k.insert(&author, "SocialPost", obj(post("Spring is here", &[nw_insta]))).await, "invalid", "image or a video");
    refused(t.k.insert(&author, "SocialPost", obj(post("Spring is here", &[nw_bsky]))).await, "invalid", "Northwind Bluesky is paused");
    let mut bad_link = post("Spring is here", &[nw_x]);
    bad_link["link"] = json!("northwind dot example");
    refused(t.k.insert(&author, "SocialPost", obj(bad_link)).await, "invalid", "is a web address");
    let spring = t.insert(&author, "SocialPost", post("Spring is here: new colours, same quality.", &[nw_x, nw_masto])).await;
    let id = uid(&spring["id"]);
    assert_eq!((spring["author"].as_str(), spring["status"].as_str()), (Some("author@brands.test"), Some("Draft")));
    refused(t.k.insert(&manager, "SocialPost", obj(post("Not mine", &[nw_x]))).await, "forbidden", "");

    // ───────────── review ─────────────
    t.act(&author, id, "send_for_review", Value::Null).await.unwrap();
    refused(t.act(&author, id, "approve", Value::Null).await, "forbidden", "");
    let colleague = t.user("colleague@brands.test", &["Social Author"]).await;
    refused(t.act(&colleague, id, "approve", Value::Null).await, "forbidden", "a Social Approver approves posts");
    let both = t.user("both@brands.test", &["Social Author", "Social Approver"]).await;
    let own = uid(&t.insert(&both, "SocialPost", post("My own", &[nw_x])).await["id"]);
    t.act(&both, own, "send_for_review", Value::Null).await.unwrap();
    refused(t.act(&both, own, "approve", Value::Null).await, "forbidden", "someone other than its author");
    refused(t.act(&approver, id, "reject", json!({"note": " "})).await, "invalid", "says why");
    t.act(&approver, id, "reject", json!({"note": "Say which colours"})).await.unwrap();
    assert_eq!(
        (t.get("SocialPost", id).await["status"].as_str(), t.get("SocialPost", id).await["review_note"].as_str()),
        (Some("Rejected"), Some("Say which colours"))
    );
    t.k.update(&author, "SocialPost", id, obj(json!({"text": "Spring is here: sage, sand and slate."})), None).await.unwrap();
    t.act(&author, id, "send_for_review", Value::Null).await.unwrap();
    t.act(&approver, id, "approve", Value::Null).await.expect("approved");
    t.k.update(&author, "SocialPost", id, obj(json!({"headline": "Spring launch, main"})), None).await.unwrap();
    assert_eq!(t.get("SocialPost", id).await["status"], "Approved", "its headline is the team's, not what goes out");
    t.k.update(&author, "SocialPost", id, obj(json!({"text": "Spring is here: sage, sand and slate!"})), None).await.unwrap();
    let changed = t.get("SocialPost", id).await;
    assert_eq!((changed["status"].as_str(), changed["approved_by"].as_str()), (Some("Draft"), None), "changed words need approving again");
    t.act(&author, id, "send_for_review", Value::Null).await.unwrap();
    t.act(&approver, id, "approve", Value::Null).await.unwrap();
    assert_eq!(t.get("SocialPost", id).await["approved_by"], "approver@brands.test");

    // ───────────── sent at its time, each through its own connection ─────────────
    ferrum_app_social::maintenance_at(&k, Utc::now()).await.unwrap();
    assert!(t.rows("SocialPublication", json!([["post", "=", id.to_string()]])).await.is_empty(), "not before its time");
    ferrum_app_social::maintenance_at(&k, at + Duration::minutes(1)).await.unwrap();
    assert_eq!(t.get("SocialPost", id).await["status"], "Sending");
    let made = t.rows("SocialPublication", json!([["post", "=", id.to_string()]])).await;
    assert_eq!(made.len(), 2);
    t.work().await;
    let sent: Vec<(Uuid, String, String)> =
        world.lock().unwrap().sent.iter().map(|(c, s, f)| (*c, s.clone(), f["text"].as_str().unwrap_or_default().to_string())).collect();
    let mut through: Vec<Uuid> = sent.iter().map(|(c, _, _)| *c).collect();
    through.sort();
    let mut expected = vec![on_x, on_masto];
    expected.sort();
    assert_eq!(through, expected, "one publication through each account's connection, and none through another");
    assert!(sent.iter().all(|(_, s, text)| s == "posts" && text == "Spring is here: sage, sand and slate!"));
    ferrum_app_social::maintenance_at(&k, at + Duration::minutes(2)).await.unwrap();
    let out = t.get("SocialPost", id).await;
    assert_eq!(out["status"], "Published");
    let publications = t.rows("SocialPublication", json!([["post", "=", id.to_string()]])).await;
    assert!(publications.iter().all(|p| p["external_id"].as_str().is_some_and(|e| e.starts_with("posts-")) && p["sent_at"].is_string()), "{publications:?}");
    refused(t.k.update(&author, "SocialPost", id, obj(json!({"text": "Oops"})), None).await, "invalid", "stays as it went out");
    ferrum_app_social::maintenance_at(&k, at + Duration::minutes(3)).await.unwrap();
    assert_eq!(world.lock().unwrap().sent.len(), 2, "sent once");

    // ───────────── what posts did, read back ─────────────
    let on_x_id = publications.iter().find(|p| uid(&p["account"]) == nw_x).unwrap()["external_id"].as_str().unwrap().to_string();
    world.lock().unwrap().posts =
        vec![(on_x_id.clone(), "Spring is here: sage, sand and slate!".into(), 1200, 87), ("native-1".into(), "Posted on the network itself".into(), 40, 2)];
    t.k.sync_connection(&t.admin, on_x, false).await.unwrap();
    t.work().await;
    let read_back = t.rows("SocialPublication", json!([["external_id", "=", on_x_id.clone()]])).await;
    assert_eq!((read_back.len(), read_back[0]["impressions"].as_i64(), read_back[0]["engagements"].as_i64()), (1, Some(1200), Some(87)), "{read_back:?}");
    assert_eq!(uid(&read_back[0]["post"]), id, "the same publication");
    let native = t.rows("SocialPublication", json!([["impressions", "=", 40]])).await;
    assert_eq!((native.len(), native[0]["post"].is_null()), (1, true), "a post made on the network is read back as one of no post of ours");

    // ───────────── the inbox ─────────────
    world.lock().unwrap().messages = vec![
        ("m-1".into(), "northwind".into(), "comment".into(), "pat".into(), "Is sage in stock?".into(), Some(on_x_id.clone())),
        ("m-2".into(), "somebody-else".into(), "mention".into(), "sam".into(), "Hello".into(), None),
        ("m-3".into(), "contoso".into(), "mention".into(), "kim".into(), "Hi both".into(), None),
    ];
    reader.lock().unwrap().messages = vec![("r-1".into(), "northwind-li".into(), "direct".into(), "lee".into(), "Can we talk?".into(), None)];
    t.k.sync_connection(&t.admin, on_x, false).await.unwrap();
    t.k.sync_connection(&t.admin, reading, false).await.unwrap();
    t.work().await;
    let comment = t.rows("SocialMessage", json!([["author", "=", "pat"]])).await.remove(0);
    assert_eq!((comment["kind"].as_str(), uid(&comment["account"]), comment["status"].as_str()), (Some("Comment"), nw_x, Some("New")));
    let stranger = t.rows("SocialMessage", json!([["author", "=", "sam"]])).await.remove(0);
    assert!(stranger["account"].is_null(), "a handle of no account of ours");
    let shared = t.rows("SocialMessage", json!([["author", "=", "kim"]])).await.remove(0);
    assert!(shared["account"].is_null(), "a handle two accounts share says nothing of which");
    let comment_id = uid(&comment["id"]);
    refused(t.k.update(&author, "SocialMessage", comment_id, obj(json!({"status": "Assigned"})), None).await, "invalid", "says to whom");
    t.k.update(&author, "SocialMessage", comment_id, obj(json!({"status": "Assigned", "assigned_to": "author@brands.test"})), None).await.unwrap();
    let reply = t.insert(&author, "SocialReply", json!({"message": comment_id.to_string(), "text": "Yes, in every size."})).await;
    assert_eq!((reply["in_reply_to"].as_str(), reply["author"].as_str()), (Some("m-1"), Some("author@brands.test")));
    assert_eq!(t.get("SocialMessage", comment_id).await["status"], "Answered");
    t.work().await;
    let replies: Vec<(Uuid, Value)> =
        world.lock().unwrap().sent.iter().filter(|(_, s, _)| s == "replies").map(|(c, _, f)| (*c, f["in_reply_to"].clone())).collect();
    assert_eq!(replies, vec![(on_x, json!("m-1"))], "back through the account's own connection, to the network's message");
    ferrum_app_social::maintenance_at(&k, Utc::now()).await.unwrap();
    assert!(t.get("SocialReply", uid(&reply["id"])).await["external_id"].as_str().is_some_and(|e| e.starts_with("replies-")));
    refused(t.k.insert(&author, "SocialReply", obj(json!({"message": stranger["id"], "text": "Hi"}))).await, "invalid", "came to no account of ours");
    let typed = t.insert(&t.admin, "SocialMessage", json!({"account_handle": "northwind", "author": "walk-in", "text": "Typed in by hand"})).await;
    assert_eq!(uid(&typed["account"]), nw_x);
    refused(t.k.insert(&author, "SocialReply", obj(json!({"message": typed["id"], "text": "Hi"}))).await, "invalid", "not read from the network");
    let direct = t.rows("SocialMessage", json!([["author", "=", "lee"]])).await.remove(0);
    assert_eq!(uid(&direct["account"]), nw_li);
    refused(
        t.k.insert(&author, "SocialReply", obj(json!({"message": direct["id"], "text": "Sure"}))).await,
        "invalid",
        "Northwind LinkedIn's connection cannot reply",
    );
    refused(t.k.insert(&manager, "SocialReply", obj(json!({"message": comment_id.to_string(), "text": "Also"}))).await, "forbidden", "");
}
