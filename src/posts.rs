//! Posts and publications. A post goes from its brand to one or more of the brand's active accounts, as the original or
//! as each account's own version (content.rs). Each copy must fit what the account's network takes and answer what it
//! asks (rules.rs), and the account's connection must offer what the copy needs (`publish`, `publish_media` for media,
//! `reply` for follow-ups that are comments). Review: review.rs; when it goes: when.rs.
//!
//! At its time, upkeep makes one publication per account and queues one job for each, keyed by the publication. The job
//! sends the copy item by item: each item's media first (`publish_media`), then the item (`publish`, or `reply` for a
//! comment), each under a key of its own, and keeps each item's network id and address (`sent_items`) as it goes, in a
//! transaction of its own. What makes that once per account and per item, under retries and two workers:
//! * the job is deduplicated by the publication while queued or running, and a job is claimed by one worker at a time;
//! * each item is sent holding a lock on the publication's row, and `sent_items` is read after taking it, so a second
//!   job for the same publication waits, then finds the item out;
//! * each call carries its key: `publish:<publication>` for the first item (0.7.0's key, so a publication under way
//!   across the upgrade is not sent twice) and `publish:<publication>:<n>` for item n after it, `media:<publication>`
//!   for the first item's first medium and `media:<publication>:<item>:<n>` for the others. The platform hands a key
//!   that went through its stored answer without calling the network, and gives the network the same key on every
//!   attempt in the description's idempotency header.
//!
//! A refusal ends the attempt at once and the service's own words are shown on the publication; "later" (a timeout, a
//! limit, a 5xx) is tried again by the job with backoff. A thread that failed half way resumes where it stopped: the
//! items out are in `sent_items` and are not sent again. An item with a delay waits it after the one before, the job
//! put back for then. The post is published when every publication is out, failed when any one ended without being.

use crate::accounts::{self, Reach, any_profile, caller, reach};
use crate::content::{self, Med, Version, fingerprint};
use crate::rules::{self, FollowAs, Rules};
use crate::{at, base64, obj};
use async_trait::async_trait;
use chrono::{DateTime, Duration, SecondsFormat, Utc};
use ferrum_domain::util::{elevated, id, rows, s};
use ferrum_kernel::connections::{self, Call};
use ferrum_kernel::jobs::{Job, JobHandler, NewJob};
use ferrum_kernel::{Ctx, Doc, DocHook, Error, Filter, HookCtx, Kernel, Result, Tx};
use serde_json::{Value, json};
use std::time::Duration as Wait;
use uuid::Uuid;

/// A publication whose job has left no trace, and that is this old, is given up on: the list of
/// jobs is cleaned after a while, and a send takes far less than this.
const STALE: Duration = Duration::hours(24);

/// Attempts at one publication while the network says "later".
const ATTEMPTS: i32 = 8;

pub(crate) const PUBLISH: &str = "social.publish";

/// What an account's network takes and asks: what its connection's description says (`meta.social`, or 0.7.0's
/// `meta.capabilities`, which is where a server's own limit lives), else the network profile the account names (its
/// `social` shape, else its fields; the seeded ones are the defaults).
pub(crate) async fn rules_of(k: &Kernel, ctx: &Ctx, tx: &mut Tx, account: &Doc) -> Result<Option<Rules>> {
    if let Some(found) = reach(tx, ctx.tenant, s(account, "connection")).await?
        && let Some(r) = found.social(s(account, "network"))?
    {
        return Ok(Some(r));
    }
    let Some(profile) = id(account, "profile") else { return Ok(None) };
    let p = k.get_tx(&elevated(ctx), tx, "SocialProfile", profile, false).await?;
    let name = s(&p, "profile_name").unwrap_or("This network");
    match p.get("social").filter(|v| v.is_object()) {
        Some(v) => Rules::parse(v, name).map(Some).map_err(|e| Error::invalid(format!("the network profile {name}: {e}"))),
        None => Ok(Some(Rules::of_fields(&p, name))),
    }
}

/// How a copy's follow-ups go out on a network with these rules.
fn follow(r: &Rules, v: &Version) -> FollowAs {
    let kind = r.kind(v.kind.as_deref()).ok().flatten();
    r.effective(kind).2.as_
}

