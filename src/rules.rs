//! What a network takes and asks, as one declarative shape: `meta.social` on a connection's description, else the same
//! shape in a network profile's `social` field, else the profile's own fields (the 0.6 and 0.7 records). One generic
//! check of a post's content for one account against it; no network is named here. The shape is documented in
//! docs/connectors/README.md ("meta.social"), and docs/connectors/examples/ holds nine networks written in it.

use serde::Deserialize;
use serde_json::{Map, Value};
use std::collections::BTreeMap;

/// How a network counts the length of a text.
#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Count {
    /// Unicode scalar values (what 0.7.0 counted).
    #[default]
    Characters,
    /// What a reader sees as one character: an emoji with its modifiers, a flag, a letter with its accents.
    Graphemes,
    /// UTF-16 code units.
    Utf16,
    /// UTF-8 bytes.
    Bytes,
    /// Weighted as X counts (its published counting rules): most Latin and general punctuation 1, anything else 2, an
    /// emoji 2, each web address `link_counts`.
    Weighted,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Text {
    pub max: Option<usize>,
    pub min: Option<usize>,
    pub count: Count,
    /// What a web address counts as, whatever its length, for a network that shortens them; none: as written.
    pub link_counts: Option<usize>,
    /// `plain`, `markdown` or `html`: what the network shows its text as (the interface's editor and preview).
    pub format: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Medium {
    pub max_bytes: Option<i64>,
    /// Content types taken (`image/png`); none: any of the kind.
    pub types: Vec<String>,
    pub min_width: Option<u32>,
    pub max_width: Option<u32>,
    pub min_height: Option<u32>,
    pub max_height: Option<u32>,
    /// Width over height.
    pub min_ratio: Option<f64>,
    pub max_ratio: Option<f64>,
    pub min_seconds: Option<f64>,
    pub max_seconds: Option<f64>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Media {
    /// Kinds taken: `image`, `video`, `document`, or any other name a medium is given. None: no media.
    pub kinds: Vec<String>,
    pub required: bool,
    pub min: Option<usize>,
    pub max: Option<usize>,
    /// The combinations taken in one item, each kind with how many at most (`[{"image": 4}, {"video": 1}]`): an item's
    /// media must fit one. None: any mix within `max`.
    pub mixes: Vec<BTreeMap<String, usize>>,
    pub image: Option<Medium>,
    pub video: Option<Medium>,
    pub document: Option<Medium>,
    /// Rules of other kinds, by name.
    pub other: BTreeMap<String, Medium>,
    /// Whether each medium's description (alt text) is sent; required; at most this long.
    pub alt: bool,
    pub alt_required: bool,
    pub alt_max: Option<usize>,
    /// Whether a video's picture (thumbnail) is sent.
    pub thumbnail: bool,
}

impl Default for Media {
    fn default() -> Self {
        Media {
            kinds: vec![],
            required: false,
            min: None,
            max: None,
            mixes: vec![],
            image: None,
            video: None,
            document: None,
            other: BTreeMap::new(),
            alt: true,
            alt_required: false,
            alt_max: None,
            thumbnail: false,
        }
    }
}

impl Media {
    fn rule(&self, kind: &str) -> Option<&Medium> {
        match kind {
            "image" => self.image.as_ref(),
            "video" => self.video.as_ref(),
            "document" => self.document.as_ref(),
            other => self.other.get(other),
        }
    }
}

/// What an item after the first is on this network.
#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum FollowAs {
    /// A post answering the one before (`publish` with `reply_to` and `root`).
    Thread,
    /// A comment under the post (`reply` with `in_reply_to` the first item's id).
    Comment,
    #[default]
    None,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct FollowUps {
    #[serde(rename = "as")]
    pub as_: FollowAs,
    /// Whether a follow-up may carry media (under the kind's media rules).
    pub media: bool,
    /// Items after the first, at most.
    pub max: Option<usize>,
    /// Its own length rules, where they differ from the post's.
    pub text: Option<Text>,
}

/// A kind of post a network takes (post, reel, story, short, article, poll...: free names), with its own rules where
/// they differ from the network's.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Kind {
    pub name: String,
    pub label: Option<String>,
    pub text: Option<Text>,
    pub media: Option<Media>,
    pub follow_ups: Option<FollowUps>,
    pub first_comment: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum Opt {
    Plain(String),
    /// The label is the interface's to show.
    Labelled {
        value: Value,
        #[allow(dead_code)]
        label: String,
    },
}

impl Opt {
    pub fn value(&self) -> Value {
        match self {
            Opt::Plain(s) => Value::String(s.clone()),
            Opt::Labelled { value, .. } => value.clone(),
        }
    }
}

/// Where choices, accounts or mentions are read from: an operation of the description, its inputs (literals, or
/// `settings.<key>`, `account.id`, `account.handle`, `query`), the list in its answer, and in each item where its value,
/// label, handle, picture and kind are (JSON pointers).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Lookup {
    pub operation: String,
    pub inputs: Map<String, Value>,
    pub items_at: String,
    pub value_at: String,
    pub label_at: Option<String>,
    pub handle_at: Option<String>,
    pub picture_at: Option<String>,
    pub kind_at: Option<String>,
    /// How a picked mention is written into the text (`@{{ value }}`, `@[{{ label }}](urn:li:organization:{{ value }})`).
    pub format: Option<String>,
}

/// A question the composer asks for this network.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Setting {
    pub key: String,
    pub label: String,
    /// `text`, `long`, `number`, `bool`, `select`, `multi` or `date`.
    #[serde(rename = "type")]
    pub kind: String,
    pub options: Vec<Opt>,
    pub choices: Option<Lookup>,
    pub required: bool,
    pub default: Option<Value>,
    /// Asked only when the post's kind is one of `kind`, and each other key's answer is (one of) what it names.
    pub when: Map<String, Value>,
    pub help: Option<String>,
    /// A number's bounds; a text's or a list's length.
    pub min: Option<f64>,
    pub max: Option<f64>,
}

