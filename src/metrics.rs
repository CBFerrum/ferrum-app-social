//! Metrics. Once a day or so, for each account whose connection says how (`account` for the account's own
//! numbers, `metrics` for a post's), Social reads them and keeps one row a day in SocialMetric, named by what
//! they are: impressions, reach, engagements, clicks and followers. Which of a network's names is which is the
//! description's `result`; a network that counts likes, reposts and replies apart may name them `likes`,
//! `reposts`, `replies`, `quotes` and `saves` and they add up to the engagements it has no single number for.
//!
//! A post's numbers also go onto its publication, which the existing insights and best-times report read.
//! The reports here read the metric rows: by account, by post, and by the tags people put on posts.

use crate::obj;
use async_trait::async_trait;
use chrono::{DateTime, Duration, NaiveDate, Utc};
use ferrum_domain::reports::{Params, ReportSource, col, read_tx};
use ferrum_domain::util::{elevated, id, s};
use ferrum_kernel::connections::{self, Call};
use ferrum_kernel::{Ctx, Doc, Error, Filter, Kernel, Result};
use serde_json::{Value, json};
use std::collections::HashMap;
use uuid::Uuid;

/// The numbers, by what they are.
const NAMES: [&str; 5] = ["impressions", "reach", "engagements", "clicks", "followers"];
/// What adds up to engagements when a network gives no single number for them.
const PARTS: [&str; 5] = ["likes", "reposts", "replies", "quotes", "saves"];
const MOST_DAYS: i64 = 366;

/// A number a network gave, however it wrote it.
fn count(v: &Value) -> Option<i64> {
    match v {
        Value::Number(n) => n.as_i64().or_else(|| n.as_f64().map(|f| f.round() as i64)),
        Value::String(t) => t.trim().replace(',', "").parse::<f64>().ok().map(|f| f.round() as i64),
        _ => None,
    }
}

/// The five numbers out of a mapped result: those it names, engagements from its parts when it names none.
pub(crate) fn read(result: &Value) -> Doc {
    let mut out: Doc = NAMES.iter().filter_map(|n| count(&result[*n]).map(|c| (n.to_string(), json!(c)))).collect();
    if !out.contains_key("engagements") {
        let parts: Vec<i64> = PARTS.iter().filter_map(|p| count(&result[*p])).collect();
        if !parts.is_empty() {
            out.insert("engagements".into(), json!(parts.iter().sum::<i64>()));
        }
    }
    out
}

/// What a read is about.
struct Target {
    account: Uuid,
    publication: Option<Uuid>,
    external: Option<String>,
}

async fn setting(k: &Kernel, ctx: &Ctx, field: &str, default: i64) -> Result<i64> {
    let mut tx = k.db.tenant_tx(ctx.tenant).await?;
    let found = k.find_tx(&elevated(ctx), &mut tx, "SocialSettings", &[], None, 1).await?;
    tx.commit().await?;
    Ok(found.first().and_then(|r| r.get(field)).and_then(Value::as_i64).unwrap_or(default))
}

/// Reads the numbers of every live account and of its recent posts, a few hours apart at the soonest.
pub(crate) async fn pull(k: &Kernel, tenant: Uuid, now: DateTime<Utc>) -> Result<()> {
    let ctx = Ctx::system(tenant, k.registry.schema(tenant).await?);
    let days = setting(k, &ctx, "metrics_days", 30).await?;
    let hours = setting(k, &ctx, "metrics_hours", 6).await?;
    let today = now.date_naive();
    let mut tx = k.db.tenant_tx(tenant).await?;
    let accounts = k.find_tx(&ctx, &mut tx, "SocialAccount", &[], None, 1000).await?;
    let mut work: Vec<(Doc, Vec<Target>)> = Vec::new();
    for a in accounts {
        let Some(account) = id(&a, "id") else { continue };
        if !a.get("active").and_then(Value::as_bool).unwrap_or(true) {
            continue;
        }
        let mut targets = vec![Target { account, publication: None, external: None }];
        for p in k.find_tx(&ctx, &mut tx, "SocialPublication", &[Filter::eq("account", account.to_string())], Some("sent_at desc"), 200).await? {
            let recent = crate::at(&p, "sent_at").is_some_and(|t| now - t <= Duration::days(days));
            if recent && s(&p, "external_id").is_some() && crate::at(&p, "deleted_at").is_none() {
                targets.push(Target { account, publication: id(&p, "id"), external: s(&p, "external_id").map(str::to_string) });
            }
        }
        work.push((a, targets));
    }
    tx.commit().await?;
    for (account, targets) in work {
        let mut failure: Option<String> = None;
        for t in &targets {
            if let Err(e) = read_one(k, &ctx, &account, t, today, now, hours).await {
                failure.get_or_insert(e.to_string());
            }
        }
        // What went wrong shows on the account; a read that worked clears it.
        let mut tx = k.db.tenant_tx(tenant).await?;
        if s(&account, "metrics_error") != failure.as_deref() {
            let values = obj(json!({"metrics_error": failure}));
            k.set_values_tx(&elevated(&ctx), &mut tx, "SocialAccount", id(&account, "id").unwrap_or_default(), values).await?;
        }
        tx.commit().await?;
    }
    Ok(())
}

