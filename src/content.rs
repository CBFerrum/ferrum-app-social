//! A post's content for one account. A post has an original and, for any account, a version of its own:
//!
//! * the original is the post's `text`, `link`, `image` (with `image_alt`; 0.7.0's one picture, first) and the `media`
//!   lines naming no account for item 1; then its follow-ups, the `items` lines naming no account, in order, each with
//!   the `media` lines numbered for it (item 2, 3, ...);
//! * an account has a version when its `targets` line has words of its own (`text`): the version is that line's `text`
//!   and `link`, the `media` lines naming the account, and the `items` lines naming it. A version is the whole post for
//!   that account; nothing of the original is mixed into it;
//! * the line's `kind` and `settings` (over the account's `default_kind` and `default_settings`) say what kind of post it
//!   is there and answer what the network asks.
//!
//! The words are final here: each `{{name}}` filled from the brand's or account's variables, and the signature added
//! when the post asks for one, never twice. What is checked is what goes out.

use crate::rules::{Copy, Item, Piece, kind_of};
use crate::{at, obj};
use ferrum_domain::util::{elevated, id, rows, s};
use ferrum_kernel::query::parse_filters;
use ferrum_kernel::{Ctx, Doc, Error, Kernel, Result, Tx};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use uuid::Uuid;

/// A stored file a post carries, as far as sending it needs.
#[derive(Debug, Clone, Default)]
pub(crate) struct Med {
    pub file: Uuid,
    pub name: String,
    pub content_type: String,
    pub bytes: i64,
    pub kind: String,
    pub alt: Option<String>,
    pub thumbnail: Option<Uuid>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub seconds: Option<f64>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct It {
    pub text: String,
    pub link: Option<String>,
    pub media: Vec<Med>,
    /// Minutes after the item before.
    pub delay: i64,
}

/// One account's copy of a post.
#[derive(Debug, Clone, Default)]
pub(crate) struct Version {
    pub kind: Option<String>,
    /// (round four) Its title: the account's own (`targets[].title`), else the post's (`title`).
    pub title: Option<String>,
    pub settings: Map<String, Value>,
    pub items: Vec<It>,
    pub first_comment: Option<String>,
    /// What is wrong with it that is not the network's rules (a variable with no value): a draft keeps it, leaving
    /// Draft refuses it.
    pub problems: Vec<String>,
}

impl Version {
    /// The kind it goes as on a network with rules `r`: the one picked, else the one its media say.
    pub fn kind_on(&self, r: &crate::rules::Rules) -> Option<String> {
        r.chosen(self.kind.as_deref(), self.items.first().and_then(|i| i.media.first()).map(|m| m.kind.as_str()))
    }

