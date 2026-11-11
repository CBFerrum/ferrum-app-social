//! Accounts, posts and publications. A post goes from its brand to one or more of the brand's
//! active accounts. It must fit every network it goes to, and each account's connection must be
//! able to publish. Someone other than its author approves it; one sent back says why, and a post
//! changed after its approval needs approving again. At its time, upkeep makes one publication per
//! account and sends it through that account's own connection, by the platform's delivery. A
//! publication is out once the network has given its id, and the post is published when all of
//! them are. What each did is read back through the connection with the network's posts.

use crate::{at, capabilities, connection, deliver, delivery, obj, sent_as};
use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use ferrum_domain::util::{elevated, id, rows, s};
use ferrum_kernel::{Ctx, Doc, DocHook, Error, Filter, HookCtx, Kernel, Result, Tx};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uuid::Uuid;

/// A publication whose delivery has left no trace in the job list, and that is this old, is given
/// up on: the list is cleaned after a while, and a send takes far less than this.
const STALE: Duration = Duration::hours(24);

/// The stream a connection sends posts on, and reads them back from.
pub(crate) const POSTS: &str = "posts";

/// What a network takes in one post: its length, and whether it takes text alone.
pub(crate) fn takes(network: &str) -> (usize, bool) {
    match network {
        "X" => (280, true),
        "Bluesky" => (300, true),
        "Mastodon" | "Threads" => (500, true),
        "LinkedIn" => (3000, true),
        "Facebook" => (63206, true),
        // Instagram takes a post with its picture; YouTube publishes video.
        _ => (2200, false),
    }
}

/// Why a post of `text` and `link`, with a picture or not, does not fit `network`, when it does not.
pub(crate) fn misfit(network: &str, text: &str, link: Option<&str>, image: bool) -> Option<String> {
    let (max, text_alone) = takes(network);
    if network == "YouTube" {
        return Some("YouTube takes video, which posts here do not carry".into());
    }
    if !text_alone && !image {
        return Some(format!("{network} takes no post without an image or a video"));
    }
    // X counts a link as 23 characters, whatever its length, after a space.
    let length = text.chars().count() + link.map_or(0, |l| if network == "X" { 24 } else { l.chars().count() + 1 });
    (length > max).then(|| format!("{network} takes {max} characters, and this post is {length}"))
}

/// What approval fixes: the words, the link, the picture and what it says of it, where and when it
/// goes.
fn fingerprint(doc: &Doc) -> String {
    let mut targets: Vec<String> = rows(doc, "targets").iter().filter_map(|t| s(t, "account").map(str::to_string)).collect();
    targets.sort();
    let what = json!([s(doc, "text"), s(doc, "link"), targets, at(doc, "scheduled_at").map(|t| t.timestamp()), s(doc, "image"), s(doc, "image_alt")]);
    hex::encode(Sha256::digest(what.to_string()))
}

pub struct AccountHook;

#[async_trait]
impl DocHook for AccountHook {
    fn name(&self) -> &'static str {
        "social.account"
    }

    async fn before_validate(&self, hc: &mut HookCtx<'_>, doc: &mut Doc) -> Result<()> {
        if let Some(handle) = s(doc, "handle").map(|h| h.trim().trim_start_matches('@').to_string()) {
            doc.insert("handle".into(), json!(handle));
        }
        // The connection by its name, as Administration, Connections shows it, or by its id. A
        // disconnected one (its credentials forgotten) is not one of them.
        if let Some(given) = s(doc, "connection").map(|c| c.trim().to_string()).filter(|c| !c.is_empty() && c.parse::<Uuid>().is_err()) {
            let named: Vec<Uuid> =
                sqlx::query_scalar("select id from connection where tenant_id = $1 and lower(name) = lower($2) and sealed is not null limit 2")
                    .bind(hc.ctx.tenant)
                    .bind(&given)
                    .fetch_all(&mut **hc.tx)
                    .await?;
            match named.as_slice() {
                [one] => {
                    doc.insert("connection".into(), json!(one.to_string()));
                }
                [] => return Err(Error::field("connection", format!("no connection is named {given}"))),
                _ => return Err(Error::field("connection", format!("more than one connection is named {given}: give its id"))),
            }
        }
        Ok(())
    }

    async fn validate(&self, hc: &mut HookCtx<'_>, doc: &Doc) -> Result<()> {
        let conn = s(doc, "connection").and_then(|c| c.trim().parse().ok()).ok_or_else(|| Error::field("connection", "is a connection's id"))?;
        if connection(hc.tx, hc.ctx.tenant, conn).await?.is_none() {
            return Err(Error::field("connection", "no connection by that id in this workspace"));
        }
        Ok(())
    }
}

