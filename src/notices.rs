//! Notices: message templates of the package (in-app and e-mail; a workspace edits them), sent with
//! `messages::send` in the transaction that has the news: a post went out (`social.went_out`) or did not
//! (`social.failed`, each account's reason), a post needs your review or was decided (`social.review`, also the
//! approval policy's own notices), an account needs connecting again (`social.reconnect`), a repeat stopped
//! (`social.repeat_stopped`), and the weekly digest (`social.digest`), sent by the upkeep once a week on the
//! workspace's clock (Social Settings' day and time) to each person holding a Social role. A person stops any of them
//! with a Notice Opt-out (the platform's opt-outs are per channel and spare operational messages).

use crate::{at, obj};
use chrono::{DateTime, Datelike, Duration, Utc};
use ferrum_domain::util::{elevated, id, s};
use ferrum_kernel::messages::{self, Message, Recipient};
use ferrum_kernel::{Ctx, Doc, Filter, Kernel, Result, Tx};
use serde_json::{Value, json};
use std::collections::HashMap;
use uuid::Uuid;

/// Sends notice `notice` (template `social.<notice>`) to `to`, but for those who opted out of it.
pub(crate) async fn send(k: &Kernel, ctx: &Ctx, tx: &mut Tx, notice: &str, to: Vec<Recipient>, about: Option<Uuid>, values: Value) -> Result<usize> {
    let system = elevated(ctx);
    let out: Vec<String> = k
        .find_tx(&system, tx, "SocialNoticeOptOut", &[Filter::eq("notice", notice)], None, 1000)
        .await?
        .iter()
        .filter_map(|o| s(o, "person").map(str::to_lowercase))
        .collect();
    let mut people: Option<HashMap<String, String>> = None;
    let mut kept = vec![];
    for r in to {
        let email = match (&r.email, r.user) {
            (Some(e), _) => Some(e.to_lowercase()),
            (None, Some(u)) => {
                if people.is_none() {
                    let dir = k.user_directory(&system).await.unwrap_or_default();
                    people = Some(dir.iter().filter_map(|p| Some((p["id"].as_str()?.to_string(), p["email"].as_str()?.to_lowercase()))).collect());
                }
                people.as_ref().and_then(|p| p.get(&u.to_string()).cloned())
            }
            _ => None,
        };
        if email.is_some_and(|e| out.contains(&e)) {
            continue;
        }
        kept.push(r);
    }
    if kept.is_empty() {
        return Ok(0);
    }
    let n = kept.len();
    let m = Message {
        template: format!("social.{notice}"),
        entity: about.map(|_| "SocialPost".to_string()),
        record: about,
        to: kept,
        values: obj(values),
        ..Default::default()
    };
    messages::send(k, tx, &system, &m).await?;
    Ok(n)
}

/// The post's author, as a recipient, when the author is a person here.
pub(crate) async fn author(tx: &mut Tx, ctx: &Ctx, doc: &Doc) -> Result<Vec<Recipient>> {
    Ok(crate::accounts::person(tx, ctx.tenant, s(doc, "author")).await?.map(Recipient::user).into_iter().collect())
}

/// The holders of `role`.
pub(crate) async fn holders(k: &Kernel, ctx: &Ctx, tx: &mut Tx, doc: &Doc, role: &str) -> Result<Vec<Recipient>> {
    messages::recipients(k, tx, &elevated(ctx), "SocialPost", &Value::Object(doc.clone()), &format!("role:{role}")).await
}

/// The weekly digest, once a week (`SocialDigestRun` is unique by the week), when the workspace's clock has passed
/// Social Settings' day and time: what went out, what is coming, what failed, and the top post, over the last seven
/// days, to each person holding a Social role.
pub(crate) async fn digest(k: &Kernel, ctx: &Ctx, tx: &mut Tx, now: DateTime<Utc>) -> Result<()> {
    let set = crate::outside::settings(k, ctx, tx).await?;
    let local = now.with_timezone(&crate::when::zone(ctx));
    let day = s(&set, "digest_day").unwrap_or("Monday");
    let time = s(&set, "digest_time").and_then(crate::when::time_of).unwrap_or_else(|| chrono::NaiveTime::from_hms_opt(9, 0, 0).unwrap_or_default());
    if day == "Never" || local.weekday().to_string() != day.get(..3).unwrap_or_default() || local.time() < time {
        return Ok(());
    }
    let week = format!("{}-W{:02}", local.iso_week().year(), local.iso_week().week());
    if !k.find_tx(ctx, tx, "SocialDigestRun", &[Filter::eq("week", week.as_str())], None, 1).await?.is_empty() {
        return Ok(());
    }
    sqlx::query("savepoint social_digest").execute(&mut **tx).await?;
    if k.insert_tx(ctx, tx, "SocialDigestRun", obj(json!({"week": week, "sent_at": now.to_rfc3339()}))).await.is_err() {
        // Another upkeep has this week's.
        sqlx::query("rollback to savepoint social_digest").execute(&mut **tx).await?;
        return Ok(());
    }
    sqlx::query("release savepoint social_digest").execute(&mut **tx).await?;
    let since = now - Duration::days(7);
    let posts = k.find_tx(ctx, tx, "SocialPost", &[], Some("scheduled_at asc"), 1000).await?;
    let line = |p: &Doc| json!({"headline": s(p, "headline"), "at": s(p, "published_at").or(s(p, "scheduled_at")), "failure": s(p, "failure")});
    let went: Vec<Value> =
        posts.iter().filter(|p| s(p, "status") == Some("Published") && at(p, "published_at").is_some_and(|t| t >= since)).map(line).collect();
    let failed: Vec<Value> = posts.iter().filter(|p| s(p, "status") == Some("Failed") && at(p, "updated_at").is_none_or(|t| t >= since)).map(line).collect();
    let coming: Vec<Value> = posts
        .iter()
        .filter(|p| matches!(s(p, "status"), Some("Approved" | "In Review")) && at(p, "scheduled_at").is_some_and(|t| t >= now && t <= now + Duration::days(7)))
        .map(line)
        .collect();
    let top = k
        .find_tx(
            ctx,
            tx,
            "SocialPublication",
            &ferrum_kernel::query::parse_filters(&json!([["sent_at", ">=", since.to_rfc3339()]]))?,
            Some("engagements desc"),
            1,
        )
        .await?
        .into_iter()
        .next();
    let top = match top {
        Some(p) => {
            let post = k.get_tx(ctx, tx, "SocialPost", id(&p, "post").unwrap_or_default(), false).await?;
            json!({"headline": s(&post, "headline"), "engagements": p.get("engagements"), "permalink": s(&p, "permalink")})
        }
        None => Value::Null,
    };
    let mut to: Vec<Recipient> = vec![];
    for role in ["Social Author", "Social Approver", "Social Manager"] {
        for r in holders(k, ctx, tx, &Doc::new(), role).await.unwrap_or_default() {
            if !to.iter().any(|x| x.user == r.user && x.email == r.email) {
                to.push(r);
            }
        }
    }
    let values = json!({"week": week, "went_out": went, "coming": coming, "failed": failed, "top": top});
    let n = send(k, ctx, tx, "digest", to, None, values).await?;
    let run = k.find_tx(ctx, tx, "SocialDigestRun", &[Filter::eq("week", week.as_str())], None, 1).await?;
    if let Some(r) = run.first().and_then(|r| id(r, "id")) {
        k.set_values_tx(ctx, tx, "SocialDigestRun", r, obj(json!({"people": n}))).await?;
    }
    Ok(())
}
