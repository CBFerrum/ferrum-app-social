//! Acting on a message on the network, and putting one aside.
//!
//! Like, hide or delete a comment is one queued job under the key `inbox:<message>:<act>`: the action only marks the
//! message (`act_pending`) and queues the job; the job reads the message, calls the account's connection with no
//! transaction open, and writes what came of it. The network is asked at most once: the connection keeps the answer of
//! a call under its key, the job has a dedupe key, and the action is refused while one is pending. A message snoozed
//! leaves the open inbox until its time, when the upkeep gives it back its status.

use crate::accounts::{Reach, caller, reach};
use crate::outside::{MOST_WAIT, heed, holds, later, waiting};
use crate::{at, obj, said};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use ferrum_domain::util::{id, s};
use ferrum_kernel::connections::{self, Call};
use ferrum_kernel::jobs::{Job, JobHandler, NewJob};
use ferrum_kernel::{Ctx, Doc, Error, Filter, HookCtx, Kernel, Result, Tx};
use serde_json::{Value, json};
use std::time::Duration as Wait;
use uuid::Uuid;

pub(crate) const ACT: &str = "social.inbox_act";
const ATTEMPTS: i32 = 8;
/// Messages woken in one round of the upkeep.
const WAKE: u32 = 500;

/// What Social's name of an act is: the operation it calls, what it says it cannot do, the field that stamps it.
fn parts(act: &str) -> Result<(&'static str, &'static str, &'static str)> {
    match act {
        "like" => Ok(("like_comment", "like a comment", "liked_at")),
        "hide" => Ok(("hide_comment", "hide a comment", "hidden_at")),
        "remove" => Ok(("delete_comment", "delete a comment", "removed_at")),
        _ => Err(Error::invalid("there is no such thing to do with a message")),
    }
}

/// Whether the message's kind and state allow `act` (the action's own `condition:`, kept here so that a caller that
/// skips the screen cannot skip it either).
fn allowed(m: &Doc, act: &str) -> bool {
    let empty = |k: &str| s(m, k).is_none();
    let kind = s(m, "kind");
    match act {
        "like" => kind != Some("Direct") && empty("liked_at") && empty("removed_at"),
        "hide" => kind == Some("Comment") && empty("hidden_at") && empty("removed_at"),
        "remove" => kind == Some("Comment") && empty("removed_at"),
        _ => false,
    }
}

/// The message's account, its connection and the connection's name for Social's operation `op`, when the connection is
/// on and has it; else the words that say why not.
async fn offered(k: &Kernel, tx: &mut Tx, tenant: Uuid, m: &Doc, op: &str, what: &str) -> Result<(Reach, Doc, String)> {
    let system = Ctx::system(tenant, k.registry.schema(tenant).await?);
    let account = id(m, "account").ok_or_else(|| Error::field("message", "it came to no account of ours, so there is nowhere to act from"))?;
    let a = k.get_tx(&system, tx, "SocialAccount", account, false).await?;
    let name = s(&a, "account_name").unwrap_or_default().to_string();
    let conn = reach(tx, tenant, s(&a, "connection")).await?.ok_or_else(|| Error::invalid("the account's connection is gone"))?;
    let theirs = match conn.social(None)? {
        Some(rules) => rules.op(op).to_string(),
        None => op.to_string(),
    };
    if !conn.enabled || !conn.has(&theirs) {
        return Err(Error::field("message", format!("{name}'s connection cannot {what}")));
    }
    Ok((conn, a, theirs))
}

