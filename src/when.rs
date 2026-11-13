//! When a post goes, on the workspace's clock (its time zone; never the browser's):
//!
//! * a draft has no time or one not yet approved; `publish_now` sends an approved post at once, and sends a draft for
//!   review with the time now, so it goes as soon as it is approved; `reschedule` moves it (an approved post keeps its
//!   approval: what was approved is its content); `unschedule` takes it back to a draft without a time before it starts
//!   sending; `duplicate` makes a new draft of the same content;
//! * the queue: posting times are records (Posting Times: an account's, else its brand's; a weekday or every day, and a
//!   time). `add_to_queue` gives the post the first of its accounts' times from now that no other post of any of those
//!   accounts has. Two posts queued at once never get the same time: the queue of a brand is taken one at a time (a
//!   transaction-scoped advisory lock on the brand), and what is taken is read after the lock, in the transaction's own
//!   committed view (read committed), so the second sees the first's time;
//! * a repeat: every N days, weeks or months from the first time, until a date or for a number of times in all. When an
//!   occurrence is published the next is made as a real post (its content, versions and answers, approved as the one
//!   before was), linked to the first, and the chain is stopped by `stop_repeat`. It is made once: the post is settled
//!   holding its row, looked at again after the lock (posts.rs, `mark_sent`), and `repeat_key` is unique.

use crate::content::{self, fingerprint};
use crate::{at, obj};
use chrono::{DateTime, Datelike, Duration, LocalResult, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Utc, Weekday};
use chrono_tz::Tz;
use ferrum_domain::util::{elevated, id, s};
use ferrum_kernel::query::parse_filters;
use ferrum_kernel::{Ctx, Doc, Error, HookCtx, Kernel, Result, Tx};
use serde_json::{Value, json};
use uuid::Uuid;

/// How far ahead the queue looks for a free time.
const HORIZON_DAYS: i64 = 366;

pub(crate) fn zone(ctx: &Ctx) -> Tz {
    ctx.schema.timezone.parse().unwrap_or(Tz::UTC)
}

/// A wall time on the workspace's clock as a moment. A time the clocks skip (going forward) is the first minute after
/// the gap; a time that happens twice (going back) is the first of them.
pub(crate) fn moment(tz: Tz, local: NaiveDateTime) -> DateTime<Utc> {
    match tz.from_local_datetime(&local) {
        LocalResult::Single(t) => t.with_timezone(&Utc),
        LocalResult::Ambiguous(first, _) => first.with_timezone(&Utc),
        LocalResult::None => (1..=180)
            .find_map(|m| tz.from_local_datetime(&(local + Duration::minutes(m))).earliest())
            .map_or_else(|| Utc.from_utc_datetime(&local), |t| t.with_timezone(&Utc)),
    }
}

/// A posting time: a weekday (none: every day) and a time of day.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Slot {
    pub weekday: Option<Weekday>,
    pub time: NaiveTime,
}

const DAYS: [(&str, Weekday); 7] = [
    ("Monday", Weekday::Mon),
    ("Tuesday", Weekday::Tue),
    ("Wednesday", Weekday::Wed),
    ("Thursday", Weekday::Thu),
    ("Friday", Weekday::Fri),
    ("Saturday", Weekday::Sat),
    ("Sunday", Weekday::Sun),
];

pub(crate) fn time_of(t: &str) -> Option<NaiveTime> {
    NaiveTime::parse_from_str(t.trim(), "%H:%M").ok()
}

impl Slot {
    fn of(doc: &Doc) -> Option<Slot> {
        let weekday = DAYS.iter().find(|(n, _)| Some(*n) == s(doc, "weekday")).map(|(_, d)| *d);
        Some(Slot { weekday, time: time_of(s(doc, "time")?)? })
    }
}

/// The posting times after `after`, earliest first, a day at a time, for `days` days.
pub(crate) fn slots_after(tz: Tz, slots: &[Slot], after: DateTime<Utc>, days: i64) -> impl Iterator<Item = DateTime<Utc>> + '_ {
    let start = after.with_timezone(&tz).date_naive();
    (0..days).flat_map(move |d| {
        let date = start + Duration::days(d);
        let mut day: Vec<DateTime<Utc>> =
            slots.iter().filter(|s| s.weekday.is_none_or(|w| w == date.weekday())).map(|s| moment(tz, date.and_time(s.time))).filter(|m| *m > after).collect();
        day.sort();
        day.dedup();
        day
    })
}

