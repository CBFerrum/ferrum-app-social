//! Writing help, through the connection Social Settings names (`writing_connection`): its `write` operation takes
//! `system`, `prompt`, `max_tokens`, `model` (Social Settings' `writing_model`) and, to describe a picture, `image_data`
//! (base64) and `image_type`, and gives `text`; its `picture` operation takes `prompt` and stores a file (`result`
//! `file`). What is asked is the package's message templates (`social.prompt_*`, which a workspace edits), read for the
//! person asking with the post's and brand's values (the brand's `voice` among them). Nothing is saved over a post: each
//! action gives what was written for the person to take or leave, and every use is kept (Writing Help Uses) with who
//! asked, about which post, what was asked and what came back. Only holders of the role Social Writing Help may ask.

use crate::content;
use crate::obj;
use crate::outside::{self, from_setting};
use crate::posts::{rules_of, text_of};
use crate::rules::{self, Text};
use ferrum_domain::util::{elevated, id, rows, s};
use ferrum_kernel::messages::{self, Message, Recipient};
use ferrum_kernel::{Doc, Error, HookCtx, Result};
use serde_json::{Map, Value, json};
use uuid::Uuid;

pub(crate) const ROLE: &str = "Social Writing Help";

/// A prompt template read for the person asking: its text body.
async fn prompt(hc: &mut HookCtx<'_>, template: &str, _about: (&str, Uuid), values: Value) -> Result<String> {
    let m = Message { template: template.into(), to: vec![Recipient::user(hc.ctx.user)], values: obj(values), ..Default::default() };
    let r = messages::render_for(hc.kernel, hc.tx, hc.ctx, &m, "email").await?;
    Ok(r.text.unwrap_or_default().trim().to_string())
}

struct Ask<'a> {
    action: &'a str,
    post: Option<Uuid>,
    brand: Option<Uuid>,
    image: Option<(String, String)>,
}

/// One use of `write`: what came back, kept with who asked and what.
async fn write(hc: &mut HookCtx<'_>, ask: Ask<'_>, system: String, text: String) -> Result<String> {
    let conn = from_setting(hc.kernel, hc.ctx, hc.tx, "writing_connection", "writing help").await?;
    let set = outside::settings(hc.kernel, hc.ctx, hc.tx).await?;
    let model = s(&set, "writing_model").filter(|m| !m.trim().is_empty()).unwrap_or("claude-sonnet-5-5").to_string();
    let mut inputs = json!({"model": model, "max_tokens": 1024, "system": system, "prompt": text});
    if let Some((data, kind)) = &ask.image {
        inputs["image_data"] = json!(data);
        inputs["image_type"] = json!(kind);
    }
    let about = ask.post.map(|p| ("SocialPost", p));
    let got = outside::call(hc.kernel, hc.ctx, &conn, "write", inputs, Some(format!("write:{}", Uuid::now_v7())), about).await?;
    let answer = text_of(&got["text"]).ok_or_else(|| Error::invalid(format!("{}'s write gives no text: its description maps one under result", conn.name)))?;
    let row = json!({"post": ask.post.map(|p| p.to_string()), "brand": ask.brand.map(|b| b.to_string()), "action": ask.action, "by": hc.ctx.email,
                     "prompt": text, "answer": answer, "connection": conn.name});
    hc.kernel.insert_tx(&elevated(hc.ctx), hc.tx, "SocialWritingUse", obj(row)).await?;
    Ok(answer)
}

/// The values every prompt reads: the brand's name and voice.
async fn brand_values(hc: &mut HookCtx<'_>, brand: Option<Uuid>) -> Result<Map<String, Value>> {
    let mut v = Map::new();
    if let Some(b) = brand {
        let doc = hc.kernel.get_tx(&elevated(hc.ctx), hc.tx, "SocialBrand", b, false).await?;
        v.insert("brand".into(), json!(s(&doc, "brand_name")));
        v.insert("voice".into(), json!(s(&doc, "voice")));
    }
    Ok(v)
}

