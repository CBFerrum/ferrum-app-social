//! A post's review, on the platform's approval policies (docs/api.md, "Approval before an action runs"; BD-19, BD-34).
//!
//! The package ships a policy on the post's hidden action `review`: one step, the Social Approver role, never whoever
//! asked (the author: Social asks in the author's name), its decisions void when the words, media, accounts or time
//! change (`reset_on`). A workspace edits it like any policy (more steps, approvers by brand through a field or an
//! expression, N of M, stand-ins, escalation) without code. Send for Review makes the post In Review and asks; the
//! approvers decide in the platform's approvals (`POST /api/v1/SocialPost/{id}/approval/decide`); the last approval runs
//! `review` as the author, which approves the post. (round five) The platform tells the post's hook when the request is
//! decided (`approval_decided`, BD-66), in the deciding transaction: a rejection sends the post back with the approver's
//! words, an approval names the deciding step's approver, an approval whose `review` was then refused sends it back saying why. The
//! upkeep's read-back stays only as the safety net for a hook that failed: the platform then rolls its writes back and
//! commits the decision anyway (audited as `approval:hook_failed`), and the next upkeep settles the post.
//!
//! What approval fixes is kept as before (`approved_hash`): an approved post that changes is a draft again, which the
//! platform's approval does not do (it ends with the decision). Without the policy (a workspace switched it off), or
//! when it cannot open (nobody holds the role), 0.7.0's rule holds: Approve It by a Social Approver or an administrator
//! who is not the author.
//!
//! A review link (`review_link`) gives someone outside a signed action link (the platform's: expiry, uses) to the post
//! as each account will get it (`preview`): they comment, and, when the link says so, approve it or ask for changes
//! (`review_comment`, `review_approve`, `review_changes`). What they say is kept as Reviews on the post; asking for
//! changes sends it back. Their approval is recorded for the approvers: a platform step's approvers are members.

use crate::accounts;
use crate::content::{self, fingerprint};
use crate::{at, obj};
use chrono::{Duration, Utc};
use ferrum_domain::util::{elevated, id, s};
use ferrum_kernel::action_link::LinkSpec;
use ferrum_kernel::messages::{self, Message, Recipient};
use ferrum_kernel::{ApprovalDecided, Ctx, Doc, Error, Filter, HookCtx, Kernel, Result, Tx};
use ferrum_meta::DocumentAction;
use serde_json::{Value, json};
use uuid::Uuid;

/// The post's action the review policy is on.
pub(crate) const REVIEW: &str = "review";

fn approver(hc: &HookCtx<'_>) -> bool {
    hc.ctx.is_admin || hc.ctx.roles.iter().any(|r| r == "Social Approver")
}

/// Whether the workspace reviews posts by an approval policy.
fn by_policy(hc: &HookCtx<'_>) -> bool {
    hc.entity.document_approvals.contains_key(&DocumentAction::Action(REVIEW.into()))
}

/// The post's latest approval request on `review`, when it is the one the post asked for last.
async fn request(k: &Kernel, ctx: &Ctx, post: Uuid, doc: &Doc) -> Result<Option<Value>> {
    let Some(asked) = s(doc, "approval_request") else { return Ok(None) };
    let r = k.approval_of(&elevated(ctx), "SocialPost", post).await?;
    Ok((r["action"] == REVIEW && r["id"] == asked).then_some(r))
}