/// A date `months` after `date`, on the same day of the month or the month's last.
fn add_months(date: NaiveDate, months: i64) -> NaiveDate {
    let total = i64::from(date.year()) * 12 + i64::from(date.month0()) + months;
    let (y, m) = (i32::try_from(total.div_euclid(12)).unwrap_or(9999), u32::try_from(total.rem_euclid(12)).unwrap_or(0) + 1);
    let last = (28..=31).rev().find(|d| NaiveDate::from_ymd_opt(y, m, *d).is_some()).unwrap_or(28);
    NaiveDate::from_ymd_opt(y, m, date.day().min(last)).unwrap_or(date)
}

/// The `n`th time after `first`, every `every` `unit` (Days, Weeks, Months) on the workspace's clock: the same wall
/// time, a month's day or its last.
pub(crate) fn occurrence(tz: Tz, first: DateTime<Utc>, every: i64, unit: &str, n: i64) -> DateTime<Utc> {
    let local = first.with_timezone(&tz).naive_local();
    let next = match unit {
        "Weeks" => local + Duration::days(7 * every * n),
        "Months" => add_months(local.date(), every * n).and_time(local.time()),
        _ => local + Duration::days(every * n),
    };
    moment(tz, next)
}

/// The next occurrence of a repeating post (`doc`, occurrence `index` counting the first as 1, the first at `first`),
/// unless its end is reached.
pub(crate) fn next_of(tz: Tz, doc: &Doc, first: DateTime<Utc>, index: i64) -> Option<DateTime<Utc>> {
    let every = doc.get("repeat_every").and_then(Value::as_i64).filter(|n| *n > 0)?;
    let unit = s(doc, "repeat_unit").unwrap_or("Days");
    if doc.get("repeat_count").and_then(Value::as_i64).is_some_and(|c| index + 1 > c) {
        return None;
    }
    let next = occurrence(tz, first, every, unit, index);
    let until = s(doc, "repeat_until").and_then(|d| NaiveDate::parse_from_str(d.get(..10).unwrap_or(d), "%Y-%m-%d").ok());
    if until.is_some_and(|u| next.with_timezone(&tz).date_naive() > u) {
        return None;
    }
    Some(next)
}

/// The post's time set to `when`, keeping its approval when it has one (the fingerprint holds the time).
async fn set_time(k: &Kernel, ctx: &Ctx, tx: &mut Tx, post: Uuid, doc: &Doc, when: Option<DateTime<Utc>>) -> Result<Doc> {
    let mut after = doc.clone();
    after.insert("scheduled_at".into(), json!(when.map(|w| w.to_rfc3339())));
    let mut values = json!({"scheduled_at": when.map(|w| w.to_rfc3339())});
    if s(doc, "status") == Some("Approved") {
        values["approved_hash"] = json!(fingerprint(&after));
    }
    k.set_values_tx(&elevated(ctx), tx, "SocialPost", post, obj(values)).await?;
    Ok(after)
}

/// The posting times for the post's accounts: each account's own, else its brand's.
async fn slots_for(k: &Kernel, ctx: &Ctx, tx: &mut Tx, brand: Option<Uuid>, accounts: &[Uuid]) -> Result<Vec<Slot>> {
    let filters = parse_filters(&json!([["active", "=", true]]))?;
    let all = k.find_tx(&elevated(ctx), tx, "SocialPostingTime", &filters, None, 500).await?;
    let mut out = vec![];
    for a in accounts {
        let own: Vec<Slot> = all.iter().filter(|t| id(t, "account") == Some(*a)).filter_map(Slot::of).collect();
        if own.is_empty() {
            out.extend(all.iter().filter(|t| id(t, "account").is_none() && id(t, "brand") == brand).filter_map(Slot::of));
        } else {
            out.extend(own);
        }
    }
    Ok(out)
}