/// Why account `a`'s copy of `doc` cannot go, in plain words naming the account; `None` when it can. Also what its
/// connection must offer for it.
pub(crate) async fn unfit(k: &Kernel, ctx: &Ctx, tx: &mut Tx, doc: &Doc, a: &Doc, any_profiles: bool) -> Result<Option<(String, String)>> {
    let name = s(a, "account_name").unwrap_or_default().to_string();
    let v = content::version(k, ctx, tx, doc, a).await.map_err(|e| match e {
        Error::Validation(f) => Error::Validation(
            f.into_iter()
                .map(|mut f| {
                    f.message = format!("for {name}: {}", f.message);
                    f
                })
                .collect(),
        ),
        other => other,
    })?;
    let r = match rules_of(k, ctx, tx, a).await? {
        Some(r) => r,
        // A workspace not yet updated has no profiles: nothing says what its networks take.
        None if any_profiles => {
            return Ok(Some(("targets".into(), format!("{name} has no network profile, so it is not known what its network takes"))));
        }
        None => {
            let open = Rules::default();
            return Ok(conn_misses(tx, ctx, a, &open, &v, &name).await?.map(|why| ("targets".into(), why)));
        }
    };
    if let Some(why) = rules::problems(&r, &v.copy()).into_iter().next() {
        return Ok(Some(("text".into(), format!("for {name}: {why}"))));
    }
    Ok(conn_misses(tx, ctx, a, &r, &v, &name).await?.map(|why| (if why.contains("pictures") { "image" } else { "targets" }.into(), why)))
}