async fn read_one(k: &Kernel, ctx: &Ctx, account: &Doc, t: &Target, today: NaiveDate, now: DateTime<Utc>, hours: i64) -> Result<()> {
    let key = format!("{}:{}:{today}", t.account, t.publication.map_or_else(|| "account".to_string(), |p| p.to_string()));
    let system = elevated(ctx);
    let mut tx = k.db.tenant_tx(ctx.tenant).await?;
    let kept = k.find_tx(&system, &mut tx, "SocialMetric", &[Filter::eq("metric_key", key.as_str())], None, 1).await?.into_iter().next();
    let conn = crate::accounts::reach(&mut tx, ctx.tenant, s(account, "connection")).await?;
    let calling = match &conn {
        Some(c) if c.enabled => Some(crate::accounts::caller(k, &mut tx, ctx.tenant, account, c).await),
        _ => None,
    };
    tx.commit().await?;
    if kept.as_ref().and_then(|m| crate::at(m, "read_at")).is_some_and(|u| now - u < Duration::hours(hours)) {
        return Ok(());
    }
    let (Some(conn), Some(calling)) = (conn, calling) else { return Ok(()) };
    let operation = if t.publication.is_some() { "metrics" } else { "account" };
    if !conn.has(operation) {
        return Ok(());
    }
    let call = Call {
        connection: conn.id.to_string(),
        operation: operation.into(),
        inputs: conn.fit(operation, json!({"id": t.external})),
        key: None,
        entity: Some(if t.publication.is_some() { "SocialPublication" } else { "SocialAccount" }.into()),
        record: Some(t.publication.unwrap_or(t.account)),
    };
    let result = connections::call(k, &calling?, Some(crate::APP), &call).await.map_err(crate::said)?;
    let numbers = read(&result);
    if numbers.is_empty() {
        return Ok(());
    }
    let mut tx = k.db.tenant_tx(ctx.tenant).await?;
    let mut numbers = numbers;
    numbers.insert("read_at".into(), json!(now.to_rfc3339()));
    match kept {
        Some(row) => k.set_values_tx(&system, &mut tx, "SocialMetric", id(&row, "id").unwrap_or_default(), numbers.clone()).await?,
        None => {
            let mut row = numbers.clone();
            row.extend(obj(
                json!({"metric_key": key, "account": t.account.to_string(), "publication": t.publication.map(|p| p.to_string()), "day": today.to_string()}),
            ));
            k.insert_tx(&system, &mut tx, "SocialMetric", row).await?;
        }
    }
    // A post's numbers go onto its publication, as what the insights and best times read.
    if let Some(p) = t.publication {
        let on: Doc = ["impressions", "engagements", "clicks"].iter().filter_map(|n| numbers.get(*n).map(|v| (n.to_string(), v.clone()))).collect();
        if !on.is_empty() {
            // Saved, not just set: the insights keep the day's numbers as the publication changes.
            k.update_tx(&system, &mut tx, "SocialPublication", p, on, None).await?;
        }
    }
    tx.commit().await?;
    Ok(())
}

pub struct MetricReports;

