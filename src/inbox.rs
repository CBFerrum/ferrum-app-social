//! The inbox. Comments, mentions and direct messages reach the app through a connection: its inbound
//! address (the platform verifies who calls, drops a replay and keeps each delivery once) or a pull
//! with a cursor, either mapped by the connection's description onto SocialMessage, found by the
//! network's id (`external_id`), so a message arrives once however often a network says it. Social
//! adds what a mapping cannot know: which account it came to (by the handle the item names, else by
//! the post it answers) and, when the workspace's Social Settings say so, the record of the person
//! who wrote.
//!
//! A reply goes out through an operation of the account's connection (`reply` to a comment or a
//! mention, `send_message` to a direct message), as one queued job for the reply, under the key
//! `reply:<reply>`: it holds a lock on the reply's row and finds it out before it calls, so it goes
//! once under retries and two workers. Someone takes each message (assigned, answered, closed),
//! with the platform's own tags on it; saved replies are records.

use crate::accounts::{caller, reach};
use crate::posts::text_of;
use crate::{obj, said};
use async_trait::async_trait;
use chrono::Utc;
use ferrum_domain::util::{elevated, id, s};
use ferrum_kernel::connections::{self, Call};
use ferrum_kernel::jobs::{Job, JobHandler, NewJob};
use ferrum_kernel::{Ctx, Doc, DocHook, Error, Filter, HookCtx, Kernel, Result};
use serde_json::{Value, json};
use std::time::Duration as Wait;
use uuid::Uuid;

pub(crate) const REPLY: &str = "social.reply";
const ATTEMPTS: i32 = 8;

/// A handle as people write it: without the @ in front.
fn bare(handle: &str) -> &str {
    handle.trim().trim_start_matches('@')
}

/// A network's HTML as plain text: paragraphs and line breaks kept, tags dropped, entities read. For a network whose
/// description says it sends words as HTML (`meta.capabilities.html_text`).
pub(crate) fn plain(html: &str) -> String {
    let spaced = html.replace("</p><p>", "\n\n").replace("<br />", "\n").replace("<br/>", "\n").replace("<br>", "\n");
    let mut out = String::with_capacity(spaced.len());
    let mut in_tag = false;
    for c in spaced.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&#39;", "'").replace("&apos;", "'").replace("&amp;", "&").trim().to_string()
}

pub struct MessageHook;

