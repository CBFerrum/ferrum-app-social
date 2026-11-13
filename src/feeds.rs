//! Feeds (autopost): each new item of a feed becomes a post of the feed's brand, to its accounts, from its template
//! or set, and is then left a draft, sent for review, added to the queue or published now, as the feed says; its words
//! rewritten by writing help first when the feed asks, its picture taken into the media library when it has one. Each
//! item once (`item_key`, unique: the feed and the item's id or address), only items newer than the feed, and what
//! was made of each, or why not, is a Feed Item.
//!
//! [`take`] does all of that from items already read. Reading is the feed's connection's operation (`feed` unless it
//! names another), its items where the description's `meta.feed` says (`items_at`, `id_at`, `link_at`, `title_at`,
//! `text_at`, `picture_at`, `published_at`). Today the platform reads JSON answers only, so a JSON feed works (example:
//! docs/connectors/services/json_feed.json). Once the connections read XML (GAPS.md P7), an RSS or Atom feed is the same
//! description with its operation marked to answer XML and its pointers into the XML's shape: nothing here changes.

use crate::outside::{self, at};
use crate::posts::text_of;
use crate::{content, obj};
use chrono::{DateTime, Duration, Utc};
use ferrum_domain::util::{elevated, id, rows, s};
use ferrum_kernel::{Ctx, Doc, Error, Filter, HookCtx, Kernel, Result, Tx};
use serde_json::{Value, json};
use uuid::Uuid;

/// An item of a feed, as read.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct FeedItem {
    pub id: Option<String>,
    pub link: Option<String>,
    pub title: Option<String>,
    pub text: Option<String>,
    pub picture: Option<String>,
    pub published_at: Option<DateTime<Utc>>,
}

/// The items of a feed's answer, where `spec` (`meta.feed`) says they are.
pub(crate) fn items_of(spec: &Value, answer: &Value) -> Vec<FeedItem> {
    let list = at(answer, spec["items_at"].as_str().unwrap_or(""));
    let p = |i: &Value, k: &str| spec[k].as_str().and_then(|ptr| text_of(&at(i, ptr)));
    list.as_array()
        .into_iter()
        .flatten()
        .map(|i| FeedItem {
            id: p(i, "id_at"),
            link: p(i, "link_at"),
            title: p(i, "title_at"),
            text: p(i, "text_at"),
            picture: p(i, "picture_at"),
            published_at: p(i, "published_at").and_then(|t| DateTime::parse_from_rfc3339(&t).ok()).map(|t| t.with_timezone(&Utc)),
        })
        .filter(|i| i.id.is_some() || i.link.is_some())
        .collect()
}