/// What account `a`'s connection lacks for copy `v`.
async fn conn_misses(tx: &mut Tx, ctx: &Ctx, a: &Doc, r: &Rules, v: &Version, name: &str) -> Result<Option<String>> {
    let conn = reach(tx, ctx.tenant, s(a, "connection")).await?;
    let offers = |op: &str| conn.as_ref().is_some_and(|c| c.has(r.op(op)));
    if !offers("publish") {
        return Ok(Some(format!("{name}'s connection cannot publish posts")));
    }
    if v.items.iter().any(|i| !i.media.is_empty()) && !offers("publish_media") {
        return Ok(Some(format!("{name}'s connection cannot publish pictures")));
    }
    if v.items.len() > 1 && follow(r, v) == FollowAs::Comment && !offers("reply") {
        return Ok(Some(format!("{name}'s connection cannot add comments")));
    }
    Ok(None)
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
        let web = |l: &str| (l.starts_with("https://") || l.starts_with("http://")) && !l.contains(char::is_whitespace);
        if s(doc, "link").map(str::trim).is_some_and(|l| !l.is_empty() && !web(l)) {
            return Err(Error::field("link", "is a web address"));
        }
        let image = s(doc, "image").filter(|i| !i.trim().is_empty());
        if image.is_some_and(|i| !i.starts_with("/files/")) {
            return Err(Error::field("image", "is a picture stored here"));
        }
        if image.is_some() && s(doc, "image_alt").is_none_or(|a| a.trim().is_empty()) {
            return Err(Error::field("image_alt", "says what the picture shows, for people who cannot see it"));
        }
        for line in rows(doc, "media") {
            if s(&line, "file").is_none_or(|f| !f.starts_with("/files/")) || s(&line, "thumbnail").is_some_and(|f| !f.is_empty() && !f.starts_with("/files/")) {
                return Err(Error::field("media", "each medium is a file stored here"));
            }
        }
        if rows(doc, "items").iter().chain(rows(doc, "targets").iter()).any(|l| s(l, "link").map(str::trim).is_some_and(|l| !l.is_empty() && !web(l))) {
            return Err(Error::field("items", "a link is a web address"));
        }
        let targets = content::accounts(doc);
        for line in rows(doc, "items").iter().chain(rows(doc, "media").iter()) {
            if let Some(a) = id(line, "account").filter(|a| !targets.contains(a)) {
                let named = hc.kernel.get_tx(&system, hc.tx, "SocialAccount", a, false).await?;
                return Err(Error::field(
                    "items",
                    format!("{} has a version here but the post does not go to it", s(&named, "account_name").unwrap_or("an account")),
                ));
            }
        }
        let profiles = any_profile(hc).await?;
        for account in targets {
            let a = hc.kernel.get_tx(&system, hc.tx, "SocialAccount", account, false).await?;
            let name = s(&a, "account_name").unwrap_or_default();
            if id(&a, "brand") != brand {
                return Err(Error::field("targets", format!("{name} is another brand's")));
            }
            if !a.get("active").and_then(Value::as_bool).unwrap_or(true) {
                return Err(Error::field("targets", format!("{name} is paused")));
            }
            if let Some((field, why)) = unfit(hc.kernel, hc.ctx, hc.tx, doc, &a, profiles).await? {
                return Err(Error::field(&field, why));
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
        if let Some(out) = crate::review::action(hc, name, doc, params).await? {
            return Ok(Some(out));
        }
        if let Some(out) = crate::when::action(hc, name, doc, params).await? {
            return Ok(Some(out));
        }
        if let Some(out) = crate::library::post_action(hc, name, doc, params).await? {
            return Ok(Some(out));
        }
        if name != "retry" {
            return Ok(None);
        }
        // Only to the accounts it did not reach, and only what did not go: the rest is out already.
        let post = id(doc, "id").ok_or_else(|| Error::internal("a post without an id"))?;
        let system = elevated(hc.ctx);
        let own = [Filter::eq("post", post.to_string())];
        for p in hc.kernel.find_tx(&system, hc.tx, "SocialPublication", &own, None, 100).await? {
            let (Some(publication), true) = (id(&p, "id"), at(&p, "sent_at").is_none()) else { continue };
            hc.kernel.set_values_tx(&system, hc.tx, "SocialPublication", publication, obj(json!({"failed_at": null, "failure": null}))).await?;
            queue(hc.tx, hc.ctx.tenant, publication).await?;
        }
        hc.kernel.set_values_tx(&system, hc.tx, "SocialPost", post, obj(json!({"status": "Sending", "failure": null}))).await?;
        Ok(Some(json!({"status": "Sending"})))
    }
}

/// One job for a publication: queued once while one is waiting or running.
pub(crate) async fn queue(tx: &mut Tx, tenant: Uuid, publication: Uuid) -> Result<()> {
    let mut job = NewJob::new(PUBLISH, json!({"publication": publication}));
    job.max_attempts = ATTEMPTS;
    job.dedupe_key = Some(format!("publish:{publication}"));
    ferrum_kernel::jobs::enqueue(tx, tenant, None, job).await?;
    Ok(())
}

/// The items a publication has out: `{item, id, url, media, sent_at}` each, 0.7.0's single one read from its fields.
fn sent_items(p: &Doc) -> Vec<Value> {
    match p.get("sent_items").and_then(Value::as_array) {
        Some(list) => list.clone(),
        None => match s(p, "external_id") {
            Some(id) => vec![
                json!({"item": 0, "id": id, "url": s(p, "permalink"), "media": s(p, "media_ref").into_iter().collect::<Vec<_>>(), "sent_at": s(p, "sent_at")}),
            ],
            None => vec![],
        },
    }
}

/// The key of item `n`'s call, and of its medium `m`.
fn key(publication: Uuid, what: &str, item: usize, medium: Option<usize>) -> String {
    match (item, medium) {
        (0, None) | (0, Some(0)) => format!("{what}:{publication}"),
        (n, None) => format!("{what}:{publication}:{n}"),
        (n, Some(m)) => format!("{what}:{publication}:{n}:{m}"),
    }
}

pub struct PublicationHook;

#[async_trait]
impl DocHook for PublicationHook {
    fn name(&self) -> &'static str {
        "social.publication.actions"
    }

    /// `delete_remote`: every item taken down on the network, last first, where the connection offers `delete`.
    async fn action(&self, hc: &mut HookCtx<'_>, name: &str, doc: &Doc, _params: &Value) -> Result<Option<Value>> {
        if name != "delete_remote" {
            return Ok(None);
        }
        let publication = id(doc, "id").ok_or_else(|| Error::internal("a publication without an id"))?;
        let items = sent_items(doc);
        if items.is_empty() {
            return Err(Error::invalid("it is not out on the network"));
        }
        let system = elevated(hc.ctx);
        let account = hc.kernel.get_tx(&system, hc.tx, "SocialAccount", id(doc, "account").unwrap_or_default(), false).await?;
        let conn = reach(hc.tx, hc.ctx.tenant, s(&account, "connection")).await?.ok_or_else(|| Error::invalid("the account's connection is gone"))?;
        let r = rules_of(hc.kernel, hc.ctx, hc.tx, &account).await?.unwrap_or_default();
        let op = r.op("delete").to_string();
        if !conn.has(&op) {
            return Err(Error::invalid(format!("{} does not offer to delete a post", conn.name)));
        }
        let calling = caller(hc.kernel, hc.tx, hc.ctx.tenant, &account, &conn).await?;
        let follows = s(doc, "follow_ups").map(str::to_string);
        for it in items.iter().rev() {
            let (Some(n), Some(external)) = (it["item"].as_u64(), text_of(&it["id"])) else { continue };
            let n = n as usize;
            let inputs = json!({"id": external, "item": n, "follow_up": if n > 0 { follows.clone() } else { None }, "root": text_of(&items[0]["id"])});
            let call = Call {
                connection: conn.id.to_string(),
                operation: op.clone(),
                inputs: conn.fit(&op, inputs),
                key: Some(key(publication, "delete", n, None)),
                entity: Some("SocialPublication".into()),
                record: Some(publication),
            };
            connections::call(hc.kernel, &calling, Some(crate::APP), &call).await.map_err(crate::said)?;
        }
        hc.kernel.set_values_tx(&system, hc.tx, "SocialPublication", publication, obj(json!({"deleted_at": Utc::now().to_rfc3339()}))).await?;
        Ok(Some(json!({"deleted": items.len()})))
    }
}

