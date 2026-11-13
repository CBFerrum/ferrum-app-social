//! Links: tracking parameters and short links.
//!
//! * A Link Template (an account's, else its brand's, else the workspace's: the most specific active one) says what
//!   parameters a post's link gets, one `name=value` a line, values with `{{post}}`, `{{post_id}}`, `{{account}}`,
//!   `{{handle}}`, `{{network}}`, `{{kind}}`, `{{brand}}`, `{{tags}}` filled for each account. They go on the post's
//!   link and, when it says so, on the links in its words; a parameter the address has already is left as it is (never
//!   doubled), the address's own query is kept, and an address whose host its exclusion list names is left alone.
//!   Done when a copy is made (content.rs), so what is checked and counted is what goes.
//! * With a shortener named in Social Settings (`shorten_connection`, operation `shorten`: `url` → `url`, `id`), each
//!   final address of a publication is shortened once, under the key `shorten:<publication>:<address digest>`, and kept
//!   on the publication (`short_links`), so a retry uses the same one. Its length is the network's rule for a link.
//!   Where the shortener offers `link_clicks` (`id`, `url` → `clicks`), the upkeep reads them into the publication's
//!   clicks, as often as other numbers.

use crate::content::Version;
use crate::outside;
use crate::posts::text_of;
use crate::{at, obj};
use chrono::{DateTime, Duration, Utc};
use ferrum_domain::util::{elevated, id, s};
use ferrum_kernel::{Ctx, Doc, Error, Filter, Kernel, Result, Tx};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use uuid::Uuid;

/// The names a link template's values may hold.
const NAMES: [&str; 8] = ["post", "post_id", "account", "handle", "network", "kind", "brand", "tags"];

/// `url` with `params` added that it does not have, unless its host is excluded or it is not a web address.
pub(crate) fn tag(url: &str, params: &[(String, String)], exclude: &[String]) -> String {
    let Ok(mut u) = reqwest::Url::parse(url) else { return url.to_string() };
    let host = u.host_str().unwrap_or_default().to_lowercase();
    if !matches!(u.scheme(), "http" | "https") || exclude.iter().any(|x| host == *x || host.ends_with(&format!(".{x}"))) {
        return url.to_string();
    }
    let have: Vec<String> = u.query_pairs().map(|(k, _)| k.into_owned()).collect();
    let add: Vec<&(String, String)> = params.iter().filter(|(k, _)| !have.contains(k)).collect();
    if add.is_empty() {
        return url.to_string();
    }
    u.query_pairs_mut().extend_pairs(add.iter().map(|(k, v)| (k.as_str(), v.as_str())));
    u.to_string()
}

/// Each web address in `text` passed through `f`.
pub(crate) fn each_link(text: &str, mut f: impl FnMut(&str) -> String) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(i) = rest.find("http") {
        let tail = &rest[i..];
        if tail.starts_with("https://") || tail.starts_with("http://") {
            let end = tail.find(char::is_whitespace).unwrap_or(tail.len());
            // A sentence's last mark is not the address's.
            let trimmed = tail[..end].trim_end_matches(['.', ',', ')', '!', '?', ';', ':']);
            out.push_str(&rest[..i]);
            out.push_str(&f(trimmed));
            rest = &tail[trimmed.len()..];
        } else {
            out.push_str(&rest[..i + 4]);
            rest = &rest[i + 4..];
        }
    }
    out.push_str(rest);
    out
}

fn lines(v: Option<&str>) -> Vec<String> {
    v.unwrap_or_default().lines().map(|l| l.trim().to_lowercase()).filter(|l| !l.is_empty()).collect()
}

/// A template's parameters: `name=value` lines.
pub(crate) fn parameters(text: &str) -> std::result::Result<Vec<(String, String)>, String> {
    let mut out = vec![];
    for l in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        let (k, v) = l.split_once('=').ok_or_else(|| format!("{l} is not name=value"))?;
        out.push((k.trim().to_string(), v.trim().to_string()));
    }
    Ok(out)
}