/// What is made of one item; the Feed Item's state and what it says.
async fn one(k: &Kernel, ctx: &Ctx, tx: &mut Tx, feed: &Doc, item: &FeedItem) -> Result<(Option<String>, &'static str, Option<String>)> {
    let system = elevated(ctx);
    let mut text = [item.title.as_deref(), item.text.as_deref()].into_iter().flatten().find(|t| !t.trim().is_empty()).unwrap_or_default().to_string();
    if feed.get("rewrite").and_then(Value::as_bool) == Some(true) {
        let conn = outside::from_setting(k, ctx, tx, "writing_connection", "writing help").await?;
        // As the system, with no person to read the prompt templates for: the feed's instruction is the prompt.
        let instruction = s(feed, "rewrite_instruction").unwrap_or("Write a social media post about this.");
        let got = outside::call(
            k,
            ctx,
            &conn,
            "write",
            json!({"system": instruction, "prompt": text, "max_tokens": 1024}),
            Some(format!("feed-write:{}", Uuid::now_v7())),
            None,
        )
        .await?;
        text = text_of(&got["text"]).unwrap_or(text);
    }
    let from = match (id(feed, "template"), id(feed, "set")) {
        (Some(t), _) => Some(k.get_tx(&system, tx, "SocialTemplate", t, false).await?),
        (None, Some(s_)) => Some(k.get_tx(&system, tx, "SocialSet", s_, false).await?),
        _ => None,
    };
    let mut values = content::content_of(
        from.as_ref().unwrap_or(&Doc::new()),
        json!({"headline": item.title.clone().unwrap_or_else(|| text.chars().take(60).collect()),
        "brand": s(feed, "brand"), "text": text, "link": item.link}),
    );
    if !rows(feed, "targets").is_empty() {
        let t: Vec<Value> = rows(feed, "targets")
            .into_iter()
            .map(|mut r| {
                for f in ["id", "parent_id", "idx"] {
                    r.remove(f);
                }
                Value::Object(r)
            })
            .collect();
        values.insert("targets".into(), Value::Array(t));
    }
    // Its picture, through the feed's connection, into the media library.
    if feed.get("take_picture").and_then(Value::as_bool) == Some(true)
        && let Some(picture) = &item.picture
        && let Some(conn) = outside::named(tx, ctx.tenant, s(feed, "connection").unwrap_or_default()).await?
        && conn.has(&outside::op(&conn, "fetch_media"))
    {
        let got = outside::call(
            k,
            ctx,
            &conn,
            "fetch_media",
            json!({"url": picture}),
            Some(format!("feed-picture:{}:{picture}", s(feed, "id").unwrap_or_default())),
            None,
        )
        .await?;
        if let Some(file) = text_of(&got["file"]) {
            let asset = k
                .insert_tx(
                    &system,
                    tx,
                    "SocialMediaAsset",
                    obj(json!({"file": format!("/files/{file}"), "brand": s(feed, "brand"), "source": "Feed", "source_id": picture,
                                                                        "alt": item.title, "added_by": s(feed, "owner")})),
                )
                .await?;
            let mut media = values.get("media").and_then(Value::as_array).cloned().unwrap_or_default();
            media.push(json!({"item": 1, "asset": s(&asset, "id"), "alt": item.title}));
            values.insert("media".into(), Value::Array(media));
        }
    }
    sqlx::query("savepoint social_feed_post").execute(&mut **tx).await?;
    let made = match k.insert_tx(&system, tx, "SocialPost", values).await {
        Ok(m) => m,
        Err(e) if !e.is_retryable() => {
            sqlx::query("rollback to savepoint social_feed_post").execute(&mut **tx).await?;
            return Ok((None, "Failed", Some(e.to_string())));
        }
        Err(e) => return Err(e),
    };
    let post = id(&made, "id").unwrap_or_default();
    k.set_values_tx(&system, tx, "SocialPost", post, obj(json!({"author": s(feed, "owner")}))).await?;
    let action = match s(feed, "then") {
        Some("Send for Review") => Some("send_for_review"),
        Some("Add to Queue") => Some("add_to_queue"),
        Some("Publish Now") => Some("publish_now"),
        _ => None,
    };
    if let Some(action) = action {
        sqlx::query("savepoint social_feed_then").execute(&mut **tx).await?;
        if let Err(e) = k.run_action_tx(&system, tx, "SocialPost", post, action, &json!({})).await {
            sqlx::query("rollback to savepoint social_feed_then").execute(&mut **tx).await?;
            return Ok((Some(post.to_string()), "Made", Some(format!("kept as a draft: {e}"))));
        }
    }
    Ok((Some(post.to_string()), "Made", None))
}

/// The items of `feed` read: each new one taken once. How many posts were made.
pub(crate) async fn take(k: &Kernel, ctx: &Ctx, tx: &mut Tx, feed: &Doc, items: &[FeedItem]) -> Result<usize> {
    let feed_id = id(feed, "id").ok_or_else(|| Error::internal("a feed without an id"))?;
    let since = crate::at(feed, "since");
    let mut made = 0;
    for item in items {
        if item.published_at.zip(since).is_some_and(|(p, s)| p < s) {
            continue;
        }
        let key = format!("{feed_id}:{}", item.id.clone().or(item.link.clone()).unwrap_or_default());
        if !k.find_tx(ctx, tx, "SocialFeedItem", &[Filter::eq("item_key", key.as_str())], None, 1).await?.is_empty() {
            continue;
        }
        let (post, state, note) = one(k, ctx, tx, feed, item).await?;
        made += usize::from(state == "Made");
        let row = json!({"feed": feed_id.to_string(), "item_key": key, "title": item.title, "link": item.link, "published_at": item.published_at.map(|p| p.to_rfc3339()),
                         "post": post, "state": state, "note": note});
        k.insert_tx(ctx, tx, "SocialFeedItem", obj(row)).await?;
    }
    Ok(made)
}