/// The report's span, from its parameters.
fn span(p: &Params<'_>, back: i64) -> Result<(NaiveDate, NaiveDate)> {
    let to = p.date("to_date")?.unwrap_or_else(ferrum_domain::reports::today);
    let from = p.date("from_date")?.unwrap_or(to - Duration::days(back));
    if from > to {
        return Err(Error::field("from_date", "must not be after the to date"));
    }
    if (to - from).num_days() >= MOST_DAYS {
        return Err(Error::field("from_date", format!("is at most {MOST_DAYS} days before the to date")));
    }
    Ok((from, to))
}

#[async_trait]
impl ReportSource for MetricReports {
    fn catalog(&self) -> Vec<Value> {
        let params = |extra: Value| {
            let mut all = vec![
                json!({"name": "from_date", "label": "From Date", "type": "date"}),
                json!({"name": "to_date", "label": "To Date", "type": "date"}),
                json!({"name": "brand", "label": "Brand", "type": "link", "to": "SocialBrand"}),
            ];
            all.extend(extra.as_array().cloned().unwrap_or_default());
            Value::Array(all)
        };
        let account = json!([{"name": "account", "label": "Account", "type": "link", "to": "SocialAccount"}]);
        vec![
            json!({"name": "social_by_account", "label": "Social Numbers by Account", "module": "social", "entity": "SocialMetric", "scoped": false, "params": params(json!([]))}),
            json!({"name": "social_by_post", "label": "Social Numbers by Post", "module": "social", "entity": "SocialMetric", "scoped": false, "params": params(account)}),
            json!({"name": "social_by_tag", "label": "Social Numbers by Tag", "module": "social", "entity": "SocialMetric", "scoped": false, "params": params(json!([]))}),
        ]
    }