/// `like`, `hide` or `remove`, asked for: the message is marked and the job queued, in the action's transaction.
pub(crate) async fn start(hc: &mut HookCtx<'_>, act: &str, doc: &Doc) -> Result<Value> {
    let (op, what, _) = parts(act)?;
    if act == "remove" && !holds(hc.ctx, &["Social Approver", "Social Manager"]) {
        return Err(Error::Forbidden("deleting a comment on the network is for a Social Approver or a Social Manager".into()));
    }
    let message = id(doc, "id").ok_or_else(|| Error::invalid("names its message"))?;
    if !allowed(doc, act) {
        return Err(Error::invalid(format!("this message cannot be treated so: {what}")));
    }
    if s(doc, "external_id").is_none() {
        return Err(Error::field("message", "it was not read from the network, so there is nothing there to act on"));
    }
    offered(hc.kernel, hc.tx, hc.ctx.tenant, doc, op, what).await?;
    if s(doc, "act_pending").is_some() {
        return Err(Error::invalid("another action on it is still going out"));
    }
    let system = Ctx::system(hc.ctx.tenant, hc.kernel.registry.schema(hc.ctx.tenant).await?);
    hc.kernel.set_values_tx(&system, hc.tx, "SocialMessage", message, obj(json!({"act_pending": act, "act_failure": null}))).await?;
    let mut job = NewJob::new(ACT, json!({"message": message, "act": act}));
    job.max_attempts = ATTEMPTS;
    job.dedupe_key = Some(format!("inbox:{message}:{act}"));
    ferrum_kernel::jobs::enqueue(hc.tx, hc.ctx.tenant, None, job).await?;
    Ok(json!({"queued": true}))
}

/// `can`: what the message's account's connection offers now, for this caller. Nothing is changed.
pub(crate) async fn can(hc: &mut HookCtx<'_>, doc: &Doc) -> Result<Value> {
    let tenant = hc.ctx.tenant;
    let direct = s(doc, "kind") == Some("Direct");
    let mut out = serde_json::Map::new();
    let reply_op = if direct { "send_message" } else { "reply" };
    let reply_ok = if direct { s(doc, "author_id").or(s(doc, "author")).is_some() } else { s(doc, "external_id").is_some() };
    let reply = reply_ok && offered(hc.kernel, hc.tx, tenant, doc, reply_op, "answer").await.is_ok();
    out.insert("reply".into(), json!(reply));
    for act in ["like", "hide", "remove"] {
        let (op, what, _) = parts(act)?;
        let mut yes = allowed(doc, act) && s(doc, "external_id").is_some();
        if yes && act == "remove" {
            yes = holds(hc.ctx, &["Social Approver", "Social Manager"]);
        }
        if yes {
            yes = offered(hc.kernel, hc.tx, tenant, doc, op, what).await.is_ok();
        }
        out.insert(act.into(), json!(yes));
    }
    Ok(Value::Object(out))
}

pub struct ActJob;

#[async_trait]
impl JobHandler for ActJob {
    fn kind(&self) -> &'static str {
        ACT
    }

    fn title(&self) -> &'static str {
        "A comment liked, hidden or deleted on the network"
    }

    async fn run(&self, k: &Kernel, job: &Job) -> Result<Value> {
        let message: Uuid = job.payload["message"].as_str().and_then(|p| p.parse().ok()).ok_or_else(|| Error::invalid("names its message"))?;
        let act = job.payload["act"].as_str().ok_or_else(|| Error::invalid("names what to do"))?.to_string();
        match perform(k, job.tenant_id, message, &act).await {
            Ok(done) => Ok(done),
            Err(e) => {
                let last = !later(&e) || job.attempts >= job.max_attempts;
                let _ = fail(k, job.tenant_id, message, &act, &e.to_string(), last).await;
                Err(match e {
                    Error::RetryAfter(why, secs) => Error::RetryAfter(why, secs.clamp(1, MOST_WAIT)),
                    other => other,
                })
            }
        }
    }

    fn timeout(&self) -> Wait {
        Wait::from_secs(120)
    }
}

