//! Posts and publications. A post goes from its brand to one or more of the brand's active
//! accounts. It must fit what each account's network profile says, and each account's connection
//! must offer what the post needs (`publish`, and `publish_media` for a picture). Someone other than
//! its author approves it; one sent back says why, and a post changed after its approval needs
//! approving again.
//!
//! At its time, upkeep makes one publication per account and queues one job for each, keyed by the
//! publication. The job sends the picture first when the connection says how (`publish_media`),
//! then the post (`publish`), each under a key of its own, and keeps the network's id and address.
//! What makes that once per account, under retries and two workers:
//! * the job is deduplicated by the publication while queued or running, and a job is claimed by
//!   one worker at a time;
//! * the job holds a lock on the publication's row for the length of its attempt and looks at
//!   `external_id` after taking it, so a second job for the same publication waits, then finds it out;
//! * each call carries the key `publish:<publication>` (and `media:<publication>`): the platform
//!   hands a key that went through its stored answer without calling the network, and gives the
//!   network the same key on every attempt in the description's idempotency header.
//!
//! A refusal ends the attempt at once and the service's own words are shown on the publication;
//! "later" (a timeout, a limit, a 5xx) is tried again by the job with backoff. The post is
//! published when every publication is out, failed when any one ended without being.

use crate::accounts::{self, Reach, any_profile, caller, reach};
use crate::rules::{self, Copy, Item, Piece, Rules};
use crate::{at, base64, obj};
use async_trait::async_trait;
use chrono::{DateTime, Duration, SecondsFormat, Utc};
use ferrum_domain::util::{elevated, id, rows, s};
use ferrum_kernel::connections::{self, Call};
use ferrum_kernel::jobs::{Job, JobHandler, NewJob};
use ferrum_kernel::{Ctx, Doc, DocHook, Error, Filter, HookCtx, Kernel, Result, Tx};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::time::Duration as Wait;
use uuid::Uuid;

/// A publication whose job has left no trace, and that is this old, is given up on: the list of
/// jobs is cleaned after a while, and a send takes far less than this.
const STALE: Duration = Duration::hours(24);

/// Attempts at one publication while the network says "later".
const ATTEMPTS: i32 = 8;

pub(crate) const PUBLISH: &str = "social.publish";

/// What approval fixes: the words, the link, the picture and what it says of it, where and when it
/// goes.
fn fingerprint(doc: &Doc) -> String {
    let mut targets: Vec<String> = rows(doc, "targets").iter().filter_map(|t| s(t, "account").map(str::to_string)).collect();
    targets.sort();
    let what = json!([
        s(doc, "text"),
        s(doc, "link"),
        targets,
        at(doc, "scheduled_at").map(|t| t.timestamp()),
        s(doc, "image"),
        s(doc, "image_alt"),
        s(doc, "first_comment")
    ]);
    hex::encode(Sha256::digest(what.to_string()))
}

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

/// The post's own content as one account's copy (0.7.0's: one text, a link, a picture, a first comment).
async fn copy_of(k: &Kernel, ctx: &Ctx, doc: &Doc) -> Result<Copy> {
    let mut media = vec![];
    if let Some(file) = s(doc, "image").and_then(|i| i.strip_prefix("/files/")).and_then(|f| f.parse::<Uuid>().ok()) {
        let info = k.file_info(&elevated(ctx), file).await?;
        media.push(Piece {
            kind: rules::kind_of(&info.content_type).into(),
            content_type: Some(info.content_type),
            bytes: Some(info.size),
            alt: s(doc, "image_alt").map(str::to_string),
            ..Default::default()
        });
    }
    let link = s(doc, "link").filter(|l| !l.trim().is_empty()).map(str::to_string);
    Ok(Copy {
        items: vec![Item { text: s(doc, "text").unwrap_or_default().to_string(), link, media }],
        first_comment: s(doc, "first_comment").map(str::to_string),
        ..Default::default()
    })
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
        let copy = copy_of(hc.kernel, hc.ctx, doc).await?;
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
            match rules_of(hc.kernel, hc.ctx, hc.tx, &a).await? {
                Some(r) => {
                    if let Some(why) = rules::problems(&r, &copy).into_iter().next() {
                        return Err(Error::field("text", format!("for {name}: {why}")));
                    }
                }
                // A workspace not yet updated has no profiles: nothing says what its networks take.
                None if any_profile(hc).await? => {
                    return Err(Error::field("targets", format!("{name} has no network profile, so it is not known what its network takes")));
                }
                None => {}
            }
            let conn = reach(hc.tx, hc.ctx.tenant, s(&a, "connection")).await?;
            if !conn.as_ref().is_some_and(|c| c.has("publish")) {
                return Err(Error::field("targets", format!("{name}'s connection cannot publish posts")));
            }
            if image.is_some() && !conn.as_ref().is_some_and(|c| c.has("publish_media")) {
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
                    hc.kernel.set_values_tx(&system, hc.tx, "SocialPublication", publication, obj(json!({"failed_at": null, "failure": null}))).await?;
                    queue(hc.tx, hc.ctx.tenant, publication).await?;
                }
                json!({"status": "Sending", "failure": null})
            }
            _ => return Ok(None),
        };
        hc.kernel.set_values_tx(&elevated(hc.ctx), hc.tx, "SocialPost", post, obj(values.clone())).await?;
        Ok(Some(json!({"status": values["status"]})))
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