/// `meta.social`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Rules {
    pub name: Option<String>,
    pub text: Text,
    pub media: Media,
    /// The first is the default.
    pub kinds: Vec<Kind>,
    pub follow_ups: FollowUps,
    pub settings: Vec<Setting>,
    /// Social's name of an operation to the description's, where they differ (`{"publish": "create_pin"}`).
    pub operations: BTreeMap<String, String>,
    pub accounts: Option<Lookup>,
    pub mentions: Option<Lookup>,
    pub first_comment: bool,
    pub link_preview: bool,
    pub scheduling: bool,
    /// `html` for a network whose messages (the inbox) arrive as HTML: kept as the words they hold.
    pub inbox_format: Option<String>,
}

impl Rules {
    /// `meta.social` as written, named `fallback` when it names nothing.
    pub fn parse(v: &Value, fallback: &str) -> Result<Rules, String> {
        let mut r: Rules = serde_json::from_value(v.clone()).map_err(|e| format!("meta.social: {e}"))?;
        r.name.get_or_insert_with(|| fallback.to_string());
        for s in &r.settings {
            if !matches!(s.kind.as_str(), "text" | "long" | "number" | "bool" | "select" | "multi" | "date") || s.key.is_empty() {
                return Err(format!("meta.social: setting {:?} is of type text, long, number, bool, select, multi or date", s.key));
            }
        }
        Ok(r)
    }

    /// The fields of a network profile record, or of 0.7.0's `meta.capabilities` (named as those fields are).
    pub fn of_fields(doc: &Map<String, Value>, fallback: &str) -> Rules {
        let number = |f: &str| doc.get(f).and_then(Value::as_i64);
        let flag = |f: &str, default: bool| doc.get(f).and_then(Value::as_bool).unwrap_or(default);
        let size = |f: &str| number(f).and_then(|n| usize::try_from(n).ok());
        let mut kinds = vec![];
        // 0.7.0: a post carried one picture, and `max_media` under one meant no pictures.
        if flag("images", true) && number("max_media").is_none_or(|m| m >= 1) {
            kinds.push("image".to_string());
        }
        if flag("videos", false) {
            kinds.push("video".to_string());
        }
        let medium = |f: &str| number(f).map(|b| Medium { max_bytes: Some(b), ..Default::default() });
        Rules {
            name: Some(doc.get("profile_name").or(doc.get("name")).and_then(Value::as_str).unwrap_or(fallback).to_string()),
            text: Text { max: size("max_text"), link_counts: size("link_length"), ..Default::default() },
            media: Media {
                kinds,
                required: flag("media_required", false),
                max: size("max_media"),
                image: medium("max_image_bytes"),
                video: medium("max_video_bytes"),
                alt: flag("alt_text", true),
                ..Default::default()
            },
            follow_ups: FollowUps { as_: if flag("threads", false) { FollowAs::Thread } else { FollowAs::None }, media: true, ..Default::default() },
            first_comment: flag("first_comment", false),
            link_preview: flag("link_preview", false),
            scheduling: flag("scheduling", false),
            ..Default::default()
        }
    }

    pub fn name(&self) -> &str {
        self.name.as_deref().unwrap_or("This network")
    }

    /// The description's operation for what Social calls `op`.
    pub fn op<'a>(&'a self, op: &'a str) -> &'a str {
        self.operations.get(op).map_or(op, String::as_str)
    }

