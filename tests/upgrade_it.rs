//! A workspace whose installed Social is the release before connections (fixtures/social_0_6_13), served by this
//! release's code, against a real PostgreSQL (`FERRUM_SOCIAL_UPGRADE_TEST_DATABASE_URL`, default
//! `postgres://ferrum:ferrum@localhost:5432/ferrum_social_upgrade_test`; reset first). Its accounts name a connection of
//! the old kind, it has no network profiles and no settings, and a post is due: the upkeep must not fail, nothing is
//! lost, what cannot go says why, and once an administrator updates the app (profiles seeded) the accounts are linked
//! to their profiles by the upkeep and can be connected.

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
    let legacy = Uuid::new_v4();
    let account = k
        .insert(
            &admin,
            "SocialAccount",
            obj(json!({"account_name": "Northwind X", "brand": brand["id"], "network": "X", "handle": "northwind", "connection": legacy.to_string()})),
        )
        .await
        .unwrap();
    let at = Utc::now() - Duration::hours(1);
    let post = k
        .insert(
            &admin,
            "SocialPost",
            obj(json!({"headline": "Old post", "brand": brand["id"], "text": "Hello", "scheduled_at": at.to_rfc3339(),
                       "targets": [{"account": account["id"]}]})),
        )
        .await
        .unwrap();
    // Approved, as an old workspace's post waiting for its time would be.
    let mut tx = k.db.tenant_tx(tenant).await.unwrap();
    sqlx::query("update e_social_post set status = 'Approved' where tenant_id = $1").bind(tenant).execute(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    let message = k.insert(&admin, "SocialMessage", obj(json!({"account_handle": "northwind", "author": "bo", "text": "Hi there"}))).await.unwrap();
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
    let a = k.get(&admin, "SocialAccount", account["id"].as_str().unwrap().parse().unwrap(), false).await.unwrap();
    assert_eq!(
        (a["state"].as_str(), a["handle"].as_str(), a["network"].as_str()),
        (Some("Not Connected"), Some("northwind"), Some("X")),
        "the account stays, saying what it lacks"
    );
    assert!(a["state_note"].as_str().unwrap().contains("connect this account again"), "{a:?}");
    let p = k.get(&admin, "SocialPost", post["id"].as_str().unwrap().parse().unwrap(), false).await.unwrap();
    assert_eq!(p["status"], "Failed", "the post is not lost or stuck: it says why");
    assert!(p["failure"].as_str().unwrap().contains("Northwind X") && p["failure"].as_str().unwrap().contains("connect"), "{p:?}");
    assert_eq!(rows("SocialMessage").await.len(), 1);
    assert_eq!(rows("SocialMessage").await[0]["id"], message["id"]);

    // The administrator updates the app: profiles are seeded, and the upkeep links the account to its own.
    k.install_packages(tenant, &["social".to_string()]).await.unwrap();
    assert!(rows("SocialProfile").await.iter().any(|p| p["profile_name"] == "X"));
    let admin = k.load_ctx(tenant, s.user, None, "it".into()).await.unwrap();
    ferrum_app_social::maintenance_at(&k, Utc::now()).await.unwrap();
    let a = k.get(&admin, "SocialAccount", account["id"].as_str().unwrap().parse().unwrap(), false).await.unwrap();
    let x = rows("SocialProfile").await.into_iter().find(|p| p["profile_name"] == "X").unwrap();
    assert_eq!(a["profile"], x["id"], "carried over by name, with no one asked");
    // Run again: nothing changes.
    ferrum_app_social::maintenance_at(&k, Utc::now()).await.unwrap();
    assert_eq!(k.get(&admin, "SocialAccount", a["id"].as_str().unwrap().parse().unwrap(), false).await.unwrap()["profile"], x["id"]);
}