    async fn run(&self, k: &Kernel, ctx: &Ctx, name: &str, params: &HashMap<String, String>) -> Result<Value> {
        let p = Params { raw: params };
        let (from, to) = span(&p, 29)?;
        let brand = p.uuid("brand")?;
        let mut tx = read_tx(k, ctx).await?;
        let meta = json!({"from_date": from.to_string(), "to_date": to.to_string()});
        let out = match name {
            "social_by_account" => {
                // The account's own numbers: what each day added, added up, and the followers it ended with.
                let rows: Vec<(String, i64, i64, i64, i64, Option<i64>, Option<i64>)> = sqlx::query_as(
                    "select a.account_name, coalesce(sum(m.impressions), 0)::bigint, coalesce(sum(m.reach), 0)::bigint, coalesce(sum(m.engagements), 0)::bigint, \
                       coalesce(sum(m.clicks), 0)::bigint, (array_agg(m.followers order by m.day desc) filter (where m.followers is not null))[1], \
                       (array_agg(m.followers order by m.day asc) filter (where m.followers is not null))[1] \
                     from e_social_account a left join e_social_metric m on m.tenant_id = a.tenant_id and m.account = a.id and m.publication is null and m.day between $2 and $3 \
                     where a.tenant_id = $1 and ($4::uuid is null or a.brand = $4) group by a.id, a.account_name order by 4 desc, a.account_name",
                )
                .bind(ctx.tenant)
                .bind(from)
                .bind(to)
                .bind(brand)
                .fetch_all(&mut *tx)
                .await?;
                let list: Vec<Value> = rows
                    .iter()
                    .map(|(n, i, r, e, c, last, first)| {
                        let grew = last.zip(*first).map(|(l, f)| l - f);
                        json!({"account": n, "impressions": i, "reach": r, "engagements": e, "clicks": c, "followers": last, "follower_change": grew})
                    })
                    .collect();
                json!({"columns": [col("account", "Account", "data"), col("impressions", "Impressions", "int"), col("reach", "Reach", "int"), col("engagements", "Engagements", "int"),
                                   col("clicks", "Clicks", "int"), col("followers", "Followers", "int"), col("follower_change", "Follower Change", "int")], "rows": list, "meta": meta})
            }
            "social_by_post" | "social_by_tag" => {
                // Each publication's latest reading by the end of the span, for those sent within it.
                let latest = "select distinct on (m.publication) m.publication, m.impressions, m.reach, m.engagements, m.clicks from e_social_metric m \
                              where m.tenant_id = $1 and m.publication is not null and m.day <= $3 order by m.publication, m.day desc";
                if name == "social_by_post" {
                    let rows: Vec<(String, String, DateTime<Utc>, Option<i64>, Option<i64>, Option<i64>, Option<i64>)> =
                        sqlx::query_as(sqlx::AssertSqlSafe(format!(
                            "select po.headline, a.account_name, p.sent_at, l.impressions, l.reach, l.engagements, l.clicks \
                         from e_social_publication p join e_social_account a on a.tenant_id = p.tenant_id and a.id = p.account \
                           join e_social_post po on po.tenant_id = p.tenant_id and po.id = p.post \
                           left join ({latest}) l on l.publication = p.id \
                         where p.tenant_id = $1 and p.sent_at is not null and p.sent_at::date between $2 and $3 and ($4::uuid is null or a.brand = $4) \
                           and ($5::uuid is null or p.account = $5) order by coalesce(l.engagements, 0) desc, p.sent_at desc"
                        )))
                        .bind(ctx.tenant)
                        .bind(from)
                        .bind(to)
                        .bind(brand)
                        .bind(p.uuid("account")?)
                        .fetch_all(&mut *tx)
                        .await?;
                    let list: Vec<Value> = rows
                        .iter()
                        .map(|(h, a, sent, i, r, e, c)| json!({"post": h, "account": a, "sent_at": sent.to_rfc3339(), "impressions": i, "reach": r, "engagements": e, "clicks": c}))
                        .collect();
                    json!({"columns": [col("post", "Post", "data"), col("account", "Account", "data"), col("sent_at", "Sent", "datetime"), col("impressions", "Impressions", "int"),
                                       col("reach", "Reach", "int"), col("engagements", "Engagements", "int"), col("clicks", "Clicks", "int")], "rows": list, "meta": meta})
                } else {
                    let rows: Vec<(String, i64, i64, i64, i64, i64)> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
                        "select t.tag, count(distinct p.id), coalesce(sum(l.impressions), 0)::bigint, coalesce(sum(l.reach), 0)::bigint, coalesce(sum(l.engagements), 0)::bigint, \
                           coalesce(sum(l.clicks), 0)::bigint \
                         from record_tag t join e_social_publication p on p.tenant_id = t.tenant_id and p.post = t.record_id \
                           join e_social_account a on a.tenant_id = p.tenant_id and a.id = p.account \
                           left join ({latest}) l on l.publication = p.id \
                         where t.tenant_id = $1 and t.entity = 'SocialPost' and p.sent_at is not null and p.sent_at::date between $2 and $3 and ($4::uuid is null or a.brand = $4) \
                         group by t.tag order by 5 desc, t.tag"
                    )))
                    .bind(ctx.tenant)
                    .bind(from)
                    .bind(to)
                    .bind(brand)
                    .fetch_all(&mut *tx)
                    .await?;
                    let list: Vec<Value> = rows
                        .iter()
                        .map(|(t, n, i, r, e, c)| json!({"tag": t, "publications": n, "impressions": i, "reach": r, "engagements": e, "clicks": c}))
                        .collect();
                    json!({"columns": [col("tag", "Tag", "data"), col("publications", "Publications", "int"), col("impressions", "Impressions", "int"), col("reach", "Reach", "int"),
                                       col("engagements", "Engagements", "int"), col("clicks", "Clicks", "int")], "rows": list, "meta": meta})
                }
            }
            _ => return Err(Error::NotFound(format!("report {name}"))),
        };
        tx.rollback().await?;
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_networks_names_become_the_five_numbers() {
        assert_eq!(read(&json!({"impressions": "1,204", "reach": 800.0, "followers": 31})), obj(json!({"impressions": 1204, "reach": 800, "followers": 31})));
        let parts = read(&json!({"likes": 3, "reposts": "2", "replies": 1, "views": 9}));
        assert_eq!(parts.get("engagements"), Some(&json!(6)), "a network that counts apart: the parts add up");
        assert_eq!(read(&json!({"engagements": 10, "likes": 3})).get("engagements"), Some(&json!(10)), "its own total wins");
        assert!(read(&json!({"clicks": null, "x": 1})).is_empty());
    }
}
