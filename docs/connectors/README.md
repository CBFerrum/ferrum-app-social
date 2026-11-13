# Connection descriptions for the networks

Social names no network. A network is a connection's description (Administration, Connections) whose operations are
named by what Social needs of them, with `meta.capabilities` saying what the network takes in a post. Four are here:

| File | Sign-in | Shows |
|---|---|---|
| `mastodon.json` | OAuth, the workspace's or each person's | text posts, a picture as `multipart/form-data`, delete, replies kept as open as what they answer, direct messages, mentions pulled by the `Link` header's `prev`, numbers, followers |
| `bluesky.json` | a session from an app password, refreshed by its own operation | text posts, a picture as the raw request body, link cards, delete, replies in their threads, notifications by reason, numbers, followers |
| `linkedin.json` | OAuth | a new post's id read from the `x-restli-id` response header; delete |
| `x.json` | OAuth | text posts, a picture as multipart, delete, replies, direct messages, mentions pulled forward by `newest_id`, numbers |

`mastodon.json` and `bluesky.json` are also the package's own data (`connections:` in package.yaml): a workspace sees
them offered in Administration, Connections, and an account carried over from 0.6.x (the networks the earlier releases
had connectors for) is given its connection from them (see CHANGELOG, 0.7.0). A test keeps the two the same.

To use one: copy it into a new connection, put the provider's client id where it says `REPLACE`, give the connection its
secrets (`client_secret` for OAuth; `handle` and `app_password` for Bluesky), list the roles that may use it when it is
each person's own (`"owner": "person"`), register the redirect address `{PUBLIC_URL}/api/connections/callback` at the
provider, turn it on, and connect the account from its page in Social. Mastodon's address is a placeholder for the
account's own server (it appears in four places); LinkedIn's `author` and X's handle and user id are placeholders too.

## What Social calls

Every operation is looked up by Social's name, or by the name `meta.social.operations` maps it to. Every call that
writes carries a key (below); each description here names an `idempotency_header`, so the network is given the key
too. Mastodon documents `Idempotency-Key` on statuses; Bluesky, LinkedIn and X do not say they honour it, so for them a
retry under the same key is answered from the platform's stored answer when the first answer came back, and a lost
answer can still post twice (see "What they still cannot do").