/// The link template for `account` of `brand`: its own, else its brand's, else the workspace's.
async fn template(k: &Kernel, ctx: &Ctx, tx: &mut Tx, brand: Option<Uuid>, account: Option<Uuid>) -> Result<Option<Doc>> {
    let all = k.find_tx(&elevated(ctx), tx, "SocialLinkTemplate", &[Filter::eq("active", true)], None, 200).await?;
    let own = all.iter().find(|t| account.is_some() && id(t, "account") == account);
    let brands = || all.iter().find(|t| id(t, "account").is_none() && brand.is_some() && id(t, "brand") == brand);
    let workspace = || all.iter().find(|t| id(t, "account").is_none() && id(t, "brand").is_none());
    Ok(own.or_else(brands).or_else(workspace).cloned())
}

/// The copy's links tagged as its account's link template says.
pub(crate) async fn track(k: &Kernel, ctx: &Ctx, tx: &mut Tx, doc: &Doc, account: &Doc, v: &mut Version) -> Result<()> {
    let Some(t) = template(k, ctx, tx, id(doc, "brand"), id(account, "id")).await? else { return Ok(()) };
    let r = crate::posts::rules_of(k, ctx, tx, account).await?.unwrap_or_default();
    let brand = match id(doc, "brand") {
        Some(b) => s(&k.get_tx(&elevated(ctx), tx, "SocialBrand", b, false).await?, "brand_name").unwrap_or_default().to_string(),
        None => String::new(),
    };
    let tags = match id(doc, "id") {
        Some(post) => k.tags(&elevated(ctx), "SocialPost", post).await.unwrap_or_default().join(","),
        None => String::new(),
    };
    let values: HashMap<String, String> = [
        ("post", s(doc, "headline").unwrap_or_default().to_string()),
        ("post_id", s(doc, "id").unwrap_or_default().to_string()),
        ("account", s(account, "account_name").unwrap_or_default().to_string()),
        ("handle", s(account, "handle").unwrap_or_default().to_string()),
        ("network", r.name().to_string()),
        ("kind", r.kind(v.kind_on(&r).as_deref()).ok().flatten().map(|k| k.name.clone()).unwrap_or_default()),
        ("brand", brand),
        ("tags", tags),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v))
    .collect();
    let params: Vec<(String, String)> = parameters(s(&t, "parameters").unwrap_or_default())
        .map_err(|e| Error::invalid(format!("the link template {}: {e}", s(&t, "template_name").unwrap_or_default())))?
        .into_iter()
        .map(|(k, val)| (k, crate::library::fill(&val, &values).unwrap_or(val)))
        .collect();
    let exclude = lines(s(&t, "exclude"));
    let in_text = t.get("in_text").and_then(Value::as_bool) == Some(true);
    for item in &mut v.items {
        if let Some(l) = &item.link {
            item.link = Some(tag(l, &params, &exclude));
        }
        if in_text {
            item.text = each_link(&item.text, |u| tag(u, &params, &exclude));
        }
    }
    Ok(())
}

/// A link template's parameters are `name=value` lines whose values name only what is filled.
pub(crate) fn check_template(doc: &Doc) -> Result<()> {
    let params = parameters(s(doc, "parameters").unwrap_or_default()).map_err(|e| Error::field("parameters", e))?;
    let probe: HashMap<String, String> = NAMES.iter().map(|n| (n.to_string(), String::new())).collect();
    for (_, v) in params {
        crate::library::fill(&v, &probe).map_err(|name| Error::field("parameters", format!("{{{{{name}}}}} is not one of {}", NAMES.join(", "))))?;
    }
    Ok(())
}

/// The item's words and link with each address shortened, once per publication and address (kept in `short`).
pub(crate) async fn shorten(
    k: &Kernel,
    ctx: &Ctx,
    tx: &mut Tx,
    publication: Uuid,
    short: &mut Map<String, Value>,
    text: &mut String,
    link: &mut Option<String>,
) -> Result<()> {
    let set = outside::settings(k, ctx, tx).await?;
    let Some(name) = s(&set, "shorten_connection").filter(|n| !n.trim().is_empty()) else { return Ok(()) };
    let conn = outside::named(tx, ctx.tenant, name)
        .await?
        .ok_or_else(|| Error::invalid(format!("no connection is named {name}: Social Settings names it to shorten links")))?;
    let mut wanted: Vec<String> = link.iter().cloned().collect();
    if set.get("shorten_text_links").and_then(Value::as_bool) == Some(true) {
        each_link(text, |u| {
            wanted.push(u.to_string());
            u.to_string()
        });
    }
    for long in wanted {
        if short.contains_key(&long) || long.starts_with(conn.base()) {
            continue;
        }
        let key = format!("shorten:{publication}:{}", hex::encode(&Sha256::digest(long.as_bytes())[..8]));
        let got = outside::call(k, ctx, &conn, "shorten", json!({"url": long}), Some(key), Some(("SocialPublication", publication))).await?;
        let url = text_of(&got["url"]).ok_or_else(|| Error::invalid(format!("{}'s shorten gives no url: its description maps one under result", conn.name)))?;
        short.insert(long.clone(), json!({"url": url, "id": text_of(&got["id"])}));
    }
    let swap = |u: &str| short.get(u).and_then(|x| x["url"].as_str()).unwrap_or(u).to_string();
    if let Some(l) = link.as_mut() {
        *l = swap(l);
    }
    if set.get("shorten_text_links").and_then(Value::as_bool) == Some(true) {
        *text = each_link(text, swap);
    }
    Ok(())
}