pub struct PostHook;

#[async_trait]
impl DocHook for PostHook {
    fn name(&self) -> &'static str {
        "social.post"
    }

    fn fills(&self) -> &'static [&'static str] {
        &["author", "status", "approved_by", "approved_hash"]
    }

    async fn before_validate(&self, hc: &mut HookCtx<'_>, doc: &mut Doc) -> Result<()> {
        if s(doc, "author").is_none() && !hc.ctx.email.is_empty() {
            doc.insert("author".into(), json!(hc.ctx.email.to_lowercase()));
        }
        // An approved post that changed needs approving again.
        if s(doc, "status") == Some("Approved") && s(doc, "approved_hash") != Some(fingerprint(doc).as_str()) {
            doc.insert("status".into(), json!("Draft"));
            doc.insert("approved_by".into(), Value::Null);
            doc.insert("approved_hash".into(), Value::Null);
        }
        Ok(())
    }

    async fn validate(&self, hc: &mut HookCtx<'_>, doc: &Doc) -> Result<()> {
        let system = elevated(hc.ctx);
        let brand = id(doc, "brand");
        let text = s(doc, "text").unwrap_or_default();
        let link = s(doc, "link").filter(|l| !l.trim().is_empty());
        if link.is_some_and(|l| !(l.starts_with("https://") || l.starts_with("http://")) || l.contains(char::is_whitespace)) {
            return Err(Error::field("link", "is a web address"));
        }
        let image = s(doc, "image").filter(|i| !i.trim().is_empty());
        if image.is_some_and(|i| !i.starts_with("/files/")) {
            return Err(Error::field("image", "is a picture stored here"));
        }
        if image.is_some() && s(doc, "image_alt").is_none_or(|a| a.trim().is_empty()) {
            return Err(Error::field("image_alt", "says what the picture shows, for people who cannot see it"));
        }
        for target in rows(doc, "targets") {
            let Some(account) = id(&target, "account") else { continue };
            let a = hc.kernel.get_tx(&system, hc.tx, "SocialAccount", account, false).await?;
            let name = s(&a, "account_name").unwrap_or_default();
            if id(&a, "brand") != brand {
                return Err(Error::field("targets", format!("{name} is another brand's")));
            }
            if !a.get("active").and_then(Value::as_bool).unwrap_or(true) {
                return Err(Error::field("targets", format!("{name} is paused")));
            }
            if let Some(why) = misfit(s(&a, "network").unwrap_or_default(), text, link, image.is_some()) {
                return Err(Error::field("text", format!("for {name}: {why}")));
            }
            let conn = s(&a, "connection").and_then(|c| c.parse().ok()).unwrap_or_default();
            let provider = connection(hc.tx, hc.ctx.tenant, conn).await?.map(|(p, _)| p).unwrap_or_default();
            let offers = capabilities(hc.kernel, hc.ctx.tenant, &provider).await?;
            if !offers.iter().any(|c| c == "publish_text") {
                return Err(Error::field("targets", format!("{name}'s connection cannot publish posts")));
            }
            if image.is_some() && !offers.iter().any(|c| c == "publish_image") {
                return Err(Error::field("image", format!("{name}'s connection cannot publish pictures")));
            }
        }
        Ok(())
    }

    async fn validate_change(&self, _hc: &mut HookCtx<'_>, old: Option<&Doc>, doc: &Doc) -> Result<()> {
        if old.is_some_and(|o| matches!(s(o, "status"), Some("Sending" | "Published" | "Failed")) && fingerprint(o) != fingerprint(doc)) {
            return Err(Error::invalid("a post stays as it went out"));
        }
        Ok(())
    }

    async fn action(&self, hc: &mut HookCtx<'_>, name: &str, doc: &Doc, params: &Value) -> Result<Option<Value>> {
        let post = id(doc, "id").ok_or_else(|| Error::internal("a post without an id"))?;
        let approver = hc.ctx.is_admin || hc.ctx.roles.iter().any(|r| r == "Social Approver");
        let values = match name {
            "send_for_review" => json!({"status": "In Review", "review_note": null}),
            "approve" => {
                if !approver {
                    return Err(Error::Forbidden("a Social Approver approves posts".into()));
                }
                if s(doc, "author").is_some_and(|a| a.eq_ignore_ascii_case(&hc.ctx.email)) {
                    return Err(Error::Forbidden("a post is approved by someone other than its author".into()));
                }
                json!({"status": "Approved", "approved_by": hc.ctx.email, "approved_hash": fingerprint(doc)})
            }
            "reject" => {
                if !approver {
                    return Err(Error::Forbidden("a Social Approver sends posts back".into()));
                }
                let note = params.get("note").and_then(Value::as_str).map(str::trim).filter(|n| !n.is_empty());
                let note = note.ok_or_else(|| Error::field("note", "sending a post back says why"))?;
                json!({"status": "Rejected", "review_note": note})
            }
            "retry" => {
                // Only to the accounts it did not reach: the others have it already.
                let system = elevated(hc.ctx);
                let own = [Filter::eq("post", post.to_string())];
                for p in hc.kernel.find_tx(&system, hc.tx, "SocialPublication", &own, None, 100).await? {
                    let (Some(publication), true) = (id(&p, "id"), s(&p, "external_id").is_none()) else { continue };
                    let account = hc.kernel.get_tx(&system, hc.tx, "SocialAccount", id(&p, "account").unwrap_or_default(), false).await?;
                    let Some(conn) = s(&account, "connection").and_then(|c| c.parse().ok()) else { continue };
                    hc.kernel.set_values_tx(&system, hc.tx, "SocialPublication", publication, obj(json!({"failed_at": null, "failure": null}))).await?;
                    deliver(hc.tx, hc.ctx.tenant, conn, POSTS, publication).await?;
                }
                json!({"status": "Sending", "failure": null})
            }
            _ => return Ok(None),
        };
        hc.kernel.set_values_tx(&elevated(hc.ctx), hc.tx, "SocialPost", post, obj(values.clone())).await?;
        Ok(Some(json!({"status": values["status"]})))
    }
}

