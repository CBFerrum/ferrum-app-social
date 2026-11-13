//! The library: templates (a post's content saved, with its versions and answers; "use" makes a draft of it, "save as
//! template" keeps a post's), sets (a named selection of accounts, with their answers and optional content, to start a
//! post from), snippets (hashtags, captions: records the composer inserts), signatures (a brand's or an account's, one
//! default in each, added at the end of a post that asks for one, never twice) and variables (`{{name}}` in a post's
//! words, an account's value before its brand's before the workspace's, filled when the post is checked and sent).
//! Tags are the platform's.

use crate::content::{Version, content_of};
use crate::obj;
use async_trait::async_trait;
use ferrum_domain::util::{elevated, id, rows, s};
use ferrum_kernel::{Ctx, Doc, DocHook, Error, Filter, HookCtx, Kernel, Result, Tx};
use serde_json::{Value, json};
use std::collections::HashMap;
use uuid::Uuid;

/// The value each variable has for `account` of `brand`: the account's, else the brand's, else the workspace's.
async fn variables(k: &Kernel, ctx: &Ctx, tx: &mut Tx, brand: Option<Uuid>, account: Option<Uuid>) -> Result<HashMap<String, String>> {
    let mut out: HashMap<String, (u8, String)> = HashMap::new();
    for v in k.find_tx(&elevated(ctx), tx, "SocialVariable", &[], None, 1000).await? {
        let rank = match (id(&v, "account"), id(&v, "brand")) {
            (Some(a), _) if Some(a) == account => 3,
            (None, Some(b)) if Some(b) == brand => 2,
            (None, None) => 1,
            _ => continue,
        };
        let (Some(name), value) = (s(&v, "variable_name"), s(&v, "value").unwrap_or_default()) else { continue };
        if out.get(name).is_none_or(|(r, _)| *r < rank) {
            out.insert(name.to_string(), (rank, value.to_string()));
        }
    }
    Ok(out.into_iter().map(|(k, (_, v))| (k, v)).collect())
}

/// `text` with each `{{name}}` filled, or the first name that has no value.
pub(crate) fn fill(text: &str, values: &HashMap<String, String>) -> std::result::Result<String, String> {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("{{") {
        let Some(len) = rest[start + 2..].find("}}") else { break };
        let name = rest[start + 2..start + 2 + len].trim();
        out.push_str(&rest[..start]);
        match values.get(name) {
            Some(v) => out.push_str(v),
            None => return Err(name.to_string()),
        }
        rest = &rest[start + 4 + len..];
    }
    out.push_str(rest);
    Ok(out)
}

/// The signature a post adds for `account`: the one it names, else the account's default, the brand's, the workspace's.
async fn signature(k: &Kernel, ctx: &Ctx, tx: &mut Tx, doc: &Doc, account: &Doc) -> Result<Option<String>> {
    let system = elevated(ctx);
    if let Some(named) = id(doc, "signature") {
        return Ok(s(&k.get_tx(&system, tx, "SocialSignature", named, false).await?, "text").map(str::to_string));
    }
    let defaults = k.find_tx(&system, tx, "SocialSignature", &[Filter::eq("is_default", true)], None, 200).await?;
    let (brand, acc) = (id(doc, "brand"), id(account, "id"));
    let own = defaults.iter().find(|d| acc.is_some() && id(d, "account") == acc);
    let brands = || defaults.iter().find(|d| id(d, "account").is_none() && brand.is_some() && id(d, "brand") == brand);
    let workspace = || defaults.iter().find(|d| id(d, "account").is_none() && id(d, "brand").is_none());
    Ok(own.or_else(brands).or_else(workspace).and_then(|d| s(d, "text")).map(str::to_string))
}

