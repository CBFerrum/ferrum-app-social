//! Plugs: what to do with a published post once it does well, or once a time has passed. A plug (a brand's, or one
//! account's) says when: a publication's number (impressions, engagements, clicks) reaching a threshold within some
//! hours of going out, or some minutes after it went out; and what: repost it from its own account, add a comment under
//! it (words with variables), or have other accounts like, repost or comment on it (not before `delay_minutes` after it
//! went out). Each plug acts at most `max_runs` times in all, and at most once for a publication and an acting account
//! (`run_key`, unique, and the call's key `plug:<plug>:<publication>:<account>`), only where the acting account's
//! connection offers the operation (`repost`, `like`, `reply`): what was done, refused or skipped is a Plug Run. Run by
//! the upkeep after the numbers are read; no timer of its own.

use crate::accounts::{caller, reach};
use crate::posts::{rules_of, text_of};
use crate::{at, obj};
use chrono::{DateTime, Duration, Utc};
use ferrum_domain::util::{id, rows, s};
use ferrum_kernel::connections::{self, Call};
use ferrum_kernel::query::parse_filters;
use ferrum_kernel::{Ctx, Doc, Filter, Kernel, Result};
use serde_json::{Value, json};
use uuid::Uuid;

/// When a plug acts on publication `p`, if it does.
pub(crate) fn due(plug: &Doc, p: &Doc, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let sent = at(p, "sent_at")?;
    let n = |f: &str| plug.get(f).and_then(Value::as_i64);
    let base = match s(plug, "trigger") {
        Some("Time Passed") => sent + Duration::minutes(n("after_minutes").unwrap_or(0)),
        _ => {
            let number = s(plug, "measure").unwrap_or("engagements");
            let have = p.get(number).and_then(Value::as_i64).unwrap_or(0);
            let within = sent + Duration::hours(n("within_hours").unwrap_or(24));
            if have < n("threshold").unwrap_or(i64::MAX) || now > within {
                return None;
            }
            now
        }
    };
    let at = base.max(sent + Duration::minutes(n("delay_minutes").unwrap_or(0)));
    (at <= now).then_some(at)
}

/// The operation an action asks of each acting account.
fn operation(action: &str) -> &'static str {
    match action {
        "Repost" | "Others Repost" => "repost",
        "Others Like" => "like",
        _ => "reply",
    }
}

/// Every active plug, over the publications of its scope sent in the last 30 days.
pub(crate) async fn run(k: &Kernel, tenant: Uuid, now: DateTime<Utc>) -> Result<()> {
    let ctx = Ctx::system(tenant, k.registry.schema(tenant).await?);
    let mut tx = k.db.tenant_tx(tenant).await?;
    let plugs = k.find_tx(&ctx, &mut tx, "SocialPlug", &[Filter::eq("active", true)], None, 200).await?;
    tx.commit().await?;
    for plug in plugs {
        let plug_id = id(&plug, "id").unwrap_or_default();
        let mut tx = k.db.tenant_tx(tenant).await?;
        let plug = k.get_tx(&ctx, &mut tx, "SocialPlug", plug_id, false).await?;
        let done = k
            .find_tx(&ctx, &mut tx, "SocialPlugRun", &parse_filters(&json!([["plug", "=", plug_id.to_string()], ["state", "=", "Done"]]))?, None, 500)
            .await?
            .len() as i64;
        let max = plug.get("max_runs").and_then(Value::as_i64).unwrap_or(1);
        let accounts = k.find_tx(&ctx, &mut tx, "SocialAccount", &[Filter::eq("brand", s(&plug, "brand").unwrap_or_default())], None, 500).await?;
        let scope: Vec<String> =
            accounts.iter().filter(|a| id(&plug, "account").is_none_or(|x| id(a, "id") == Some(x))).filter_map(|a| s(a, "id").map(str::to_string)).collect();
        let since = (now - Duration::days(30)).to_rfc3339();
        let filters = parse_filters(&json!([["account", "in", scope], ["sent_at", ">=", since]]))?;
        let pubs = k.find_tx(&ctx, &mut tx, "SocialPublication", &filters, Some("sent_at asc"), 500).await?;
        tx.commit().await?;
        let mut runs = done;
        let action = s(&plug, "action").unwrap_or("Repost").to_string();
        for p in pubs.iter().filter(|p| s(p, "external_id").is_some() && at(p, "deleted_at").is_none()) {
            if runs >= max || due(&plug, p, now).is_none() {
                continue;
            }
            let own = id(p, "account");
            let acting: Vec<Uuid> = if action.starts_with("Others") {
                rows(&plug, "targets").iter().filter_map(|t| id(t, "account")).filter(|a| Some(*a) != own).collect()
            } else {
                own.into_iter().collect()
            };
            for account in acting {
                if runs >= max {
                    break;
                }
                if act(k, &ctx, &plug, p, account, &action).await? {
                    runs += 1;
                }
            }
        }
    }
    Ok(())
}