/// Approved posts whose time has come: one publication per account, each sent through that
/// account's connection.
pub(crate) async fn send_due(k: &Kernel, ctx: &Ctx, tx: &mut Tx, now: DateTime<Utc>) -> Result<()> {
    for found in k.find_tx(ctx, tx, "SocialPost", &[Filter::eq("status", "Approved")], Some("scheduled_at asc"), 500).await? {
        if at(&found, "scheduled_at").is_none_or(|t| t > now) {
            continue;
        }
        let post = id(&found, "id").unwrap_or_default();
        let doc = k.get_tx(ctx, tx, "SocialPost", post, false).await?;
        for target in rows(&doc, "targets") {
            let Some(account) = id(&target, "account") else { continue };
            let a = k.get_tx(ctx, tx, "SocialAccount", account, false).await?;
            let Some(conn) = s(&a, "connection").and_then(|c| c.parse().ok()) else { continue };
            let made = k
                .insert_tx(
                    ctx,
                    tx,
                    "SocialPublication",
                    obj(json!({"post": post.to_string(), "account": account.to_string(), "text": s(&doc, "text"), "link": s(&doc, "link"),
                               "image": s(&doc, "image"), "image_alt": s(&doc, "image_alt")})),
                )
                .await?;
            deliver(tx, ctx.tenant, conn, POSTS, id(&made, "id").unwrap_or_default()).await?;
        }
        k.set_values_tx(ctx, tx, "SocialPost", post, obj(json!({"status": "Sending"}))).await?;
    }
    Ok(())
}