    pub fn kind(&self, name: Option<&str>) -> Result<Option<&Kind>, String> {
        match name.filter(|n| !n.is_empty()) {
            None => Ok(self.kinds.first()),
            Some(n) => self.kinds.iter().find(|k| k.name == n).map(Some).ok_or_else(|| format!("{} has no kind of post called {n}", self.name())),
        }
    }

    /// The rules for kind `kind` (the network's where the kind says nothing of its own).
    pub fn effective(&self, kind: Option<&Kind>) -> (Text, Media, FollowUps, bool) {
        let k = kind.cloned().unwrap_or_default();
        (
            k.text.unwrap_or_else(|| self.text.clone()),
            k.media.unwrap_or_else(|| self.media.clone()),
            k.follow_ups.unwrap_or_else(|| self.follow_ups.clone()),
            k.first_comment.unwrap_or(self.first_comment),
        )
    }
}

/// A medium as a post carries it, as far as the rules care (what is not known is not checked).
#[derive(Debug, Clone, Default)]
pub struct Piece {
    pub kind: String,
    pub content_type: Option<String>,
    pub bytes: Option<i64>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub seconds: Option<f64>,
    pub alt: Option<String>,
}

/// The kind a stored file is, by its content type.
pub fn kind_of(content_type: &str) -> &'static str {
    match content_type.split('/').next().unwrap_or_default() {
        "image" => "image",
        "video" => "video",
        _ if content_type == "application/pdf" => "document",
        _ => "file",
    }
}

#[derive(Debug, Clone, Default)]
pub struct Item {
    pub text: String,
    pub link: Option<String>,
    pub media: Vec<Piece>,
}

/// One account's copy of a post, its words final (variables filled, signature added).
#[derive(Debug, Clone, Default)]
pub struct Copy {
    pub kind: Option<String>,
    pub items: Vec<Item>,
    pub first_comment: Option<String>,
    pub settings: Map<String, Value>,
}

/// Bytes in plain words.
pub fn size(bytes: i64) -> String {
    if bytes >= 1 << 20 { format!("{:.1} MB", bytes as f64 / f64::from(1 << 20)) } else { format!("{} KB", (bytes + 1023) / 1024) }
}

fn plural(kind: &str) -> String {
    match kind {
        "image" => "pictures".into(),
        "video" => "video".into(),
        "document" => "documents".into(),
        other => format!("{other}s"),
    }
}

/// Whether `c` joins the character before it into what a reader sees as one.
fn extends(c: char) -> bool {
    matches!(c as u32,
        0x0300..=0x036F | 0x1AB0..=0x1AFF | 0x1DC0..=0x1DFF | 0x20D0..=0x20FF | 0xFE20..=0xFE2F // combining marks
        | 0xFE00..=0xFE0F | 0xE0100..=0xE01EF // variation selectors
        | 0x200D // zero-width joiner
        | 0x1F3FB..=0x1F3FF // skin tones
        | 0xE0020..=0xE007F) // tags (subdivision flags)
}

/// The text split into what a reader sees as characters.
// ponytail: an approximation of Unicode grapheme clusters (combining marks, variation selectors, joined emoji, skin
// tones, flags, tags); Hangul jamo and Indic conjuncts count apart. A segmentation crate if a network proves it matters.
pub fn clusters(text: &str) -> Vec<&str> {
    let mut out: Vec<&str> = vec![];
    let (mut start, mut prev, mut regional) = (0usize, None::<char>, false);
    for (i, c) in text.char_indices() {
        let ri = (0x1F1E6..=0x1F1FF).contains(&(c as u32));
        let joins = prev.is_some() && (extends(c) || prev == Some('\u{200D}') || (ri && regional));
        if !joins && i > 0 {
            out.push(&text[start..i]);
            start = i;
        }
        regional = ri && !(joins && regional);
        prev = Some(c);
    }
    if start < text.len() {
        out.push(&text[start..]);
    }
    out
}

fn light(c: char) -> bool {
    matches!(c as u32, 0..=4351 | 8192..=8205 | 8208..=8223 | 8242..=8247)
}

/// The web addresses a text holds: (start, end) byte offsets.
fn links(text: &str) -> Vec<(usize, usize)> {
    let mut out = vec![];
    let mut at = 0;
    while let Some(i) = text[at..].find("http").map(|i| i + at) {
        let rest = &text[i..];
        if rest.starts_with("https://") || rest.starts_with("http://") {
            let end = rest.find(char::is_whitespace).map_or(text.len(), |e| i + e);
            out.push((i, end));
            at = end;
        } else {
            at = i + 4;
        }
    }
    out
}

