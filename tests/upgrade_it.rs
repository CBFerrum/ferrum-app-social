//! A workspace whose installed Social is the release before connections (fixtures/social_0_6_13), served by this
//! release's code, against a real PostgreSQL (`FERRUM_SOCIAL_UPGRADE_TEST_DATABASE_URL`, default
//! `postgres://ferrum:ferrum@localhost:5432/ferrum_social_upgrade_test`; reset first). Its accounts name a connection of
//! the old kind (a Mastodon one, a Bluesky one, and one of a network that never had a connector), it has no network
//! profiles and no settings, and posts are due: the upkeep must not fail, nothing is lost, each account of the two
//! networks that had connectors is given a connection, turned off, from the description the package ships (its server
//! kept, no secret copied), the administrators are told once, a post for an account that cannot be reached waits and
//! says why, and once an administrator updates the app (profiles seeded) the accounts are linked to their profiles and
//! the post goes when its account is connected.

use chrono::{Duration, Utc};
use ferrum_kernel::secret::SecretKey;
use ferrum_kernel::tenant::NewTenant;
use ferrum_kernel::{Db, DbConfig, Doc, HookRegistry, Kernel, KernelSettings, ListQuery};
use ferrum_meta::Catalog;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use uuid::Uuid;

fn obj(v: Value) -> Doc {
    v.as_object().cloned().unwrap()
}

async fn boot(db: Db, old: bool) -> std::sync::Arc<Kernel> {
    let mut packages = ferrum_domain::packages().unwrap();
    if old {
        let files: BTreeMap<String, String> = [("package.yaml", "package.yaml"), ("entities/social.yaml", "social.yaml")]
            .into_iter()
            .map(|(k, f)| (k.to_string(), std::fs::read_to_string(format!("{}/tests/fixtures/social_0_6_13/{f}", env!("CARGO_MANIFEST_DIR"))).unwrap()))
            .collect();
        packages.push(ferrum_meta::catalog::load_package_files(&files).unwrap());
    } else {
        packages.extend(ferrum_app_social::packages().unwrap());
    }
    let mut hooks = HookRegistry::default();
    ferrum_domain::register(&mut hooks);
    if !old {
        ferrum_app_social::register(&mut hooks);
    }
    let settings = KernelSettings { secret_key: Some(SecretKey::new([7u8; 32])), connection_hosts: vec!["127.0.0.1".into()], ..Default::default() };
    Kernel::boot(db, Catalog::from_packages(packages).unwrap(), hooks, settings, None).await.unwrap()
}

#[test]
fn an_older_workspace_keeps_working() {
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().thread_stack_size(16 << 20).build().unwrap();
    rt.block_on(async { tokio::spawn(scenario()).await.unwrap() });
}