/// Starts sending `post` (`doc`, its row locked) now: one publication per account and one queued send for each, or
/// says what it waits for. An account the connection cannot reach yet (not connected, its sign-in refused, or switched
/// off) is waited for, saying which and why, instead of sending the post to fail: it goes when every account it names
/// can be reached.
pub(crate) async fn start(k: &Kernel, ctx: &Ctx, tx: &mut Tx, post: Uuid, doc: &Doc) -> Result<bool> {
    let mut waiting = Vec::new();
    for account in content::accounts(doc) {
        let a = k.get_tx(ctx, tx, "SocialAccount", account, false).await?;
        let (state, note, _) = accounts::state_of(tx, ctx.tenant, &a).await?;
        if state != "Connected" {
            waiting.push(format!("{}: {state}{}", s(&a, "account_name").unwrap_or("an account"), note.map(|n| format!(": {n}")).unwrap_or_default()));
        }
    }
    if !waiting.is_empty() {
        let why = waiting.join("\n");
        if s(doc, "waiting_for") != Some(why.as_str()) {
            k.set_values_tx(ctx, tx, "SocialPost", post, obj(json!({"waiting_for": why}))).await?;
        }
        return Ok(false);
    }
    for account in content::accounts(doc) {
        let made = k
            .insert_tx(
                ctx,
                tx,
                "SocialPublication",
                obj(json!({"post": post.to_string(), "account": account.to_string(), "text": s(doc, "text"), "link": s(doc, "link"),
                           "image": s(doc, "image"), "image_alt": s(doc, "image_alt")})),
            )
            .await?;
        queue(tx, ctx.tenant, id(&made, "id").unwrap_or_default()).await?;
    }
    k.set_values_tx(ctx, tx, "SocialPost", post, obj(json!({"status": "Sending", "waiting_for": null}))).await?;
    Ok(true)
}