/// Every active feed due, read through its connection and taken (the upkeep). A feed that cannot be read says why.
pub(crate) async fn read(k: &Kernel, tenant: Uuid, now: DateTime<Utc>) -> Result<()> {
    let ctx = Ctx::system(tenant, k.registry.schema(tenant).await?);
    let mut tx = k.db.tenant_tx(tenant).await?;
    let feeds = k.find_tx(&ctx, &mut tx, "SocialFeed", &[Filter::eq("active", true)], None, 200).await?;
    tx.commit().await?;
    for f in feeds {
        let every = f.get("every_minutes").and_then(Value::as_i64).unwrap_or(60).max(5);
        if crate::at(&f, "read_at").is_some_and(|r| now - r < Duration::minutes(every)) {
            continue;
        }
        let feed_id = id(&f, "id").unwrap_or_default();
        let mut tx = k.db.tenant_tx(tenant).await?;
        // The feed's row held, so two upkeeps read it once.
        let feed = k.get_tx(&ctx, &mut tx, "SocialFeed", feed_id, true).await?;
        if crate::at(&feed, "read_at").is_some_and(|r| now - r < Duration::minutes(every)) {
            continue;
        }
        let got = async {
            let conn = outside::named(&mut tx, tenant, s(&feed, "connection").unwrap_or_default())
                .await?
                .ok_or_else(|| Error::invalid("no connection by that name"))?;
            let answer = outside::call(k, &ctx, &conn, s(&feed, "operation").unwrap_or("feed"), json!({}), None, None).await?;
            Ok::<_, Error>(items_of(&conn.meta()["feed"], &answer))
        }
        .await;
        let error = match got {
            Ok(items) => take(k, &ctx, &mut tx, &feed, &items).await.err(),
            Err(e) => Some(e),
        };
        // Asked to wait (the feed's host, or writing help): what was taken stays, the rest at the next upkeep after it.
        if error.as_ref().is_some_and(outside::later) {
            tx.commit().await?;
            continue;
        }
        let error = error.map(|e| e.to_string());
        k.set_values_tx(&ctx, &mut tx, "SocialFeed", feed_id, obj(json!({"read_at": now.to_rfc3339(), "last_error": error}))).await?;
        tx.commit().await?;
    }
    Ok(())
}

pub struct FeedHook;

#[async_trait::async_trait]
impl ferrum_kernel::DocHook for FeedHook {
    fn name(&self) -> &'static str {
        "social.feed"
    }

    fn fills(&self) -> &'static [&'static str] {
        &["since", "owner"]
    }

    async fn before_validate(&self, hc: &mut HookCtx<'_>, doc: &mut Doc) -> Result<()> {
        before_save(hc, doc).await
    }
}

/// A new feed takes items from now on, as its owner's.
async fn before_save(hc: &mut HookCtx<'_>, doc: &mut Doc) -> Result<()> {
    if s(doc, "since").is_none() {
        doc.insert("since".into(), json!(Utc::now().to_rfc3339()));
    }
    if s(doc, "owner").is_none() && !hc.ctx.email.is_empty() {
        doc.insert("owner".into(), json!(hc.ctx.email.to_lowercase()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn items_where_the_description_says() {
        let spec = json!({"items_at": "/items", "id_at": "/id", "link_at": "/url", "title_at": "/title", "text_at": "/content_text", "picture_at": "/image", "published_at": "/date_published"});
        let answer = json!({"items": [{"id": "1", "url": "https://b.example/1", "title": "One", "date_published": "2026-11-13T10:00:00Z", "image": "https://b.example/1.png"},
                                      {"title": "No id or address"}]});
        let got = items_of(&spec, &answer);
        assert_eq!(got.len(), 1);
        assert_eq!(
            (got[0].title.as_deref(), got[0].picture.as_deref(), got[0].published_at.map(|p| p.to_rfc3339())),
            (Some("One"), Some("https://b.example/1.png"), Some("2026-11-13T10:00:00+00:00".into()))
        );
    }
}