/// The clicks the shortener counted for each publication's short links, into its clicks, a few hours apart.
pub(crate) async fn clicks(k: &Kernel, tenant: Uuid, now: DateTime<Utc>) -> Result<()> {
    let ctx = Ctx::system(tenant, k.registry.schema(tenant).await?);
    let mut tx = k.db.tenant_tx(tenant).await?;
    let set = outside::settings(k, &ctx, &mut tx).await?;
    let hours = set.get("metrics_hours").and_then(Value::as_i64).unwrap_or(6);
    let days = set.get("metrics_days").and_then(Value::as_i64).unwrap_or(30);
    let Some(conn) = (match s(&set, "shorten_connection") {
        Some(n) => outside::named(&mut tx, tenant, n).await?,
        None => None,
    }) else {
        return Ok(());
    };
    if !conn.has(&outside::op(&conn, "link_clicks")) {
        return Ok(());
    }
    let since = (now - Duration::days(days)).to_rfc3339();
    let filters = ferrum_kernel::query::parse_filters(&json!([["sent_at", ">=", since]]))?;
    let mut found = k.find_tx(&ctx, &mut tx, "SocialPublication", &filters, None, 500).await?;
    found.retain(|p| p.get("short_links").and_then(Value::as_object).is_some_and(|m| !m.is_empty()));
    tx.commit().await?;
    for p in found {
        if at(&p, "clicks_read_at").is_some_and(|t| now - t < Duration::hours(hours)) {
            continue;
        }
        let mut total = 0i64;
        for (long, short) in p.get("short_links").and_then(Value::as_object).into_iter().flatten() {
            match outside::call(k, &ctx, &conn, "link_clicks", json!({"id": short["id"], "url": short["url"], "long": long}), None, None).await {
                Ok(g) => total += g["clicks"].as_i64().unwrap_or(0),
                // Asked to wait: read another time, the clicks kept as they were.
                Err(e) if outside::later(&e) => return Ok(()),
                Err(_) => {}
            }
        }
        let mut tx = k.db.tenant_tx(tenant).await?;
        k.set_values_tx(
            &ctx,
            &mut tx,
            "SocialPublication",
            id(&p, "id").unwrap_or_default(),
            obj(json!({"clicks": total, "clicks_read_at": now.to_rfc3339()})),
        )
        .await?;
        tx.commit().await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tracking_kept_never_doubled_never_where_excluded() {
        let p = vec![("utm_source".to_string(), "Chirp".to_string()), ("utm_medium".to_string(), "social".to_string())];
        assert_eq!(tag("https://shop.example/a?x=1&utm_medium=email", &p, &[]), "https://shop.example/a?x=1&utm_medium=email&utm_source=Chirp");
        let once = tag("https://shop.example/a", &p, &[]);
        assert_eq!(tag(&once, &p, &[]), once, "never twice");
        assert_eq!(tag("https://cdn.partner.example/x", &p, &["partner.example".into()]), "https://cdn.partner.example/x");
        assert_eq!(tag("mailto:a@b.example", &p, &[]), "mailto:a@b.example");
        assert_eq!(each_link("See https://a.example/x. And http://b.example!", |u| format!("<{u}>")), "See <https://a.example/x>. And <http://b.example>!");
        assert_eq!(parameters("utm_source={{network}}\n\nref = x").unwrap(), vec![("utm_source".into(), "{{network}}".into()), ("ref".into(), "x".into())]);
        assert!(parameters("nonsense").is_err());
    }
}