/// Sends post `post` for review: In Review, and, under the policy, an approval asked in its author's name.
pub(crate) async fn send_for_review(hc: &mut HookCtx<'_>, post: Uuid) -> Result<Value> {
    let system = elevated(hc.ctx);
    // A draft kept with problems may not leave Draft with them.
    let stored = hc.kernel.get_tx(&system, hc.tx, "SocialPost", post, false).await?;
    crate::posts::ready(hc, &stored).await?;
    hc.kernel.set_values_tx(&system, hc.tx, "SocialPost", post, obj(json!({"status": "In Review", "review_note": null}))).await?;
    if !by_policy(hc) {
        tell_approvers(hc, post).await?;
        return Ok(json!({"status": "In Review"}));
    }
    let doc = hc.kernel.get_tx(&system, hc.tx, "SocialPost", post, false).await?;
    // Asked as the author, so the policy's "never whoever asked" is "never the author".
    let who = match accounts::person(hc.tx, hc.ctx.tenant, s(&doc, "author")).await? {
        Some(user) if user != hc.ctx.user => hc.kernel.load_ctx(hc.ctx.tenant, user, None, hc.ctx.request_id.clone()).await.unwrap_or_else(|_| hc.ctx.clone()),
        _ => hc.ctx.clone(),
    };
    sqlx::query("savepoint social_review").execute(&mut **hc.tx).await?;
    match hc.kernel.run_action_tx(&who, hc.tx, "SocialPost", post, REVIEW, &json!({})).await {
        Ok(out) => {
            sqlx::query("release savepoint social_review").execute(&mut **hc.tx).await?;
            let Some(asked) = out.get("_approval") else {
                // Every step of the policy was skipped: approved at once.
                return Ok(out);
            };
            hc.kernel.set_values_tx(&system, hc.tx, "SocialPost", post, obj(json!({"approval_request": asked["id"]}))).await?;
            Ok(json!({"status": "In Review", "approval": asked}))
        }
        // A request is pending already (sent back and sent again): it goes on, its decisions void if the post changed.
        Err(Error::Conflict(_)) => {
            sqlx::query("rollback to savepoint social_review").execute(&mut **hc.tx).await?;
            Ok(json!({"status": "In Review"}))
        }
        Err(e) if e.is_retryable() => Err(e),
        // The policy cannot open (nobody holds the role, say): Approve It decides, as before.
        Err(e) => {
            sqlx::query("rollback to savepoint social_review").execute(&mut **hc.tx).await?;
            hc.kernel.set_values_tx(&system, hc.tx, "SocialPost", post, obj(json!({"approval_request": null}))).await?;
            tell_approvers(hc, post).await?;
            Ok(json!({"status": "In Review", "approval": null, "approval_refused": e.to_string()}))
        }
    }
}

/// Without a platform request (which tells its approvers itself), the Social Approvers are told.
async fn tell_approvers(hc: &mut HookCtx<'_>, post: Uuid) -> Result<()> {
    let doc = hc.kernel.get_tx(&elevated(hc.ctx), hc.tx, "SocialPost", post, false).await?;
    let to = crate::notices::holders(hc.kernel, hc.ctx, hc.tx, &doc, "Social Approver").await?;
    crate::notices::send(hc.kernel, hc.ctx, hc.tx, "review", to, Some(post), json!({"notice": "waiting", "title": s(&doc, "headline")})).await?;
    Ok(())
}

/// The author told how the review went (0.7.0's Approve It and Send It Back; the policy's own notices tell them otherwise).
async fn tell_author(hc: &mut HookCtx<'_>, post: Uuid, doc: &Doc, notice: &str) -> Result<()> {
    let to = crate::notices::author(hc.tx, hc.ctx, doc).await?;
    crate::notices::send(hc.kernel, hc.ctx, hc.tx, "review", to, Some(post), json!({"notice": notice, "title": s(doc, "headline")})).await?;
    Ok(())
}