/// One plug acting on one publication as one account, once: whether it was done.
async fn act(k: &Kernel, ctx: &Ctx, plug: &Doc, p: &Doc, account: Uuid, action: &str) -> Result<bool> {
    let (plug_id, publication) = (id(plug, "id").unwrap_or_default(), id(p, "id").unwrap_or_default());
    let run_key = format!("plug:{plug_id}:{publication}:{account}");
    let mut tx = k.db.tenant_tx(ctx.tenant).await?;
    if !k.find_tx(ctx, &mut tx, "SocialPlugRun", &[Filter::eq("run_key", run_key.as_str())], None, 1).await?.is_empty() {
        return Ok(false);
    }
    let a = k.get_tx(ctx, &mut tx, "SocialAccount", account, false).await?;
    let name = s(&a, "account_name").unwrap_or("the account").to_string();
    let r = rules_of(k, ctx, &mut tx, &a).await?.unwrap_or_default();
    let op = r.op(operation(action)).to_string();
    let external = s(p, "external_id").unwrap_or_default().to_string();
    let conn = reach(&mut tx, ctx.tenant, s(&a, "connection")).await?;
    let (state, note, made) = match conn.as_ref().filter(|c| c.enabled && c.has(&op)) {
        None => ("Skipped", Some(format!("{name}'s connection does not offer {op}")), None),
        Some(conn) => {
            let text = match s(plug, "text") {
                Some(t) => crate::library::fill_for(k, ctx, &mut tx, id(plug, "brand"), Some(account), t).await.unwrap_or_else(|_| t.to_string()),
                None => String::new(),
            };
            let calling = caller(k, &mut tx, ctx.tenant, &a, conn).await?;
            let inputs = json!({"id": external, "in_reply_to": external, "root": external, "text": text, "account_id": crate::picker::network_id(&a)});
            let call = Call {
                connection: conn.id.to_string(),
                operation: op.clone(),
                inputs: conn.fit(&op, inputs),
                key: Some(run_key.clone()),
                entity: Some("SocialPublication".into()),
                record: Some(publication),
            };
            match connections::call(k, &calling, Some(crate::APP), &call).await {
                Ok(out) => ("Done", None, text_of(&out["id"])),
                Err(ferrum_kernel::Error::Unavailable(_) | ferrum_kernel::Error::RetryAfter(..)) => return Ok(false),
                Err(e) => ("Refused", Some(crate::said(e).to_string()), None),
            }
        }
    };
    sqlx::query("savepoint social_plug").execute(&mut *tx).await?;
    let row = json!({"plug": plug_id.to_string(), "publication": publication.to_string(), "account": account.to_string(), "run_key": run_key, "state": state,
                     "note": note, "external_id": made, "at": Utc::now().to_rfc3339()});
    match k.insert_tx(ctx, &mut tx, "SocialPlugRun", obj(row)).await {
        Ok(_) => {}
        // Another upkeep wrote it first: the call above was answered from its key, nothing went twice.
        Err(ferrum_kernel::Error::Conflict(_)) => {
            sqlx::query("rollback to savepoint social_plug").execute(&mut *tx).await?;
            tx.commit().await?;
            return Ok(false);
        }
        Err(e) => return Err(e),
    }
    tx.commit().await?;
    Ok(state == "Done")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn when_a_plug_acts() {
        let doc = |v: Value| v.as_object().cloned().unwrap();
        let sent = "2026-11-13T10:00:00Z";
        let at_ = |t: &str| DateTime::parse_from_rfc3339(t).unwrap().with_timezone(&Utc);
        let p = doc(json!({"sent_at": sent, "impressions": 500}));
        let reached = doc(json!({"trigger": "Number Reached", "measure": "impressions", "threshold": 400, "within_hours": 24}));
        assert_eq!(due(&reached, &p, at_("2026-11-13T12:00:00Z")), Some(at_("2026-11-13T12:00:00Z")));
        assert_eq!(due(&reached, &p, at_("2026-11-14T11:00:00Z")), None, "too late");
        assert_eq!(due(&doc(json!({"trigger": "Number Reached", "measure": "impressions", "threshold": 600})), &p, at_("2026-11-13T12:00:00Z")), None);
        let later = doc(json!({"trigger": "Time Passed", "after_minutes": 90}));
        assert_eq!(due(&later, &p, at_("2026-11-13T11:00:00Z")), None);
        assert_eq!(due(&later, &p, at_("2026-11-13T11:31:00Z")), Some(at_("2026-11-13T11:30:00Z")));
        let delayed = doc(json!({"trigger": "Time Passed", "after_minutes": 0, "delay_minutes": 60}));
        assert_eq!(due(&delayed, &p, at_("2026-11-13T10:30:00Z")), None);
    }
}
