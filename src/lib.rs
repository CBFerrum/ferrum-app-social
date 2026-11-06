//! Social media management (FERRUM-ECOSYSTEM-005) on the platform's connections
//! (FERRUM-CONNECT-001): brands and their accounts, each reached through a connection an
//! administrator set up; posts approved by someone other than their author and sent at their time
//! to each account, within what its network takes and its provider offers; the inbox read and
//! answered; and what each post did.
//!
//! * `posts`: posts and their review, what each network takes, publications sent through their
//!   account's own connection, and their insights.
//! * `insights`: each publication's numbers kept day by day, and the report of what each day added.
//! * `inbox`: messages read through connections, routed to their account, and replies sent back.
//! * `mastodon`: a connector for any Mastodon server, for accounts there.
//! * `bluesky`: a connector for Bluesky (any AT Protocol server), for accounts there.
//!
//! This app's own design; no code was copied (docs/PROVENANCE.md). Hooks are registered per entity,
//! so a workspace without the `social` package never runs them.

pub mod bluesky;
mod inbox;
mod insights;
pub mod mastodon;
mod posts;

use chrono::{DateTime, Utc};
use ferrum_domain::reports::ReportSource;
use ferrum_kernel::{Ctx, Doc, Error, HookRegistry, Kernel, Result, Tx};
use serde_json::{Value, json};
use std::sync::Arc;
use uuid::Uuid;

include!(concat!(env!("OUT_DIR"), "/packages.rs"));
include!(concat!(env!("OUT_DIR"), "/ui.rs"));

/// This app's manifest (`app.yaml`).
pub const MANIFEST: &str = include_str!("../app.yaml");

/// This app's metadata packages, embedded at build time.
pub fn packages() -> std::result::Result<Vec<ferrum_meta::Package>, ferrum_meta::MetaError> {
    ferrum_meta::catalog::load_embedded(EMBEDDED)
}

/// Registers this app's document hooks, and its connectors for Mastodon and Bluesky.
pub fn register(hooks: &mut HookRegistry) {
    hooks
        .register("SocialAccount", Arc::new(posts::AccountHook))
        .register("SocialPost", Arc::new(posts::PostHook))
        .register("SocialPublication", Arc::new(insights::PublicationHook))
        .register("SocialMessage", Arc::new(inbox::MessageHook))
        .register("SocialReply", Arc::new(inbox::ReplyHook));
    hooks.add_connector(Arc::new(mastodon::Mastodon)).add_connector(Arc::new(bluesky::Bluesky));
}

/// This app's reports: what each day added to the publications' numbers.
pub fn reports() -> Vec<Arc<dyn ReportSource>> {
    vec![Arc::new(insights::SocialReports)]
}

/// Upkeep: approved posts sent when their time comes, sent publications marked out with the
/// network's id, and insights brought to their publications.
pub async fn maintenance(k: &Kernel) -> Result<()> {
    maintenance_at(k, Utc::now()).await
}

/// [`maintenance`] as of `now`.
pub async fn maintenance_at(k: &Kernel, now: DateTime<Utc>) -> Result<()> {
    let mut tx = k.db.system_tx().await?;
    let tenants: Vec<Uuid> = sqlx::query_scalar("select id from tenant where status = 'active' order by id").fetch_all(&mut *tx).await?;
    tx.commit().await?;
    let mut first_error: Option<Error> = None;
    for tenant in tenants {
        let run = async {
            let schema = k.registry.schema(tenant).await?;
            if !schema.packages.iter().any(|p| p == "social") {
                return Ok(());
            }
            let ctx = Ctx::system(tenant, schema);
            let mut tx = k.db.tenant_tx(tenant).await?;
            posts::send_due(k, &ctx, &mut tx, now).await?;
            posts::mark_sent(k, &ctx, &mut tx).await?;
            inbox::mark_replies(k, &ctx, &mut tx).await?;
            tx.commit().await?;
            Ok::<(), Error>(())
        };
        if let Err(e) = run.await {
            first_error.get_or_insert(e);
        }
    }
    first_error.map_or(Ok(()), Err)
}

/// A document from JSON.
pub(crate) fn obj(v: Value) -> Doc {
    v.as_object().cloned().unwrap_or_default()
}

