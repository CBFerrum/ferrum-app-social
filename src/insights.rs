//! What publications did over time. Networks show a post's latest numbers only, so each reading
//! that changes a publication's impressions, engagements or clicks is kept as that day's row, one a
//! day holding the day's last numbers (the workspace's day). The report "Social Engagement by Day"
//! reads what each day added: a day's numbers less those of the publication's day before.

use crate::obj;
use async_trait::async_trait;
use chrono::{Duration, NaiveDate};
use ferrum_domain::reports::{Params, ReportSource, col, read_tx};
use ferrum_domain::util::{elevated, id, s};
use ferrum_kernel::{Ctx, Doc, DocHook, Error, HookCtx, Kernel, Result};
use serde_json::{Value, json};
use std::collections::HashMap;
use uuid::Uuid;

const NUMBERS: [&str; 3] = ["impressions", "engagements", "clicks"];

/// The days a report spans at most.
const MOST_DAYS: i64 = 366;

pub struct PublicationHook;

#[async_trait]
impl DocHook for PublicationHook {
    fn name(&self) -> &'static str {
        "social.publication"
    }

    async fn after_save(&self, hc: &mut HookCtx<'_>, doc: &Doc, old: Option<&Doc>) -> Result<()> {
        let number = |d: Option<&Doc>, f: &str| d.and_then(|d| d.get(f)).and_then(Value::as_i64);
        if NUMBERS.iter().all(|f| number(Some(doc), f) == number(old, f)) {
            return Ok(());
        }
        let Some(publication) = id(doc, "id") else { return Ok(()) };
        let day = hc.ctx.today();
        let numbers: Doc = NUMBERS.iter().map(|f| (f.to_string(), json!(number(Some(doc), f)))).collect();
        let system = elevated(hc.ctx);
        let kept: Option<Uuid> = sqlx::query_scalar("select id from e_social_insight where tenant_id = $1 and publication = $2 and day = $3")
            .bind(hc.ctx.tenant)
            .bind(publication)
            .bind(day)
            .fetch_optional(&mut **hc.tx)
            .await?;
        match kept {
            Some(row) => hc.kernel.set_values_tx(&system, hc.tx, "SocialInsight", row, numbers).await,
            None => {
                let mut row = numbers;
                row.extend(obj(json!({"publication": publication.to_string(), "account": s(doc, "account"), "post": s(doc, "post"), "day": day.to_string()})));
                hc.kernel.insert_tx(&system, hc.tx, "SocialInsight", row).await.map(|_| ())
            }
        }
    }
}

pub struct SocialReports;

/// What each day from `from` to `to` added, for one brand or account or all: (day, publications
/// read with new numbers, impressions, engagements, clicks).
async fn by_day(
    k: &Kernel,
    ctx: &Ctx,
    from: NaiveDate,
    to: NaiveDate,
    brand: Option<Uuid>,
    account: Option<Uuid>,
) -> Result<Vec<(NaiveDate, i64, i64, i64, i64)>> {
    let mut tx = read_tx(k, ctx).await?;
    // A day's numbers less the publication's last day before, looked for before `from` as well.
    let rows = sqlx::query_as(
        "with d as (select i.day, i.publication, \
           coalesce(i.impressions, 0) - coalesce(lag(i.impressions) over w, 0) as impressions, \
           coalesce(i.engagements, 0) - coalesce(lag(i.engagements) over w, 0) as engagements, \
           coalesce(i.clicks, 0) - coalesce(lag(i.clicks) over w, 0) as clicks \
         from e_social_insight i left join e_social_account a on a.tenant_id = i.tenant_id and a.id = i.account \
         where i.tenant_id = $1 and i.day <= $3 and ($4::uuid is null or a.brand = $4) and ($5::uuid is null or i.account = $5) \
         window w as (partition by i.publication order by i.day)) \
         select g.day::date, count(d.publication), coalesce(sum(d.impressions), 0)::bigint, coalesce(sum(d.engagements), 0)::bigint, \
           coalesce(sum(d.clicks), 0)::bigint \
         from generate_series($2::date, $3::date, interval '1 day') g(day) left join d on d.day = g.day::date group by g.day order by g.day",
    )
    .bind(ctx.tenant)
    .bind(from)
    .bind(to)
    .bind(brand)
    .bind(account)
    .fetch_all(&mut *tx)
    .await?;
    tx.rollback().await?;
    Ok(rows)
}

#[async_trait]
impl ReportSource for SocialReports {
    fn catalog(&self) -> Vec<Value> {
        vec![json!({"name": "social_engagement_by_day", "label": "Social Engagement by Day", "module": "social", "entity": "SocialInsight", "scoped": false,
                    "params": [{"name": "from_date", "label": "From Date", "type": "date"}, {"name": "to_date", "label": "To Date", "type": "date"},
                               {"name": "brand", "label": "Brand", "type": "link", "to": "SocialBrand"},
                               {"name": "account", "label": "Account", "type": "link", "to": "SocialAccount"}]})]
    }

    async fn run(&self, k: &Kernel, ctx: &Ctx, name: &str, params: &HashMap<String, String>) -> Result<Value> {
        if name != "social_engagement_by_day" {
            return Err(Error::NotFound(format!("report {name}")));
        }
        let p = Params { raw: params };
        let to = p.date("to_date")?.unwrap_or_else(ferrum_domain::reports::today);
        let from = p.date("from_date")?.unwrap_or(to - Duration::days(29));
        if from > to {
            return Err(Error::field("from_date", "must not be after the to date"));
        }
        if (to - from).num_days() >= MOST_DAYS {
            return Err(Error::field("from_date", format!("is at most {MOST_DAYS} days before the to date")));
        }
        let days = by_day(k, ctx, from, to, p.uuid("brand")?, p.uuid("account")?).await?;
        let total = |pick: fn(&(NaiveDate, i64, i64, i64, i64)) -> i64| days.iter().map(pick).sum::<i64>();
        let totals = json!({"impressions": total(|d| d.2), "engagements": total(|d| d.3), "clicks": total(|d| d.4)});
        let rows: Vec<Value> = days
            .iter()
            .map(|(day, read, impressions, engagements, clicks)| {
                json!({"date": day.to_string(), "publications": read, "impressions": impressions, "engagements": engagements, "clicks": clicks})
            })
            .collect();
        Ok(json!({
            "columns": [col("date", "Date", "date"), col("publications", "Publications Read", "int"), col("impressions", "Impressions", "int"),
                col("engagements", "Engagements", "int"), col("clicks", "Clicks", "int")],
            "rows": rows,
            "totals": totals,
            "meta": {"from_date": from.to_string(), "to_date": to.to_string()},
        }))
    }
}