/// The first posting time of the post's accounts from `now` that no other post of theirs has.
async fn free_time(k: &Kernel, ctx: &Ctx, tx: &mut Tx, post: Uuid, doc: &Doc, now: DateTime<Utc>) -> Result<DateTime<Utc>> {
    let accounts = content::accounts(doc);
    if accounts.is_empty() {
        return Err(Error::field("targets", "the post names no account to queue it for"));
    }
    let brand = id(doc, "brand");
    // One queue of a brand at a time, to the end of this transaction.
    sqlx::query("select pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(format!("social.queue:{}:{}", ctx.tenant, brand.unwrap_or_default()))
        .execute(&mut **tx)
        .await?;
    let slots = slots_for(k, ctx, tx, brand, &accounts).await?;
    if slots.is_empty() {
        return Err(Error::invalid("no posting times are set for its accounts or its brand: add some under Posting Times"));
    }
    let to = now + Duration::days(HORIZON_DAYS + 1);
    let taken: Vec<DateTime<Utc>> = content::taken(k, &elevated(ctx), tx, &accounts, &now.to_rfc3339(), &to.to_rfc3339(), post)
        .await?
        .iter()
        .filter_map(|p| at(p, "scheduled_at"))
        .collect();
    slots_after(zone(ctx), &slots, now, HORIZON_DAYS)
        .find(|t| !taken.iter().any(|x| x.timestamp() == t.timestamp()))
        .ok_or_else(|| Error::invalid("every posting time of its accounts in the coming year is taken"))
}

/// The post's actions about when it goes; `None` for another action.
pub(crate) async fn action(hc: &mut HookCtx<'_>, name: &str, doc: &Doc, params: &Value) -> Result<Option<Value>> {
    let post = id(doc, "id").ok_or_else(|| Error::internal("a post without an id"))?;
    let system = elevated(hc.ctx);
    let out = match name {
        "add_to_queue" => {
            crate::posts::ready(hc, doc).await?;
            let when = free_time(hc.kernel, hc.ctx, hc.tx, post, doc, Utc::now()).await?;
            set_time(hc.kernel, hc.ctx, hc.tx, post, doc, Some(when)).await?;
            json!({"scheduled_at": when.to_rfc3339(), "status": s(doc, "status")})
        }
        "reschedule" => {
            let when = params.get("at").and_then(Value::as_str).and_then(|t| DateTime::parse_from_rfc3339(t).ok()).map(|t| t.with_timezone(&Utc));
            let when = when.ok_or_else(|| Error::field("at", "is a date and time"))?;
            set_time(hc.kernel, hc.ctx, hc.tx, post, doc, Some(when)).await?;
            json!({"scheduled_at": when.to_rfc3339(), "status": s(doc, "status")})
        }
        "unschedule" => {
            let values = json!({"status": "Draft", "scheduled_at": null, "approved_by": null, "approved_hash": null, "waiting_for": null});
            hc.kernel.set_values_tx(&system, hc.tx, "SocialPost", post, obj(values)).await?;
            json!({"status": "Draft"})
        }
        "publish_now" => {
            let now = Utc::now();
            let after = set_time(hc.kernel, hc.ctx, hc.tx, post, doc, Some(now)).await?;
            if s(doc, "status") == Some("Approved") {
                let started = crate::posts::start(hc.kernel, &system, hc.tx, post, &after).await?;
                json!({"status": if started { "Sending" } else { "Approved" }, "scheduled_at": now.to_rfc3339()})
            } else {
                // Not approved yet: sent for review with the time now, it goes when it is approved.
                let out = crate::review::send_for_review(hc, post).await?;
                json!({"status": out["status"], "scheduled_at": now.to_rfc3339()})
            }
        }
        "duplicate" => {
            let mut values = content::content_of(doc, json!({"scheduled_at": null}));
            for f in ["repeat_every", "repeat_unit", "repeat_until", "repeat_count"] {
                if let Some(v) = doc.get(f).filter(|v| !v.is_null()) {
                    values.insert(f.into(), v.clone());
                }
            }
            let made = hc.kernel.insert_tx(hc.ctx, hc.tx, "SocialPost", values).await?;
            let made_id = id(&made, "id").unwrap_or_default();
            hc.kernel.set_values_tx(&system, hc.tx, "SocialPost", made_id, obj(json!({"origin": post.to_string()}))).await?;
            json!({"entity": "SocialPost", "id": made_id.to_string()})
        }
        "stop_repeat" => {
            let stop = json!({"repeat_every": null, "repeat_until": null, "repeat_count": null});
            for which in std::iter::once(post).chain(id(doc, "repeat_next")) {
                let d = hc.kernel.get_tx(&system, hc.tx, "SocialPost", which, true).await?;
                if matches!(s(&d, "status"), Some("Sending" | "Published" | "Failed")) && which != post {
                    continue;
                }
                let mut after = d.clone();
                after.extend(obj(stop.clone()));
                let mut values = stop.clone();
                if s(&d, "status") == Some("Approved") {
                    values["approved_hash"] = json!(fingerprint(&after));
                }
                hc.kernel.set_values_tx(&system, hc.tx, "SocialPost", which, obj(values)).await?;
            }
            json!({"stopped": true})
        }
        _ => return Ok(None),
    };
    Ok(Some(out))
}

/// A published occurrence of a repeating post (`doc`, its row locked): the next one made, approved as this one was.
/// Where it cannot be made (an account paused since, say), the post says why and the chain ends there.
pub(crate) async fn repeat(k: &Kernel, ctx: &Ctx, tx: &mut Tx, doc: &Doc) -> Result<()> {
    let Some(post) = id(doc, "id") else { return Ok(()) };
    if id(doc, "repeat_next").is_some() || doc.get("repeat_every").and_then(Value::as_i64).is_none_or(|n| n < 1) {
        return Ok(());
    }
    let Some(first) = at(doc, "repeat_from").or_else(|| at(doc, "scheduled_at")) else { return Ok(()) };
    let index = doc.get("repeat_index").and_then(Value::as_i64).unwrap_or(1);
    let Some(next) = next_of(zone(ctx), doc, first, index) else { return Ok(()) };
    let origin = id(doc, "repeat_of").unwrap_or(post);
    let mut values = content::content_of(doc, json!({"scheduled_at": next.to_rfc3339()}));
    for f in ["repeat_every", "repeat_unit", "repeat_until", "repeat_count"] {
        values.insert(f.into(), doc.get(f).cloned().unwrap_or(Value::Null));
    }
    sqlx::query("savepoint social_repeat").execute(&mut **tx).await?;
    match k.insert_tx(ctx, tx, "SocialPost", values).await {
        Ok(made) => {
            sqlx::query("release savepoint social_repeat").execute(&mut **tx).await?;
            let made_id = id(&made, "id").unwrap_or_default();
            let link = json!({"repeat_of": origin.to_string(), "repeat_from": first.to_rfc3339(), "repeat_index": index + 1,
                              "repeat_key": format!("{origin}#{}", index + 1), "author": s(doc, "author")});
            k.set_values_tx(ctx, tx, "SocialPost", made_id, obj(link)).await?;
            let full = k.get_tx(ctx, tx, "SocialPost", made_id, false).await?;
            let approval = json!({"status": "Approved", "approved_by": s(doc, "approved_by"), "approved_hash": fingerprint(&full)});
            k.set_values_tx(ctx, tx, "SocialPost", made_id, obj(approval)).await?;
            k.set_values_tx(ctx, tx, "SocialPost", post, obj(json!({"repeat_next": made_id.to_string()}))).await?;
        }
        Err(e) if !e.is_retryable() => {
            sqlx::query("rollback to savepoint social_repeat").execute(&mut **tx).await?;
            k.set_values_tx(ctx, tx, "SocialPost", post, obj(json!({"repeat_note": e.to_string()}))).await?;
        }
        Err(e) => return Err(e),
    }
    Ok(())
}

pub struct PostingTimeHook;

#[async_trait::async_trait]
impl ferrum_kernel::DocHook for PostingTimeHook {
    fn name(&self) -> &'static str {
        "social.posting_time"
    }

    fn fills(&self) -> &'static [&'static str] {
        &["brand"]
    }

    async fn before_validate(&self, hc: &mut HookCtx<'_>, doc: &mut Doc) -> Result<()> {
        posting_time(hc, doc).await
    }
}