/// A moment a record holds.
pub(crate) fn at(doc: &Doc, k: &str) -> Option<DateTime<Utc>> {
    doc.get(k).and_then(Value::as_str).and_then(|v| DateTime::parse_from_rfc3339(v).ok()).map(|d| d.with_timezone(&Utc))
}

/// The connection behind an account: its provider and whether it is on.
pub(crate) async fn connection(tx: &mut Tx, tenant: Uuid, id: Uuid) -> Result<Option<(String, bool)>> {
    Ok(sqlx::query_as("select provider, enabled from connection where tenant_id = $1 and id = $2").bind(tenant).bind(id).fetch_optional(&mut **tx).await?)
}

/// What a connection's provider says it can do (`publish_text`, `reply` ...).
/// The picture a post carries, as a connector sends it: (file name, content type, bytes, what it
/// shows), read from where it is stored; nothing for a post without one.
pub(crate) async fn picture(
    k: &Kernel,
    c: &ferrum_kernel::connect::Connection,
    change: &ferrum_kernel::connect::Outgoing,
) -> Result<Option<(String, String, Vec<u8>, String)>> {
    let Some(url) = change.fields.get("image").and_then(Value::as_str).filter(|u| !u.is_empty()) else { return Ok(None) };
    let file: Uuid = url.strip_prefix("/files/").and_then(|f| f.parse().ok()).ok_or_else(|| Error::invalid("a post's picture is a file stored here"))?;
    let ctx = Ctx::system(c.tenant, k.registry.schema(c.tenant).await?);
    let (name, ctype, bytes) = k.read_file(&ctx, file).await?;
    let alt = change.fields.get("image_alt").and_then(Value::as_str).unwrap_or_default().to_string();
    Ok(Some((name, ctype, bytes, alt)))
}

pub(crate) async fn capabilities(k: &Kernel, tenant: Uuid, provider: &str) -> Result<Vec<String>> {
    if let Some(c) = k.hooks.connectors().find(|c| c.info().provider == provider) {
        return Ok(c.info().capabilities);
    }
    match k.hooks.connector_source() {
        Some(source) => Ok(source.find(k, tenant, provider).await?.map(|c| c.info().capabilities).unwrap_or_default()),
        None => Ok(vec![]),
    }
}

/// Sends `record` of a stream through one connection, by the platform's own delivery: retried,
/// waited for when the provider says so, a dead letter in the end. The connection maps the stream
/// without pushing it by itself, so a record goes only through the connection it is for.
pub(crate) async fn deliver(tx: &mut Tx, tenant: Uuid, connection: Uuid, stream: &str, record: Uuid) -> Result<()> {
    let key = format!("{connection}:{record}:{stream}");
    let mut job = ferrum_kernel::jobs::NewJob::new(
        ferrum_kernel::connect::DELIVER_KIND,
        json!({"connection": connection, "stream": stream, "record": record, "key": key}),
    );
    job.max_attempts = 8;
    job.dedupe_key = Some(key);
    ferrum_kernel::jobs::enqueue(tx, tenant, None, job).await?;
    Ok(())
}

/// The network's id for `record` once its delivery went through.
pub(crate) async fn sent_as(tx: &mut Tx, tenant: Uuid, connection: Uuid, stream: &str, record: Uuid) -> Result<Option<String>> {
    Ok(sqlx::query_scalar("select external_id from connection_link where tenant_id = $1 and connection_id = $2 and stream = $3 and record_id = $4")
        .bind(tenant)
        .bind(connection)
        .bind(stream)
        .bind(record)
        .fetch_optional(&mut **tx)
        .await?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_interface_is_a_release_the_platform_takes() {
        let m = ferrum_apps::AppManifest::parse(MANIFEST).expect("app.yaml");
        assert_eq!(m.routes(), vec![("social".to_string(), vec![])]);
        let files = UI.iter().map(|(p, b)| (p.to_string(), b.to_vec())).collect();
        let ui = ferrum_apps::ui::prepare("social", files).expect("ui/dist is an interface release");
        assert!(UI.iter().any(|(p, _)| *p == "index.html") && !UI.iter().any(|(p, _)| p.starts_with('.')), "{}", ui.digest);
    }
}