/// Approved posts whose time has come, started. Each is locked and looked at again first, so two upkeeps at once start
/// it once.
pub(crate) async fn send_due(k: &Kernel, ctx: &Ctx, tx: &mut Tx, now: DateTime<Utc>) -> Result<()> {
    for found in k.find_tx(ctx, tx, "SocialPost", &[Filter::eq("status", "Approved")], Some("scheduled_at asc"), 500).await? {
        if at(&found, "scheduled_at").is_none_or(|t| t > now) {
            continue;
        }
        let post = id(&found, "id").unwrap_or_default();
        let doc = k.get_tx(ctx, tx, "SocialPost", post, true).await?;
        if s(&doc, "status") != Some("Approved") {
            continue;
        }
        start(k, ctx, tx, post, &doc).await?;
    }
    Ok(())
}

/// The text of a mapped value a network gave as its id: a string or a number.
pub(crate) fn text_of(v: &Value) -> Option<String> {
    match v {
        Value::String(t) if !t.trim().is_empty() => Some(t.trim().to_string()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

pub struct PublishJob;

#[async_trait]
impl JobHandler for PublishJob {
    fn kind(&self) -> &'static str {
        PUBLISH
    }

    fn title(&self) -> &'static str {
        "A post sent to an account"
    }

    async fn run(&self, k: &Kernel, job: &Job) -> Result<Value> {
        let publication: Uuid = job.payload["publication"].as_str().and_then(|p| p.parse().ok()).ok_or_else(|| Error::invalid("names its publication"))?;
        // Bounded: a step either sends an item, waits or ends, and a post has far fewer items than this.
        for _ in 0..100 {
            match step(k, job.tenant_id, publication).await {
                Ok(Step::Sent) => continue,
                Ok(Step::Done(v)) => return Ok(v),
                // An item's delay: the job is put back for then, which costs no attempt.
                Ok(Step::Wait(secs)) => return Err(Error::Wait(format!("the next item goes in {secs} seconds"), secs)),
                Err(e) => {
                    // Said on the publication as it happens: a refusal ends it, "later" is shown as waiting.
                    let last = !matches!(e, Error::Unavailable(_) | Error::Wait(..) | Error::RetryAfter(..)) || job.attempts >= job.max_attempts;
                    let _ = fail(k, job.tenant_id, publication, &e.to_string(), last).await;
                    return Err(e);
                }
            }
        }
        Err(Error::internal("a publication's items did not settle"))
    }

    fn timeout(&self) -> Wait {
        Wait::from_secs(180)
    }
}

/// What the service said, on the publication.
async fn fail(k: &Kernel, tenant: Uuid, publication: Uuid, why: &str, last: bool) -> Result<()> {
    let ctx = Ctx::system(tenant, k.registry.schema(tenant).await?);
    let mut tx = k.db.tenant_tx(tenant).await?;
    let p = k.get_tx(&ctx, &mut tx, "SocialPublication", publication, true).await?;
    if at(&p, "sent_at").is_none() {
        let values =
            if last { json!({"failed_at": Utc::now().to_rfc3339(), "failure": why}) } else { json!({"failure": format!("Waiting to try again: {why}")}) };
        k.set_values_tx(&ctx, &mut tx, "SocialPublication", publication, obj(values)).await?;
        if let Ok(account) = k.get_tx(&ctx, &mut tx, "SocialAccount", id(&p, "account").unwrap_or_default(), false).await {
            // A refused sign-in shows on the account at once.
            let _ = accounts::refresh(k, &ctx, &mut tx, &account).await;
        }
    }
    tx.commit().await?;
    Ok(())
}

enum Step {
    /// An item went out; the next is looked at.
    Sent,
    /// The next item waits this many seconds.
    Wait(u64),
    Done(Value),
}

/// The inputs of a medium's upload.
#[allow(clippy::too_many_arguments)]
async fn media_inputs(k: &Kernel, ctx: &Ctx, conn: &Reach, op: &str, r: &Rules, m: &Med, item: usize, index: usize, extra: &Value) -> Result<Value> {
    // `file` is the stored file's id, for a description that sends it as a part or as the body; its bytes as base64 only
    // for one that says it takes `data` (a network that wants it inside JSON).
    let mut inputs = json!({"file": m.file, "name": m.name, "content_type": m.content_type, "alt": if r.media.alt { m.alt.clone().unwrap_or_default() } else { String::new() },
                            "size": m.bytes, "kind": m.kind, "thumbnail": if r.media.thumbnail { m.thumbnail } else { None }, "width": m.width,
                            "height": m.height, "seconds": m.seconds, "item": item, "index": index});
    for (k2, v) in extra.as_object().into_iter().flatten() {
        inputs[k2] = v.clone();
    }
    let declared = conn.inputs(op);
    if declared.is_empty() || declared.iter().any(|d| d == "data") {
        let (_, _, bytes) = k.read_file(&elevated(ctx), m.file).await?;
        inputs["data"] = json!(base64(&bytes));
    }
    Ok(inputs)
}

/// Sends the publication's next item that is not out, holding its row: one item a transaction.
async fn step(k: &Kernel, tenant: Uuid, publication: Uuid) -> Result<Step> {
    let ctx = Ctx::system(tenant, k.registry.schema(tenant).await?);
    let mut tx = k.db.tenant_tx(tenant).await?;
    // The lock that makes a second send of the same publication wait, and then find what is out.
    let p = k.get_tx(&ctx, &mut tx, "SocialPublication", publication, true).await?;
    if at(&p, "sent_at").is_some() {
        tx.commit().await?;
        return Ok(Step::Done(json!({"done": "already out"})));
    }
    let account = k.get_tx(&ctx, &mut tx, "SocialAccount", id(&p, "account").unwrap_or_default(), false).await?;
    let name = s(&account, "account_name").unwrap_or("the account").to_string();
    let post = k.get_tx(&ctx, &mut tx, "SocialPost", id(&p, "post").unwrap_or_default(), false).await?;
    let conn: Reach =
        reach(&mut tx, tenant, s(&account, "connection")).await?.ok_or_else(|| Error::invalid(format!("{name} has no connection here: connect it again")))?;
    if !conn.enabled {
        return Err(Error::invalid(format!("the connection {} is turned off", conn.name)));
    }
    // What its network takes may have been changed since the post was approved: looked at again.
    let r = rules_of(k, &ctx, &mut tx, &account).await?.ok_or_else(|| Error::invalid(format!("{name} has no network profile")))?;
    let v = content::version(k, &ctx, &mut tx, &post, &account).await?;
    if let Some(why) = rules::problems(&r, &v.copy()).into_iter().next() {
        return Err(Error::invalid(format!("for {name}: {why}")));
    }
    let kind = r.kind(v.kind.as_deref()).ok().flatten().map(|k| k.name.clone());
    let settings = rules::answers(&r, kind.as_deref(), &v.settings).map_err(|w| Error::invalid(format!("for {name}: {}", w.join("; "))))?;
    let calling = caller(k, &mut tx, tenant, &account, &conn).await?;
    let mut sent = sent_items(&p);
    let Some(n) = (0..v.items.len()).find(|n| !sent.iter().any(|s| s["item"].as_u64() == Some(*n as u64))) else {
        return finish(k, &ctx, tx, publication, &post, &r, &conn, &calling, &sent).await.map(Step::Done);
    };
    let item = &v.items[n];
    if n > 0 && item.delay > 0 {
        let before = sent
            .iter()
            .find(|s| s["item"].as_u64() == Some(n as u64 - 1))
            .and_then(|s| s["sent_at"].as_str())
            .and_then(|t| DateTime::parse_from_rfc3339(t).ok());
        let due = before.map(|b| b.with_timezone(&Utc) + Duration::minutes(item.delay));
        if let Some(due) = due.filter(|d| *d > Utc::now()) {
            k.set_values_tx(&ctx, &mut tx, "SocialPublication", publication, obj(json!({"next_at": due.to_rfc3339()}))).await?;
            tx.commit().await?;
            return Ok(Step::Wait((due - Utc::now()).num_seconds().max(1) as u64));
        }
    }
    let as_ = follow(&r, &v);
    let op = if n == 0 || as_ == FollowAs::Thread { "publish" } else { "reply" };
    let root = sent.first().map(|s| s["id"].clone()).unwrap_or(Value::Null);
    let before = sent.last().cloned().unwrap_or(Value::Null);
    let common = json!({"kind": kind, "settings": settings, "account_id": s(&account, "provider_account"), "handle": s(&account, "handle"), "item": n, "items": v.items.len()});
    let call = |operation: &str, inputs: Value, key: String| Call {
        connection: conn.id.to_string(),
        operation: r.op(operation).into(),
        inputs: conn.fit(r.op(operation), inputs),
        key: Some(key),
        entity: Some("SocialPublication".into()),
        record: Some(publication),
    };
    // Each medium first, under a key of its own; 0.7.0 kept the first item's picture in `media_ref`.
    let mut media: Vec<String> = vec![];
    for (i, m) in item.media.iter().enumerate() {
        if n == 0
            && i == 0
            && let Some(kept) = s(&p, "media_ref")
        {
            media.push(kept.to_string());
            continue;
        }
        let inputs = media_inputs(k, &ctx, &conn, r.op("publish_media"), &r, m, n, i, &common).await?;
        let up = connections::call(k, &calling, Some(crate::APP), &call("publish_media", inputs, key(publication, "media", n, Some(i))))
            .await
            .map_err(crate::said)?;
        media.push(text_of(&up["id"]).ok_or_else(|| Error::invalid("the connection's publish_media gives no id: its description maps one under result"))?);
    }
    let first = item.media.first();
    let link = item.link.as_deref();
    // Stable across attempts: a call's inputs under one key must be the same each time.
    let created = at(&p, "created_at").unwrap_or_else(Utc::now).to_rfc3339_opts(SecondsFormat::Millis, true);
    let described: Vec<Value> = item
        .media
        .iter()
        .zip(&media)
        .map(|(m, id)| json!({"id": id, "kind": m.kind, "alt": m.alt, "content_type": m.content_type, "size": m.bytes, "width": m.width, "height": m.height, "seconds": m.seconds}))
        .collect();
    let mut inputs = json!({"text": item.text, "link": link, "text_with_link": link.map_or_else(|| item.text.clone(), |l| format!("{} {l}", item.text)),
                            "media": media, "media_id": media.first(), "media_items": described, "alt": first.and_then(|m| m.alt.clone()), "created_at": created,
                            "picture_type": first.map(|m| m.content_type.clone()), "picture_size": first.map(|m| m.bytes),
                            "reply_to": text_of(&before["id"]), "root": text_of(&root), "previous": before, "in_reply_to": text_of(&root)});
    for (k2, val) in common.as_object().into_iter().flatten() {
        inputs[k2] = val.clone();
    }
    let out = connections::call(k, &calling, Some(crate::APP), &call(op, inputs, key(publication, "publish", n, None))).await.map_err(crate::said)?;
    let external = text_of(&out["id"]).ok_or_else(|| Error::invalid(format!("the connection's {op} gives no id: its description maps one under result")))?;
    let mut answer = out.as_object().cloned().unwrap_or_default();
    answer.remove("id");
    answer.remove("url");
    sent.push(json!({"item": n, "id": external, "url": text_of(&out["url"]), "media": media, "sent_at": Utc::now().to_rfc3339(), "answer": answer}));
    let mut values = json!({"sent_items": sent, "next_at": null, "failure": null, "follow_ups": if v.items.len() > 1 { Some(format!("{as_:?}").to_lowercase()) } else { None }});
    if n == 0 {
        values["external_id"] = json!(external);
        values["permalink"] = json!(text_of(&out["url"]));
        values["media_ref"] = json!(media.first());
    }
    k.set_values_tx(&ctx, &mut tx, "SocialPublication", publication, obj(values)).await?;
    tx.commit().await?;
    Ok(Step::Sent)
}

/// Every item out: the first comment where the network takes one and the connection can reply, and the publication
/// marked out.
#[allow(clippy::too_many_arguments)]
async fn finish(k: &Kernel, ctx: &Ctx, mut tx: Tx, publication: Uuid, post: &Doc, r: &Rules, conn: &Reach, calling: &Ctx, sent: &[Value]) -> Result<Value> {
    let external = sent.first().and_then(|s| text_of(&s["id"])).unwrap_or_default();
    let mut values = json!({"sent_at": Utc::now().to_rfc3339(), "failed_at": null, "failure": null, "next_at": null});
    let comment = s(post, "first_comment").filter(|c| !c.trim().is_empty());
    if let (Some(comment), true, true) = (comment, r.first_comment, conn.has(r.op("reply"))) {
        let inputs = json!({"text": comment, "in_reply_to": external, "root": external});
        let call = Call {
            connection: conn.id.to_string(),
            operation: r.op("reply").into(),
            inputs: conn.fit(r.op("reply"), inputs),
            key: Some(format!("first_comment:{publication}")),
            entity: Some("SocialPublication".into()),
            record: Some(publication),
        };
        if let Err(e) = connections::call(k, calling, Some(crate::APP), &call).await {
            values["failure"] = json!(format!("It is out, but its first comment was not: {}", crate::said(e)));
        }
    }
    k.set_values_tx(ctx, &mut tx, "SocialPublication", publication, obj(values)).await?;
    tx.commit().await?;
    Ok(json!({"id": external, "items": sent.len()}))
}

/// Posts all of whose publications are settled: published, or failed when any publication ended without going out. A
/// publication whose job ended without saying so, or left no trace for a day, is marked failed too. Each post is locked
/// and looked at again first, so two upkeeps at once settle it once (and a repeat makes its next occurrence once).
pub(crate) async fn mark_sent(k: &Kernel, ctx: &Ctx, tx: &mut Tx, now: DateTime<Utc>) -> Result<()> {
    for found in k.find_tx(ctx, tx, "SocialPost", &[Filter::eq("status", "Sending")], None, 500).await? {
        let post = id(&found, "id").unwrap_or_default();
        let mut pending = false;
        let mut failures = Vec::new();
        for p in k.find_tx(ctx, tx, "SocialPublication", &[Filter::eq("post", post.to_string())], None, 100).await? {
            let publication = id(&p, "id").unwrap_or_default();
            if at(&p, "sent_at").is_some() {
                continue;
            }
            let account = k.get_tx(ctx, tx, "SocialAccount", id(&p, "account").unwrap_or_default(), false).await?;
            let name = s(&account, "account_name").unwrap_or("an account");
            if at(&p, "failed_at").is_some() {
                failures.push(format!("{name}: {}", s(&p, "failure").unwrap_or("it did not go out")));
                continue;
            }
            // ponytail: reads the platform's job table by the publication's key; the platform has no call for a job by key.
            let job: Option<(String, Option<String>)> = sqlx::query_as(
                "select status::text, last_error from job where tenant_id = $1 and kind = $2 and dedupe_key = $3 order by created_at desc limit 1",
            )
            .bind(ctx.tenant)
            .bind(PUBLISH)
            .bind(format!("publish:{publication}"))
            .fetch_optional(&mut **tx)
            .await?;
            let why = match job {
                Some((status, _)) if status == "queued" || status == "running" => None,
                Some((_, error)) => Some(error.unwrap_or_else(|| "the network gave no id for it".into())),
                None if at(&p, "created_at").is_some_and(|c| now - c > STALE) => Some("sending it left no trace and it never went out".into()),
                None => None,
            };
            match why {
                None => pending = true,
                Some(why) => {
                    k.set_values_tx(ctx, tx, "SocialPublication", publication, obj(json!({"failed_at": Utc::now().to_rfc3339(), "failure": why}))).await?;
                    failures.push(format!("{name}: {why}"));
                }
            }
        }
        if pending {
            continue;
        }
        let doc = k.get_tx(ctx, tx, "SocialPost", post, true).await?;
        if s(&doc, "status") != Some("Sending") {
            continue;
        }
        let values = if failures.is_empty() {
            json!({"status": "Published", "published_at": Utc::now().to_rfc3339()})
        } else {
            json!({"status": "Failed", "failure": failures.join("\n")})
        };
        k.set_values_tx(ctx, tx, "SocialPost", post, obj(values)).await?;
        if failures.is_empty() {
            crate::when::repeat(k, ctx, tx, &doc).await?;
        }
    }
    Ok(())
}