/// The review's actions; `None` for another.
pub(crate) async fn action(hc: &mut HookCtx<'_>, name: &str, doc: &Doc, params: &Value) -> Result<Option<Value>> {
    let post = id(doc, "id").ok_or_else(|| Error::internal("a post without an id"))?;
    let system = elevated(hc.ctx);
    match name {
        "send_for_review" => send_for_review(hc, post).await.map(Some),
        REVIEW => {
            // Run when the policy's last step approved (or the policy skipped every step): as the author, by the policy.
            if !by_policy(hc) {
                return Err(Error::invalid("no approval policy reviews posts here: Approve It"));
            }
            approved(hc, post, doc, None).await.map(Some)
        }
        "approve" | "reject" => {
            if by_policy(hc) && request(hc.kernel, hc.ctx, post, doc).await?.is_some_and(|r| r["status"] == "pending") {
                return Err(Error::invalid("this post's review is decided in its approval: approve it or send it back there"));
            }
            if !approver(hc) {
                return Err(Error::Forbidden(if name == "approve" { "a Social Approver approves posts" } else { "a Social Approver sends posts back" }.into()));
            }
            if name == "approve" {
                if s(doc, "author").is_some_and(|a| a.eq_ignore_ascii_case(&hc.ctx.email)) {
                    return Err(Error::Forbidden("a post is approved by someone other than its author".into()));
                }
                let out = approved(hc, post, doc, Some(hc.ctx.email.clone())).await?;
                tell_author(hc, post, doc, "approved").await?;
                return Ok(Some(out));
            }
            let note = params.get("note").and_then(Value::as_str).map(str::trim).filter(|n| !n.is_empty());
            let note = note.ok_or_else(|| Error::field("note", "sending a post back says why"))?;
            hc.kernel.set_values_tx(&system, hc.tx, "SocialPost", post, obj(json!({"status": "Rejected", "review_note": note}))).await?;
            tell_author(hc, post, doc, "rejected").await?;
            Ok(Some(json!({"status": "Rejected"})))
        }
        "review_link" => link(hc, post, doc, params).await.map(Some),
        "review_comment" | "review_approve" | "review_changes" => outside(hc, post, doc, name, params).await.map(Some),
        _ => Ok(None),
    }
}

/// Post `post` approved: what was approved fixed, and sent at once when its time has come.
async fn approved(hc: &mut HookCtx<'_>, post: Uuid, doc: &Doc, by: Option<String>) -> Result<Value> {
    let system = elevated(hc.ctx);
    let values = json!({"status": "Approved", "approved_by": by, "approved_hash": fingerprint(doc)});
    hc.kernel.set_values_tx(&system, hc.tx, "SocialPost", post, obj(values)).await?;
    if at(doc, "scheduled_at").is_some_and(|t| t <= Utc::now()) {
        let now = hc.kernel.get_tx(&system, hc.tx, "SocialPost", post, true).await?;
        if crate::posts::start(hc.kernel, &system, hc.tx, post, &now).await? {
            return Ok(json!({"status": "Sending"}));
        }
    }
    Ok(json!({"status": "Approved"}))
}

/// (round five) The post's review decided (BD-66): shown on the post in the deciding transaction.
pub(crate) async fn decided(hc: &mut HookCtx<'_>, doc: &Doc, d: &ApprovalDecided<'_>) -> Result<()> {
    let post = id(doc, "id").unwrap_or_default();
    if d.action != REVIEW || s(doc, "approval_request") != Some(d.request.to_string().as_str()) {
        return Ok(());
    }
    let system = elevated(hc.ctx);
    match d.outcome {
        "rejected" => {
            let values = json!({"status": "Rejected", "review_note": d.comment.unwrap_or_default()});
            hc.kernel.set_values_tx(&system, hc.tx, "SocialPost", post, obj(values)).await?;
        }
        "failed" => {
            let why = format!("approved, but it could not be approved here: {}", d.failure.unwrap_or_default());
            hc.kernel.set_values_tx(&system, hc.tx, "SocialPost", post, obj(json!({"status": "Rejected", "review_note": why}))).await?;
        }
        // Approved: `review` made it so in this transaction; who approved it is the deciding step's approver (the request
        // cannot be read back inside the deciding transaction).
        _ => {
            let who = d.on_behalf_of.unwrap_or(d.decided_by);
            let people = hc.kernel.user_directory(&system).await.unwrap_or_default();
            if let Some(email) = people.iter().find(|p| p["id"] == json!(who.to_string())).and_then(|p| p["email"].as_str()) {
                hc.kernel.set_values_tx(&system, hc.tx, "SocialPost", post, obj(json!({"approved_by": email}))).await?;
            }
        }
    }
    Ok(())
}