async fn scenario() {
    let url = std::env::var("FERRUM_SOCIAL_UPGRADE_TEST_DATABASE_URL")
        .unwrap_or_else(|_| "postgres://ferrum:ferrum@localhost:5432/ferrum_social_upgrade_test".into());
    let db = Db::connect(DbConfig { url: url.clone(), max_connections: 8, ..Default::default() }).await.unwrap();
    sqlx::raw_sql("drop schema if exists public cascade; create schema public;").execute(&db.pool).await.unwrap();
    // The workspace as the old release left it: no hooks of this release, so records go in as they did.
    let k = boot(db, true).await;
    let tenant = k
        .create_tenant(NewTenant {
            slug: "old".into(),
            name: "Old".into(),
            admin_email: "admin@old.test".into(),
            admin_password: "Correct-Horse-9".into(),
            admin_name: None,
            packages: vec!["social".into()],
            settings: Some(json!({"timezone": "UTC"})),
        })
        .await
        .unwrap();
    let s = k.login("old", "admin@old.test", "Correct-Horse-9").await.unwrap();
    let admin = k.load_ctx(tenant, s.user, Some(s.token_version), "it".into()).await.unwrap();
    let brand = k.insert(&admin, "SocialBrand", obj(json!({"brand_name": "Northwind"}))).await.unwrap();
    // The old connections of the old hub: what the old connectors kept, the server among it.
    let (old_mastodon, old_bluesky, old_x) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
    {
        let mut tx = k.db.tenant_tx(tenant).await.unwrap();
        for (id, provider, config) in
            [(old_mastodon, "mastodon", json!({"instance": "https://fosstodon.example/"})), (old_bluesky, "bluesky", json!({"service": "https://pds.example"}))]
        {
            sqlx::query("insert into connection (tenant_id, id, provider, name, config, sealed) values ($1, $2, $3, $3, $4, 'sealed-token')")
                .bind(tenant)
                .bind(id)
                .bind(provider)
                .bind(config)
                .execute(&mut *tx)
                .await
                .unwrap();
        }
        tx.commit().await.unwrap();
    }
    let mut accounts = Vec::new();
    for (name, network, handle, connection) in [
        ("Northwind Mastodon", "Mastodon", "northwind", old_mastodon),
        ("Northwind Bluesky", "Bluesky", "northwind.bsky.social", old_bluesky),
        ("Northwind X", "X", "northwind", old_x),
    ] {
        let account = k
            .insert(
                &admin,
                "SocialAccount",
                obj(json!({"account_name": name, "brand": brand["id"], "network": network, "handle": handle, "connection": connection.to_string()})),
            )
            .await
            .unwrap();
        accounts.push(account);
    }
    let at = Utc::now() - Duration::hours(1);
    let mut posts = Vec::new();
    for (head, account) in [("Old post", &accounts[0]), ("Old X post", &accounts[2])] {
        let post = k
            .insert(
                &admin,
                "SocialPost",
                obj(json!({"headline": head, "brand": brand["id"], "text": "Hello", "scheduled_at": at.to_rfc3339(), "targets": [{"account": account["id"]}]})),
            )
            .await
            .unwrap();
        posts.push(post);
    }
    // Approved, as an old workspace's posts waiting for their time would be.
    let mut tx = k.db.tenant_tx(tenant).await.unwrap();
    sqlx::query("update e_social_post set status = 'Approved' where tenant_id = $1").bind(tenant).execute(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    let message = k.insert(&admin, "SocialMessage", obj(json!({"account_handle": "northwind", "author": "bo", "text": "Hi there"}))).await.unwrap();
    let account = accounts[0].clone();
    let post = posts[0].clone();
    drop(admin);
    drop(k);

    // This release's code on the same data, the installed package still the old one.
    let db = Db::connect(DbConfig { url, max_connections: 8, ..Default::default() }).await.unwrap();
    let k = boot(db, false).await;
    let admin = k.load_ctx(tenant, s.user, None, "it".into()).await.unwrap();
    let rows = |entity: &'static str| {
        let (k, admin) = (k.clone(), admin.clone());
        async move { k.list(&admin, entity, &ListQuery { limit: Some(50), ..Default::default() }).await.unwrap().rows }
    };
    assert!(rows("SocialProfile").await.is_empty(), "profiles arrive when an administrator updates the app");
    let work = || {
        let k = k.clone();
        async move {
            loop {
                let mut tx = k.db.system_tx().await.unwrap();
                sqlx::query("update job set run_at = now() where status = 'queued'").execute(&mut *tx).await.unwrap();
                tx.commit().await.unwrap();
                if k.run_jobs_once(4).await.unwrap() == 0 {
                    break;
                }
            }
        }
    };
    ferrum_app_social::maintenance_at(&k, Utc::now()).await.expect("the upkeep copes");
    work().await;
    ferrum_app_social::maintenance_at(&k, Utc::now()).await.expect("and again");
    let get = |id: &Value| {
        let (k, admin, id) = (k.clone(), admin.clone(), id.as_str().unwrap().parse().unwrap());
        async move { k.get(&admin, "SocialAccount", id, false).await.unwrap() }
    };
    let defs = k.connection_defs(&admin).await.unwrap();
    let made = |name: &str| defs.iter().find(|c| c["name"] == name && c["id"].is_string()).unwrap_or_else(|| panic!("no connection {name}: {defs:?}")).clone();
    let (mastodon, bluesky) = (made("mastodon_northwind"), made("bluesky_northwind_bsky_social"));
    for c in [&mastodon, &bluesky] {
        assert_eq!((c["enabled"].as_bool(), c["secrets"].as_array().map(Vec::len)), (Some(false), Some(0)), "turned off, and no token or password copied: {c}");
        assert_eq!(c["description"]["apps"], json!(["social"]));
    }
    assert_eq!(mastodon["description"]["base"], "https://fosstodon.example", "the server the old connection spoke to, kept");
    assert_eq!(mastodon["description"]["auth"]["provider"]["token_url"], "https://fosstodon.example/oauth/token");
    assert_eq!(bluesky["description"]["base"], "https://pds.example/xrpc");
    assert!(bluesky["description"]["operations"]["create_session"].is_object(), "from the description the package ships");
    let a = get(&account["id"]).await;
    assert_eq!(a["connection"], mastodon["id"], "linked to it");
    assert_eq!(
        (a["state"].as_str(), a["handle"].as_str(), a["network"].as_str()),
        (Some("Connection Off"), Some("northwind"), Some("Mastodon")),
        "the account stays, saying what it lacks"
    );
    assert!(a["state_note"].as_str().unwrap().contains("mastodon_northwind is turned off"), "{a:?}");
    assert_eq!(get(&accounts[1]["id"]).await["connection"], bluesky["id"]);
    let x = get(&accounts[2]["id"]).await;
    assert_eq!(
        (x["connection"].as_str(), x["state"].as_str()),
        (Some(old_x.to_string().as_str()), Some("Not Connected")),
        "a network that never had a connector is left as it is"
    );
    assert_eq!(defs.iter().filter(|c| c["id"].is_string()).count(), 2, "and none is made for it");
    // The administrators were told once, in the app, which accounts and where.
    let told = "select count(*) from notification where tenant_id = $1 and title like '2 Social accounts to connect again'";
    let count = |sql: &'static str| {
        let k = k.clone();
        async move {
            let mut tx = k.db.tenant_tx(tenant).await.unwrap();
            let n: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(sql.to_string())).bind(tenant).fetch_one(&mut *tx).await.unwrap();
            tx.commit().await.unwrap();
            n
        }
    };
    assert_eq!(count(told).await, 1, "one for the one administrator, however often the upkeep ran");
    let text: String = {
        let mut tx = k.db.tenant_tx(tenant).await.unwrap();
        let t = sqlx::query_scalar("select body from notification where tenant_id = $1 and title like '2 Social accounts%'")
            .bind(tenant)
            .fetch_one(&mut *tx)
            .await
            .unwrap();
        tx.commit().await.unwrap();
        t
    };
    assert!(text.contains("Northwind Mastodon: in Administration, Connections, add the secrets of Mastodon (Northwind Mastodon)"), "{text}");
    assert!(text.contains("Northwind Bluesky"), "{text}");
    // A post for an account that cannot be reached waits, saying so; it does not fail, and nothing is lost.
    for post in &posts {
        let p = k.get(&admin, "SocialPost", post["id"].as_str().unwrap().parse().unwrap(), false).await.unwrap();
        assert_eq!(p["status"], "Approved", "waiting, not failed: {p:?}");
        assert!(p["waiting_for"].as_str().is_some_and(|w| w.contains("Northwind") && w.contains(": ")), "{p:?}");
    }
    let p = k.get(&admin, "SocialPost", post["id"].as_str().unwrap().parse().unwrap(), false).await.unwrap();
    assert!(p["waiting_for"].as_str().unwrap().contains("Northwind Mastodon: Connection Off"), "{p:?}");
    assert_eq!(rows("SocialMessage").await.len(), 1);
    assert_eq!(rows("SocialMessage").await[0]["id"], message["id"]);

    // The administrator turns the connection on (here a test connection answering from a fixture stands for the
    // network): the post goes, once.
    let kept = made("mastodon_northwind");
    let on = json!({"name": "mastodon_northwind", "label": "Mastodon", "enabled": true, "description": {
        "test": true, "apps": ["social"], "meta": kept["description"]["meta"],
        "operations": {"publish": {"method": "POST", "path": "/api/v1/statuses", "body": "{\"status\": {{ inputs.text_with_link | json }}}", "result": {"id": "/id"},
                                   "inputs": ["text_with_link"], "fixture": {"status": 200, "body": {"id": "s1"}}}}}});
    k.save_connection_def(&admin, Some(kept["id"].as_str().unwrap().parse().unwrap()), &serde_json::from_value(on).unwrap()).await.unwrap();
    ferrum_app_social::maintenance_at(&k, Utc::now()).await.unwrap();
    work().await;
    ferrum_app_social::maintenance_at(&k, Utc::now()).await.unwrap();
    let p = k.get(&admin, "SocialPost", post["id"].as_str().unwrap().parse().unwrap(), false).await.unwrap();
    assert_eq!((p["status"].as_str(), p["waiting_for"].is_null()), (Some("Published"), true), "{p:?}");
    let x_post = k.get(&admin, "SocialPost", posts[1]["id"].as_str().unwrap().parse().unwrap(), false).await.unwrap();
    assert_eq!(x_post["status"], "Approved", "the one whose account is still not connected keeps waiting");
    assert_eq!(count(told).await, 1, "and nobody is told again");

    // The administrator updates the app: profiles are seeded, and the upkeep links the account to its own.
    k.install_packages(tenant, &["social".to_string()]).await.unwrap();
    assert!(rows("SocialProfile").await.iter().any(|p| p["profile_name"] == "Mastodon"));
    let admin = k.load_ctx(tenant, s.user, None, "it".into()).await.unwrap();
    ferrum_app_social::maintenance_at(&k, Utc::now()).await.unwrap();
    for (account, name) in [(&accounts[0], "Mastodon"), (&accounts[2], "X")] {
        let a = k.get(&admin, "SocialAccount", account["id"].as_str().unwrap().parse().unwrap(), false).await.unwrap();
        let profile = rows("SocialProfile").await.into_iter().find(|p| p["profile_name"] == name).unwrap();
        assert_eq!(a["profile"], profile["id"], "carried over by name, with no one asked");
    }
    // Run again: nothing changes, and nobody is told again.
    ferrum_app_social::maintenance_at(&k, Utc::now()).await.unwrap();
    assert_eq!(count(told).await, 1);
}