/// The length of `text` as `t` counts it.
pub fn length(t: &Text, text: &str) -> usize {
    let mut n = 0;
    let mut at = 0;
    let unit = |s: &str| match t.count {
        Count::Characters => s.chars().count(),
        Count::Graphemes => clusters(s).len(),
        Count::Utf16 => s.encode_utf16().count(),
        Count::Bytes => s.len(),
        Count::Weighted => clusters(s).iter().map(|c| if c.chars().count() == 1 && c.chars().all(light) { 1 } else { 2 }).sum(),
    };
    if let Some(each) = t.link_counts {
        for (s, e) in links(text) {
            n += unit(&text[at..s]) + each;
            at = e;
        }
    }
    n + unit(&text[at..])
}

/// What is wrong with `copy` for a network with these rules, in plain words: empty when nothing is.
pub fn problems(r: &Rules, copy: &Copy) -> Vec<String> {
    let name = r.name();
    let kind = match r.kind(copy.kind.as_deref()) {
        Ok(k) => k,
        Err(why) => return vec![why],
    };
    let (text, media, follow, first_comment) = r.effective(kind);
    let mut out = vec![];
    for (i, item) in copy.items.iter().enumerate() {
        let after = i > 0;
        if after && follow.as_ == FollowAs::None {
            out.push(format!("{name} takes nothing after the post, and this one has {} more", copy.items.len() - 1));
            break;
        }
        if item.text.trim().is_empty() && item.media.is_empty() && item.link.is_none() {
            out.push(if after { format!("item {} is empty", i + 1) } else { "the post is empty".into() });
        }
        if after && !item.media.is_empty() && !follow.media {
            out.push(format!("{name} takes no media after the post"));
        }
        if !after && media.required && item.media.is_empty() {
            out.push(format!("{name} takes no post without an image or a video"));
        }
        for m in &item.media {
            out.extend(medium(name, &media, m));
        }
        if let Some(why) = mix(name, &media, &item.media, i) {
            out.push(why);
        }
        let t = if after { follow.text.as_ref().unwrap_or(&text) } else { &text };
        // A link the post carries goes after the words, after a space; a network that shortens links counts it as theirs.
        let n = length(t, &item.text) + item.link.as_deref().map_or(0, |l| t.link_counts.unwrap_or_else(|| length(t, l)) + 1);
        if let Some(max) = t.max.filter(|m| n > *m) {
            out.push(if after {
                format!("{name} takes {max} characters, and item {} is {n}", i + 1)
            } else {
                format!("{name} takes {max} characters, and this post is {n}")
            });
        }
        if let Some(min) = t.min.filter(|m| n < *m) {
            out.push(format!("{name} takes at least {min} characters, and {} is {n}", if after { format!("item {}", i + 1) } else { "this post".into() }));
        }
    }
    if let Some(max) = follow.max.filter(|m| copy.items.len().saturating_sub(1) > *m && follow.as_ != FollowAs::None) {
        out.push(format!("{name} takes at most {max} items after the post, and this one has {}", copy.items.len() - 1));
    }
    if copy.first_comment.as_deref().is_some_and(|c| !c.trim().is_empty()) && !first_comment {
        out.push(format!("{name} takes no first comment"));
    }
    out.extend(answers(r, copy.kind.as_deref().or(kind.map(|k| k.name.as_str())), &copy.settings).err().into_iter().flatten());
    out
}

fn medium(name: &str, media: &Media, m: &Piece) -> Vec<String> {
    let mut out = vec![];
    let what = plural(&m.kind);
    if !media.kinds.contains(&m.kind) {
        let others: Vec<String> = media.kinds.iter().map(|k| plural(k)).collect();
        out.push(if others.is_empty() { format!("{name} takes no {what}") } else { format!("{name} takes no {what}, only {}", others.join(" and ")) });
        return out;
    }
    if let Some(rule) = media.rule(&m.kind) {
        if let (Some(most), Some(b)) = (rule.max_bytes, m.bytes.filter(|b| rule.max_bytes.is_some_and(|x| *b > x))) {
            out.push(format!("{name} takes {what} of at most {}, and this one is {}", size(most), size(b)));
        }
        if let Some(t) = m.content_type.as_deref().filter(|t| !rule.types.is_empty() && !rule.types.iter().any(|x| x == t)) {
            out.push(format!("{name} takes {what} of type {}, and this one is {t}", rule.types.join(", ")));
        }
        let (w, h) = (m.width, m.height);
        let below = |v: Option<u32>, min: Option<u32>| v.zip(min).is_some_and(|(v, m)| v < m);
        let above = |v: Option<u32>, max: Option<u32>| v.zip(max).is_some_and(|(v, m)| v > m);
        if below(w, rule.min_width) || below(h, rule.min_height) || above(w, rule.max_width) || above(h, rule.max_height) {
            let bound = |a: Option<u32>, b: Option<u32>| format!("{}×{}", a.map_or("any".into(), |x| x.to_string()), b.map_or("any".into(), |x| x.to_string()));
            out.push(format!(
                "{name} takes {what} from {} to {} pixels, and this one is {}×{}",
                bound(rule.min_width, rule.min_height),
                bound(rule.max_width, rule.max_height),
                w.unwrap_or_default(),
                h.unwrap_or_default()
            ));
        }
        if let (Some(w), Some(h)) = (w, h.filter(|h| *h > 0)) {
            let ratio = f64::from(w) / f64::from(h);
            if rule.min_ratio.is_some_and(|m| ratio < m - 1e-9) || rule.max_ratio.is_some_and(|m| ratio > m + 1e-9) {
                out.push(format!(
                    "{name} takes {what} {} wide for each high at most, and this one is {ratio:.2}",
                    rule.max_ratio.or(rule.min_ratio).unwrap_or_default()
                ));
            }
        }
        if let Some(s) = m.seconds
            && (rule.min_seconds.is_some_and(|x| s < x) || rule.max_seconds.is_some_and(|x| s > x))
        {
            out.push(format!(
                "{name} takes {what} of {} to {} seconds, and this one is {s:.0}",
                rule.min_seconds.unwrap_or(0.0),
                rule.max_seconds.map_or("any".into(), |x| x.to_string())
            ));
        }
    }
    let alt = m.alt.as_deref().map(str::trim).unwrap_or_default();
    if media.alt && media.alt_required && alt.is_empty() {
        out.push(format!("{name} needs a description of each of its media"));
    }
    if let Some(max) = media.alt_max.filter(|x| alt.chars().count() > *x) {
        out.push(format!("{name} takes descriptions of at most {max} characters"));
    }
    out
}