/// The job: A reads, the call is made with nothing open, B writes what came of it.
async fn perform(k: &Kernel, tenant: Uuid, message: Uuid, act: &str) -> Result<Value> {
    let (op, what, stamp) = parts(act)?;
    let system = Ctx::system(tenant, k.registry.schema(tenant).await?);
    let mut tx = k.db.tenant_tx(tenant).await?;
    let m = k.get_tx(&system, &mut tx, "SocialMessage", message, false).await?;
    if s(&m, "act_pending") != Some(act) {
        tx.commit().await?;
        return Ok(json!({"done": "already done"}));
    }
    let (conn, account, theirs) = offered(k, &mut tx, tenant, &m, op, what).await?;
    let calling = caller(k, &mut tx, tenant, &account, &conn).await?;
    tx.commit().await?;
    if let Some(secs) = waiting(tenant, conn.id) {
        return Err(Error::RetryAfter(format!("{} asked to be left alone for a while", conn.name), secs));
    }
    let inputs = conn.fit(
        &theirs,
        json!({"id": s(&m, "external_id"), "post": s(&m, "post_external_id"), "account_id": crate::picker::network_id(&account), "handle": s(&account, "handle")}),
    );
    let call = Call {
        connection: conn.id.to_string(),
        operation: theirs,
        inputs,
        key: Some(format!("inbox:{message}:{act}")),
        entity: Some("SocialMessage".into()),
        record: Some(message),
    };
    heed(tenant, conn.id, connections::call(k, &calling, Some(crate::APP), &call).await.map_err(said))?;
    let mut tx = k.db.tenant_tx(tenant).await?;
    let m = k.get_tx(&system, &mut tx, "SocialMessage", message, true).await?;
    if s(&m, "act_pending") == Some(act) {
        let mut done = json!({stamp: Utc::now().to_rfc3339(), "act_pending": null, "act_failure": null});
        if act == "remove" {
            done["status"] = json!("Closed");
            done["snoozed_until"] = Value::Null;
            done["snoozed_from"] = Value::Null;
        }
        k.set_values_tx(&system, &mut tx, "SocialMessage", message, obj(done)).await?;
    }
    tx.commit().await?;
    Ok(json!({"done": act}))
}

/// What the network said when it would not, or that the job will try again.
async fn fail(k: &Kernel, tenant: Uuid, message: Uuid, act: &str, why: &str, last: bool) -> Result<()> {
    let ctx = Ctx::system(tenant, k.registry.schema(tenant).await?);
    let mut tx = k.db.tenant_tx(tenant).await?;
    let m = k.get_tx(&ctx, &mut tx, "SocialMessage", message, true).await?;
    if s(&m, "act_pending") == Some(act) {
        let why: String = why.chars().take(500).collect();
        let values = if last { json!({"act_pending": null, "act_failure": why}) } else { json!({"act_failure": format!("Waiting to try again: {why}")}) };
        k.set_values_tx(&ctx, &mut tx, "SocialMessage", message, obj(values)).await?;
    }
    tx.commit().await?;
    Ok(())
}

/// The upkeep: snoozed messages whose time has come go back to the status they had. Only where the status is still
/// Snoozed (it is read in this transaction), so a person who reopened one meanwhile is not overwritten.
pub(crate) async fn wake(k: &Kernel, ctx: &Ctx, tx: &mut Tx, now: DateTime<Utc>) -> Result<()> {
    let snoozed = k.find_tx(ctx, tx, "SocialMessage", &[Filter::eq("status", "Snoozed")], None, WAKE).await?;
    for m in snoozed.iter().filter(|m| at(m, "snoozed_until").is_some_and(|until| until <= now)) {
        let Some(message) = id(m, "id") else { continue };
        let back = match s(m, "snoozed_from") {
            Some("Assigned") if s(m, "assigned_to").is_some() => "Assigned",
            Some("Answered") => "Answered",
            _ => "New",
        };
        k.set_values_tx(ctx, tx, "SocialMessage", message, obj(json!({"status": back, "snoozed_until": null, "snoozed_from": null}))).await?;
    }
    Ok(())
}
