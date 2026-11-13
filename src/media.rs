//! The media library (Social Media Assets): pictures, video, GIFs and documents kept with their description,
//! dimensions, length, where they came from and whose they are, for posts to use (a post's media line naming an asset
//! is filled from it). Three ways in besides an upload, each through a connection, so the platform's guard fetches the
//! bytes (its hosts only, no credentials sent, at most the description's `max_bytes`: 10 MB unless it says, 25 MB at
//! most) and stores them:
//!
//! * `media_search` / `media_take`: stock pictures and GIFs from the sources named in Social Settings (`media_sources`),
//!   each a description offering `search_media` (query, page) and `fetch_media` (the chosen one's `id` and `url`; its
//!   `result` stores the file), and, where the source asks to be told of a download, `media_downloaded`;
//! * `media_from_address`: an address, through the connection named in `fetch_connection` (`fetch_media` with `url`).
//!
//! What a source's answer holds is said in its `meta.social.media_search` (pointers into each item).

use crate::obj;
use crate::outside::{self, at, from_setting};
use crate::posts::text_of;
use ferrum_domain::util::{elevated, id, s};
use ferrum_kernel::{Doc, Error, HookCtx, Result};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uuid::Uuid;

/// What a stock item is, read where the source's `meta.social.media_search` says.
fn item(spec: &Value, i: &Value) -> Option<Value> {
    let p = |k: &str| spec[k].as_str().map(|ptr| at(i, ptr)).unwrap_or(Value::Null);
    let id_ = text_of(&p("id_at"))?;
    Some(json!({"id": id_, "preview": text_of(&p("preview_at")), "url": text_of(&p("url_at")), "credit": text_of(&p("credit_at")),
                "credit_link": text_of(&p("credit_link_at")), "licence": spec["licence"], "width": p("width_at"), "height": p("height_at"),
                "alt": text_of(&p("alt_at")), "kind": spec["kind"].as_str().unwrap_or("image")}))
}

/// The stock source a person asked for: one of those Social Settings name, the first when none is said.
async fn source(hc: &mut HookCtx<'_>, given: Option<&str>) -> Result<crate::accounts::Reach> {
    let set = outside::settings(hc.kernel, hc.ctx, hc.tx).await?;
    let listed: Vec<String> =
        s(&set, "media_sources").unwrap_or_default().split([',', '\n']).map(str::trim).filter(|t| !t.is_empty()).map(str::to_string).collect();
    let want = given.map(str::trim).filter(|g| !g.is_empty()).map(str::to_string).or_else(|| listed.first().cloned());
    let Some(want) = want.filter(|w| listed.iter().any(|l| l.eq_ignore_ascii_case(w))) else {
        return Err(Error::invalid("no picture source of that name is set: an administrator lists them in Social Settings"));
    };
    outside::named(hc.tx, hc.ctx.tenant, &want).await?.ok_or_else(|| Error::invalid(format!("no connection is named {want}")))
}

/// A file a connection stored, kept as an asset of `brand`.
async fn keep(hc: &mut HookCtx<'_>, brand: Uuid, file: &Value, values: Value) -> Result<Value> {
    let file = text_of(file).ok_or_else(|| Error::invalid("the connection's fetch_media stores no file: its description maps one under result as file"))?;
    let mut row = json!({"brand": brand.to_string(), "file": format!("/files/{file}"), "added_by": hc.ctx.email});
    for (k, v) in values.as_object().into_iter().flatten() {
        if !v.is_null() {
            row[k] = v.clone();
        }
    }
    // As the system: the file was stored by the connection's call, so it is the system's until the asset holds it. Who
    // may run the action (Social Author, Social Manager) may make assets.
    let made = hc.kernel.insert_tx(&elevated(hc.ctx), hc.tx, "SocialMediaAsset", obj(row)).await?;
    Ok(json!({"entity": "SocialMediaAsset", "id": s(&made, "id"), "file": s(&made, "file")}))
}

fn param<'a>(params: &'a Value, k: &str) -> Option<&'a str> {
    params.get(k).and_then(Value::as_str).map(str::trim).filter(|v| !v.is_empty())
}