/// The upkeep's look at the platform's decisions, the safety net for a hook that failed: a post In Review whose request
/// was rejected is sent back with the approver's words; an approved one is told who approved it.
pub(crate) async fn settle(k: &Kernel, ctx: &Ctx, tx: &mut Tx) -> Result<()> {
    let asked = |status: &str| [Filter::eq("status", status)];
    let mut people: Option<Vec<Value>> = None;
    for status in ["In Review", "Approved"] {
        for found in k.find_tx(ctx, tx, "SocialPost", &asked(status), None, 500).await? {
            let post = id(&found, "id").unwrap_or_default();
            if s(&found, "approval_request").is_none() || (status == "Approved" && s(&found, "approved_by").is_some()) {
                continue;
            }
            settle_one(k, ctx, tx, post, &found, &mut people).await?;
        }
    }
    Ok(())
}

/// Post `post` (as `found`) after its request's decision: sent back when rejected, told who approved it when approved.
async fn settle_one(k: &Kernel, ctx: &Ctx, tx: &mut Tx, post: Uuid, found: &Doc, people: &mut Option<Vec<Value>>) -> Result<()> {
    let Some(r) = request(k, ctx, post, found).await? else { return Ok(()) };
    let decided: Vec<&Value> = r["decisions"].as_array().into_iter().flatten().filter(|d| d["void"] != true).collect();
    match (s(found, "status"), r["status"].as_str()) {
        (Some("In Review"), Some("rejected")) => {
            let why = decided.iter().rev().find(|d| d["outcome"] == "reject").and_then(|d| d["comment"].as_str()).unwrap_or_default();
            k.set_values_tx(ctx, tx, "SocialPost", post, obj(json!({"status": "Rejected", "review_note": why}))).await?;
        }
        (Some("Approved"), Some("approved")) => {
            if people.is_none() {
                *people = Some(k.user_directory(ctx).await.unwrap_or_default());
            }
            let by: Vec<String> = decided
                .iter()
                .filter(|d| d["outcome"] == "approve")
                .filter_map(|d| people.iter().flatten().find(|p| p["id"] == d["decider"]).and_then(|p| p["email"].as_str()).map(str::to_string))
                .collect();
            if !by.is_empty() {
                k.set_values_tx(ctx, tx, "SocialPost", post, obj(json!({"approved_by": by.join(", ")}))).await?;
            }
        }
        _ => {}
    }
    Ok(())
}

/// What each account gets, as plain text for a reader outside: each account's name and network, then its items, each
/// medium's description in brackets. No words of Social's own, so nothing here needs translating.
pub(crate) async fn preview(k: &Kernel, ctx: &Ctx, tx: &mut Tx, doc: &Doc) -> Result<String> {
    let mut out = vec![];
    for a in content::accounts(doc) {
        let account = k.get_tx(&elevated(ctx), tx, "SocialAccount", a, false).await?;
        let v = content::version(k, ctx, tx, doc, &account).await?;
        let mut part = format!("{} · {}", s(&account, "account_name").unwrap_or_default(), s(&account, "network").unwrap_or_default());
        for (n, item) in v.items.iter().enumerate() {
            part.push_str(&format!("\n\n{}{}", if n > 0 { format!("{}/{} ", n + 1, v.items.len()) } else { String::new() }, item.text));
            if let Some(l) = &item.link {
                part.push_str(&format!("\n{l}"));
            }
            for m in &item.media {
                part.push_str(&format!("\n[{}]", m.alt.clone().unwrap_or_else(|| m.name.clone())));
            }
        }
        out.push(part);
    }
    Ok(out.join("\n\n―――\n\n"))
}