pub struct PublicationHook;

#[async_trait]
impl DocHook for PublicationHook {
    fn name(&self) -> &'static str {
        "social.publication.actions"
    }

    /// `delete_remote`: the post taken down on the network, where the connection offers `delete`.
    async fn action(&self, hc: &mut HookCtx<'_>, name: &str, doc: &Doc, _params: &Value) -> Result<Option<Value>> {
        if name != "delete_remote" {
            return Ok(None);
        }
        let publication = id(doc, "id").ok_or_else(|| Error::internal("a publication without an id"))?;
        let external = s(doc, "external_id").ok_or_else(|| Error::invalid("it is not out on the network"))?;
        let system = elevated(hc.ctx);
        let account = hc.kernel.get_tx(&system, hc.tx, "SocialAccount", id(doc, "account").unwrap_or_default(), false).await?;
        let conn = reach(hc.tx, hc.ctx.tenant, s(&account, "connection")).await?.ok_or_else(|| Error::invalid("the account's connection is gone"))?;
        if !conn.has("delete") {
            return Err(Error::invalid(format!("{} does not offer to delete a post", conn.name)));
        }
        let calling = caller(hc.kernel, hc.tx, hc.ctx.tenant, &account, &conn).await?;
        let call = Call {
            connection: conn.id.to_string(),
            operation: "delete".into(),
            inputs: conn.fit("delete", json!({"id": external})),
            key: Some(format!("delete:{publication}")),
            entity: Some("SocialPublication".into()),
            record: Some(publication),
        };
        connections::call(hc.kernel, &calling, Some(crate::APP), &call).await.map_err(crate::said)?;
        hc.kernel.set_values_tx(&system, hc.tx, "SocialPublication", publication, obj(json!({"deleted_at": Utc::now().to_rfc3339()}))).await?;
        Ok(Some(json!({"deleted": true})))
    }
}