/// The brand's media actions; `None` for another.
pub(crate) async fn action(hc: &mut HookCtx<'_>, name: &str, doc: &Doc, params: &Value) -> Result<Option<Value>> {
    let brand = id(doc, "id").ok_or_else(|| Error::internal("a brand without an id"))?;
    if matches!(name, "media_take" | "media_from_address") && !outside::holds(hc.ctx, &["Social Author", "Social Manager"]) {
        return Err(Error::Forbidden("a Social Author or Manager adds to the media library".into()));
    }
    match name {
        "media_search" => {
            let conn = source(hc, param(params, "source")).await?;
            let spec = conn.meta()["social"]["media_search"].clone();
            let query = param(params, "query").ok_or_else(|| Error::field("query", "say what to look for"))?;
            let page = params.get("page").and_then(Value::as_i64).unwrap_or(1).max(1);
            let out = outside::call(hc.kernel, hc.ctx, &conn, "search_media", json!({"query": query, "page": page, "per_page": 20}), None, None).await?;
            let items: Vec<Value> = at(&out, spec["items_at"].as_str().unwrap_or("")).as_array().into_iter().flatten().filter_map(|i| item(&spec, i)).collect();
            Ok(Some(json!({"source": conn.name, "items": items, "page": page})))
        }
        "media_take" => {
            let conn = source(hc, param(params, "source")).await?;
            let item_id = param(params, "id").ok_or_else(|| Error::field("id", "say which one to take"))?.to_string();
            let url = param(params, "url");
            let key = format!("take:{brand}:{}:{item_id}", conn.id);
            let got = outside::call(hc.kernel, hc.ctx, &conn, "fetch_media", json!({"id": item_id, "url": url}), Some(key.clone()), None).await?;
            let kept = keep(
                hc,
                brand,
                &got["file"],
                json!({"source": "Stock", "source_name": conn.name, "source_id": item_id, "credit": param(params, "credit"), "credit_link": param(params, "credit_link"),
                       "licence": param(params, "licence"), "alt": param(params, "alt"), "width": params.get("width"), "height": params.get("height"),
                       "asset_name": param(params, "name").unwrap_or(&item_id)}),
            )
            .await?;
            // A source that asks to be told of each download (its terms) is told, once.
            if conn.has(&outside::op(&conn, "media_downloaded")) {
                outside::call(hc.kernel, hc.ctx, &conn, "media_downloaded", json!({"id": item_id, "url": url}), Some(format!("{key}:told")), None).await?;
            }
            Ok(Some(kept))
        }
        "media_from_address" => {
            let url = param(params, "url")
                .filter(|u| u.starts_with("https://") || u.starts_with("http://"))
                .ok_or_else(|| Error::field("url", "is a web address"))?;
            let conn = from_setting(hc.kernel, hc.ctx, hc.tx, "fetch_connection", "fetching media from an address").await?;
            let key = format!("fetch:{brand}:{}:{}", hex::encode(&Sha256::digest(url.as_bytes())[..8]), Uuid::now_v7());
            let got = outside::call(hc.kernel, hc.ctx, &conn, "fetch_media", json!({"url": url}), Some(key), None).await?;
            let name = url.rsplit('/').next().unwrap_or(url).split('?').next().unwrap_or(url).to_string();
            Ok(Some(
                keep(
                    hc,
                    brand,
                    &got["file"],
                    json!({"source": "Address", "source_id": url, "asset_name": param(params, "name").unwrap_or(&name), "alt": param(params, "alt")}),
                )
                .await?,
            ))
        }
        _ => Ok(None),
    }
}

/// An asset's kind, size and type from its file; a post's media line naming an asset is filled from it.
pub struct AssetHook;

#[async_trait::async_trait]
impl ferrum_kernel::DocHook for AssetHook {
    fn name(&self) -> &'static str {
        "social.asset"
    }

    fn fills(&self) -> &'static [&'static str] {
        &["kind", "content_type", "bytes"]
    }

    async fn before_validate(&self, hc: &mut HookCtx<'_>, doc: &mut Doc) -> Result<()> {
        let Some(file) = s(doc, "file").and_then(|f| f.strip_prefix("/files/")).and_then(|f| f.parse::<Uuid>().ok()) else {
            return Err(Error::field("file", "is a file stored here"));
        };
        let info = hc.kernel.file_info(&elevated(hc.ctx), file).await?;
        if s(doc, "kind").is_none_or(|k| k.trim().is_empty()) {
            doc.insert("kind".into(), json!(crate::rules::kind_of(&info.content_type)));
        }
        if s(doc, "asset_name").is_none_or(|n| n.trim().is_empty()) {
            doc.insert("asset_name".into(), json!(info.name));
        }
        doc.insert("content_type".into(), json!(info.content_type));
        doc.insert("bytes".into(), json!(info.size));
        Ok(())
    }
}

/// A post's (or template's, or set's) media lines that name an asset, filled from it where they say nothing: its file,
/// kind, description, dimensions and length.
pub(crate) async fn fill_lines(hc: &mut HookCtx<'_>, doc: &mut Doc) -> Result<()> {
    let Some(lines) = doc.get_mut("media").and_then(Value::as_array_mut) else { return Ok(()) };
    let mut out = lines.clone();
    for line in out.iter_mut() {
        let Some(asset) = line["asset"].as_str().and_then(|a| a.parse::<Uuid>().ok()) else { continue };
        let a = hc.kernel.get_tx(&elevated(hc.ctx), hc.tx, "SocialMediaAsset", asset, false).await?;
        for (to, from) in [("file", "file"), ("kind", "kind"), ("alt", "alt"), ("width", "width"), ("height", "height"), ("seconds", "seconds")] {
            if line.get(to).is_none_or(|v| v.is_null() || *v == "") && a.get(from).is_some_and(|v| !v.is_null()) {
                line[to] = a[from].clone();
            }
        }
    }
    if let Some(lines) = doc.get_mut("media").and_then(Value::as_array_mut) {
        *lines = out;
    }
    Ok(())
}
