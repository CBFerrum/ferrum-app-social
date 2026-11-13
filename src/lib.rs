//! Social media management (FERRUM-ECOSYSTEM-005) on the platform's connections (docs/api.md, "Connections"):
//! brands and their accounts, each an account on a connection (the workspace's sign-in at a provider, or a person's
//! own, through the platform's broker); posts approved by someone other than their author and sent at their time
//! to each account, within what its network profile says it takes; the inbox read and answered; and what each post
//! and account did.
//!
//! No network is named here. A network is a connection's description, whose operations are named by what Social
//! needs of them (`publish`, `publish_media`, `delete`, `comments`, `reply`, `messages`, `send_message`, `metrics`,
//! `account`), and a network profile record saying what a post may carry (examples: docs/connectors/).
//!
//! * `accounts`: accounts, their state on the connection, and the profile each names.
//! * `rules`: what a network takes and asks (`meta.social`, or a network profile), and the one check of a copy against it.
//! * `content`: a post's original and per-account versions, items and media, as one account's copy.
//! * `posts`: posts, their validation, and each account's publication, sent item by item by a queued job, once.
//! * `review`: review by the platform's approval policy, and review links for people outside.
//! * `when`: publish now, move, back to draft, duplicate, the queue of posting times, repeats.
//! * `library`: templates, sets, snippets, signatures, variables.
//! * `picker`: what a sign-in reaches, picked as accounts; an account's rules, mentions and a setting's choices.
//! * `inbox`: messages arriving through a connection's inbound address or pulls, and replies sent back.
//! * `upgrade`: accounts of the networks the earlier releases had connectors for, given a connection from the shipped
//!   description (turned off), and the administrators told.
//! * `metrics`: numbers read for each account and post, by day.
//! * `insights`: each publication's numbers kept day by day, and the reports of them.
//!
//! This app's own design; no code was copied (docs/PROVENANCE.md). Hooks are registered per entity,
//! so a workspace without the `social` package never runs them.

mod accounts;
mod content;
mod inbox;
mod insights;
mod library;
mod links;
mod media;
mod metrics;
mod outside;
mod picker;
mod plugs;
mod posts;
mod review;
mod rules;
mod upgrade;
mod when;
mod writing;

use chrono::{DateTime, Utc};
use ferrum_domain::reports::ReportSource;
pub(crate) use ferrum_kernel::Doc;
use ferrum_kernel::{Ctx, Error, HookRegistry, Kernel, Result};
use serde_json::Value;
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

/// The name this app calls connections by: a connection lists it under `apps`.
pub(crate) const APP: &str = "social";

/// Registers this app's document hooks and the jobs that send posts and replies.
pub fn register(hooks: &mut HookRegistry) {
    hooks
        .register("SocialAccount", Arc::new(accounts::AccountHook))
        .register("SocialPost", Arc::new(posts::PostHook))
        .register("SocialPublication", Arc::new(posts::PublicationHook))
        .register("SocialPublication", Arc::new(insights::InsightHook))
        .register("SocialMessage", Arc::new(inbox::MessageHook))
        .register("SocialReply", Arc::new(inbox::ReplyHook))
        .register("SocialPostingTime", Arc::new(when::PostingTimeHook))
        .register("SocialMediaAsset", Arc::new(media::AssetHook))
        .register("SocialBrand", Arc::new(BrandHook));
    for library in ["SocialTemplate", "SocialSet", "SocialSignature", "SocialVariable", "SocialLinkTemplate"] {
        hooks.register(library, Arc::new(library::LibraryHook));
    }
    hooks.add_job_handler(Arc::new(posts::PublishJob)).add_job_handler(Arc::new(inbox::ReplyJob));
}

/// This app's reports: what each day added to the publications' numbers, when to post, and the numbers by
/// account, by post and by tag.
pub fn reports() -> Vec<Arc<dyn ReportSource>> {
    vec![Arc::new(insights::SocialReports), Arc::new(metrics::MetricReports)]
}

/// Upkeep: accounts carried over and their states, approved posts queued when their time comes, posts settled, and
/// the numbers read. It asks for nothing an administrator's update of the app brings (network profiles are seeded
/// then): a workspace not yet updated keeps working as far as it can, and what cannot be sent says why.
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
            upgrade::carry_connections(k, &ctx, &mut tx).await?;
            accounts::carry_over(k, &ctx, &mut tx).await?;
            accounts::refresh_all(k, &ctx, &mut tx).await?;
            review::settle(k, &ctx, &mut tx).await?;
            posts::send_due(k, &ctx, &mut tx, now).await?;
            posts::mark_sent(k, &ctx, &mut tx, now).await?;
            metrics::brand_numbers(k, &ctx, &mut tx, now.with_timezone(&when::zone(&ctx)).date_naive()).await?;
            tx.commit().await?;
            metrics::pull(k, tenant, now).await?;
            links::clicks(k, tenant, now).await?;
            plugs::run(k, tenant, now).await?;
            Ok::<(), Error>(())
        };
        if let Err(e) = run.await {
            first_error.get_or_insert(e);
        }
    }
    first_error.map_or(Ok(()), Err)
}

/// The brand's actions: its media library's ways in, and writing help from a blank start.
pub struct BrandHook;

#[async_trait::async_trait]
impl ferrum_kernel::DocHook for BrandHook {
    fn name(&self) -> &'static str {
        "social.brand"
    }

    async fn action(&self, hc: &mut ferrum_kernel::HookCtx<'_>, name: &str, doc: &Doc, params: &Value) -> Result<Option<Value>> {
        if let Some(out) = media::action(hc, name, doc, params).await? {
            return Ok(Some(out));
        }
        writing::action(hc, "SocialBrand", name, doc, params).await
    }
}

/// A document from JSON.
pub(crate) fn obj(v: Value) -> Doc {
    v.as_object().cloned().unwrap_or_default()
}

/// A moment a record holds.
pub(crate) fn at(doc: &Doc, k: &str) -> Option<DateTime<Utc>> {
    doc.get(k).and_then(Value::as_str).and_then(|v| DateTime::parse_from_rfc3339(v).ok()).map(|d| d.with_timezone(&Utc))
}

/// A connection's refusal in words that say what to do about the commonest one: a connection answers an app only
/// when it lists the app.
pub(crate) fn said(e: Error) -> Error {
    match e {
        Error::Forbidden(m) if m.contains("may not use this connection") => {
            Error::invalid(format!("the connection does not list {APP} under apps, so Social may not use it: an administrator adds it ({m})"))
        }
        other => other,
    }
}

/// Bytes as base64 (standard alphabet, padded), for a network that takes a picture inside JSON.
pub(crate) fn base64(bytes: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for c in bytes.chunks(3) {
        let n = (u32::from(c[0]) << 16) | (u32::from(*c.get(1).unwrap_or(&0)) << 8) | u32::from(*c.get(2).unwrap_or(&0));
        for i in 0..4 {
            out.push(if i <= c.len() { A[((n >> (18 - 6 * i)) & 63) as usize] as char } else { '=' });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_as_base64() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn the_interface_is_a_release_the_platform_takes() {
        let m = ferrum_apps::AppManifest::parse(MANIFEST).expect("app.yaml");
        assert_eq!(m.routes(), vec![("social".to_string(), vec![])]);
        let files = UI.iter().map(|(p, b)| (p.to_string(), b.to_vec())).collect();
        let ui = ferrum_apps::ui::prepare("social", files).expect("ui/dist is an interface release");
        assert!(UI.iter().any(|(p, _)| *p == "index.html") && !UI.iter().any(|(p, _)| p.starts_with('.')), "{}", ui.digest);
    }
}