/// Approved posts whose time has come: one publication per account, and one queued send for each.
pub(crate) async fn send_due(k: &Kernel, ctx: &Ctx, tx: &mut Tx, now: DateTime<Utc>) -> Result<()> {
    for found in k.find_tx(ctx, tx, "SocialPost", &[Filter::eq("status", "Approved")], Some("scheduled_at asc"), 500).await? {
        if at(&found, "scheduled_at").is_none_or(|t| t > now) {
            continue;
        }
        let post = id(&found, "id").unwrap_or_default();
        let doc = k.get_tx(ctx, tx, "SocialPost", post, false).await?;
        // An account the connection cannot reach yet (not connected, its sign-in refused, or switched off) is waited for,
        // saying which and why, instead of sending the post to fail: it goes when every account it names can be reached.
        let mut waiting = Vec::new();
        for target in rows(&doc, "targets") {
            let Some(account) = id(&target, "account") else { continue };
            let a = k.get_tx(ctx, tx, "SocialAccount", account, false).await?;
            let (state, note, _) = accounts::state_of(tx, ctx.tenant, &a).await?;
            if state != "Connected" {
                waiting.push(format!("{}: {state}{}", s(&a, "account_name").unwrap_or("an account"), note.map(|n| format!(": {n}")).unwrap_or_default()));
            }
        }
        if !waiting.is_empty() {
            let why = waiting.join("\n");
            if s(&doc, "waiting_for") != Some(why.as_str()) {
                k.set_values_tx(ctx, tx, "SocialPost", post, obj(json!({"waiting_for": why}))).await?;
            }
            continue;
        }
        for target in rows(&doc, "targets") {
            let Some(account) = id(&target, "account") else { continue };
            let made = k
                .insert_tx(
                    ctx,
                    tx,
                    "SocialPublication",
                    obj(json!({"post": post.to_string(), "account": account.to_string(), "text": s(&doc, "text"), "link": s(&doc, "link"),
                               "image": s(&doc, "image"), "image_alt": s(&doc, "image_alt")})),
                )
                .await?;
            queue(tx, ctx.tenant, id(&made, "id").unwrap_or_default()).await?;
        }
        k.set_values_tx(ctx, tx, "SocialPost", post, obj(json!({"status": "Sending", "waiting_for": null}))).await?;
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
        match send(k, job.tenant_id, publication).await {
            Ok(done) => Ok(done),
            Err(e) => {
                // Said on the publication as it happens: a refusal ends it, "later" is shown as waiting.
                let last = !matches!(e, Error::Unavailable(_) | Error::Wait(..)) || job.attempts >= job.max_attempts;
                let _ = fail(k, job.tenant_id, publication, &e.to_string(), last).await;
                Err(e)
            }
        }
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
    if s(&p, "external_id").is_none() {
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

async fn send(k: &Kernel, tenant: Uuid, publication: Uuid) -> Result<Value> {
    let ctx = Ctx::system(tenant, k.registry.schema(tenant).await?);
    let mut tx = k.db.tenant_tx(tenant).await?;
    // The lock that makes a second send of the same publication wait, and then find it out.
    let p = k.get_tx(&ctx, &mut tx, "SocialPublication", publication, true).await?;
    if s(&p, "external_id").is_some() {
        tx.commit().await?;
        return Ok(json!({"done": "already out"}));
    }
    let account = k.get_tx(&ctx, &mut tx, "SocialAccount", id(&p, "account").unwrap_or_default(), false).await?;
    let name = s(&account, "account_name").unwrap_or("the account").to_string();
    let post = k.get_tx(&ctx, &mut tx, "SocialPost", id(&p, "post").unwrap_or_default(), false).await?;
    let conn: Reach =
        reach(&mut tx, tenant, s(&account, "connection")).await?.ok_or_else(|| Error::invalid(format!("{name} has no connection here: connect it again")))?;
    if !conn.enabled {
        return Err(Error::invalid(format!("the connection {} is turned off", conn.name)));
    }
    // What its network takes may have been changed since the post was approved.
    let profile = rules_of(k, &ctx, &mut tx, &account).await?.ok_or_else(|| Error::invalid(format!("{name} has no network profile")))?;
    let image = s(&post, "image").filter(|i| !i.is_empty());
    let copy = copy_of(k, &ctx, &post).await?;
    let picture = copy.items[0].media.first().cloned();
    if let Some(why) = rules::problems(&profile, &copy).into_iter().next() {
        return Err(Error::invalid(format!("for {name}: {why}")));
    }
    let calling = caller(k, &mut tx, tenant, &account, &conn).await?;
    let call = |operation: &str, inputs: Value, key: String| Call {
        connection: conn.id.to_string(),
        operation: profile.op(operation).into(),
        inputs: conn.fit(profile.op(operation), inputs),
        key: Some(key),
        entity: Some("SocialPublication".into()),
        record: Some(publication),
    };
    let mut media = s(&p, "media_ref").map(str::to_string);
    if let (Some(image), None) = (image, &media) {
        if !conn.has("publish_media") {
            return Err(Error::invalid(format!("{name}'s connection cannot publish pictures")));
        }
        let file: Uuid = image.strip_prefix("/files/").and_then(|f| f.parse().ok()).ok_or_else(|| Error::invalid("a post's picture is a file stored here"))?;
        let info = k.file_info(&elevated(&ctx), file).await?;
        let alt = if profile.media.alt { s(&post, "image_alt").unwrap_or_default() } else { "" };
        // `file` is the stored file's id, for a description that sends it as a part or as the body; the picture's bytes
        // as base64 only for one that says it takes `data` (a network that wants it inside JSON).
        let mut inputs = json!({"file": file, "name": info.name, "content_type": info.content_type, "alt": alt, "size": info.size});
        let declared = conn.inputs("publish_media");
        if declared.is_empty() || declared.iter().any(|d| d == "data") {
            let (_, _, bytes) = k.read_file(&elevated(&ctx), file).await?;
            inputs["data"] = json!(base64(&bytes));
        }
        let up = connections::call(k, &calling, Some(crate::APP), &call("publish_media", inputs, format!("media:{publication}"))).await.map_err(crate::said)?;
        media = Some(text_of(&up["id"]).ok_or_else(|| Error::invalid("the connection's publish_media gives no id: its description maps one under result"))?);
    }
    let text = s(&post, "text").unwrap_or_default();
    let link = s(&post, "link").filter(|l| !l.is_empty());
    // Stable across attempts: a call's inputs under one key must be the same each time.
    let created = at(&p, "created_at").unwrap_or_else(Utc::now).to_rfc3339_opts(SecondsFormat::Millis, true);
    let inputs = json!({"text": text, "link": link, "text_with_link": link.map_or_else(|| text.to_string(), |l| format!("{text} {l}")),
                        "media": media.iter().collect::<Vec<_>>(), "media_id": media, "alt": s(&post, "image_alt"), "created_at": created,
                        "picture_type": picture.as_ref().and_then(|p| p.content_type.clone()), "picture_size": picture.as_ref().and_then(|p| p.bytes)});
    let out = connections::call(k, &calling, Some(crate::APP), &call("publish", inputs, format!("publish:{publication}"))).await.map_err(crate::said)?;
    let external = text_of(&out["id"]).ok_or_else(|| Error::invalid("the connection's publish gives no id: its description maps one under result"))?;
    let values = json!({"external_id": external, "sent_at": Utc::now().to_rfc3339(), "permalink": text_of(&out["url"]), "media_ref": media,
                        "failed_at": null, "failure": null});
    k.set_values_tx(&ctx, &mut tx, "SocialPublication", publication, obj(values)).await?;
    // The first comment follows the post, where the network takes one and the connection can reply.
    let comment = s(&post, "first_comment").filter(|c| !c.trim().is_empty());
    if let (Some(comment), true, true) = (comment, profile.first_comment, conn.has("reply")) {
        let inputs = json!({"text": comment, "in_reply_to": external});
        if let Err(e) = connections::call(k, &calling, Some(crate::APP), &call("reply", inputs, format!("first_comment:{publication}"))).await {
            let why = format!("It is out, but its first comment was not: {}", crate::said(e));
            k.set_values_tx(&ctx, &mut tx, "SocialPublication", publication, obj(json!({"failure": why}))).await?;
        }
    }
    tx.commit().await?;
    Ok(json!({"id": external}))
}

/// Posts all of whose publications are settled: published, or failed when any publication ended without
/// going out. A publication whose job ended without saying so, or left no trace for a day, is marked
/// failed too.
pub(crate) async fn mark_sent(k: &Kernel, ctx: &Ctx, tx: &mut Tx, now: DateTime<Utc>) -> Result<()> {
    for found in k.find_tx(ctx, tx, "SocialPost", &[Filter::eq("status", "Sending")], None, 500).await? {
        let post = id(&found, "id").unwrap_or_default();
        let mut pending = false;
        let mut failures = Vec::new();
        for p in k.find_tx(ctx, tx, "SocialPublication", &[Filter::eq("post", post.to_string())], None, 100).await? {
            let publication = id(&p, "id").unwrap_or_default();
            if s(&p, "external_id").is_some() {
                continue;
            }
            let account = k.get_tx(ctx, tx, "SocialAccount", id(&p, "account").unwrap_or_default(), false).await?;
            let name = s(&account, "account_name").unwrap_or("an account");
            if at(&p, "failed_at").is_some() {
                failures.push(format!("{name}: {}", s(&p, "failure").unwrap_or("it did not go out")));
                continue;
            }
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
        let values = if failures.is_empty() {
            json!({"status": "Published", "published_at": Utc::now().to_rfc3339()})
        } else {
            json!({"status": "Failed", "failure": failures.join("\n")})
        };
        k.set_values_tx(ctx, tx, "SocialPost", post, obj(values)).await?;
    }
    Ok(())
}