* `publish`, once for each item of a copy that goes as a post: the first item, and in a thread (`follow_ups.as:
  thread`) each one after it. Inputs:
  * `text` (the item's final words: variables filled, the signature added), `link`, `text_with_link` (`text`, a space
    and `link`);
  * `media` (the network's ids of this item's media, in order), `media_id` (the first), `media_items` (`[{id, kind, alt,
    content_type, size, width, height, seconds}]`), `alt`, `picture_type`, `picture_size` (the first medium's);
  * `kind` (the kind of post, a name of `meta.social.kinds`), `settings` (the answers to what the network asks: only
    those that apply to the kind, defaults filled), `account_id` (the account's id on the network: the page,
    organisation, location or channel picked after sign-in, else the signed-in account), `handle`;
  * `item` (its number from 0), `items` (how many), `reply_to` (the network's id of the item before; null for the
    first), `root` and `in_reply_to` (the first item's id), `previous` (what the item before gave back: `{item, id,
    url, media, sent_at, answer}`, `answer` holding whatever else its `result` maps, so a description can map Bluesky's
    `cid` there and read `inputs.previous.answer.cid`);
  * `created_at`: the publication's own moment, the same on every attempt.
  Result: `id` (required) and `url`; anything else it maps is kept with the item.
* `reply`, for each item after the first when follow-ups are comments (`follow_ups.as: comment`), with the same
  inputs, `in_reply_to` the first item's id; and for the post's First Comment after every item (`text`, `in_reply_to`,
  `root`), where the rules take one. Also the inbox's answers (`text`, `in_reply_to`, `author`, `thread`,
  `created_at`). Result `id`.
* `publish_media`, before the item that carries the medium, for each medium (inputs `file`, the stored file's id, for a
  part or a body; `name`, `content_type`, `size`, `kind`, `alt` when `media.alt`, `thumbnail` (a stored file's id) when
  `media.thumbnail`, `width`, `height`, `seconds`, `item`, `index` (its place in the item), `account_id`, `handle`,
  `kind`, `settings`; and `data`, base64, only when the description lists it among its `inputs`). Result `id`.
* Keys: `publish:<publication>` for the first item and `publish:<publication>:<n>` for item n after it (the first keeps
  0.7.0's key, so a publication under way across the upgrade is not sent twice); `media:<publication>` for the first
  item's first medium and `media:<publication>:<item>:<index>` for the others; `first_comment:<publication>`;
  `delete:<publication>` and `delete:<publication>:<n>`.
* `delete`, for each item out, the last first (`id`, `item`, `follow_up`: `thread` or `comment` for an item after the
  first, `root`). `send_message` (`to`, `author`, `text`, `in_reply_to`, `thread`; result `id`).
* `metrics` (`id`, `account_id`, `handle`) and `account` (`account_id`, `handle`): results named `impressions`,
  `reach`, `engagements`, `clicks`, `followers`. A network that counts likes, boosts and replies apart adds them in the
  description: `{"expr": "answer.favourites_count + answer.reblogs_count"}` under `engagements`. `account` may map
  `name`, `handle` and `picture` too: Check reads them onto the account.
* `accounts` (or what `meta.social.accounts.operation` names): what the sign-in reaches, read where
  `meta.social.accounts` says (without it: items `{id, name, handle, picture}` of the whole answer). `mentions` and a
  setting's `choices` operations: as `meta.social.mentions` and the setting's `choices` say. A lookup that is not a
  GET is called under the key `look:<account>:<operation>:<minute>:<inputs>`.
* `messages` is pulled by the description's `pulls`, or its `inbound` takes them, mapped onto `SocialMessage` found by
  `external_id`, with `"outside": true` (what is read is not sent back out). A field the network may leave out is a
  pointer (`/status/in_reply_to_id`, null when it is not there). `via_connection: "connection.id"` lets Social find the
  account by the connection when the item names neither a handle nor a post (a person's own connection is read as her, so
  the account she connected). `thread` is what the answer needs from the item: Mastodon's keeps the status's openness (so
  an answer is never more open), Bluesky's the thread's root and the comment's own reference.
* `meta.capabilities` (0.7.0): `max_text`, `link_length`, `images`, `videos`, `media_required`, `max_media`,
  `max_image_bytes`, `alt_text`, `first_comment` (as a network profile's fields), `name`, and `html_text` for a server
  that sends its words as HTML. Still read when a description has no `meta.social`; `meta.social` replaces it.

## `meta.social`: what a network takes and asks

One declarative shape on a connection's description (`meta.social`), read by Social and by its screens. Nothing about a
network is written in Rust: adding a network is adding a description. Where a description has none, the account's
network profile (Social, Network Profiles) is the fallback: its `Rules` field takes the same shape, and without it the
profile's own fields are read as 0.7.0 read them. Order: `meta.social`, then `meta.capabilities`, then the profile's
`Rules`, then the profile's fields. Unknown keys are refused, so a typo is said when a post is checked.

```jsonc
{
  "name": "Pinterest",                       // what people call it; else the account's network, else the connection's name
  "text": {
    "max": 500,                              // longest text; absent: any
    "min": 0,
    "count": "characters",                   // characters | graphemes | utf16 | bytes | weighted
    "link_counts": 23,                       // a web address in the text (and the post's link) counts as this, whatever its length
    "format": "plain",                       // plain | markdown | html: what the composer edits and previews
    "fold": 125                              // (round two) characters after which the network shows "see more": the preview's, never checked
  },
  "media": {
    "kinds": ["image", "video"],             // kinds taken; absent or []: none. A medium's kind is its own (`kind` on the media
                                             // line) or its file's type: image/*, video/*, application/pdf = document
    "required": true,                        // no post without one
    "min": 1, "max": 5,                      // per item
    "mixes": [{"image": 5}, {"video": 1}],   // combinations taken in one item, each kind with how many at most; absent: any mix
    "image": {"max_bytes": 20971520, "types": ["image/jpeg"], "min_width": 0, "max_width": 6000,
              "min_height": 0, "max_height": 6000, "min_ratio": 0.8, "max_ratio": 1.91},
    "video": {"max_bytes": 314572800, "types": ["video/mp4"], "min_seconds": 3, "max_seconds": 600},
    "document": {"types": ["application/pdf"]},
    "other": {"gif": {"max_bytes": 15728640}},
    "alt": true, "alt_required": false, "alt_max": 1000,   // descriptions sent; needed; longest
    "thumbnail": false                       // a video's picture is sent (`thumbnail` input)
  },
  "kinds": [                                 // free names; the first is the default. Each may carry its own text, media,
    {"name": "pin", "label": "Pin"},         // follow_ups and first_comment where they differ from the network's
    {"name": "story", "label": "Story", "media": {"kinds": ["image", "video"], "required": true, "max": 1},
     "follow_ups": {"as": "none"}}
  ],
  "follow_ups": {                            // what an item after the first is here
    "as": "thread",                          // thread: `publish` again, answering the item before | comment: `reply` under the
                                             // first item | none: refused
    "media": true,                           // may a follow-up carry media
    "max": 24,                               // items after the first, at most
    "text": {"max": 280}                     // its own length rules, where they differ
  },
  "settings": [                              // what the composer asks, kept per account on the post (`targets[].settings`)
    {"key": "board", "label": "Board", "type": "select", "required": true,
     "choices": {"operation": "boards", "inputs": {}, "items_at": "/items", "value_at": "/id", "label_at": "/name"}},
    {"key": "title", "label": "Title", "type": "text", "max": 100, "help": "Shown above the pin"},
    {"key": "privacy", "label": "Who can see it", "type": "select", "options": ["PUBLIC", {"value": "SELF", "label": "Only me"}]},
    {"key": "event_start", "label": "Starts on", "type": "date", "required": true, "when": {"kind": ["event"], "cta": ["BOOK"]}},
    {"key": "made_for_kids", "label": "Made for children", "type": "bool", "default": false}
  ],
  "operations": {"publish": "create_pin"},   // Social's name of an operation -> the description's, where they differ
  "accounts": {"operation": "accounts", "items_at": "/items", "value_at": "/id", "label_at": "/name",
               "handle_at": "/username", "picture_at": "/picture", "kind_at": "/type"},
  "mentions": {"operation": "mentions", "inputs": {"q": "query"}, "items_at": "", "value_at": "/id", "label_at": "/name",
               "picture_at": "/avatar", "format": "<@{{ value }}>"},
  "first_comment": true,                     // the post's First Comment is taken (sent by `reply` after the items)
  "link_preview": true,                      // the network shows a card for a link (the composer's preview)
  "scheduling": false,                       // recorded only: Social always sends at the time itself
  "inbox_format": "html"                     // messages read through the connection arrive as HTML
}
```

* **Settings** types: `text`, `long` (several lines), `number`, `bool`, `select`, `multi` (a list), `date` (YYYY-MM-DD).
  `options` are strings or `{value, label}`; `choices` reads them from the network instead (or as well): an operation,
  its `inputs` (a literal, or `settings.<key>` for another answer, `account.id` for the account's id on the network,
  `account.handle`, `query` for what the person typed), the list in its answer (`items_at`, a JSON pointer; `""` the
  answer itself) and, in each item, the value and its label (pointers; `""` the item itself). `when` asks a setting only
  when the post's kind is one of `kind` and each other key's answer is (one of) what it names. `min`/`max` bound a
  number, a text's length or a list's length. A default fills an answer not given.
* **Checked** at every save of a post and again when it is sent, for each account: the kind is one the network has;
  each item's length counted as the network counts it (a link the post carries goes after the words, after a space);
  empty items; media kinds, counts, mixes, sizes, types, dimensions and duration where the medium's line says them
  (unknown ones are not checked); descriptions; follow-ups (how many, whether they may carry media); the first comment;
  settings. An answer to a key the shape does not declare is refused (`Pinterest asks nothing called sticky`); a
  required one missing names the account and the field (`for Northwind Pins: Board is needed`); an answer of a setting
  that does not apply to the kind is not passed on.
* **Examples**: `examples/` holds the shape for YouTube, Reddit, Pinterest, TikTok, Instagram, LinkedIn page, Discord,
  Google Business Profile and WordPress (operations not written yet; `src/rules.rs` tests check posts against each).

## Round two: what Social calls outside any account

Connections named in Social Settings, called as the workspace (examples in `services/`, each loaded by a test):

* **Picture and GIF sources** (`media_sources`; `unsplash.json`, `pexels.json`, `giphy.json`): `search_media` (inputs
  `query`, `page`, `per_page`), its items read where `meta.social.media_search` says (`items_at`, `id_at`, `preview_at`,
  `url_at`, `credit_at`, `credit_link_at`, `alt_at`, `width_at`, `height_at`, and literals `licence`, `kind`);
  `fetch_media` (`id`, `url`), whose `result` maps `file` to a stored file (`{"file": {"from": "/url", "max_bytes": …}}`:
  the platform fetches the address within the description's `allowed_hosts`, without its credentials, at most
  `max_bytes`, 10 MB by default and 25 MB at most); `media_downloaded` (`id`, `url`) where the source asks to be told of
  a download (Unsplash's `/photos/:id/download` is both, so its `fetch_media` is it).
* **Media from an address** (`fetch_connection`): `fetch_media` with `url`, the same `file` result.
* **Shortener** (`shorten_connection`; `dub.json`, `kutt.json`): `shorten` (`url` → `url`, `id`), key
  `shorten:<publication>:<digest>`; `link_clicks` (`id`, `url`, `long` → `clicks`), read into the publication's clicks.
* **Writing help** (`writing_connection`; `claude.json`, the Claude Messages API): `write` (`model`, `max_tokens`,
  `system`, `prompt`, and `image_data` (base64), `image_type` to describe a picture → `text`); `picture` (`prompt` →
  `file`), which the Claude example does not offer. Each call has a key of its own (`write:<uuid>`).
* **Feeds** (a feed's own `connection`; `json_feed.json`): its operation (`feed` unless the feed names another), the
  items where `meta.feed` says (`items_at`, `id_at`, `link_at`, `title_at`, `text_at`, `picture_at`, `published_at`);
  its `fetch_media` (`url`) keeps an item's picture. An RSS or Atom feed waits on the platform reading XML answers (GAPS.md
  P7): then the same description, its `feed` operation marked to answer XML as P7 names it and its pointers into the
  XML's shape, and nothing in Social changes.

On an account's connection (plugs): `repost` (`id`, `account_id`), `like` (`id`, `account_id`), `reply` (`text`,
`in_reply_to`, `root`), each keyed `plug:<plug>:<publication>:<account>`. A connection that lacks the operation is
skipped and the run says so.

## Pages read (12 and 13 November 2026)

* Mastodon, docs.joinmastodon.org: methods/statuses, methods/media, methods/notifications, methods/accounts,
  api/rate-limits, api/oauth-scopes. (Its rate limit page names no `Retry-After`; the platform honours one when sent.)
* Bluesky: the lexicons in github.com/bluesky-social/atproto (com.atproto.server.createSession, refreshSession,
  com.atproto.repo.createRecord, uploadBlob, deleteRecord) and github.com/bluesky-social/social-app/lexicons
  (app.bsky.feed.post, getPosts and defs, app.bsky.embed.images and external, app.bsky.actor.getProfile and defs,
  app.bsky.notification.listNotifications). docs.bsky.app redirects to an endpoint list and was not used.
* LinkedIn, learn.microsoft.com/linkedin: marketing/community-management/shares/posts-api, images-api, and
  shared/authentication/authorization-code-flow (versioned APIs, `Linkedin-Version: 202610`).
* X, docs.x.com: x-api/posts/creation-of-a-post, delete-post, post-lookup-by-post-id, users/get-mentions,
  users/get-user-by-username, media/upload-media, direct-messages, oauth-2-0, fundamentals/counting-characters.

These are tested against a fake service shaped by those pages (`tests/connectors_it.rs`), not against the networks:
a first connection to each is the first time its real answers are read.

## What they still cannot do

* **Mastodon**: a mention is a Mention (the old connector told a comment from a mention by comparing the status's
  parent with the account's own id, which a mapping cannot); the account's own native posts are not read back (the path
  needs the account's id from an earlier call); the first pull reads the newest page, not the history; each app is
  registered by hand at the server (its registration is not RFC 7591's).
* **Bluesky**: a link is a card, or text when there is a picture (a clickable link is a facet with byte offsets, which a
  template cannot count); a lost answer can post twice (the old connector made a record key from the delivery's key, and a
  description cannot make a TID); no first comment (a reply needs the post's `cid`, and Social keeps only its address);
  chat messages are not read or sent; only the newest 50 notifications are read each time (its cursor looks back, not
  forward); no permalink; characters are counted as characters, not graphemes.
* **LinkedIn**: no picture (initialising an upload gives an address to `PUT` the bytes to, a second call that depends on
  the first's answer); no `\`-escaping of the characters its text format reserves; no numbers or comments (those need
  permissions LinkedIn gives to approved partners).
* **X, LinkedIn, Bluesky**: the key goes in an `Idempotency-Key` header, which their pages do not say they honour.
* **All**: a pick of a pull cannot read the clock, so the numbers a day are Social's; Retry-After is honoured for pulls
  and for the platform's own queued calls but an app's own call (Social's publish job) sees only "later" and is asked
  again by the jobs' backoff.