#[async_trait]
impl DocHook for MessageHook {
    fn name(&self) -> &'static str {
        "social.message"
    }

    fn fills(&self) -> &'static [&'static str] {
        &["account", "contact_entity", "contact_id"]
    }

    async fn before_validate(&self, hc: &mut HookCtx<'_>, doc: &mut Doc) -> Result<()> {
        // Words a network sends as HTML are kept as the words they hold.
        if let Some(conn) = reach(hc.tx, hc.ctx.tenant, s(doc, "via_connection")).await?
            && (conn.meta()["capabilities"]["html_text"] == true || conn.meta()["social"]["inbox_format"] == "html")
            && let Some(text) = s(doc, "text")
        {
            doc.insert("text".into(), json!(plain(text)));
        }
        if id(doc, "account").is_some() {
            return Ok(());
        }
        let system = elevated(hc.ctx);
        // The account its handle names; a handle two accounts share says nothing of which one.
        if let Some(handle) = s(doc, "account_handle").map(bare).filter(|h| !h.is_empty()) {
            let found = hc.kernel.find_tx(&system, hc.tx, "SocialAccount", &[Filter::eq("handle", handle)], None, 2).await?;
            if let [one] = found.as_slice() {
                doc.insert("account".into(), json!(s(one, "id")));
                return Ok(());
            }
        }
        // A comment names the post it answers: the account that posted it is the one it came to.
        if let Some(post) = s(doc, "post_external_id").filter(|p| !p.is_empty()) {
            let found = hc.kernel.find_tx(&system, hc.tx, "SocialPublication", &[Filter::eq("external_id", post)], None, 2).await?;
            if let [one] = found.as_slice() {
                doc.insert("account".into(), json!(s(one, "account")));
                return Ok(());
            }
        }
        // Read through a connection that reaches one account: that one. A person's own connection is read as her, so
        // the account she connected.
        if let Some(conn) = s(doc, "via_connection").filter(|c| !c.is_empty()) {
            let mut accounts = hc.kernel.find_tx(&system, hc.tx, "SocialAccount", &[Filter::eq("connection", conn)], None, 50).await?;
            if !hc.ctx.is_system && !hc.ctx.email.is_empty() {
                accounts.retain(|a| s(a, "connected_by").is_none_or(|by| by.eq_ignore_ascii_case(&hc.ctx.email)));
            }
            if let [one] = accounts.as_slice() {
                doc.insert("account".into(), json!(s(one, "id")));
            }
        }
        Ok(())
    }

    async fn validate(&self, _hc: &mut HookCtx<'_>, doc: &Doc) -> Result<()> {
        if s(doc, "status") == Some("Assigned") && s(doc, "assigned_to").is_none() {
            return Err(Error::field("assigned_to", "a message assigned says to whom"));
        }
        Ok(())
    }

    /// The person who wrote, found among the records the workspace says hold people's handles, when it says so and the
    /// entity is installed.
    async fn after_save(&self, hc: &mut HookCtx<'_>, doc: &Doc, old: Option<&Doc>) -> Result<()> {
        if old.is_some() || s(doc, "contact_id").is_some() {
            return Ok(());
        }
        let system = elevated(hc.ctx);
        let Some(settings) = hc.kernel.find_tx(&system, hc.tx, "SocialSettings", &[], None, 1).await?.into_iter().next() else { return Ok(()) };
        let on = settings.get("match_crm").and_then(Value::as_bool).unwrap_or(false);
        let entity = s(&settings, "crm_entity").map(str::trim).filter(|e| !e.is_empty()).unwrap_or("Lead").to_string();
        let (Some(field), Some(author), true) = (s(&settings, "crm_field").map(str::trim).filter(|f| !f.is_empty()), s(doc, "author").map(bare), on) else {
            return Ok(());
        };
        // Only when the entity is installed here and holds such a field.
        let Some(def) = hc.ctx.schema.entity(&entity) else { return Ok(()) };
        if author.is_empty() || !def.def.fields.iter().any(|f| f.name == field) {
            return Ok(());
        }
        let found = hc.kernel.find_tx(&system, hc.tx, &entity, &[Filter::eq(field, author)], None, 2).await?;
        // Two records holding one handle say nothing of which person it is.
        if let ([one], Some(message)) = (found.as_slice(), id(doc, "id")) {
            hc.kernel.set_values_tx(&system, hc.tx, "SocialMessage", message, obj(json!({"contact_entity": entity, "contact_id": s(one, "id")}))).await?;
        }
        Ok(())
    }

    /// `reply`: answered from the message itself, as a reply of the one answering, which goes out
    /// as any other does. Its words are given, or a saved reply's.
    async fn action(&self, hc: &mut HookCtx<'_>, name: &str, doc: &Doc, params: &Value) -> Result<Option<Value>> {
        if name != "reply" {
            return Ok(None);
        }
        // Named apart from the message's own text, which would otherwise fill it in.
        let mut text = params.get("reply").and_then(Value::as_str).map(str::trim).filter(|t| !t.is_empty()).map(str::to_string);
        if let (None, Some(saved)) = (&text, params.get("saved_reply").and_then(Value::as_str).and_then(|r| r.parse::<Uuid>().ok())) {
            let saved = hc.kernel.get_tx(&elevated(hc.ctx), hc.tx, "SocialSavedReply", saved, false).await?;
            text = s(&saved, "text").map(str::to_string);
        }
        let text = text.ok_or_else(|| Error::field("reply", "a reply says something, or is a saved reply"))?;
        let made = hc.kernel.insert_tx(hc.ctx, hc.tx, "SocialReply", obj(json!({"message": s(doc, "id"), "text": text}))).await?;
        Ok(Some(json!({"reply": s(&made, "id")})))
    }
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
            let m = hc.kernel.get_tx(&elevated(hc.ctx), hc.tx, "SocialMessage", message, false).await?;
            if let Some(external) = s(&m, "external_id") {
                doc.insert("in_reply_to".into(), json!(external));
            }
        }
        Ok(())
    }

    async fn validate(&self, hc: &mut HookCtx<'_>, doc: &Doc) -> Result<()> {
        let system = elevated(hc.ctx);
        let m = hc.kernel.get_tx(&system, hc.tx, "SocialMessage", id(doc, "message").unwrap_or_default(), false).await?;
        let account = id(&m, "account").ok_or_else(|| Error::field("message", "it came to no account of ours, so there is nowhere to answer from"))?;
        let a = hc.kernel.get_tx(&system, hc.tx, "SocialAccount", account, false).await?;
        let name = s(&a, "account_name").unwrap_or_default();
        let conn = reach(hc.tx, hc.ctx.tenant, s(&a, "connection")).await?;
        let direct = s(&m, "kind") == Some("Direct");
        if direct {
            if !conn.as_ref().is_some_and(|c| c.has("send_message")) {
                return Err(Error::field("message", format!("{name}'s connection cannot send direct messages")));
            }
            if s(&m, "author_id").or(s(&m, "author")).is_none() {
                return Err(Error::field("message", "it names nobody to send the answer to"));
            }
        } else {
            if !conn.as_ref().is_some_and(|c| c.has("reply")) {
                return Err(Error::field("message", format!("{name}'s connection cannot reply")));
            }
            if s(&m, "external_id").is_none() {
                return Err(Error::field("message", "it was not read from the network, so there is nothing there to answer"));
            }
        }
        Ok(())
    }

    async fn after_save(&self, hc: &mut HookCtx<'_>, doc: &Doc, old: Option<&Doc>) -> Result<()> {
        // A reply is written by a person here; one fetched from outside is what the network already has, and sending
        // it back out would say it twice.
        if old.is_some() || hc.ctx.origin.as_deref() == Some("outside") {
            return Ok(());
        }
        let reply = id(doc, "id").unwrap_or_default();
        let mut job = NewJob::new(REPLY, json!({"reply": reply}));
        job.max_attempts = ATTEMPTS;
        job.dedupe_key = Some(format!("reply:{reply}"));
        ferrum_kernel::jobs::enqueue(hc.tx, hc.ctx.tenant, None, job).await?;
        Ok(())
    }
}