fn mix(name: &str, media: &Media, pieces: &[Piece], item: usize) -> Option<String> {
    let mut count: BTreeMap<&str, usize> = BTreeMap::new();
    for p in pieces {
        *count.entry(p.kind.as_str()).or_default() += 1;
    }
    let total = pieces.len();
    let where_ = if item == 0 { "a post".to_string() } else { format!("item {}", item + 1) };
    if let Some(max) = media.max.filter(|m| total > *m) {
        return Some(format!("{name} takes at most {max} media in {where_}, and this one has {total}"));
    }
    if let Some(min) = media.min.filter(|m| total > 0 && total < *m) {
        return Some(format!("{name} takes at least {min} media in {where_}, and this one has {total}"));
    }
    if total > 0 && !media.mixes.is_empty() && !media.mixes.iter().any(|m| count.iter().all(|(k, n)| m.get(*k).is_some_and(|most| n <= most))) {
        let said: Vec<String> = count.iter().map(|(k, n)| format!("{n} {}", if *n == 1 { k.to_string() } else { plural(k) })).collect();
        return Some(format!("{name} does not take {} together in one post", said.join(" and ")));
    }
    None
}

/// Whether setting `s` is asked of a post of kind `kind` with these answers.
pub fn applies(s: &Setting, kind: Option<&str>, given: &Map<String, Value>) -> bool {
    s.when.iter().all(|(k, want)| {
        let have = if k == "kind" { kind.map(|x| Value::String(x.into())) } else { given.get(k).cloned() };
        let have = have.unwrap_or(Value::Null);
        match want {
            Value::Array(any) => any.contains(&have),
            one => *one == have,
        }
    })
}

/// The answers to pass on: what was given for each setting that applies, else its default. A given answer the rules do
/// not declare, one of the wrong type or outside its options, or a required one missing, is refused, each in words.
pub fn answers(r: &Rules, kind: Option<&str>, given: &Map<String, Value>) -> Result<Map<String, Value>, Vec<String>> {
    let mut out = Map::new();
    let mut wrong = vec![];
    for key in given.keys() {
        if !r.settings.iter().any(|s| s.key == *key) {
            wrong.push(format!("{} asks nothing called {key}", r.name()));
        }
    }
    for s in r.settings.iter().filter(|s| applies(s, kind, given)) {
        let v = given.get(&s.key).filter(|v| !v.is_null() && *v != "" && !v.as_array().is_some_and(Vec::is_empty)).or(s.default.as_ref());
        let Some(v) = v else {
            if s.required {
                wrong.push(format!("{} is needed", s.label));
            }
            continue;
        };
        match setting_value(s, v) {
            Ok(v) => {
                out.insert(s.key.clone(), v);
            }
            Err(why) => wrong.push(format!("{} {why}", s.label)),
        }
    }
    if wrong.is_empty() { Ok(out) } else { Err(wrong) }
}

