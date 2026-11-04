//! The inbox. Messages are read through each account's connection (comments, mentions and direct
//! messages) and come to the account whose handle they name. Someone takes each one; a message
//! assigned names whom to. A reply goes back through the message's account's connection, to the
//! network's own message, when that connection can reply; the message is then answered.

use crate::{capabilities, connection, deliver, obj, sent_as};
use async_trait::async_trait;
use ferrum_domain::util::{elevated, id, s};
use ferrum_kernel::{Ctx, Doc, DocHook, Error, Filter, HookCtx, Kernel, Result, Tx};
use serde_json::json;
use uuid::Uuid;

/// The streams messages are read on, and replies sent on.
pub(crate) const MESSAGES: &str = "messages";
pub(crate) const REPLIES: &str = "replies";

pub struct MessageHook;

#[async_trait]
impl DocHook for MessageHook {
    fn name(&self) -> &'static str {
        "social.message"
    }

    fn fills(&self) -> &'static [&'static str] {
        &["account"]
    }

    async fn before_validate(&self, hc: &mut HookCtx<'_>, doc: &mut Doc) -> Result<()> {
        if id(doc, "account").is_some() {
            return Ok(());
        }
        let Some(handle) = s(doc, "account_handle").map(|h| h.trim().trim_start_matches('@').to_string()) else { return Ok(()) };
        let found = hc.kernel.find_tx(&elevated(hc.ctx), hc.tx, "SocialAccount", &[Filter::eq("handle", handle)], None, 2).await?;
        // A handle two accounts share says nothing of which one.
        if let [one] = found.as_slice() {
            doc.insert("account".into(), json!(s(one, "id")));
        }
        Ok(())
    }

    async fn validate(&self, _hc: &mut HookCtx<'_>, doc: &Doc) -> Result<()> {
        if s(doc, "status") == Some("Assigned") && s(doc, "assigned_to").is_none() {
            return Err(Error::field("assigned_to", "a message assigned says to whom"));
        }
        Ok(())
    }
}

/// The message's account's connection, and the network's id of the message.
async fn reached(k: &Kernel, ctx: &Ctx, tx: &mut Tx, message: &Doc) -> Result<(Uuid, String, Option<String>)> {
    let account = id(message, "account").ok_or_else(|| Error::field("message", "it came to no account of ours, so there is nowhere to answer from"))?;
    let a = k.get_tx(ctx, tx, "SocialAccount", account, false).await?;
    let conn: Uuid = s(&a, "connection").and_then(|c| c.parse().ok()).unwrap_or_default();
    let external = sent_as(tx, ctx.tenant, conn, MESSAGES, id(message, "id").unwrap_or_default()).await?;
    Ok((conn, s(&a, "account_name").unwrap_or_default().to_string(), external))
}

pub struct ReplyHook;

#[async_trait]
impl DocHook for ReplyHook {
    fn name(&self) -> &'static str {
        "social.reply"
    }

    fn fills(&self) -> &'static [&'static str] {
        &["author", "in_reply_to"]
    }

    async fn before_validate(&self, hc: &mut HookCtx<'_>, doc: &mut Doc) -> Result<()> {
        if s(doc, "author").is_none() && !hc.ctx.email.is_empty() {
            doc.insert("author".into(), json!(hc.ctx.email.to_lowercase()));
        }
        if let Some(message) = id(doc, "message") {
            let system = elevated(hc.ctx);
            let m = hc.kernel.get_tx(&system, hc.tx, "SocialMessage", message, false).await?;
            if let Ok((_, _, Some(external))) = reached(hc.kernel, &system, hc.tx, &m).await {
                doc.insert("in_reply_to".into(), json!(external));
            }
        }
        Ok(())
    }

    async fn validate(&self, hc: &mut HookCtx<'_>, doc: &Doc) -> Result<()> {
        let system = elevated(hc.ctx);
        let m = hc.kernel.get_tx(&system, hc.tx, "SocialMessage", id(doc, "message").unwrap_or_default(), false).await?;
        let (conn, name, external) = reached(hc.kernel, &system, hc.tx, &m).await?;
        if external.is_none() {
            return Err(Error::field("message", "it was not read from the network, so there is nothing there to answer"));
        }
        let provider = connection(hc.tx, hc.ctx.tenant, conn).await?.map(|(p, _)| p).unwrap_or_default();
        if !capabilities(hc.kernel, hc.ctx.tenant, &provider).await?.iter().any(|c| c == "reply") {
            return Err(Error::field("message", format!("{name}'s connection cannot reply")));
        }
        Ok(())
    }

    async fn after_save(&self, hc: &mut HookCtx<'_>, doc: &Doc, old: Option<&Doc>) -> Result<()> {
        if old.is_some() {
            return Ok(());
        }
        let system = elevated(hc.ctx);
        let message = id(doc, "message").unwrap_or_default();
        let m = hc.kernel.get_tx(&system, hc.tx, "SocialMessage", message, false).await?;
        let (conn, _, _) = reached(hc.kernel, &system, hc.tx, &m).await?;
        deliver(hc.tx, hc.ctx.tenant, conn, REPLIES, id(doc, "id").unwrap_or_default()).await?;
        hc.kernel.set_values_tx(&system, hc.tx, "SocialMessage", message, obj(json!({"status": "Answered"}))).await?;
        Ok(())
    }
}

/// Replies the network has taken, marked with its id.
pub(crate) async fn mark_replies(k: &Kernel, ctx: &Ctx, tx: &mut Tx) -> Result<()> {
    for r in k.find_tx(ctx, tx, "SocialReply", &[], Some("created_at desc"), 1000).await? {
        if s(&r, "external_id").is_some() {
            continue;
        }
        let m = k.get_tx(ctx, tx, "SocialMessage", id(&r, "message").unwrap_or_default(), false).await?;
        let Ok((conn, _, _)) = reached(k, ctx, tx, &m).await else { continue };
        if let Some(external) = sent_as(tx, ctx.tenant, conn, REPLIES, id(&r, "id").unwrap_or_default()).await? {
            k.set_values_tx(ctx, tx, "SocialReply", id(&r, "id").unwrap_or_default(), obj(json!({"external_id": external}))).await?;
        }
    }
    Ok(())
}