/// One account's copy of a post with its words made final: the signature added when the post asks for one (not when
/// its words end with it already), then each variable filled.
pub(crate) async fn finish(k: &Kernel, ctx: &Ctx, tx: &mut Tx, doc: &Doc, account: &Doc, v: &mut Version) -> Result<()> {
    if doc.get("add_signature").and_then(Value::as_bool) == Some(true)
        && let Some(sig) = signature(k, ctx, tx, doc, account).await?.map(|t| t.trim().to_string()).filter(|t| !t.is_empty())
        && !v.items[0].text.trim_end().ends_with(&sig)
    {
        let first = &mut v.items[0].text;
        *first = format!("{}\n\n{sig}", first.trim_end());
    }
    let asks = v.items.iter().any(|i| i.text.contains("{{")) || v.first_comment.as_deref().is_some_and(|c| c.contains("{{"));
    if !asks {
        return Ok(());
    }
    let values = variables(k, ctx, tx, id(doc, "brand"), id(account, "id")).await?;
    let unknown = |name: String| Error::field("text", format!("no variable is called {name} for {}", s(account, "account_name").unwrap_or("this account")));
    for item in &mut v.items {
        item.text = fill(&item.text, &values).map_err(unknown)?;
    }
    if let Some(c) = v.first_comment.as_mut() {
        *c = fill(c, &values).map_err(unknown)?;
    }
    Ok(())
}

/// The accounts an action was given: a list of ids, or ids separated by commas or lines.
fn accounts_given(params: &Value) -> Vec<String> {
    match params.get("accounts") {
        Some(Value::Array(a)) => a.iter().filter_map(Value::as_str).map(str::to_string).collect(),
        Some(Value::String(t)) => t.split([',', '\n']).map(str::trim).filter(|t| !t.is_empty()).map(str::to_string).collect(),
        _ => vec![],
    }
}

/// A draft made from a template or a set (`from`), as the person pressing it.
async fn draft_from(hc: &mut HookCtx<'_>, from: &Doc, params: &Value, name_field: &str) -> Result<Value> {
    let headline = params.get("headline").and_then(Value::as_str).filter(|h| !h.trim().is_empty()).or(s(from, "headline")).or(s(from, name_field));
    let brand = params.get("brand").and_then(Value::as_str).or(s(from, "brand"));
    let mut values = content_of(from, json!({"headline": headline, "brand": brand}));
    let given = accounts_given(params);
    if !given.is_empty() {
        let kept: Vec<Value> =
            rows(from, "targets").into_iter().filter(|t| s(t, "account").is_some_and(|a| given.iter().any(|g| g == a))).map(Value::Object).collect();
        let mut targets = kept.clone();
        for a in given.iter().filter(|a| !kept.iter().any(|t| t["account"] == **a)) {
            targets.push(json!({"account": a}));
        }
        values.insert("targets".into(), Value::Array(targets));
    }
    if let Some(set) = params.get("set").and_then(Value::as_str).and_then(|t| t.parse::<Uuid>().ok()) {
        let set = hc.kernel.get_tx(hc.ctx, hc.tx, "SocialSet", set, false).await?;
        let targets: Vec<Value> = rows(&set, "targets")
            .into_iter()
            .map(|mut r| {
                for f in ["id", "parent_id", "idx"] {
                    r.remove(f);
                }
                Value::Object(r)
            })
            .collect();
        values.insert("targets".into(), Value::Array(targets));
        if values.get("brand").is_none_or(Value::is_null) {
            values.insert("brand".into(), json!(s(&set, "brand")));
        }
    }
    if values.get("targets").and_then(Value::as_array).is_none_or(Vec::is_empty) {
        return Err(Error::field("accounts", "say which accounts it goes to: a set, or the accounts"));
    }
    let made = hc.kernel.insert_tx(hc.ctx, hc.tx, "SocialPost", values).await?;
    Ok(json!({"entity": "SocialPost", "id": s(&made, "id")}))
}

