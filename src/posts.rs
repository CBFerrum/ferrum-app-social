//! Accounts, posts and publications. A post goes from its brand to one or more of the brand's
//! active accounts. It must fit every network it goes to, and each account's connection must be
//! able to publish. Someone other than its author approves it; one sent back says why, and a post
//! changed after its approval needs approving again. At its time, upkeep makes one publication per
//! account and sends it through that account's own connection, by the platform's delivery. A
//! publication is out once the network has given its id, and the post is published when all of
//! them are. What each did is read back through the connection with the network's posts.

use crate::{at, capabilities, connection, deliver, obj, sent_as};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use ferrum_domain::util::{elevated, id, rows, s};
use ferrum_kernel::{Ctx, Doc, DocHook, Error, Filter, HookCtx, Kernel, Result, Tx};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

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
        // Instagram and YouTube publish media, which posts here do not carry.
        _ => (2200, false),
    }
}

/// Why a post of `text` and `link` does not fit `network`, when it does not.
pub(crate) fn misfit(network: &str, text: &str, link: Option<&str>) -> Option<String> {
    let (max, text_alone) = takes(network);
    if !text_alone {
        return Some(format!("{network} takes no post without an image or a video, which posts here do not carry"));
    }
    // X counts a link as 23 characters, whatever its length, after a space.
    let length = text.chars().count() + link.map_or(0, |l| if network == "X" { 24 } else { l.chars().count() + 1 });
    (length > max).then(|| format!("{network} takes {max} characters, and this post is {length}"))
}

/// What approval fixes: the words, the link, where and when it goes.
fn fingerprint(doc: &Doc) -> String {
    let mut targets: Vec<String> = rows(doc, "targets").iter().filter_map(|t| s(t, "account").map(str::to_string)).collect();
    targets.sort();
    let what = json!([s(doc, "text"), s(doc, "link"), targets, at(doc, "scheduled_at").map(|t| t.timestamp())]);
    hex::encode(Sha256::digest(what.to_string()))
}

pub struct AccountHook;

#[async_trait]
impl DocHook for AccountHook {
    fn name(&self) -> &'static str {
        "social.account"
    }

    async fn before_validate(&self, _hc: &mut HookCtx<'_>, doc: &mut Doc) -> Result<()> {
        if let Some(handle) = s(doc, "handle").map(|h| h.trim().trim_start_matches('@').to_string()) {
            doc.insert("handle".into(), json!(handle));
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
            if let Some(why) = misfit(s(&a, "network").unwrap_or_default(), text, link) {
                return Err(Error::field("text", format!("for {name}: {why}")));
            }
            let conn = s(&a, "connection").and_then(|c| c.parse().ok()).unwrap_or_default();
            let provider = connection(hc.tx, hc.ctx.tenant, conn).await?.map(|(p, _)| p).unwrap_or_default();
            if !capabilities(hc.kernel, hc.ctx.tenant, &provider).await?.iter().any(|c| c == "publish_text") {
                return Err(Error::field("targets", format!("{name}'s connection cannot publish posts")));
            }
        }
        Ok(())
    }

    async fn validate_change(&self, _hc: &mut HookCtx<'_>, old: Option<&Doc>, doc: &Doc) -> Result<()> {
        if old.is_some_and(|o| matches!(s(o, "status"), Some("Sending" | "Published")) && fingerprint(o) != fingerprint(doc)) {
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
                    obj(json!({"post": post.to_string(), "account": account.to_string(), "text": s(&doc, "text"), "link": s(&doc, "link")})),
                )
                .await?;
            deliver(tx, ctx.tenant, conn, POSTS, id(&made, "id").unwrap_or_default()).await?;
        }
        k.set_values_tx(ctx, tx, "SocialPost", post, obj(json!({"status": "Sending"}))).await?;
    }
    Ok(())
}

/// Publications the network has given its id to, marked out; posts all of whose are, published.
pub(crate) async fn mark_sent(k: &Kernel, ctx: &Ctx, tx: &mut Tx) -> Result<()> {
    for found in k.find_tx(ctx, tx, "SocialPost", &[Filter::eq("status", "Sending")], None, 500).await? {
        let post = id(&found, "id").unwrap_or_default();
        let mut all_out = true;
        for p in k.find_tx(ctx, tx, "SocialPublication", &[Filter::eq("post", post.to_string())], None, 100).await? {
            if s(&p, "external_id").is_some() {
                continue;
            }
            let account = k.get_tx(ctx, tx, "SocialAccount", id(&p, "account").unwrap_or_default(), false).await?;
            let conn = s(&account, "connection").and_then(|c| c.parse().ok()).unwrap_or_default();
            match sent_as(tx, ctx.tenant, conn, POSTS, id(&p, "id").unwrap_or_default()).await? {
                Some(external) => {
                    let values = json!({"external_id": external, "sent_at": Utc::now().to_rfc3339()});
                    k.set_values_tx(ctx, tx, "SocialPublication", id(&p, "id").unwrap_or_default(), obj(values)).await?;
                }
                None => all_out = false,
            }
        }
        if all_out {
            k.set_values_tx(ctx, tx, "SocialPost", post, obj(json!({"status": "Published", "published_at": Utc::now().to_rfc3339()}))).await?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_each_network_takes() {
        let long = "a".repeat(281);
        assert!(misfit("X", &long, None).is_some_and(|w| w.contains("280")));
        assert!(misfit("X", &"a".repeat(256), Some("https://example.org/a-very-long-path-that-x-shortens")).is_none(), "a link is 23 and a space");
        assert!(misfit("X", &"a".repeat(257), Some("https://e.org")).is_some());
        assert!(misfit("LinkedIn", &long, None).is_none());
        assert!(misfit("Mastodon", &"a".repeat(490), Some("https://example.org/0123456789")).is_some(), "Mastodon counts the link");
        assert!(misfit("Instagram", "Hello", None).is_some_and(|w| w.contains("image or a video")));
    }
}
