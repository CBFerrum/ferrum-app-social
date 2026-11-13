//! Metrics. Once a day or so, for each account whose connection says how (`account` for the account's own
//! numbers, `metrics` for a post's), Social reads them and keeps one row a day in SocialMetric, named by what
//! they are: impressions, reach, engagements, clicks and followers. Which of a network's names is which is the
//! description's `result`; a network that counts likes, reposts and replies apart adds them up in an expression
//! (`{"expr": "answer.likes + answer.shares"}` under `engagements`).
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
const MOST_DAYS: i64 = 366;

/// A number a network gave, however it wrote it.
fn count(v: &Value) -> Option<i64> {
    match v {
        Value::Number(n) => n.as_i64().or_else(|| n.as_f64().map(|f| f.round() as i64)),
        Value::String(t) => t.trim().replace(',', "").parse::<f64>().ok().map(|f| f.round() as i64),
        _ => None,
    }
}

/// The five numbers out of a mapped result, those it names: what a network counts apart (likes, reposts, replies)
/// the description adds up itself, in an expression under `engagements`.
pub(crate) fn read(result: &Value) -> Doc {
    NAMES.iter().filter_map(|n| count(&result[*n]).map(|c| (n.to_string(), json!(c)))).collect()
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
        inputs: conn.fit(operation, json!({"id": t.external, "account_id": crate::picker::network_id(account), "handle": s(account, "handle")})),
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

/// Publications out from `from` to `to` (of a brand), and their impressions, engagements and clicks.
pub(crate) async fn totals(tx: &mut sqlx::PgConnection, tenant: Uuid, from: NaiveDate, to: NaiveDate, brand: Option<Uuid>) -> Result<[i64; 4]> {
    let (n, i, e, c): (i64, i64, i64, i64) = sqlx::query_as(
        "select count(*), coalesce(sum(p.impressions), 0)::bigint, coalesce(sum(p.engagements), 0)::bigint, coalesce(sum(p.clicks), 0)::bigint \
         from e_social_publication p join e_social_account a on a.tenant_id = p.tenant_id and a.id = p.account \
         where p.tenant_id = $1 and p.sent_at is not null and p.sent_at::date between $2 and $3 and ($4::uuid is null or a.brand = $4)",
    )
    .bind(tenant)
    .bind(from)
    .bind(to)
    .bind(brand)
    .fetch_one(tx)
    .await?;
    Ok([n, i, e, c])
}

/// Each brand's numbers of the last seven days against the seven before, and its top post, kept on the brand for its
/// weekly report document (`social.brand_report`) and the digest; once a day at most.
pub(crate) async fn brand_numbers(k: &Kernel, ctx: &Ctx, tx: &mut ferrum_kernel::Tx, today: NaiveDate) -> Result<()> {
    for b in k.find_tx(ctx, tx, "SocialBrand", &[], None, 500).await? {
        if b.get("numbers").and_then(|n| n["to"].as_str()).is_some_and(|t| t == today.to_string()) {
            continue;
        }
        let brand = id(&b, "id");
        let (from, pfrom) = (today - Duration::days(6), today - Duration::days(13));
        let now = totals(&mut *tx, ctx.tenant, from, today, brand).await?;
        let before = totals(&mut *tx, ctx.tenant, pfrom, from - Duration::days(1), brand).await?;
        let top: Option<(String, Option<i64>, Option<String>)> = sqlx::query_as(
            "select po.headline, p.engagements, p.permalink from e_social_publication p join e_social_account a on a.tenant_id = p.tenant_id and a.id = p.account \
             join e_social_post po on po.tenant_id = p.tenant_id and po.id = p.post \
             where p.tenant_id = $1 and p.sent_at::date between $2 and $3 and a.brand = $4 order by coalesce(p.engagements, 0) desc limit 1",
        )
        .bind(ctx.tenant)
        .bind(from)
        .bind(today)
        .bind(brand)
        .fetch_optional(&mut **tx)
        .await?;
        let numbers = json!({"from": from.to_string(), "to": today.to_string(), "publications": now[0], "impressions": now[1], "engagements": now[2], "clicks": now[3],
                             "before": {"publications": before[0], "impressions": before[1], "engagements": before[2], "clicks": before[3]},
                             "top": top.map(|(h, e, l)| json!({"headline": h, "engagements": e, "permalink": l}))});
        k.set_values_tx(&elevated(ctx), tx, "SocialBrand", brand.unwrap_or_default(), obj(json!({"numbers": numbers}))).await?;
    }
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
        let by = json!([{"name": "by", "label": "By", "type": "select", "options": ["engagements", "impressions", "clicks"]}, {"name": "limit", "label": "How Many", "type": "int"}]);
        vec![
            json!({"name": "social_by_account", "label": "Social Numbers by Account", "module": "social", "entity": "SocialMetric", "scoped": false, "params": params(json!([]))}),
            json!({"name": "social_by_post", "label": "Social Numbers by Post", "module": "social", "entity": "SocialMetric", "scoped": false, "params": params(account.clone())}),
            json!({"name": "social_by_tag", "label": "Social Numbers by Tag", "module": "social", "entity": "SocialMetric", "scoped": false, "params": params(json!([]))}),
            json!({"name": "social_by_network", "label": "Social Numbers by Network", "module": "social", "entity": "SocialPublication", "scoped": false, "params": params(json!([]))}),
            json!({"name": "social_by_kind", "label": "Social Numbers by Kind of Post", "module": "social", "entity": "SocialPublication", "scoped": false, "params": params(json!([]))}),
            json!({"name": "social_period_compare", "label": "Social Numbers Against the Period Before", "module": "social", "entity": "SocialPublication", "scoped": false, "params": params(json!([]))}),
            json!({"name": "social_followers_by_day", "label": "Social Followers by Day", "module": "social", "entity": "SocialMetric", "scoped": false, "params": params(account.clone())}),
            json!({"name": "social_top_posts", "label": "Social Top Posts", "module": "social", "entity": "SocialPublication", "scoped": false, "params": params(by)}),
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
            "social_by_network" | "social_by_kind" => {
                let group = if name == "social_by_network" { "coalesce(nullif(a.network, ''), '-')" } else { "coalesce(nullif(p.kind, ''), '-')" };
                let rows: Vec<(String, i64, i64, i64, i64)> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
                    "select {group}, count(*), coalesce(sum(p.impressions), 0)::bigint, coalesce(sum(p.engagements), 0)::bigint, coalesce(sum(p.clicks), 0)::bigint \
                     from e_social_publication p join e_social_account a on a.tenant_id = p.tenant_id and a.id = p.account \
                     where p.tenant_id = $1 and p.sent_at is not null and p.sent_at::date between $2 and $3 and ($4::uuid is null or a.brand = $4) group by 1 order by 4 desc, 1"
                )))
                .bind(ctx.tenant)
                .bind(from)
                .bind(to)
                .bind(brand)
                .fetch_all(&mut *tx)
                .await?;
                let key = if name == "social_by_network" { "network" } else { "kind" };
                let list: Vec<Value> =
                    rows.iter().map(|(g, n, i, e, c)| json!({key: g, "publications": n, "impressions": i, "engagements": e, "clicks": c})).collect();
                let label = if name == "social_by_network" { "Network" } else { "Kind" };
                json!({"columns": [col(key, label, "data"), col("publications", "Publications", "int"), col("impressions", "Impressions", "int"), col("engagements", "Engagements", "int"),
                                   col("clicks", "Clicks", "int")], "rows": list, "meta": meta})
            }
            "social_period_compare" => {
                let days = (to - from).num_days() + 1;
                let (pfrom, pto) = (from - Duration::days(days), from - Duration::days(1));
                let now = totals(&mut tx, ctx.tenant, from, to, brand).await?;
                let before = totals(&mut tx, ctx.tenant, pfrom, pto, brand).await?;
                let list: Vec<Value> = ["publications", "impressions", "engagements", "clicks"]
                    .iter()
                    .zip(now.iter().zip(before.iter()))
                    .map(|(m, (n, b))| {
                        json!({"measure": m, "this_period": n, "period_before": b, "change": n - b,
                                              "change_pct": if *b == 0 { Value::Null } else { json!(((n - b) as f64 * 1000.0 / *b as f64).round() / 10.0) }})
                    })
                    .collect();
                json!({"columns": [col("measure", "Number", "data"), col("this_period", "This Period", "int"), col("period_before", "Period Before", "int"), col("change", "Change", "int"),
                                   col("change_pct", "Change (%)", "decimal")], "rows": list,
                       "meta": {"from_date": from.to_string(), "to_date": to.to_string(), "before_from": pfrom.to_string(), "before_to": pto.to_string()}})
            }
            "social_followers_by_day" => {
                let rows: Vec<(NaiveDate, String, Option<i64>)> = sqlx::query_as(
                    "select m.day, a.account_name, m.followers from e_social_metric m join e_social_account a on a.tenant_id = m.tenant_id and a.id = m.account \
                     where m.tenant_id = $1 and m.publication is null and m.followers is not null and m.day between $2 and $3 and ($4::uuid is null or a.brand = $4) \
                       and ($5::uuid is null or m.account = $5) order by m.day, a.account_name",
                )
                .bind(ctx.tenant)
                .bind(from)
                .bind(to)
                .bind(brand)
                .bind(p.uuid("account")?)
                .fetch_all(&mut *tx)
                .await?;
                let list: Vec<Value> = rows.iter().map(|(d, a, f)| json!({"day": d.to_string(), "account": a, "followers": f})).collect();
                json!({"columns": [col("day", "Day", "date"), col("account", "Account", "data"), col("followers", "Followers", "int")], "rows": list, "meta": meta})
            }
            "social_top_posts" => {
                let by = params.get("by").map(String::as_str).filter(|b| matches!(*b, "impressions" | "clicks")).unwrap_or("engagements");
                let limit = params.get("limit").and_then(|l| l.parse::<i64>().ok()).unwrap_or(10).clamp(1, 100);
                let rows: Vec<(String, String, DateTime<Utc>, Option<String>, Option<i64>, Option<i64>, Option<i64>)> =
                    sqlx::query_as(sqlx::AssertSqlSafe(format!(
                        "select po.headline, a.account_name, p.sent_at, p.permalink, p.impressions, p.engagements, p.clicks \
                     from e_social_publication p join e_social_account a on a.tenant_id = p.tenant_id and a.id = p.account \
                       join e_social_post po on po.tenant_id = p.tenant_id and po.id = p.post \
                     where p.tenant_id = $1 and p.sent_at is not null and p.sent_at::date between $2 and $3 and ($4::uuid is null or a.brand = $4) \
                     order by coalesce(p.{by}, 0) desc, p.sent_at desc limit $5"
                    )))
                    .bind(ctx.tenant)
                    .bind(from)
                    .bind(to)
                    .bind(brand)
                    .bind(limit)
                    .fetch_all(&mut *tx)
                    .await?;
                let list: Vec<Value> = rows
                    .iter()
                    .map(|(h, a, sent, link, i, e, c)| json!({"post": h, "account": a, "sent_at": sent.to_rfc3339(), "permalink": link, "impressions": i, "engagements": e, "clicks": c}))
                    .collect();
                json!({"columns": [col("post", "Post", "data"), col("account", "Account", "data"), col("sent_at", "Sent", "datetime"), col("permalink", "Address", "data"),
                                   col("impressions", "Impressions", "int"), col("engagements", "Engagements", "int"), col("clicks", "Clicks", "int")], "rows": list, "meta": meta})
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
        assert!(read(&json!({"likes": 3, "reposts": "2"})).is_empty(), "adding up is the description's");
        assert!(read(&json!({"clicks": null, "x": 1})).is_empty());
    }
}