/// A posting time's account names its brand.
async fn posting_time(hc: &mut HookCtx<'_>, doc: &mut Doc) -> Result<()> {
    if s(doc, "time").and_then(time_of).is_none() {
        return Err(Error::field("time", "is a time of day, as 09:30"));
    }
    if let Some(a) = id(doc, "account") {
        let account = hc.kernel.get_tx(&elevated(hc.ctx), hc.tx, "SocialAccount", a, false).await?;
        doc.insert("brand".into(), json!(s(&account, "brand")));
    }
    if id(doc, "brand").is_none() {
        return Err(Error::field("brand", "a posting time is a brand's, or an account's"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utc(t: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(t).unwrap().with_timezone(&Utc)
    }

    #[test]
    fn months_end_on_the_months_last_day() {
        let first = utc("2026-01-31T09:00:00Z");
        let tz = Tz::UTC;
        assert_eq!(occurrence(tz, first, 1, "Months", 1), utc("2026-02-28T09:00:00Z"));
        assert_eq!(occurrence(tz, first, 1, "Months", 2), utc("2026-03-31T09:00:00Z"), "from the first, so the 31st comes back");
        assert_eq!(occurrence(tz, utc("2024-01-31T09:00:00Z"), 1, "Months", 1), utc("2024-02-29T09:00:00Z"), "a leap year");
        assert_eq!(occurrence(tz, first, 12, "Months", 1), utc("2027-01-31T09:00:00Z"));
        assert_eq!(occurrence(tz, utc("2026-11-30T09:00:00Z"), 3, "Months", 1), utc("2027-02-28T09:00:00Z"), "across the year");
    }

    #[test]
    fn the_same_wall_time_across_a_change_of_the_clocks() {
        let berlin: Tz = "Europe/Berlin".parse().unwrap();
        // 09:00 in Berlin is 08:00 UTC in winter and 07:00 UTC in summer (clocks forward on 29 March 2026).
        let first = utc("2026-03-27T08:00:00Z");
        assert_eq!(occurrence(berlin, first, 1, "Days", 3), utc("2026-03-30T07:00:00Z"));
        assert_eq!(occurrence(berlin, first, 1, "Weeks", 1), utc("2026-04-03T07:00:00Z"));
        // 02:30 does not happen on 29 March: the first minute after the gap.
        assert_eq!(moment(berlin, NaiveDate::from_ymd_opt(2026, 3, 29).unwrap().and_hms_opt(2, 30, 0).unwrap()), utc("2026-03-29T01:00:00Z"));
        // 02:30 happens twice on 25 October: the first.
        assert_eq!(moment(berlin, NaiveDate::from_ymd_opt(2026, 10, 25).unwrap().and_hms_opt(2, 30, 0).unwrap()), utc("2026-10-25T00:30:00Z"));
    }

    #[test]
    fn posting_times_by_weekday_wrap_into_next_week() {
        let tz: Tz = "Asia/Kolkata".parse().unwrap();
        let slot = |w: Option<Weekday>, t: &str| Slot { weekday: w, time: time_of(t).unwrap() };
        let slots = [slot(Some(Weekday::Mon), "09:00"), slot(Some(Weekday::Fri), "18:30"), slot(None, "12:00")];
        // Friday 13 November 2026, 19:00 in Kolkata: past Friday's 18:30 and 12:00.
        let after = utc("2026-11-13T13:30:00Z");
        let next: Vec<DateTime<Utc>> = slots_after(tz, &slots, after, 4).take(4).collect();
        assert_eq!(
            next,
            vec![utc("2026-11-14T06:30:00Z"), utc("2026-11-15T06:30:00Z"), utc("2026-11-16T03:30:00Z"), utc("2026-11-16T06:30:00Z")],
            "Saturday noon, Sunday noon, then Monday 09:00 before Monday noon"
        );
    }

    #[test]
    fn a_repeat_ends_at_its_date_or_count() {
        let tz = Tz::UTC;
        let first = utc("2026-01-01T10:00:00Z");
        let doc = |v: Value| v.as_object().cloned().unwrap();
        let every = doc(json!({"repeat_every": 2, "repeat_unit": "Weeks", "repeat_count": 3}));
        assert_eq!(next_of(tz, &every, first, 1), Some(utc("2026-01-15T10:00:00Z")));
        assert_eq!(next_of(tz, &every, first, 2), Some(utc("2026-01-29T10:00:00Z")));
        assert_eq!(next_of(tz, &every, first, 3), None, "three in all");
        let until = doc(json!({"repeat_every": 1, "repeat_unit": "Days", "repeat_until": "2026-01-02"}));
        assert_eq!(next_of(tz, &until, first, 1), Some(utc("2026-01-02T10:00:00Z")));
        assert_eq!(next_of(tz, &until, first, 2), None, "the 3rd is after its last day");
        assert_eq!(next_of(tz, &doc(json!({"repeat_every": 0})), first, 1), None);
    }
}