/// `review_link`: a signed link for someone outside, mailed to them when an address is given.
async fn link(hc: &mut HookCtx<'_>, post: Uuid, doc: &Doc, params: &Value) -> Result<Value> {
    let system = elevated(hc.ctx);
    let text = preview(hc.kernel, hc.ctx, hc.tx, doc).await?;
    hc.kernel.set_values_tx(&system, hc.tx, "SocialPost", post, obj(json!({"preview": text}))).await?;
    let days = params.get("days").and_then(Value::as_i64).filter(|d| (1..=90).contains(d)).unwrap_or(7);
    let uses = params.get("uses").and_then(Value::as_i64).and_then(|u| i32::try_from(u).ok()).filter(|u| (1..=100).contains(u)).unwrap_or(20);
    let email = params.get("email").and_then(Value::as_str).map(str::trim).filter(|e| !e.is_empty());
    let name = params.get("name").and_then(Value::as_str).map(str::trim).filter(|n| !n.is_empty());
    let mut actions = vec!["review_comment".to_string()];
    if params.get("decide").and_then(Value::as_bool) == Some(true) {
        actions.extend(["review_approve".to_string(), "review_changes".to_string()]);
    }
    let spec = LinkSpec {
        actions,
        show: vec!["headline".into(), "scheduled_at".into(), "preview".into()],
        expires_at: Some(Utc::now() + Duration::days(days)),
        uses: Some(uses),
        recipient: name.or(email).map(str::to_string),
        // Who it is for, fixed: their address is how a request for their data finds what they said.
        params: obj(json!({"reviewer": email, "reviewer_name": name})),
        ..Default::default()
    };
    let issued = hc.kernel.issue_link(hc.ctx, hc.tx, "SocialPost", post, &spec).await?;
    if let Some(to) = email {
        let message = Message {
            template: "social.review_link".into(),
            entity: Some("SocialPost".into()),
            record: Some(post),
            to: vec![Recipient::address(to)],
            values: obj(json!({"url": issued.url, "expires_at": issued.expires_at.to_rfc3339(), "from": hc.ctx.full_name, "name": name})),
            secret: vec!["url".into()],
            ..Default::default()
        };
        messages::send(hc.kernel, hc.tx, &system, &message).await?;
    }
    Ok(json!({"id": issued.id, "url": issued.url, "expires_at": issued.expires_at.to_rfc3339(), "sent_to": email}))
}

/// What a reviewer outside said, kept on the post; asking for changes sends it back.
async fn outside(hc: &mut HookCtx<'_>, post: Uuid, doc: &Doc, name: &str, params: &Value) -> Result<Value> {
    let system = elevated(hc.ctx);
    let comment = params.get("comment").and_then(Value::as_str).map(str::trim).filter(|c| !c.is_empty());
    if name != "review_approve" && comment.is_none() {
        return Err(Error::field("comment", "say what you think of it"));
    }
    let given = |k: &str| params.get(k).and_then(Value::as_str).map(str::trim).filter(|v| !v.is_empty()).map(str::to_string);
    let email = given("reviewer").or_else(|| (!hc.ctx.email.is_empty()).then(|| hc.ctx.email.clone()));
    let who = given("reviewer_name").or_else(|| (!hc.ctx.full_name.is_empty()).then(|| hc.ctx.full_name.clone()));
    let reviewer = who.clone().or(email.clone()).unwrap_or_default();
    let outcome = match name {
        "review_approve" => "Approved",
        "review_changes" => "Changes Asked",
        _ => "Comment",
    };
    hc.kernel
        .insert_tx(
            &system,
            hc.tx,
            "SocialReview",
            obj(json!({"post": post.to_string(), "reviewer": email, "reviewer_name": who, "outcome": outcome, "text": comment, "at": Utc::now().to_rfc3339()})),
        )
        .await?;
    if name == "review_changes" && matches!(s(doc, "status"), Some("In Review" | "Approved")) {
        let values =
            json!({"status": "Rejected", "review_note": format!("{reviewer}: {}", comment.unwrap_or_default()), "approved_by": null, "approved_hash": null});
        hc.kernel.set_values_tx(&system, hc.tx, "SocialPost", post, obj(values)).await?;
    }
    Ok(json!({"outcome": outcome}))
}