fn with(mut base: Map<String, Value>, more: Value) -> Value {
    base.extend(obj(more));
    Value::Object(base)
}

fn given<'a>(params: &'a Value, k: &str) -> Option<&'a str> {
    params.get(k).and_then(Value::as_str).map(str::trim).filter(|v| !v.is_empty())
}

/// The writing actions of a post, and the brand's `write_new`; `None` for another.
pub(crate) async fn action(hc: &mut HookCtx<'_>, entity: &str, name: &str, doc: &Doc, params: &Value) -> Result<Option<Value>> {
    let known = match entity {
        "SocialBrand" => matches!(name, "write_new"),
        _ => matches!(name, "write" | "rewrite" | "adapt" | "hashtags" | "describe" | "thread" | "picture"),
    };
    if !known {
        return Ok(None);
    }
    if !outside::holds(hc.ctx, &[ROLE]) {
        return Err(Error::Forbidden(format!("writing help is for those holding {ROLE}")));
    }
    let record = id(doc, "id").ok_or_else(|| Error::internal("a record without an id"))?;
    let (post, brand) = if entity == "SocialBrand" { (None, Some(record)) } else { (Some(record), id(doc, "brand")) };
    let base = brand_values(hc, brand).await?;
    let about = (entity, record);
    let system = prompt(hc, "social.prompt_system", about, Value::Object(base.clone())).await?;
    let ask = |action| Ask { action, post, brand, image: None };
    let out = match name {
        "write" | "write_new" => {
            let p = given(params, "prompt").ok_or_else(|| Error::field("prompt", "say what to write"))?;
            let text = prompt(hc, "social.prompt_write", about, with(base, json!({"prompt": p}))).await?;
            json!({"text": write(hc, ask(name), system, text).await?})
        }
        "rewrite" => {
            let instruction = match (given(params, "instruction"), params.get("preset").and_then(Value::as_str).and_then(|p| p.parse::<Uuid>().ok())) {
                (Some(i), _) => i.to_string(),
                (None, Some(p)) => {
                    s(&hc.kernel.get_tx(&elevated(hc.ctx), hc.tx, "SocialWritingPreset", p, false).await?, "instruction").unwrap_or_default().to_string()
                }
                _ => return Err(Error::field("instruction", "say how to rewrite it, or pick a preset")),
            };
            let text = prompt(hc, "social.prompt_rewrite", about, with(base, json!({"instruction": instruction, "text": s(doc, "text")}))).await?;
            json!({"text": write(hc, ask("rewrite"), system, text).await?})
        }
        "adapt" => {
            // Each account's version, written within what its network takes; given back, not saved.
            let mut versions = vec![];
            for account in content::accounts(doc) {
                let a = hc.kernel.get_tx(&elevated(hc.ctx), hc.tx, "SocialAccount", account, false).await?;
                let r = rules_of(hc.kernel, hc.ctx, hc.tx, &a).await?.unwrap_or_default();
                let target = rows(doc, "targets").into_iter().find(|t| id(t, "account") == Some(account)).unwrap_or_default();
                let kind = r.kind(s(&target, "kind")).ok().flatten();
                let (t, _, _, _): (Text, _, _, _) = r.effective(kind);
                let values = with(
                    base.clone(),
                    json!({"network": r.name(), "max": t.max, "format": t.format.clone().unwrap_or_else(|| "plain".into()), "kind": kind.map(|k| k.name.clone()), "text": s(doc, "text")}),
                );
                let text = prompt(hc, "social.prompt_adapt", about, values).await?;
                let written = write(hc, ask("adapt"), system.clone(), text).await?;
                let length = rules::length(&t, &written);
                versions.push(json!({"account": account, "account_name": s(&a, "account_name"), "text": written, "length": length, "max": t.max,
                                     "fits": t.max.is_none_or(|m| length <= m)}));
            }
            json!({"versions": versions})
        }
        "hashtags" => {
            let count = params.get("count").and_then(Value::as_i64).unwrap_or(5).clamp(1, 30);
            let text = prompt(hc, "social.prompt_hashtags", about, with(base, json!({"text": s(doc, "text"), "count": count}))).await?;
            let said = write(hc, ask("hashtags"), system, text).await?;
            let mut tags: Vec<String> = vec![];
            for w in said.split(|c: char| c.is_whitespace() || c == ',').filter(|w| w.starts_with('#') && w.len() > 1) {
                if !tags.iter().any(|t| t.eq_ignore_ascii_case(w)) {
                    tags.push(w.to_string());
                }
            }
            tags.truncate(usize::try_from(count).unwrap_or(5));
            json!({"hashtags": tags})
        }
        "describe" => {
            let index = params.get("medium").and_then(Value::as_u64).unwrap_or(0) as usize;
            let line = rows(doc, "media").into_iter().nth(index);
            let file = line.as_ref().and_then(|l| s(l, "file")).or(s(doc, "image").filter(|_| index == 0));
            let file: Uuid =
                file.and_then(|f| f.strip_prefix("/files/")).and_then(|f| f.parse().ok()).ok_or_else(|| Error::field("medium", "no picture there"))?;
            let (_, kind, bytes) = hc.kernel.read_file(&elevated(hc.ctx), file).await?;
            let text = prompt(hc, "social.prompt_alt", about, with(base, json!({"text": s(doc, "text")}))).await?;
            let mut a = ask("describe");
            a.image = Some((crate::base64(&bytes), kind));
            json!({"alt": write(hc, a, system, text).await?, "medium": index})
        }
        "thread" => {
            let parts = params.get("parts").and_then(Value::as_i64).unwrap_or(3).clamp(2, 25);
            let text = prompt(hc, "social.prompt_thread", about, with(base, json!({"text": s(doc, "text"), "parts": parts}))).await?;
            let said = write(hc, ask("thread"), system, text).await?;
            let items: Vec<String> = said.split("\n---").map(|p| p.trim().trim_start_matches('-').trim().to_string()).filter(|p| !p.is_empty()).collect();
            json!({"items": items})
        }
        _ => {
            // `picture`: made by the connection's `picture`, kept as a media asset (Generated).
            let p = given(params, "prompt").ok_or_else(|| Error::field("prompt", "say what to draw"))?;
            let text = prompt(hc, "social.prompt_picture", about, with(base, json!({"prompt": p}))).await?;
            let conn = from_setting(hc.kernel, hc.ctx, hc.tx, "writing_connection", "writing help").await?;
            let got = outside::call(hc.kernel, hc.ctx, &conn, "picture", json!({"prompt": text}), Some(format!("picture:{}", Uuid::now_v7())), None).await?;
            let file = text_of(&got["file"])
                .ok_or_else(|| Error::invalid(format!("{}'s picture stores no file: its description maps one under result as file", conn.name)))?;
            let made = hc
                .kernel
                .insert_tx(&elevated(hc.ctx), hc.tx, "SocialMediaAsset", obj(json!({"file": format!("/files/{file}"), "brand": brand.map(|b| b.to_string()),
                                                                                      "source": "Generated", "source_name": conn.name, "alt": p, "added_by": hc.ctx.email})))
                .await?;
            let row = json!({"post": post.map(|p| p.to_string()), "brand": brand.map(|b| b.to_string()), "action": "picture", "by": hc.ctx.email, "prompt": text,
                             "answer": s(&made, "file"), "connection": conn.name});
            hc.kernel.insert_tx(&elevated(hc.ctx), hc.tx, "SocialWritingUse", obj(row)).await?;
            json!({"entity": "SocialMediaAsset", "id": s(&made, "id"), "file": s(&made, "file")})
        }
    };
    Ok(Some(out))
}