/// The post's library action (`save_as_template`); `None` for another.
pub(crate) async fn post_action(hc: &mut HookCtx<'_>, name: &str, doc: &Doc, params: &Value) -> Result<Option<Value>> {
    if name != "save_as_template" {
        return Ok(None);
    }
    let title =
        params.get("name").and_then(Value::as_str).map(str::trim).filter(|n| !n.is_empty()).ok_or_else(|| Error::field("name", "a template is named"))?;
    let mut values = content_of(doc, json!({"template_name": title}));
    for f in ["status", "scheduled_at"] {
        values.remove(f);
    }
    let made = hc.kernel.insert_tx(hc.ctx, hc.tx, "SocialTemplate", values).await?;
    Ok(Some(json!({"entity": "SocialTemplate", "id": s(&made, "id")})))
}

pub struct LibraryHook;

#[async_trait]
impl DocHook for LibraryHook {
    fn name(&self) -> &'static str {
        "social.library"
    }

    fn fills(&self) -> &'static [&'static str] {
        &["brand"]
    }

    async fn before_validate(&self, hc: &mut HookCtx<'_>, doc: &mut Doc) -> Result<()> {
        // A signature or a variable of an account is its brand's too.
        if matches!(hc.entity.def.name.as_str(), "SocialSignature" | "SocialVariable")
            && let Some(a) = id(doc, "account")
        {
            let account = hc.kernel.get_tx(&elevated(hc.ctx), hc.tx, "SocialAccount", a, false).await?;
            doc.insert("brand".into(), json!(s(&account, "brand")));
        }
        Ok(())
    }

    async fn validate(&self, hc: &mut HookCtx<'_>, doc: &Doc) -> Result<()> {
        if hc.entity.def.name != "SocialVariable" {
            return Ok(());
        }
        let name = s(doc, "variable_name").unwrap_or_default();
        if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return Err(Error::field("variable_name", "is letters, digits and _, as written between {{ and }}"));
        }
        let same = hc.kernel.find_tx(&elevated(hc.ctx), hc.tx, "SocialVariable", &[Filter::eq("variable_name", name)], None, 200).await?;
        let scope = |d: &Doc| (id(d, "account"), if id(d, "account").is_some() { None } else { id(d, "brand") });
        if same.iter().any(|o| id(o, "id") != id(doc, "id") && scope(o) == scope(doc)) {
            return Err(Error::field("variable_name", format!("{name} has a value here already")));
        }
        Ok(())
    }

    /// One default signature in each scope: a new default ends the others'.
    async fn after_save(&self, hc: &mut HookCtx<'_>, doc: &Doc, _old: Option<&Doc>) -> Result<()> {
        if hc.entity.def.name != "SocialSignature" || doc.get("is_default").and_then(Value::as_bool) != Some(true) {
            return Ok(());
        }
        let system = elevated(hc.ctx);
        let scope = |d: &Doc| (id(d, "account"), if id(d, "account").is_some() { None } else { id(d, "brand") });
        for other in hc.kernel.find_tx(&system, hc.tx, "SocialSignature", &[Filter::eq("is_default", true)], None, 200).await? {
            if id(&other, "id") != id(doc, "id") && scope(&other) == scope(doc) {
                hc.kernel.set_values_tx(&system, hc.tx, "SocialSignature", id(&other, "id").unwrap_or_default(), obj(json!({"is_default": false}))).await?;
            }
        }
        Ok(())
    }

    async fn action(&self, hc: &mut HookCtx<'_>, name: &str, doc: &Doc, params: &Value) -> Result<Option<Value>> {
        match (hc.entity.def.name.as_str(), name) {
            ("SocialTemplate", "use_template") => draft_from(hc, doc, params, "template_name").await.map(Some),
            ("SocialSet", "use_set") => draft_from(hc, doc, params, "set_name").await.map(Some),
            _ => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn variables_filled_or_named() {
        let values: HashMap<String, String> = [("shop".to_string(), "https://n.example/shop".to_string())].into();
        assert_eq!(fill("Buy at {{shop}} or {{ shop }}.", &values).unwrap(), "Buy at https://n.example/shop or https://n.example/shop.");
        assert_eq!(fill("No {{ nope }} here", &values), Err("nope".into()));
        assert_eq!(fill("An open {{ brace", &values).unwrap(), "An open {{ brace");
    }
}
