//! What an account's sign-in reaches, and what the composer looks up through it, each through an operation its
//! description names under `meta.social` (rules.rs, `Lookup`):
//!
//! * `reachable` lists the pages, organisations, locations, blogs or channels the sign-in reaches (`accounts`); `choose`
//!   makes each picked one an account of its own (or updates the account that has it), its id on the network in
//!   `network_id`, which is what is sent as `account_id`;
//! * `check` reads the account's name, handle and picture where the description's `account` operation gives them;
//! * `mentions` looks up whom a post may mention, and how to write it;
//! * `choices` reads the choices a setting fetches from the network (boards, subreddits, flairs, channels).

use crate::accounts::{Reach, caller, reach};
use crate::obj;
use crate::posts::{rules_of, text_of};
use crate::rules::{Lookup, Rules};
use chrono::Utc;
use ferrum_domain::util::{elevated, id, s};
use ferrum_kernel::connections::{self, Call};
use ferrum_kernel::{Doc, Error, Filter, HookCtx, Result};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use uuid::Uuid;

/// What a reached account is when the description says nothing of where: `{id, name, handle, picture}` items.
fn plain(operation: &str) -> Lookup {
    Lookup {
        operation: operation.into(),
        items_at: String::new(),
        value_at: "/id".into(),
        label_at: Some("/name".into()),
        handle_at: Some("/handle".into()),
        picture_at: Some("/picture".into()),
        ..Default::default()
    }
}

/// The account's id on the network, sent as `account_id`.
pub(crate) fn network_id(account: &Doc) -> Option<String> {
    s(account, "network_id").or(s(account, "provider_account")).map(str::to_string)
}

fn at(item: &Value, pointer: Option<&str>) -> Value {
    match pointer {
        Some("") => item.clone(),
        Some(p) => item.pointer(p).cloned().unwrap_or(Value::Null),
        None => Value::Null,
    }
}

/// Calls `look`'s operation for `account` and gives the items of its answer.
async fn call(hc: &mut HookCtx<'_>, account: &Doc, r: &Rules, look: &Lookup, query: Option<&str>, settings: &Map<String, Value>) -> Result<Vec<Value>> {
    let conn: Reach = reach(hc.tx, hc.ctx.tenant, s(account, "connection")).await?.ok_or_else(|| Error::invalid("no connection by that id is here"))?;
    let op = r.op(&look.operation).to_string();
    if !conn.has(&op) {
        return Err(Error::invalid(format!("{} does not offer {op}", conn.name)));
    }
    let mut inputs = Map::new();
    for (name, v) in &look.inputs {
        let given = match v.as_str() {
            Some("query") => json!(query),
            Some("account.id") => json!(network_id(account)),
            Some("account.handle") => json!(s(account, "handle")),
            Some(t) if t.starts_with("settings.") => settings.get(&t["settings.".len()..]).cloned().unwrap_or(Value::Null),
            _ => v.clone(),
        };
        inputs.insert(name.clone(), given);
    }
    inputs.entry("account_id").or_insert_with(|| json!(network_id(account)));
    let inputs = Value::Object(inputs);
    // A lookup is asked again each minute: a key (needed past a GET) of the minute and what was asked.
    let digest = hex::encode(&Sha256::digest(inputs.to_string())[..8]);
    let key = (!conn.reads(&op)).then(|| format!("look:{}:{op}:{}:{digest}", s(account, "id").unwrap_or_default(), Utc::now().format("%Y%m%d%H%M")));
    let calling = caller(hc.kernel, hc.tx, hc.ctx.tenant, account, &conn).await?;
    let c = Call {
        connection: conn.id.to_string(),
        operation: op.clone(),
        inputs: conn.fit(&op, inputs),
        key,
        entity: Some("SocialAccount".into()),
        record: id(account, "id"),
    };
    let out = connections::call(hc.kernel, &calling, Some(crate::APP), &c).await.map_err(crate::said)?;
    let list = if look.items_at.is_empty() { out } else { out.pointer(&look.items_at).cloned().unwrap_or(Value::Null) };
    Ok(list.as_array().cloned().unwrap_or_default())
}