fn setting_value(s: &Setting, v: &Value) -> Result<Value, String> {
    let within = |n: f64, what: &str| {
        if s.min.is_some_and(|m| n < m) || s.max.is_some_and(|m| n > m) {
            Err(format!("is {what} from {} to {}", s.min.map_or("any".into(), |m| m.to_string()), s.max.map_or("any".into(), |m| m.to_string())))
        } else {
            Ok(())
        }
    };
    let options: Vec<Value> = s.options.iter().map(Opt::value).collect();
    let listed = |x: &Value| options.is_empty() || options.contains(x);
    let names = || s.options.iter().map(|o| o.value().as_str().map_or_else(|| o.value().to_string(), str::to_string)).collect::<Vec<_>>().join(", ");
    match s.kind.as_str() {
        "text" | "long" => {
            let t = v.as_str().ok_or("is words")?;
            within(t.chars().count() as f64, "a text of characters numbering")?;
            Ok(v.clone())
        }
        "number" => {
            let n = v.as_f64().or_else(|| v.as_str().and_then(|t| t.trim().parse().ok())).ok_or("is a number")?;
            within(n, "a number")?;
            Ok(serde_json::json!(n))
        }
        "bool" => v.as_bool().map(Value::Bool).ok_or_else(|| "is yes or no".into()),
        "date" => {
            let t = v.as_str().ok_or("is a date")?;
            chrono::NaiveDate::parse_from_str(t.get(..10).unwrap_or(t), "%Y-%m-%d").map_err(|_| "is a date (YYYY-MM-DD)".to_string())?;
            Ok(v.clone())
        }
        "select" => {
            if !listed(v) {
                return Err(format!("is one of {}", names()));
            }
            Ok(v.clone())
        }
        _ => {
            let list = v.as_array().ok_or("is a list")?;
            within(list.len() as f64, "a list of items numbering")?;
            if let Some(x) = list.iter().find(|x| !listed(x)) {
                return Err(format!("takes only {}, not {x}", names()));
            }
            Ok(v.clone())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn example(name: &str) -> Rules {
        let text = std::fs::read_to_string(format!("{}/docs/connectors/examples/{name}.json", env!("CARGO_MANIFEST_DIR"))).unwrap();
        let v: Value = serde_json::from_str(&text).unwrap();
        Rules::parse(&v["meta"]["social"], name).unwrap_or_else(|e| panic!("{name}: {e}"))
    }

    fn post(text: &str, media: Vec<Piece>) -> Copy {
        Copy { items: vec![Item { text: text.into(), link: None, media }], ..Default::default() }
    }

    fn pic(kind: &str, ct: &str, bytes: i64) -> Piece {
        Piece { kind: kind.into(), content_type: Some(ct.into()), bytes: Some(bytes), alt: Some("A cat".into()), ..Default::default() }
    }

    fn with(mut c: Copy, kind: &str, settings: Value) -> Copy {
        c.kind = Some(kind.into());
        c.settings = settings.as_object().cloned().unwrap();
        c
    }

    fn says(p: &[String], what: &str) -> bool {
        p.iter().any(|w| w.contains(what))
    }

    #[test]
    fn lengths_as_each_network_counts_them() {
        let t = |count, link| Text { count, link_counts: link, ..Default::default() };
        assert_eq!(length(&t(Count::Characters, None), "héllo"), 5);
        assert_eq!(length(&t(Count::Bytes, None), "héllo"), 6);
        assert_eq!(length(&t(Count::Utf16, None), "a😀"), 3);
        assert_eq!(length(&t(Count::Characters, None), "a😀"), 2);
        // A family (joined), a flag (two regional letters), a thumb with its tone, an e with its accent: one each.
        assert_eq!(length(&t(Count::Graphemes, None), "👨\u{200D}👩\u{200D}👧🇮🇳👍🏽e\u{301}"), 4);
        assert_eq!(clusters("🇮🇳🇫🇷").len(), 2, "two flags, not one");
        // X: Latin 1, CJK and emoji 2, a link 23 whatever its length.
        assert_eq!(length(&t(Count::Weighted, Some(23)), "hi 日本 😀"), 3 + 4 + 1 + 2);
        assert_eq!(length(&t(Count::Weighted, Some(23)), "see https://example.org/a/very/long/path ok"), 4 + 23 + 3);
        assert_eq!(length(&t(Count::Characters, None), "see https://e.org"), 17, "as written without link_counts");
    }

    #[test]
    fn a_profile_record_is_the_same_shape() {
        let fields = |v: Value| Rules::of_fields(v.as_object().unwrap(), "x");
        let short = fields(json!({"profile_name": "Short", "max_text": 280, "link_length": 23}));
        let link = |text: &str, l: &str| Copy { items: vec![Item { text: text.into(), link: Some(l.into()), media: vec![] }], ..Default::default() };
        assert!(says(&problems(&short, &post(&"a".repeat(281), vec![])), "Short takes 280 characters, and this post is 281"));
        assert!(problems(&short, &link(&"a".repeat(256), "https://example.org/a-very-long-path-that-is-shortened")).is_empty(), "a link is 23 and a space");
        assert!(!problems(&short, &link(&"a".repeat(257), "https://e.org")).is_empty());
        let plain = fields(json!({"profile_name": "Plain", "max_text": 500}));
        assert!(!problems(&plain, &link(&"a".repeat(490), "https://example.org/0123456789")).is_empty(), "a link counts as written");
        let pictured = fields(json!({"profile_name": "Pictures", "max_text": 2200, "media_required": true}));
        assert!(says(&problems(&pictured, &post("Hello", vec![])), "image or a video"));
        assert!(problems(&pictured, &post("Hello", vec![pic("image", "image/png", 10)])).is_empty());
        let video = fields(json!({"profile_name": "Video", "max_text": 2200, "images": false, "videos": true, "media_required": true}));
        assert!(says(&problems(&video, &post("Hello", vec![pic("image", "image/png", 10)])), "no pictures, only video"));
        assert!(problems(&video, &post("Hello", vec![pic("video", "video/mp4", 10)])).is_empty(), "posts carry video now");
        let small = fields(json!({"profile_name": "Small", "max_text": 300, "max_image_bytes": 1_000_000}));
        assert!(says(&problems(&small, &post("Hi", vec![pic("image", "image/png", 2_000_000)])), "at most 977 KB, and this one is 1.9 MB"));
        let none = fields(json!({"profile_name": "None", "max_text": 300}));
        assert!(says(&problems(&none, &Copy { first_comment: Some("first!".into()), ..post("Hi", vec![]) }), "takes no first comment"));
        let words = fields(json!({"profile_name": "Words", "max_text": 300, "images": false}));
        assert!(says(&problems(&words, &post("Hi", vec![pic("image", "image/png", 1)])), "Words takes no pictures"));
        let four = fields(json!({"profile_name": "Four", "max_text": 300, "max_media": 4}));
        assert!(says(&problems(&four, &post("Hi", vec![pic("image", "image/png", 1); 5])), "at most 4 media in a post, and this one has 5"));
        let threads = fields(json!({"profile_name": "T", "max_text": 10, "threads": true}));
        let thread =
            Copy { items: vec![Item { text: "one".into(), ..Default::default() }, Item { text: "a".repeat(11), ..Default::default() }], ..Default::default() };
        assert!(says(&problems(&threads, &thread), "T takes 10 characters, and item 2 is 11"));
        assert!(says(&problems(&none, &thread), "takes nothing after the post"));
    }

    #[test]
    fn nine_networks_without_a_line_of_rust() {
        // YouTube: a video with a title, its visibility from a list, tags; a short is a kind of its own.
        let yt = example("youtube");
        let video = vec![pic("video", "video/mp4", 50 << 20)];
        let p = problems(&yt, &post("A description", video.clone()));
        assert!(says(&p, "Title is needed") && says(&p, "Visibility is needed"), "{p:?}");
        assert!(problems(&yt, &with(post("Desc", video.clone()), "video", json!({"title": "Spring", "type": "public"}))).is_empty());
        assert!(says(
            &problems(&yt, &with(post("Desc", video.clone()), "video", json!({"title": "Spring", "type": "everyone"}))),
            "Visibility is one of public, private, unlisted"
        ));
        assert!(says(
            &problems(&yt, &with(post("Desc", vec![pic("image", "image/png", 9)]), "video", json!({"title": "S", "type": "public"}))),
            "takes no pictures, only video"
        ));
        let long = Piece { seconds: Some(200.0), ..video[0].clone() };
        assert!(says(&problems(&yt, &with(post("Desc", vec![long]), "short", json!({"title": "S", "type": "public"}))), "of 0 to 180 seconds"));
        // Reddit: a subreddit fetched from the network, a title, a flair whose choices depend on the subreddit; a link post.
        let reddit = example("reddit");
        let p = problems(&reddit, &with(post("Body", vec![]), "self", json!({"title": "Hi"})));
        assert!(says(&p, "Subreddit is needed"), "{p:?}");
        assert!(problems(&reddit, &with(post("Body", vec![]), "self", json!({"subreddit": "rust", "title": "Hi"}))).is_empty());
        let flair = reddit.settings.iter().find(|s| s.key == "flair").unwrap();
        assert_eq!(flair.choices.as_ref().unwrap().inputs["subreddit"], "settings.subreddit");
        assert!(says(&problems(&reddit, &with(post("x", vec![]), "self", json!({"subreddit": "r", "title": "t", "nsfw": "yes"}))), "NSFW is yes or no"));
        // Pinterest: a board, one to five pictures or one video, no follow-ups.
        let pin = example("pinterest");
        let p = problems(&pin, &with(post("Pin", vec![pic("image", "image/jpeg", 1); 6]), "pin", json!({"board": "b1"})));
        assert!(says(&p, "at most 5 media"), "{p:?}");
        let mixed = vec![pic("image", "image/jpeg", 1), pic("video", "video/mp4", 1)];
        assert!(says(&problems(&pin, &with(post("Pin", mixed), "pin", json!({"board": "b1"}))), "does not take 1 image and 1 video together"));
        assert!(says(&problems(&pin, &with(post("Pin", vec![]), "pin", json!({"board": "b1"}))), "without an image or a video"));
        // TikTok: privacy from the network's list, flags that must be answered, video only.
        let tt = example("tiktok");
        let p = problems(&tt, &with(post("Dance", vec![pic("video", "video/mp4", 1)]), "video", json!({"privacy_level": "PUBLIC_TO_EVERYONE"})));
        assert!(says(&p, "Allow comments is needed"), "{p:?}");
        assert!(
            problems(
                &tt,
                &with(
                    post("Dance", vec![pic("video", "video/mp4", 1)]),
                    "video",
                    json!({"privacy_level": "SELF_ONLY", "comment": true, "duet": false, "stitch": false})
                )
            )
            .is_empty()
        );
        // Instagram: reels, stories, carousels of 2 to 10; collaborators only for posts and reels; comments as follow-ups,
        // text only.
        let ig = example("instagram");
        assert!(problems(&ig, &with(post("Hi", vec![pic("image", "image/jpeg", 1); 3]), "post", json!({"collaborators": ["ana"]}))).is_empty());
        let p = problems(&ig, &with(post("Hi", vec![pic("image", "image/jpeg", 1)]), "story", json!({"collaborators": ["ana"]})));
        assert!(p.is_empty(), "collaborators are not asked of a story, so not passed: {p:?}");
        let story_answers = answers(&ig, Some("story"), &json!({"collaborators": ["ana"]}).as_object().cloned().unwrap()).unwrap();
        assert!(story_answers.is_empty());
        assert!(says(&problems(&ig, &with(post("Hi", vec![pic("image", "image/jpeg", 1); 2]), "reel", json!({}))), "takes no pictures, only video"));
        let mut commented = with(post("Hi", vec![pic("image", "image/jpeg", 1)]), "post", json!({}));
        commented.items.push(Item { text: "more".into(), media: vec![pic("image", "image/jpeg", 1)], ..Default::default() });
        assert!(says(&problems(&ig, &commented), "takes no media after the post"));
        // LinkedIn page: a document carousel is a kind; comments; a picker of organisations; company mentions.
        let li = example("linkedin_page");
        assert!(li.accounts.as_ref().is_some_and(|a| a.operation == "accounts"));
        assert!(li.mentions.as_ref().and_then(|m| m.format.as_deref()).is_some_and(|f| f.contains("urn:li:organization")));
        assert!(problems(&li, &with(post("Deck", vec![pic("document", "application/pdf", 1)]), "document", json!({"document_title": "Q3"}))).is_empty());
        assert!(says(&problems(&li, &with(post("Deck", vec![pic("document", "application/pdf", 1)]), "document", json!({}))), "Document title is needed"));
        // Discord: a channel fetched from the server, markdown, follow-ups as a thread.
        let dc = example("discord");
        assert!(says(&problems(&dc, &post("hi", vec![])), "Channel is needed"));
        let mut thread = with(post("hi", vec![]), "message", json!({"channel": "c1"}));
        thread.items.push(Item { text: "more".into(), ..Default::default() });
        assert!(problems(&dc, &thread).is_empty());
        assert_eq!(dc.text.format.as_deref(), Some("markdown"));
        // Google Business: what kind of update, its event's title and dates only for an event, a call to action.
        let gmb = example("google_business");
        assert!(problems(&gmb, &with(post("Open late", vec![]), "standard", json!({"cta": "NONE"}))).is_empty());
        let p = problems(&gmb, &with(post("Party", vec![]), "event", json!({})));
        assert!(says(&p, "Event title is needed") && says(&p, "Starts on is needed"), "{p:?}");
        assert!(says(
            &problems(&gmb, &with(post("Party", vec![]), "event", json!({"event_title": "P", "event_start": "soon", "event_end": "2026-12-02"}))),
            "Starts on is a date"
        ));
        assert!(says(&problems(&gmb, &with(post("x", vec![pic("image", "image/jpeg", 1); 2]), "standard", json!({}))), "at most 1 media"));
        // WordPress: an article with a title, html, a status, categories fetched from the site.
        let wp = example("wordpress");
        assert_eq!(wp.text.format.as_deref(), Some("html"));
        assert!(problems(&wp, &with(post("<p>Body</p>", vec![]), "article", json!({"title": "T", "status": "draft"}))).is_empty());
        assert!(says(
            &problems(&wp, &with(post("x", vec![]), "article", json!({"title": "T", "status": "draft", "sticky": true}))),
            "asks nothing called sticky"
        ));
    }
}
