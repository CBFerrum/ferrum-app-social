//! A post's review: someone other than its author approves it. Sending it for review makes it In Review; approving it
//! fixes what was approved (`approved_hash`), and a change after that sends it back to a draft; sending it back says
//! why. A post approved after its time goes at once.

use crate::content::fingerprint;
use crate::{at, obj};
use chrono::Utc;
use ferrum_domain::util::{elevated, id, s};
use ferrum_kernel::{Error, HookCtx, Result};
use serde_json::{Value, json};
use uuid::Uuid;

fn approver(hc: &HookCtx<'_>) -> bool {
    hc.ctx.is_admin || hc.ctx.roles.iter().any(|r| r == "Social Approver")
}

/// Sends post `post` for review.
pub(crate) async fn send_for_review(hc: &mut HookCtx<'_>, post: Uuid) -> Result<Value> {
    let values = json!({"status": "In Review", "review_note": null});
    hc.kernel.set_values_tx(&elevated(hc.ctx), hc.tx, "SocialPost", post, obj(values)).await?;
    Ok(json!({"status": "In Review"}))
}

/// The review's actions; `None` for another.
pub(crate) async fn action(hc: &mut HookCtx<'_>, name: &str, doc: &crate::Doc, params: &Value) -> Result<Option<Value>> {
    let post = id(doc, "id").ok_or_else(|| Error::internal("a post without an id"))?;
    let system = elevated(hc.ctx);
    match name {
        "send_for_review" => send_for_review(hc, post).await.map(Some),
        "approve" => {
            if !approver(hc) {
                return Err(Error::Forbidden("a Social Approver approves posts".into()));
            }
            if s(doc, "author").is_some_and(|a| a.eq_ignore_ascii_case(&hc.ctx.email)) {
                return Err(Error::Forbidden("a post is approved by someone other than its author".into()));
            }
            approved(hc, post, doc, Some(hc.ctx.email.clone())).await.map(Some)
        }
        "reject" => {
            if !approver(hc) {
                return Err(Error::Forbidden("a Social Approver sends posts back".into()));
            }
            let note = params.get("note").and_then(Value::as_str).map(str::trim).filter(|n| !n.is_empty());
            let note = note.ok_or_else(|| Error::field("note", "sending a post back says why"))?;
            hc.kernel.set_values_tx(&system, hc.tx, "SocialPost", post, obj(json!({"status": "Rejected", "review_note": note}))).await?;
            Ok(Some(json!({"status": "Rejected"})))
        }
        _ => Ok(None),
    }
}

/// Post `post` approved: what was approved fixed, and sent at once when its time has come.
async fn approved(hc: &mut HookCtx<'_>, post: Uuid, doc: &crate::Doc, by: Option<String>) -> Result<Value> {
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