/// What the sign-in reaches: `{id, name, handle, picture, kind, account}`, `account` the Social account that has it.
async fn reachable(hc: &mut HookCtx<'_>, account: &Doc, r: &Rules) -> Result<Vec<Value>> {
    let look = r.accounts.clone().unwrap_or_else(|| plain("accounts"));
    let items = call(hc, account, r, &look, None, &Map::new()).await?;
    let have = hc
        .kernel
        .find_tx(&elevated(hc.ctx), hc.tx, "SocialAccount", &[Filter::eq("connection", s(account, "connection").unwrap_or_default())], None, 500)
        .await?;
    Ok(items
        .iter()
        .filter_map(|i| {
            let id_ = text_of(&at(i, Some(&look.value_at)))?;
            let taken = have.iter().find(|a| s(a, "network_id") == Some(id_.as_str())).and_then(|a| s(a, "id"));
            Some(json!({"id": id_, "name": text_of(&at(i, look.label_at.as_deref())), "handle": text_of(&at(i, look.handle_at.as_deref())),
                        "picture": text_of(&at(i, look.picture_at.as_deref())), "kind": text_of(&at(i, look.kind_at.as_deref())), "account": taken}))
        })
        .collect())
}

/// An account name not yet taken: the network's name, else with its handle, else numbered.
async fn free_name(hc: &mut HookCtx<'_>, want: &str, handle: Option<&str>) -> Result<String> {
    let mut tries = vec![want.to_string()];
    if let Some(h) = handle {
        tries.push(format!("{want} ({h})"));
    }
    tries.extend((2..20).map(|n| format!("{want} {n}")));
    for t in tries {
        if hc.kernel.find_tx(&elevated(hc.ctx), hc.tx, "SocialAccount", &[Filter::eq("account_name", t.as_str())], None, 1).await?.is_empty() {
            return Ok(t);
        }
    }
    Ok(format!("{want} {}", Uuid::now_v7()))
}

fn picked(params: &Value) -> Vec<String> {
    match params.get("picked") {
        Some(Value::Array(a)) => a.iter().filter_map(text_of).collect(),
        Some(Value::String(t)) => t.split([',', '\n']).map(str::trim).filter(|t| !t.is_empty()).map(str::to_string).collect(),
        _ => vec![],
    }
}