/// Publications the network has given its id to, marked out; one whose delivery ended without,
/// marked failed with the reason (a send that left no trace for a day is given up on too); and
/// posts all of whose are settled: published, or failed when any publication is.
pub(crate) async fn mark_sent(k: &Kernel, ctx: &Ctx, tx: &mut Tx, now: DateTime<Utc>) -> Result<()> {
    for found in k.find_tx(ctx, tx, "SocialPost", &[Filter::eq("status", "Sending")], None, 500).await? {
        let post = id(&found, "id").unwrap_or_default();
        let mut pending = false;
        let mut failures = Vec::new();
        for p in k.find_tx(ctx, tx, "SocialPublication", &[Filter::eq("post", post.to_string())], None, 100).await? {
            let publication = id(&p, "id").unwrap_or_default();
            let account = k.get_tx(ctx, tx, "SocialAccount", id(&p, "account").unwrap_or_default(), false).await?;
            let name = s(&account, "account_name").unwrap_or("an account");
            if s(&p, "external_id").is_some() {
                continue;
            }
            if at(&p, "failed_at").is_some() {
                failures.push(format!("{name}: {}", s(&p, "failure").unwrap_or("it did not go out")));
                continue;
            }
            let conn = s(&account, "connection").and_then(|c| c.parse().ok()).unwrap_or_default();
            let why = match sent_as(tx, ctx.tenant, conn, POSTS, publication).await? {
                Some(external) => {
                    let values = json!({"external_id": external, "sent_at": Utc::now().to_rfc3339()});
                    k.set_values_tx(ctx, tx, "SocialPublication", publication, obj(values)).await?;
                    continue;
                }
                None => match delivery(tx, ctx.tenant, conn, POSTS, publication).await? {
                    Some((status, _)) if status == "queued" || status == "running" => None,
                    Some((_, error)) => Some(error.unwrap_or_else(|| "the network gave no id for it".into())),
                    None if at(&p, "created_at").is_some_and(|c| now - c > STALE) => Some("sending it left no trace and it never went out".into()),
                    None => None,
                },
            };
            match why {
                None => pending = true,
                Some(why) => {
                    let values = json!({"failed_at": Utc::now().to_rfc3339(), "failure": why});
                    k.set_values_tx(ctx, tx, "SocialPublication", publication, obj(values)).await?;
                    failures.push(format!("{name}: {why}"));
                }
            }
        }
        if pending {
            continue;
        }
        let values = if failures.is_empty() {
            json!({"status": "Published", "published_at": Utc::now().to_rfc3339()})
        } else {
            json!({"status": "Failed", "failure": failures.join("\n")})
        };
        k.set_values_tx(ctx, tx, "SocialPost", post, obj(values)).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_each_network_takes() {
        let long = "a".repeat(281);
        assert!(misfit("X", &long, None, false).is_some_and(|w| w.contains("280")));
        assert!(misfit("X", &"a".repeat(256), Some("https://example.org/a-very-long-path-that-x-shortens"), false).is_none(), "a link is 23 and a space");
        assert!(misfit("X", &"a".repeat(257), Some("https://e.org"), false).is_some());
        assert!(misfit("LinkedIn", &long, None, false).is_none());
        assert!(misfit("Mastodon", &"a".repeat(490), Some("https://example.org/0123456789"), false).is_some(), "Mastodon counts the link");
        assert!(misfit("Instagram", "Hello", None, false).is_some_and(|w| w.contains("image or a video")));
        assert_eq!(misfit("Instagram", "Hello", None, true), None, "with its picture");
        assert!(misfit("YouTube", "Hello", None, true).is_some_and(|w| w.contains("video")), "a picture is not a video");
    }
}