    pub fn copy(&self) -> Copy {
        let piece = |m: &Med| Piece {
            kind: m.kind.clone(),
            content_type: Some(m.content_type.clone()),
            bytes: Some(m.bytes),
            width: m.width,
            height: m.height,
            seconds: m.seconds,
            alt: m.alt.clone(),
        };
        Copy {
            kind: self.kind.clone(),
            title: self.title.clone(),
            items: self.items.iter().map(|i| Item { text: i.text.clone(), link: i.link.clone(), media: i.media.iter().map(piece).collect() }).collect(),
            first_comment: self.first_comment.clone(),
            settings: self.settings.clone(),
        }
    }
}

fn file_id(v: Option<&str>) -> Option<Uuid> {
    v.and_then(|i| i.strip_prefix("/files/")).and_then(|f| f.parse().ok())
}

fn number<T: TryFrom<i64>>(d: &Doc, f: &str) -> Option<T> {
    d.get(f).and_then(Value::as_i64).and_then(|n| T::try_from(n).ok())
}

/// The account a content line is for: none for the original's.
fn for_account(line: &Doc) -> Option<Uuid> {
    id(line, "account")
}

async fn medium(k: &Kernel, ctx: &Ctx, line: &Doc, file: Uuid) -> Result<Med> {
    let info = k.file_info(&elevated(ctx), file).await?;
    let kind = s(line, "kind").filter(|k| !k.trim().is_empty()).map_or_else(|| kind_of(&info.content_type).to_string(), |k| k.trim().to_lowercase());
    Ok(Med {
        file,
        name: info.name,
        bytes: info.size,
        content_type: info.content_type,
        kind,
        alt: s(line, "alt").map(str::to_string),
        thumbnail: file_id(s(line, "thumbnail")),
        width: number(line, "width"),
        height: number(line, "height"),
        seconds: line.get("seconds").and_then(|v| v.as_f64().or_else(|| v.as_str().and_then(|t| t.parse().ok()))),
    })
}

/// The content lines (`items`, `media`) and words of a post, a template or a set, for `account` (its `targets` line
/// `target`, when it has one).
pub(crate) async fn version(k: &Kernel, ctx: &Ctx, tx: &mut Tx, doc: &Doc, account: &Doc) -> Result<Version> {
    let account_id = id(account, "id");
    let target = rows(doc, "targets").into_iter().find(|t| id(t, "account") == account_id && account_id.is_some()).unwrap_or_default();
    let own = s(&target, "text").is_some_and(|t| !t.trim().is_empty());
    let whose = if own { account_id } else { None };
    let (text, link) = if own { (s(&target, "text"), s(&target, "link")) } else { (s(doc, "text"), s(doc, "link")) };
    let link = link.map(str::trim).filter(|l| !l.is_empty()).map(str::to_string);
    let mut items = vec![It { text: text.unwrap_or_default().to_string(), link, ..Default::default() }];
    for line in rows(doc, "items").into_iter().filter(|l| for_account(l) == whose) {
        items.push(It {
            text: s(&line, "text").unwrap_or_default().to_string(),
            link: s(&line, "link").map(str::trim).filter(|l| !l.is_empty()).map(str::to_string),
            delay: number(&line, "delay_minutes").unwrap_or(0),
            media: vec![],
        });
    }
    if !own && let Some(file) = file_id(s(doc, "image")) {
        let mut legacy = Doc::new();
        legacy.insert("alt".into(), json!(s(doc, "image_alt")));
        items[0].media.push(medium(k, ctx, &legacy, file).await?);
    }
    for line in rows(doc, "media").into_iter().filter(|l| for_account(l) == whose) {
        let n: usize = number(&line, "item").unwrap_or(1).max(1);
        let Some(file) = file_id(s(&line, "file")) else { continue };
        let m = medium(k, ctx, &line, file).await?;
        match items.get_mut(n - 1) {
            Some(it) => it.media.push(m),
            None => return Err(Error::field("media", format!("a medium is for item {n}, and {} has {}", copy_name(account), items.len()))),
        }
    }
    let mut settings = account.get("default_settings").and_then(Value::as_object).cloned().unwrap_or_default();
    if let Some(given) = target.get("settings").and_then(Value::as_object) {
        settings.extend(given.clone());
    }
    let kind = s(&target, "kind").or(s(account, "default_kind")).map(str::trim).filter(|k| !k.is_empty()).map(str::to_string);
    let title = s(&target, "title").or(s(doc, "title")).map(str::trim).filter(|t| !t.is_empty()).map(str::to_string);
    let first_comment = s(doc, "first_comment").filter(|c| !c.trim().is_empty()).map(str::to_string);
    let mut v = Version { kind, title, settings, items, first_comment, problems: vec![] };
    crate::library::finish(k, ctx, tx, doc, account, &mut v).await?;
    crate::links::track(k, ctx, tx, doc, account, &mut v).await?;
    Ok(v)
}

fn copy_name(account: &Doc) -> String {
    s(account, "account_name").unwrap_or("the account").to_string()
}

/// What approval fixes: the words, the link, the media and what they say of them, the follow-ups, where and when it
/// goes and what each account is asked, and how it repeats. For a post with nothing but 0.7.0's content this is 0.7.0's
/// fingerprint, so an approval given before the upgrade still holds.
pub(crate) fn fingerprint(doc: &Doc) -> String {
    let mut targets: Vec<String> = rows(doc, "targets").iter().filter_map(|t| s(t, "account").map(str::to_string)).collect();
    targets.sort();
    let mut what = json!([
        s(doc, "text"),
        s(doc, "link"),
        targets,
        at(doc, "scheduled_at").map(|t| t.timestamp()),
        s(doc, "image"),
        s(doc, "image_alt"),
        s(doc, "first_comment")
    ]);
    let lines = |table: &str, fields: &[&str]| -> Vec<Value> {
        rows(doc, table).iter().map(|r| Value::Array(fields.iter().map(|f| r.get(*f).cloned().unwrap_or(Value::Null)).collect())).collect()
    };
    let mut more = Map::new();
    let mut targets_more = lines("targets", &["account", "kind", "text", "link", "settings", "title"]);
    targets_more.retain(|t| t.as_array().is_some_and(|a| a[1..].iter().any(|v| !v.is_null() && *v != "" && *v != json!({}))));
    targets_more.sort_by_key(Value::to_string);
    for (name, v) in [
        ("targets", Value::Array(targets_more)),
        ("items", Value::Array(lines("items", &["account", "text", "link", "delay_minutes"]))),
        ("media", Value::Array(lines("media", &["account", "item", "file", "kind", "alt", "thumbnail"]))),
    ] {
        if v.as_array().is_some_and(|a| !a.is_empty()) {
            more.insert(name.into(), v);
        }
    }
    for f in ["add_signature", "signature", "repeat_every", "repeat_unit", "repeat_until", "repeat_count", "title"] {
        if let Some(v) = doc.get(f).filter(|v| !v.is_null() && **v != json!(false) && **v != json!("")) {
            more.insert(f.into(), v.clone());
        }
    }
    if !more.is_empty() {
        what.as_array_mut().into_iter().for_each(|a| a.push(Value::Object(more.clone())));
    }
    hex::encode(Sha256::digest(what.to_string()))
}

/// The fields a copy of a post's content carries: to a template, a set, a duplicate, the next occurrence of a repeat.
pub(crate) const CONTENT: [&str; 10] = ["headline", "title", "brand", "text", "link", "image", "image_alt", "first_comment", "add_signature", "signature"];

/// `from`'s content as a new record's values (child lines without their ids), its own fields over them.
pub(crate) fn content_of(from: &Doc, own: Value) -> Doc {
    let mut out = Doc::new();
    for f in CONTENT {
        if let Some(v) = from.get(f).filter(|v| !v.is_null()) {
            out.insert(f.into(), v.clone());
        }
    }
    for table in ["targets", "items", "media"] {
        let lines: Vec<Value> = rows(from, table)
            .into_iter()
            .map(|mut r| {
                for f in ["id", "parent_id", "idx", "created_at", "updated_at", "tenant_id", "parent", "parentfield", "parenttype"] {
                    r.remove(f);
                }
                Value::Object(r)
            })
            .collect();
        out.insert(table.into(), Value::Array(lines));
    }
    out.extend(obj(own));
    out
}

/// The accounts a post goes to.
pub(crate) fn accounts(doc: &Doc) -> Vec<Uuid> {
    rows(doc, "targets").iter().filter_map(|t| id(t, "account")).collect()
}

/// Posts that name any of `accounts` and go out between `from` and `to` (the queue's look at what is taken).
pub(crate) async fn taken(k: &Kernel, ctx: &Ctx, tx: &mut Tx, accounts: &[Uuid], from: &str, to: &str, except: Uuid) -> Result<Vec<Doc>> {
    let ids: Vec<Value> = accounts.iter().map(|a| json!(a.to_string())).collect();
    let filters = parse_filters(&json!([["scheduled_at", ">=", from], ["scheduled_at", "<=", to], ["targets.account", "in", ids]]))?;
    let found = k.find_tx(ctx, tx, "SocialPost", &filters, Some("scheduled_at asc"), 500).await?;
    Ok(found.into_iter().filter(|p| id(p, "id") != Some(except)).collect())
}