/// The account's lookup actions; `None` for another.
pub(crate) async fn action(hc: &mut HookCtx<'_>, name: &str, doc: &Doc, params: &Value) -> Result<Option<Value>> {
    if !matches!(name, "reachable" | "choose" | "mentions" | "choices" | "check" | "rules") {
        return Ok(None);
    }
    let r = rules_of(hc.kernel, hc.ctx, hc.tx, doc).await?.unwrap_or_default();
    let account = id(doc, "id").ok_or_else(|| Error::internal("an account without an id"))?;
    let system = elevated(hc.ctx);
    match name {
        // What its network takes and asks, as the composer reads it: the description's, else the profile's.
        "rules" => Ok(Some(json!({"rules": r}))),
        "reachable" => Ok(Some(json!({"accounts": reachable(hc, doc, &r).await?}))),
        "choose" => {
            let want = picked(params);
            if want.is_empty() {
                return Err(Error::field("picked", "pick one or more of what the sign-in reaches"));
            }
            let found = reachable(hc, doc, &r).await?;
            let mut out = vec![];
            let mut this_used = s(doc, "network_id").is_some();
            for p in want {
                let item = found.iter().find(|f| f["id"] == p).ok_or_else(|| Error::field("picked", format!("the sign-in does not reach {p}")))?;
                let display = item["name"].as_str().unwrap_or(&p).to_string();
                let handle = item["handle"].as_str().map(str::to_string);
                let values = json!({"network_id": p, "display_name": display, "picture": item["picture"]});
                let existing = item["account"].as_str().and_then(|a| a.parse::<Uuid>().ok());
                let (target, created) = match existing {
                    Some(a) => (a, false),
                    None if !this_used => {
                        this_used = true;
                        (account, false)
                    }
                    None => {
                        let name = free_name(hc, &display, handle.as_deref()).await?;
                        let mut fresh = json!({"account_name": name, "brand": s(doc, "brand"), "network": s(doc, "network"), "profile": s(doc, "profile"),
                                               "handle": handle.clone().unwrap_or_else(|| p.clone()), "connection": s(doc, "connection"),
                                               "connected_by": s(doc, "connected_by"), "default_kind": s(doc, "default_kind")});
                        if let Some(d) = doc.get("default_settings").filter(|d| !d.is_null()) {
                            fresh["default_settings"] = d.clone();
                        }
                        let made = hc.kernel.insert_tx(hc.ctx, hc.tx, "SocialAccount", obj(fresh)).await?;
                        (id(&made, "id").unwrap_or_default(), true)
                    }
                };
                let mut values = values;
                if let Some(h) = &handle {
                    values["handle"] = json!(h.trim_start_matches('@'));
                }
                hc.kernel.set_values_tx(&system, hc.tx, "SocialAccount", target, obj(values)).await?;
                let row = hc.kernel.get_tx(&system, hc.tx, "SocialAccount", target, false).await?;
                out.push(json!({"id": target, "account_name": s(&row, "account_name"), "network_id": p, "created": created}));
            }
            Ok(Some(json!({"accounts": out})))
        }
        "mentions" => {
            let look = r.mentions.clone().ok_or_else(|| Error::invalid(format!("{} offers no mentions to look up", r.name())))?;
            let query = params.get("query").and_then(Value::as_str);
            let items = call(hc, doc, &r, &look, query, &Map::new()).await?;
            let found: Vec<Value> = items
                .iter()
                .filter_map(|i| {
                    let value = text_of(&at(i, Some(&look.value_at)))?;
                    let label = text_of(&at(i, look.label_at.as_deref())).unwrap_or_else(|| value.clone());
                    let text = look.format.as_deref().map_or_else(
                        || format!("@{value}"),
                        |f| f.replace("{{ value }}", &value).replace("{{value}}", &value).replace("{{ label }}", &label).replace("{{label}}", &label),
                    );
                    Some(json!({"value": value, "label": label, "picture": text_of(&at(i, look.picture_at.as_deref())), "text": text}))
                })
                .collect();
            Ok(Some(json!({"mentions": found})))
        }
        "choices" => {
            let key = params.get("setting").and_then(Value::as_str).unwrap_or_default();
            let setting = r.settings.iter().find(|s| s.key == key).ok_or_else(|| Error::field("setting", format!("{} asks nothing called {key}", r.name())))?;
            let settings = params.get("settings").and_then(Value::as_object).cloned().unwrap_or_default();
            let mut out: Vec<Value> = setting.options.iter().map(|o| json!({"value": o.value(), "label": o.label()})).collect();
            if let Some(look) = &setting.choices {
                let items = call(hc, doc, &r, look, params.get("query").and_then(Value::as_str), &settings).await?;
                out = items
                    .iter()
                    .filter_map(|i| {
                        let v = at(i, Some(&look.value_at));
                        (!v.is_null()).then(|| json!({"value": v, "label": text_of(&at(i, look.label_at.as_deref())).or_else(|| text_of(&v))}))
                    })
                    .collect();
            }
            Ok(Some(json!({"choices": out})))
        }
        _ => {
            // `check`: the connection's state, then the account's name, handle and picture where the description gives them.
            let (state, note) = crate::accounts::refresh(hc.kernel, hc.ctx, hc.tx, doc).await?;
            let mut said = Value::Null;
            if state == "Connected"
                && let Some(conn) = reach(hc.tx, hc.ctx.tenant, s(doc, "connection")).await?
                && conn.has(r.op("account"))
                && conn.reads(r.op("account"))
            {
                let calling = caller(hc.kernel, hc.tx, hc.ctx.tenant, doc, &conn).await?;
                let op = r.op("account").to_string();
                let inputs = json!({"account_id": network_id(doc), "handle": s(doc, "handle")});
                let c = Call { connection: conn.id.to_string(), operation: op.clone(), inputs: conn.fit(&op, inputs), key: None, entity: None, record: None };
                if let Ok(got) = connections::call(hc.kernel, &calling, Some(crate::APP), &c).await {
                    let mut values = Map::new();
                    for (field, from) in [("display_name", "name"), ("handle", "handle"), ("picture", "picture")] {
                        if let Some(v) = text_of(&got[from]) {
                            values.insert(field.into(), json!(if field == "handle" { v.trim_start_matches('@').to_string() } else { v }));
                        }
                    }
                    if !values.is_empty() {
                        hc.kernel.set_values_tx(&system, hc.tx, "SocialAccount", account, values.clone()).await?;
                    }
                    said = Value::Object(values);
                }
            }
            Ok(Some(json!({"state": state, "note": note, "read": said})))
        }
    }
}