pub struct ReplyJob;

#[async_trait]
impl JobHandler for ReplyJob {
    fn kind(&self) -> &'static str {
        REPLY
    }

    fn title(&self) -> &'static str {
        "A reply sent"
    }

    async fn run(&self, k: &Kernel, job: &Job) -> Result<Value> {
        let reply: Uuid = job.payload["reply"].as_str().and_then(|p| p.parse().ok()).ok_or_else(|| Error::invalid("names its reply"))?;
        match send(k, job.tenant_id, reply).await {
            Ok(done) => Ok(done),
            Err(e) => {
                let last = !crate::outside::later(&e) || job.attempts >= job.max_attempts;
                let _ = fail(k, job.tenant_id, reply, &e.to_string(), last).await;
                Err(match e {
                    Error::RetryAfter(why, secs) => Error::RetryAfter(why, secs.clamp(1, crate::outside::MOST_WAIT)),
                    other => other,
                })
            }
        }
    }

    fn timeout(&self) -> Wait {
        Wait::from_secs(120)
    }
}

async fn fail(k: &Kernel, tenant: Uuid, reply: Uuid, why: &str, last: bool) -> Result<()> {
    let ctx = Ctx::system(tenant, k.registry.schema(tenant).await?);
    let mut tx = k.db.tenant_tx(tenant).await?;
    let r = k.get_tx(&ctx, &mut tx, "SocialReply", reply, true).await?;
    if s(&r, "external_id").is_none() {
        let values =
            if last { json!({"failed_at": Utc::now().to_rfc3339(), "failure": why}) } else { json!({"failure": format!("Waiting to try again: {why}")}) };
        k.set_values_tx(&ctx, &mut tx, "SocialReply", reply, obj(values)).await?;
    }
    tx.commit().await?;
    Ok(())
}

async fn send(k: &Kernel, tenant: Uuid, reply: Uuid) -> Result<Value> {
    let ctx = Ctx::system(tenant, k.registry.schema(tenant).await?);
    let mut tx = k.db.tenant_tx(tenant).await?;
    let r = k.get_tx(&ctx, &mut tx, "SocialReply", reply, true).await?;
    if s(&r, "external_id").is_some() {
        tx.commit().await?;
        return Ok(json!({"done": "already sent"}));
    }
    let m = k.get_tx(&ctx, &mut tx, "SocialMessage", id(&r, "message").unwrap_or_default(), false).await?;
    let account = k.get_tx(&ctx, &mut tx, "SocialAccount", id(&m, "account").unwrap_or_default(), false).await?;
    let conn = reach(&mut tx, tenant, s(&account, "connection")).await?.ok_or_else(|| Error::invalid("the account's connection is gone"))?;
    if !conn.enabled {
        return Err(Error::invalid(format!("the connection {} is turned off", conn.name)));
    }
    let calling = caller(k, &mut tx, tenant, &account, &conn).await?;
    let direct = s(&m, "kind") == Some("Direct");
    let operation = if direct { "send_message" } else { "reply" };
    let created = crate::at(&r, "created_at").unwrap_or_else(Utc::now).to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let inputs = json!({"text": s(&r, "text"), "in_reply_to": s(&m, "external_id"), "author": s(&m, "author").map(bare), "to": s(&m, "author_id").or(s(&m, "author").map(bare)),
                        "thread": s(&m, "thread"), "created_at": created});
    let call = Call {
        connection: conn.id.to_string(),
        operation: operation.into(),
        inputs: conn.fit(operation, inputs),
        key: Some(format!("reply:{reply}")),
        entity: Some("SocialReply".into()),
        record: Some(reply),
    };
    let out = connections::call(k, &calling, Some(crate::APP), &call).await.map_err(said)?;
    let external = text_of(&out["id"]).unwrap_or_else(|| "sent".into());
    k.set_values_tx(&ctx, &mut tx, "SocialReply", reply, obj(json!({"external_id": external, "failed_at": null, "failure": null}))).await?;
    if s(&m, "status") != Some("Closed") {
        k.set_values_tx(&ctx, &mut tx, "SocialMessage", id(&m, "id").unwrap_or_default(), obj(json!({"status": "Answered"}))).await?;
    }
    tx.commit().await?;
    Ok(json!({"id": external}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn html_as_the_words_it_holds() {
        assert_eq!(plain("<p>Spring is here &amp; <a href=\"x\">#sage</a></p><p>Second</p>"), "Spring is here & #sage\n\nSecond");
        assert_eq!(plain("one<br />two &lt;3"), "one\ntwo <3");
    }
}
