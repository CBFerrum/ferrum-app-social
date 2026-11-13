# Connection descriptions for the networks

Social names no network. A network is a connection's description (Administration, Connections) whose operations are
named by what Social needs of them, with `meta.social` saying what the network takes and asks. **Thirty-three networks
are described here**, each in its own file: X, LinkedIn (member and page), Facebook Page, Instagram (through a page, and
its own sign-in), Threads, YouTube, TikTok, TikTok Business, Pinterest, Reddit, Mastodon (any server), Bluesky, Google
Business Profile, Telegram, Discord, Slack, WordPress, Medium, DEV, Hashnode, Tumblr, Lemmy, VK, Twitch, Kick, Dribbble,
Whop, MeWe, Listmonk, Moltbook, Farcaster; and the services Social calls outside an account under `services/`
(Unsplash, Pexels, GIPHY, Dub, Kutt, the Claude Messages API, feeds). Nostr and Skool have none (below).

**Tested against fakes of each network's documented API, never run against the real network**
(`tests/networks_a_it.rs`, `tests/networks_b_it.rs`, Social's own behaviour in `tests/social_it.rs`): the first
connection to each is the first time its real answers are read. What is marked unverified below is what neither the
network's pages nor Postiz settled.

**Shipped with the package.** Every file here is the package's data (`connections:` in package.yaml, written by
`scripts/ship_connections.py`; `tests/connectors_it.rs` fails when they differ): a workspace sees each offered in
Administration, Connections, turned off and without secrets. None is switched on by itself. To add a network:

1. Administration, Connections: take the offered network (it becomes the workspace's connection, off).
2. Do what its table below says under "Set up at the network" (a developer app, its redirect address
   `{PUBLIC_URL}/api/connections/callback`, the permissions it needs).
3. Put the app's client id where the description says, and the secrets it names under the connection's secrets
   (`client_secret`, an API key, a bot token, an app password); a server's own address where the description has a
   placeholder (Mastodon, Lemmy, WordPress, Listmonk). Leave `social` in its `apps`. (round five) Meta's networks: the webhook's verify token under `verify_token`, and
   the inbound address (Administration, Connections) given to Meta for `comments` (Instagram) or `messages` (a page).
4. Turn it on.
5. In Social, Accounts: add an account on that connection and Connect it (OAuth signs in in the browser); where the
   network has an `accounts` operation, pick the pages, channels, boards' owners or locations it reaches (Choose
   Accounts).

**Conventions every description follows.** Never twice: a post is an operation of steps, sent again after a lost answer
only where the network honours a key (Mastodon's `Idempotency-Key`, Discord's nonce, Neynar's `idem`, Bluesky's record
key from the call's key) or after a read-back proves it did not happen (`confirm`); elsewhere a lost answer fails the call
saying it may have been sent. Steps that leave nothing public carry the key in `X-Ferrum-Upload-Key`, which networks
ignore. A reading operation made of steps reads by its steps (round five, BD-69). Answers to what a network asks come in
`inputs.settings`; the title in `inputs.title` (round four).

## What Social calls

Every operation is looked up by Social's name, or by the name `meta.social.operations` maps it to. Every call that
writes carries a key (below); each description here names an `idempotency_header`, so the network is given the key
too. Mastodon documents `Idempotency-Key` on statuses; Bluesky, LinkedIn and X do not say they honour it, so for them a
retry under the same key is answered from the platform's stored answer when the first answer came back, and a lost
answer can still post twice (see each network below).

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
* (round three) **A `publish` made of steps** (BD-53 to BD-55) may upload its own media: then the description needs no
  `publish_media`, Social asks for none, and the item's media come only in `media_items`: `[{id (null: nothing uploaded
  apart), file (the stored file's id, for a step's `range`, `chunks` or a file body), name, kind, content_type, size,
  width, height, seconds, alt (when `media.alt`), thumbnail (a stored file's id, when `media.thumbnail`)}]`. (round
  five) A network that pulls media by address makes the address itself from `file` (below, round five). Each item also gets `root_item` (the first item as kept: `{item, id, url, media, sent_at, answer}`, `answer`
  holding every other field its `result` maps, a `cid` say) beside `previous` (the item before, the same shape):
  `inputs.previous.answer.cid`, `inputs.root_item.url`.
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
* (round three) **The account's own posts read back**: a pull (its `inputs` from the signed-in `account`, its `each`
  asking each post's numbers, BD-60) mapped onto `SocialPublication` with `"as": "system"`, `by: {external_id}`, `set`
  of `text`, `permalink`, `sent_at`, `impressions`, `engagements`, `clicks` (from `item.<each's as>`), and `create` of
  `native: true` and `via_connection: connection.id` (create-only, so a post sent from here and read back keeps
  `native` false and gets its numbers). Its account is the one the connection reaches, else the one `account_handle`
  names. It has no post; the upkeep reads its numbers as any publication's after that. **Comment or mention**: a
  mapping sets `kind` as it can and `post_external_id` (what the item answers); a Mention answering a publication of the
  brand's is kept as a Comment.
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
  its `fetch_media` (`url`) keeps an item's picture. RSS and Atom are `feeds.json` (XML answers, round three). (round
  five) Both read any public host (`any_host`), so a feed is only an address: no administrator step per site.

On an account's connection (plugs): `repost` (`id`, `account_id`), `like` (`id`, `account_id`), `reply` (`text`,
`in_reply_to`, `root`), each keyed `plug:<plug>:<publication>:<account>`. A connection that lacks the operation is
skipped and the run says so.

## Round four: the inputs contract, as the networks asked

* `title` (round four): the post's title (`SocialPost.title`) or the account's own (`targets[].title`), given to every
  operation of the post; a network that takes one says `meta.social.title: {required, max}` and is checked; no network
  asks it as a setting.
* `publish_media` (round four): the medium's own `kind` (image, video, document) is kept; the post's kind is
  `post_kind`. (round five: `url` is gone; the description makes the address, below.) A network that takes some media
  only by address says `meta.social.media.by_address: [kinds]`.
* `media_items[].file` (round three, used now): the stored file, for a `publish` that sends files itself; Telegram,
  Discord, Tumblr and Dribbble no longer have a read-only `publish_media`.
* `reply` (round four): `account_id` and `handle` on every reply, the first comment's (`first_comment: true`) and the
  inbox's answers. The first comment's id is kept (`first_comment_id`) and deleted with the post, first.
* `delete` (round four): `id`, `item`, `follow_up`, `first_comment`, `root`, and `account_id`, `handle`, `kind`,
  `settings` and `items` (every item as kept: `{item, id, url, media, sent_at, answer}`).
* `check`, `metrics`, lookups (round four): an operation reads when it says `"changes": false`, else when its method
  is GET (an operation of steps without a method changes something).
* `kind_by_media` (round four): `meta.social.kind_by_media: {"none": "message", "image": "media"}` picks the kind
  by the first medium when the person picks none (Telegram, Slack); `_calc`'s `checks` show the kind chosen.
* Read-back posts (round three): a pull onto `SocialPublication` sets `via_connection` (create) to find the account.

## Round five: on the platform's 0.3.8

* **Public addresses** (BD-64): a network that pulls media by address names them in `public_files` on the operation
  Social calls (a step's own operation cannot have them; a step reads `public_files.<name>`), a fixed set of names
  (`p0`, `p1`, …) for the media an item may carry, a name past the item's media taking its first file again. A text post
  has no file to make one from, so Social calls `publish_with_media` for an item with media where a description has it
  (Threads, Google Business), `publish` otherwise. An address lives an hour or two (the network fetches while it
  processes), Pinterest's 60 minutes. A server without `PUBLIC_URL` refuses such media on the post, in the platform's
  words, before it leaves Draft. The test fakes pull each address (a range first, then the whole) and check its bytes.
* **Refusals in a 2xx answer** (BD-68): `refused_if`, `error`, `temporary_if` on every operation of Slack, Hashnode, VK,
  Twitch, Kick, TikTok Business and Reddit; the person reads the network's words.
* **Sign-in** (BD-72, BD-77, BD-78): TikTok's `client_key` and Slack's and TikTok's comma-joined scopes; Bluesky's
  permalink from the DID its session keeps (`not_secret`); Meta's webhook handshake on the inbound address.
* **Per operation** (BD-73, BD-74, BD-75, BD-76, BD-70, BD-69): posting limits (X, LinkedIn, TikTok, Instagram);
  Instagram's upload host given the token as `OAuth`; one request with a file part for each item (Discord, Tumblr,
  Telegram); Whop's upload headers from its answer; Google Business numbers over a window from `now`; WordPress's
  content types sorted.
* **Feeds** (BD-65): `any_host`; a workspace's feeds connection of the earlier shape takes the new one at the upkeep.

## Each network

What each statement below says Social would need was done in rounds four and five. Every network is tested against a
fake of its documented API; none has run against the real network.

### X (`x.json`)

| | |
|---|---|
| Can | OAuth 2 with PKCE, client by basic authentication, the user read after sign-in (`after: me`, kept `user_id`); text (weighted counting, links as 23); up to 4 pictures or one video by INIT/APPEND/FINALIZE, processing waited for, alt text set; threads as reply chains; first comment; delete, repost, like; direct messages sent; numbers (public metrics), followers, name and picture; mentions lookup; mentions pulled forward by `since_id`, a reply told as a comment on the post it answers. (round five) Posting is limited to X's 100 posts in 15 minutes on the operation itself. |
| Cannot yet, and why | a lost post is not resent (X honours no idempotency key; reading back would compare text X rewrites with t.co links); direct messages are not read (a DM pull would read the account's own messages too, and a mapping cannot tell them apart); link clicks (`non_public_metrics`) not read; Premium's longer posts (the limit is fixed at 280 in the rules). |
| Set up at the network | a developer app with OAuth 2.0 on, type "Web App" (confidential), callback `{PUBLIC_URL}/api/connections/callback`, a plan that allows posting and media. |
| Unverified | the v2 media endpoints `/2/media/upload/initialize\|{id}/append\|{id}/finalize`, `/2/media/metadata`, the `STATUS` query. |


### LinkedIn (member) and LinkedIn page (`linkedin.json`, `linkedin_page.json`)

| | |
|---|---|
| Can | sign-in (client in the body), the member read from `/v2/userinfo`; the page picked from the organisations the member administers; text escaped as LinkedIn's little format wants (as Postiz does); pictures, PDF documents (title from the settings) and video by initialise → put (a video in the parts LinkedIn names, finalised with their ETags) → post, videos waited for (a page's pictures and documents too); several pictures as `multiImage`; comments as follow-ups and first comment (a page's as the organisation); delete of posts and comments; a page's numbers (impressions, reach, clicks, likes + comments + shares) and followers. A page's lost post is found among its recent posts by its commentary. (round five) A member's 150 posts a day are a limit on the posting operation. |
| Cannot yet, and why | a member's lost post fails (reading a member's posts needs `r_member_social`, which LinkedIn grants to partners); mentions and hashtags are escaped to plain text (a template cannot leave `@[name](urn:…)` unescaped while escaping the rest); member numbers (`r_member_postAnalytics`); comments are not read (webhooks for approved partners); a member's token is not refreshed unless the app has programmatic refresh. |
| Set up at the network | an app with "Share on LinkedIn" and "Sign In with LinkedIn using OpenID Connect" (member), "Community Management API" (page, a review), the redirect address, and the page's admin role for the signing member. |
| Unverified | `Linkedin-Version: 202610`; a comment's id in the answer body (`id`) else `x-restli-id`; deleting a comment without `actor`; `organizationalEntityShareStatistics` with `shares=List(…)` for a Posts API URN. |


### Facebook Page (`facebook.json`)

| | |
|---|---|
| Can | sign-in, the short token exchanged for a long-lived one, and every page's own token kept **sealed with the sign-in** (`keep: pages`), never in an answer (tested: it rests nowhere); a page's calls are made with its own token; the long-lived token renewed by exchanging it again; the page picked; text with the link as its card; up to 10 photos uploaded unpublished then attached; a video post (bytes to `graph-video`); reels and video stories by start → bytes to `rupload` → wait → finish; photo stories; comments and first comment; delete, like; numbers (`post_media_view`, `post_total_media_view_unique`, `post_clicks`, reactions + comments + shares) and followers. A lost feed post is found among the page's latest by its message; a lost reel by its publishing status. (round five) Messenger messages arrive by the inbound address: Meta's handshake is answered for the connection's `verify_token`, deliveries are checked against the app's secret (`X-Hub-Signature-256`). |
| Cannot yet, and why | a lost story fails; a page added after sign-in needs signing in again (its token is kept then); a page's comments are not read (its `feed` webhook mixes comments with likes and posts, and an inbound mapping cannot pass over the others); a delivery with several entries is read for its first; videos over 25 MB. |
| Set up at the network | a Meta app (Business type) with Facebook Login for Business, the permissions `pages_show_list pages_manage_posts pages_read_engagement pages_manage_engagement read_insights business_management` through App Review, the redirect address, the app live. |
| Unverified | Graph `v25.0`; the `post_media_view` / `post_total_media_view_unique` metric names (Meta replaced impressions in 2025); `rupload` taking `offset`/`file_size` headers with the bytes. |


### Instagram (through a Facebook page) and Instagram (its own sign-in) (`instagram.json`, `instagram_standalone.json`)

| | |
|---|---|
| Can | the account picked from the pages' Instagram accounts (or the signed-in one, `/me`); a picture by address, a video by its bytes to the upload host (`upload_type=resumable`); carousels (children made, a video child's bytes sent, all waited for together with `?ids=`), reels (trial reels), stories; collaborators; published once, a lost answer found among the account's media by its caption, then its permalink read; first comment and comments; numbers (`views`, `reach`, `total_interactions`) and followers; long-lived tokens exchanged and renewed (`fb_exchange_token`; `ig_exchange_token` and `ig_refresh_token`). (round five) Pictures by an address the description makes from the stored file (`public_files`), stories with their picture; the upload host given the token as `OAuth`; Instagram's 100 posts a day on the publishing operation; comments by the inbound address (the app subscribed to `comments`), after Meta's handshake. |
| Cannot yet, and why | delete (the Instagram API has none); a lost story fails (no caption to find it by); location and user tags not offered; a delivery with several entries is read for its first; an item makes ten addresses whatever its media (a fixed set of names; a video's is never sent). |
| Set up at the network | a Meta app with "Instagram API with Facebook Login" (or "with Instagram Login"), `instagram_content_publish` (resp. `instagram_business_content_publish`) and the rest through App Review, the account a Business or Creator account (linked to a page for the first), the redirect address. |
| Unverified | multi-id status reads (`/?ids=`) on graph.instagram.com; `views` as the impressions metric; the upload host taking `Bearer`. |


### Threads (`threads.json`)

| | |
|---|---|
| Can | sign-in, long-lived token exchanged (`th_exchange_token`) and renewed (`th_refresh_token`); text with the link as `link_attachment`; pictures and videos by address; carousels with children waited for together; reply chains; first comment; who may reply; topic tag; published once (a lost answer found among the account's posts by its text); permalink; repost; delete; numbers (views; likes + replies + reposts + quotes + shares) and followers; mentions pulled, a mention answering a post as a comment on it. (round five) Pictures and videos by an address the description makes (`publish_with_media`, which Social calls for an item with media). |
| Cannot yet, and why | replies to the account's posts other than mentions are not read (per-post `/replies`); comments by webhook (Threads' webhooks are not described). |
| Set up at the network | a Meta app with the Threads use case, the `threads_*` permissions, the redirect address. |
| Unverified | `?ids=` status reads on graph.threads.net; deleting by `DELETE /{id}`; `topic_tag`. |


### YouTube (`youtube.json`)

| | |
|---|---|
| Can | Google sign-in with `access_type=offline`, the channel kept; the channel picked; a video's resumable upload started with title, description, tags, category (choices from `videoCategories`), visibility and made-for-children, sent 8 MiB at a time; a lost chunk settled by asking the session what it holds (a complete upload is never sent again); thumbnail set; shorts (their URL); first comment (a new thread) and inbox answers (a reply to a comment); comments pulled for the channel; numbers (views, likes + comments), subscribers, like, delete. |
| Cannot yet, and why | videos over 25 MB; impressions (YouTube Analytics, another API) — views stand in; region of the category list fixed to `US`. |
| Set up at the network | a Google Cloud project with the YouTube Data API v3, an OAuth client (web), the consent screen (verification for `youtube.upload`/`youtube.force-ssl`), the redirect address; thumbnails need a verified channel. |
| Unverified | `allThreadsRelatedToChannelId` still served. |


### TikTok (`tiktok.json`)

| | |
|---|---|
| Can | privacy options from the creator's own as choices; comment/duet/stitch and branded settings; a video initialised with its chunk size and count and sent in chunks (the remainder in the last), its publishing waited for, the public post's id kept; photo carousels by address; numbers (views; likes + comments + shares) and followers. (round five) TikTok's own sign-in: `client_key` on the authorisation address and every token request, scopes joined by commas (tested); photos by an address the description makes, at most ten (a fixed set of names; TikTok takes 35); six initialisations a minute on each. |
| Cannot yet, and why | a lost chunk stops the call (TikTok publishes an upload once whole and offers no way to ask what it holds, so restarting could post twice); delete and comments (no API); more than ten photos; videos over 25 MB. |
| Set up at the network | a TikTok developer app with Login Kit and Content Posting API (Direct Post needs an audit; unaudited apps post privately), the redirect address, `user.info.stats` and `video.list` scopes. (round five) Verify the domain of the server's public address (`PUBLIC_URL`) in the app: TikTok pulls photos only from a verified domain. |
| Unverified | `publicaly_available_post_id` being the id `video/query` takes. |


### Pinterest (`pinterest.json`)

| | |
|---|---|
| Can | sign-in with basic client authentication and refresh; boards as choices; a pin of one picture or several (carousel) by address, or of a video registered, sent to the storage address with the fields the answer names, and waited for; title, link, alt text, colour; a lost pin found on its board by its description; numbers (impressions, pin + outbound clicks, saves + reactions + comments), followers, delete. (round five) Pictures by addresses the description makes (up to five, an hour each). |
| Cannot yet, and why | a video's own cover (key frame 0 is used); comments (no API). |
| Set up at the network | a Pinterest app with Standard access (trial access posts only to sandbox), the redirect address. |
| Unverified | `pin_metrics.lifetime_metrics` field names. |


### Reddit (`reddit.json`)

| | |
|---|---|
| Can | a permanent sign-in, the user kept; subreddits searched and their flairs as choices; text, link and picture posts (an upload lease, the file to storage with the lease's fields) and galleries; NSFW, spoiler; a lost post found among the account's submissions by its title; first comment and comments; the inbox (replies as comments, mentions, private messages); private messages sent; votes; numbers (score + comments), delete; one request at a time. |
| Cannot yet, and why | video posts (Reddit wants a poster picture and a websocket for processing). (round five) Reddit's refusals in a 200 are read in its words. |
| Set up at the network | a "web app" at reddit.com/prefs/apps, the redirect address, and a User-Agent naming the app's Reddit user (replace the placeholder in the file's headers). |
| Unverified | `/api/subreddit_autocomplete_v2`; `submit_gallery_post.json`'s shape. |


### Mastodon (any server) (`mastodon.json`)

| | |
|---|---|
| Can | the platform's client registered at the server by `POST /api/v1/apps` (once), a token that never expires, the account's id kept after sign-in; visibility, content warning, language, polls; up to 4 media with descriptions, waited for while processing; threads; first comment; comments told from mentions again (a pull asks for the account with each page, and a reply to the account's own status is a comment); direct messages answered directly; favourite, boost, delete; numbers and followers; mentions to the composer. A lost answer is sent again under the same `Idempotency-Key`, which Mastodon honours. |
| Cannot yet, and why | the account's own native posts read back (Social, item 4); the server's own character limit (500 is assumed). |
| Set up at the network | nothing at the server (registered on first sign-in); the connection's address is the server's (four places in the file). |
| Unverified | none beyond the docs read for 0.7.0. |


### Bluesky (`bluesky.json`)

| | |
|---|---|
| Can | a session from the handle and an app password, refreshed; the record key made from the call's key (`key \| tid`), so a lost answer is found where the record would be and never made twice; links, tags and mentions as facets at their byte offsets (mentions resolved to DIDs; an unknown handle stays text); link cards; up to 4 pictures with descriptions and shapes, or a video; threads and the first comment with the root's and the parent's references; likes, reposts; direct messages through the chat service; languages; notifications pulled as before; numbers and followers. (round five) The post's permalink, `bsky.app/profile/<did>/post/<rkey>`, from the DID the session keeps (`not_secret`). |
| Cannot yet, and why | link cards carry no title, description or picture (fetching the page is not a network call); a permalink with the handle (the handle is the connection's secret, masked wherever it shows: the permalink names the DID, which Bluesky resolves the same). |
| Set up at the network | an app password (with direct messages allowed, for DMs). |
| Unverified | a video by `uploadBlob` and `app.bsky.embed.video` without the video service. |


### Google Business Profile (`google_business.json`)

| | |
|---|---|
| Can | Google sign-in with refresh; the location picked from the first account's; updates, events and offers with their button, dates, coupon, redeem address and terms; one picture by address; a lost post found among the location's by its words; numbers (search views, button clicks), the location's name, delete. (round five) The picture by an address the description makes (`publish_with_media`); numbers over the last 540 days from the start of a day, worked out from now. |
| Cannot yet, and why | locations of a second account (the picker reads the first); no comments (posts have none). |
| Set up at the network | a Google Cloud project with the Business Profile APIs (access by application to Google), an OAuth client, the redirect address. |
| Unverified | `localPosts:reportInsights` still served, and from a fixed start date. |


### Telegram (`telegram.json`)

| | |
|---|---|
| Can | bot token as the connection's secret (in the URL path, as Telegram wants it); the chats the bot was added to are what the sign-in reaches (`getUpdates`, `my_chat_member`), the chat id is the account's id on the network; two kinds: `message` (HTML, 4096) and `media` (caption 1024, one to ten photos, videos or files); text, a single photo, video or document, and an album of two to ten (`sendMediaGroup`, a part for each file; photos and videos may mix, documents only with documents; the words are the first item's caption); follow-ups as replies (`reply_parameters`), text only; delete (`deleteMessages`, every message of the item in one call, the id is `<chat>:<first>,<second>...`); the account's members (`getChatMemberCount`) and name. |
| Cannot yet, and why | numbers for a post (the Bot API gives a bot no views or reactions); mentions; files above 25 MB (Telegram takes 50 MB, the platform's part limit is 25 MB); a lost answer on a send fails the call (no idempotency header and no way to read a message back). (round five) An album takes ten, in one request with a part for each item. |
| Set up at the network | create the bot with @BotFather, put its token under the connection's `bot_token`, add the bot to the group or channel as an administrator with the right to post, send one message there (a channel: make a post) so that `getUpdates` shows it, then pick the chat in Social. Do not set a webhook on the bot (`getUpdates` stops working). |
| Unverified | the token's `:` is sent as it is (the `url` form of an operation prints it unencoded; a path would percent-encode it); an empty `caption` on the second and later pictures; that 429 carries a `Retry-After` header (the API gives `parameters.retry_after` in the body, which the platform does not read, so the call is "later" with the platform's own backoff); `characters` as the counting unit (the docs say "characters after entities parsing"). |


### Discord (`discord.json`)

| | |
|---|---|
| Can | a bot token (`Authorization: Bot`); the servers the bot is in are what the sign-in reaches (server icon as the picture), the server id is the account's id on the network, the channel is a required setting whose choices are the server's text and announcement channels (categories and voice channels filtered out by the description); markdown text up to 2000 characters; up to 10 files in one message (a `payload_json` part and `files[n]`, a part for each file), each with its description; follow-ups as a thread made from the first message (made once, every later item posted in it); a first comment as a reply in the channel; delete by channel and message; mentions (`<@id>`, member search); the server's name and member count. Never twice: Discord has no idempotency header but takes a `nonce` with `enforce_nonce`: the description sends the call's key as the nonce (`{{ key \| tid }}`, 13 characters) so a second send is answered with the first message. The operations name an `idempotency_header` (`X-Idempotency-Key`, ignored by Discord) only so that the platform may send again after a lost answer; the protection is the nonce. A thread made twice is refused by Discord (160004); its step reads the thread back (`confirm`) instead of sending again. |
| Cannot yet, and why | numbers for a post (reactions cannot be summed in the platform's expression language); embeds, stickers, polls, forum or voice channels, a thread name of its own (always "Replies"); files above 10 MiB (see unverified); the message's permalink (the link needs the server id, which `publish` is not given: Social would have to pass `account_id` into `url`, the description does not map one). |
| Set up at the network | a Discord application with a bot (discord.com/developers), the bot token under `bot_token`, the bot invited with View Channel, Send Messages, Attach Files, Create Public Threads, Send Messages in Threads (and Manage Messages to delete other people's), the Server Members intent for mentions. |
| Unverified | the 10 MiB file limit (Discord's limit for bots has changed more than once, 8 / 25 MiB before); that a new upload may be described in `attachments` by `id` and `description` without `filename`; that the member search needs the privileged Server Members intent (it does in the docs I know); the `User-Agent` form. |


### Slack (`slack.json`)

| | |
|---|---|
| Can | OAuth v2 (the workspace's Slack app, bot token; the workspace is the account, picked from `auth.test`); the channel is a required setting with choices from `conversations.list`; two kinds: `message` (4000 characters) and `images` (3000, a section block and one image block for each of up to 10 pictures); pictures through Slack's external upload flow (`files.getUploadURLExternal`, the bytes to the signed address with no credentials, `files.completeUploadExternal`); follow-ups as replies in the first message's thread (`thread_ts`); delete (`chat.delete`, the id is `<channel>:<ts>`); the message's permalink (`chat.getPermalink`); mentions (`<@id>`, from `users.list`); the workspace's name. Never twice: Slack has no idempotency header. Each post carries the call's key in the message's `metadata`; if the answer is lost the call reads the channel's recent messages (`conversations.history` with `include_all_metadata`) and takes the message with that key as the answer. If it cannot prove it (the bot is not a member of the channel, so it cannot read it) the call fails and is not sent again. The upload's completion is read back (`files.info`) the same way. |
| Cannot yet, and why | numbers (Slack gives none for a bot's post); mrkdwn is shown as plain text in the composer (no `mrkdwn` text format in `meta.social`); posts to a channel the bot is not in need `chat:write.public` and cannot be read back; user-scope tokens; token rotation (`never_expires` is set; a workspace that turns rotation on must connect again). (round five) Slack's refusals in a 200 (`ok: false`) are read in its words, `ratelimited` as "later"; scopes go comma-separated. |
| Set up at the network | a Slack app (api.slack.com/apps) with the redirect `{PUBLIC_URL}/api/connections/callback`, the client id in the description and the client secret under `client_secret`; the bot scopes the description lists; add the bot to the channels it posts to (or leave `chat:write.public`, losing the read-back there). |
| Unverified | that an image block's `slack_file` may name a file completed without a `channel_id` (Slack's docs say the file must be visible to the bot; Postiz shares the file into the channel); that `chat.getPermalink` needs no scope beyond `chat:write`; the `Retry-After` of a 429 (Slack sends it). |


### WordPress (`wordpress.json`)

| | |
|---|---|
| Can | the site's REST API with an application password (basic auth; the user name is in the description, the password under `application_password`, the site address is the placeholder in `base`, `accounts` and `account`: four places); the site is the account; html text; settings: title (required), kind of content (`/types`, so pages and custom types with `show_in_rest`), status (publish, draft, pending, private), categories and tags (choices, searchable), address name; a cover picture (upload, then its description, then `featured_media`); the post's permalink; delete to the trash (no `force`), the id is `<rest_base>:<id>`. Never twice: WordPress has no idempotency header. The post step reads back after a lost answer: posts of that kind with that exact title (`context=edit`, `status=any`) created after the publication's `created_at`. A medium's upload is a one-step operation: a lost answer fails the call rather than uploading twice. |
| Cannot yet, and why | new tags or categories (only existing ones are chosen); inline pictures (only the cover; Social has no `html` of a medium); scheduled posts (`future` needs a date setting: add when Social passes the post's time); excerpt, comment status, sticky, formats; a different author; follow-ups (none); numbers (WordPress core has none). The read-back would match a different post with the same title made in the same moments by someone else. (round five) The content types come sorted by name. |
| Set up at the network | a user with the right to publish, an application password (Users, Profile), HTTPS and the REST API reachable at `/wp-json` (pretty permalinks or `?rest_route=` not supported here); put the site address where the placeholder is (four places) and the user name in `auth.username`. |
| Unverified | that `after` accepts the RFC 3339 `created_at` with a `Z` (WordPress compares in the site's time zone); the `Content-Disposition` form with a quoted file name for names with quotes; no rate limit is described (hosts differ). |


### Medium (`medium.json`)

| | |
|---|---|
| Can | an integration token (bearer); the token's user is the account (`/me`); a story in markdown with title (100), tags (comma-separated text, trimmed, empty ones dropped), original address, visibility (public, draft, unlisted), licence, "tell followers", and a publication (choices from `/users/{id}/publications`; the story then goes to `/publications/{id}/posts`); a picture: uploaded to `/images` and written at the top of the story as `![alt](address)`; the story's address. |
| Cannot yet, and why | delete or edit (the API has neither: `delete_remote` says the connection does not offer it); numbers (none); more than three tags (Medium ignores the rest; the setting does not count them); several pictures (one is the lead; more would be inline, which needs Social to place media in the text); follow-ups (none); a lost answer on the story fails the call ("may have been sent"): Medium offers no list of one's stories to read back, and no idempotency header. |
| Set up at the network | an integration token from Medium's settings (only an account that already has one), under `integration_token`. |
| Unverified | all of it against the live service: Medium stopped issuing integration tokens to new accounts in 2023 (existing tokens still work); the picture types; the 100-character title limit; `reject("equalto", "")` in the tags template is minijinja's, checked only by the fake. |


### DEV Community (`devto.json`)

| | |
|---|---|
| Can | an API key (`api-key` header, the v1 `Accept` type); the key's user is the account; a markdown article with title, tags (up to four, choices from `/tags`), cover picture by address, original address, summary, series, an organisation (choices) and a draft switch; the article's address; "delete" unpublishes (`PUT /articles/{id}` with `published: false`; the API has no delete). Never twice: no idempotency header. After a lost answer the call reads the author's own articles (`/articles/me/all`) for one with that title published since the publication's `created_at` (compared to the second); a draft cannot be found that way, so a lost answer on a draft fails the call instead of being resent. |
| Cannot yet, and why | upload a cover (the API has no upload: the cover is an address typed as a setting; a public address for a stored file would serve, not described yet); list all organisations (Forem has no call for a user's own: the choices are the organisations of the articles already written, as Postiz does); edit; numbers (page views and reactions are in `/articles/me/all`; they are not mapped here); follow-ups (none). |
| Set up at the network | an API key (Settings, Extensions) under `api_key`. |
| Unverified | rate limits (the pages I could recall name none that I trust, so none is set); that `PUT /articles/{id}` unpublishes with an API key for the author's own article; `published_timestamp` as the field to read back by; the 128-character title limit; a tag outside the list is refused (the choices only offer the list). |


### Hashnode (`hashnode.json`)

| | |
|---|---|
| Can | GraphQL at `gql.hashnode.com` with a personal access token (the `Authorization` header, no prefix); the token's user is the account; a markdown article with publication (required, choices from `me.publications`), title, subtitle, tags (comma-separated names, sent as `{slug, name}`), cover picture by address, original address, address name; the article's address; delete (`removePost`). Never twice: no idempotency header. After a lost answer the call reads the publication's ten latest posts for the same title published since the publication's `created_at` and takes that post as the answer; if it is not there the call fails instead of sending again. |
| Cannot yet, and why | upload a cover (an address only: Hashnode would fetch a stored file by a public address, not described yet); choose tags from Hashnode's own list (a text setting; the tag's slug is the lower-cased name, so a tag Hashnode knows under another slug is created or refused); series, scheduling, newsletter switch, co-authors, drafts; numbers; follow-ups. (round five) Hashnode's refusals in a 200 (`errors`) are read in its words. |
| Set up at the network | a personal access token (hashnode.com/settings/developer) under `access_token`. |
| Unverified | all of it against the live API (the mutation and field names are from Hashnode's public schema as I know it; Postiz still calls the old `gql-beta` address); the mutation limit of 500 a minute that `limit` sets; that unknown tag slugs are accepted; the 250-character title limit. |


### Tumblr (`tumblr.json`)

| | |
|---|---|
| Can | OAuth 2 (the workspace's Tumblr app; scopes `basic write offline_access`, refreshed by the platform); the blogs the sign-in reaches are the accounts to pick (the blog's name is its id); posts in NPF: the text split into a text block for each paragraph (blank line), tags (text, commas) and a state (published, draft, queue, private); up to 10 pictures and videos in one post (the post as the `json` part and each file as a part the blocks name by `identifier`), a picture's description as `alt_text`; the post's address; delete (`post/delete`; the id is `<blog>:<post>`); the post's notes (`note_count`) as engagements; the blog's name and followers; the documented limit of 1000 requests an hour. Never twice: no idempotency header. After a lost answer the call reads the blog's ten latest posts (`npf=true`) for one made since the publication began whose first text block is this post's first paragraph; a post without text cannot be found that way, so a lost answer there fails the call. (round five) The files in one request with a part for each item. |
| Cannot yet, and why | formatting (bold, links inside the text: NPF takes them as `formatting` ranges with offsets, which a template cannot count; the text is plain and a link is written after it); reblogs, polls, audio, links as link blocks; the 250 posts a day limit is Tumblr's, not counted here; a video above 25 MB (the platform's part limit; Tumblr takes 100 MB); follow-ups (none); mentions; the account's picture is Tumblr's avatar address. |
| Set up at the network | a Tumblr app (tumblr.com/oauth/apps) with the redirect `{PUBLIC_URL}/api/connections/callback`; the consumer key as `client_id` in the description and the consumer secret under `client_secret`. |
| Unverified | that the `json` part needs no content type of its own; that the token address takes the client secret in the body; the error text pointer (`/errors/0/detail`; an answer without it shows the raw body); `timestamp()` of the publication's moment against a post's epoch `timestamp`. |


### Lemmy (`lemmy.json`)

| | |
|---|---|
| Can | a session from the account's user name and password (secrets `username` and `password`; `/user/login`, a jwt as a bearer, signed in again by the platform when it is refused); the instance address is the placeholder in `base` and `publish_media` (two places); the user is the account; the community is a required setting (the month's top communities as choices, or a search when something is typed); a post with title, markdown body, not-safe-for-work switch, a link (`url`), a picture uploaded to the instance's pictrs and used as the link's thumbnail (`custom_thumbnail`), or as the post itself when there is no link, the picture's description (`alt_text`); follow-ups and the first comment as comments (`/comment`); delete of a post or a comment (`/post/delete`, `/comment/delete`, the comment first); numbers (upvotes, downvotes and comments of the post); the user's name and picture. Never twice: no idempotency header. After a lost answer the call reads the community's ten newest posts (or the post's newest comments) for one with that title (content) published since the publication began and takes it as the answer. A lost answer on the picture's upload fails the call instead of uploading twice. |
| Cannot yet, and why | say more than Lemmy's error code (`language_not_allowed`, `rate_limit_error`: the API gives codes, not sentences); a language, a nested comment; edit; the instance's rate limits (set per instance: none is described); mentions of users or communities; followers (a Lemmy user has none in the API); a v4 (Lemmy 1.0) instance (the paths are `/api/v3`; edit the base). |
| Set up at the network | an account on the instance (a bot account is polite), its user name and password under the connection's secrets, the instance address in place of the placeholder (twice). |
| Unverified | that `custom_thumbnail` and `alt_text` exist on the instance's version (0.19 and later); that pictrs takes the bearer header (0.19 does; older versions read the cookie); `search` with a typed query; the 50000-character body. |


### VK (`vk.json`)

| | |
|---|---|
| Can | VK ID OAuth 2 with PKCE (`id.vk.ru`, a public client: no client secret; scopes `vkid.personal_info wall photos video`; `keep: ["device_id"]`, a `refresh_vk` operation that sends the kept device id); the user is the account; wall posts with up to 10 attachments, a picture through `photos.getWallUploadServer`, the upload and `photos.saveWallPhoto`, a clip through `video.save` and its upload; the attachments named `photo<owner>_<id>`, `video<owner>_<id>`; a friends-only switch; follow-ups and the first comment as comments (`wall.createComment`); delete of a post or a comment (the comment first); views as impressions, likes + reposts + comments as engagements (`wall.getById`); the user's name, picture and followers; the documented 3 requests a second. Never twice: no idempotency header. After a lost answer the call reads the wall's ten newest posts (`wall.get`) for one with those words dated since the publication began. A lost answer on a comment or an upload step fails the call. (round five) VK's refusals in a 200 (`error`) read in its words; too many requests (codes 6, 9, 29) as "later". |
| Cannot yet, and why | **connect.** VK ID wants the callback's `device_id` (and `state`) in the code exchange, and in every refresh; the platform's exchange sends neither and its `refresh` operation is not given `auth.*`. The description is written for what the brief said (the device id in the token answer, `keep`) but I believe VK gives it only in the redirect's query. *Needed from the platform*: (1) extra parameters of the redirect (`device_id`, `ext_id`) added to the token request and kept with the tokens (e.g. `provider.callback_params`); (2) the kept fields (`auth.*`) available to the `refresh` and `after` operations (they are `bare` now, so `{{ auth.device_id }}` is undefined and the refresh fails with "a template failed"). The fake in the test takes the code without a device id and gives tokens that outlast the test, so the exchange with a device id and the refresh are **not proved**. Also: say why VK refused in VK's words (VK answers 200 with `error`; the description makes the missing id fail the call: *needed from the platform*, a refusal marker for a 2xx answer, as for Slack); `publish_media` is given `kind` as the post's kind, not the medium's (the description reads `content_type`; *needed from Social*: a separate name for the medium's kind, e.g. `media_kind`); posting as a community (a community's own token and `owner_id` `-<group>`: a second account kind); stories, polls, albums; a post's address on a custom domain. |
| Set up at the network | a VK ID application (id.vk.ru/business) with the redirect `{PUBLIC_URL}/api/connections/callback`; its id as `client_id` in the description (twice: `provider` and `refresh_vk`). |
| Unverified | the hosts (VK moved from vk.com to vk.ru; the upload servers' hosts vary and each must be in `allowed_hosts`: the list has the ones I know); `v=5.251` (Postiz's); the bearer header on API methods (VK documents it; Postiz puts the token in the query); the 16384-character message limit; that scopes are passed space-separated. |


### Twitch (`twitch.json`)

| | |
|---|---|
| Can | OAuth 2 (the workspace's Twitch app; `Client-Id` header beside the bearer, so the client id appears twice); the signed-in user's own channel is the account; two kinds: a chat message (500 characters, `/chat/messages`) and an announcement (`/chat/announcements`, a colour setting); follow-ups as replies to the message before (`reply_parent_message_id`); delete of a chat message (`/moderation/chat`, the answer first); followers of the channel as the account's number; the documented 20 messages in 30 seconds. Never twice: Twitch has no idempotency header and no call that reads a chat back, so a lost answer on a message or an announcement fails the call ("may have been sent") and is not sent again. |
| Cannot yet, and why | give an announcement an id (Twitch returns none: the id is `<channel>:announcement-<moment>` and deleting it is refused by Twitch); post in another channel (the account's own only); numbers for a message (none exist); media; mentions (typed `@name`). (round five) A dropped message is refused with Twitch's `drop_reason`. |
| Set up at the network | a Twitch application (dev.twitch.tv/console) with the redirect `{PUBLIC_URL}/api/connections/callback`, a confidential client; its id twice in the description, its secret under `client_secret`; connect as the broadcaster. |
| Unverified | the scope list (`moderator:manage:chat_messages` for deleting, `moderator:read:followers` for the count); the 20 messages in 30 seconds applies to an ordinary chatter (a moderator's is 100); `account_at: /user_id` (Twitch's token answer has no user; the account then shows without a name until `check`); a 204 answer for an announcement. |


### Kick (`kick.json`)

| | |
|---|---|
| Can | OAuth 2.1 with PKCE (`id.kick.com`; Kick's app is confidential: the client secret goes in the body too); the signed-in user's channel is the account (`/users`, the user id is the broadcaster id); chat messages up to 500 characters (`POST /chat`, `type: user`); follow-ups as replies (`reply_to_message_id`); delete (`DELETE /chat/{id}`). Never twice: no idempotency header and no call that reads a chat back: a lost answer fails the call and is not sent again. |
| Cannot yet, and why | say why Kick dropped a message beyond that it did (a 200 with `is_sent: false` and no reason: refused as such since round five); followers or numbers (I could not confirm a call for them, so none is described); chat in another channel; announcements; media; rate limits (none published that I could confirm, so no `limit`). |
| Set up at the network | an app in Kick's developer settings (needs two-factor sign-in) with the redirect `{PUBLIC_URL}/api/connections/callback`; its id in the description, its secret under `client_secret`. |
| Unverified | all of it against the live API, including the scope names (`user:read channel:read chat:write moderation:chat_message:manage`), that `type: user` with the broadcaster's own id sends as the signed-in user, and that the token address takes the client secret in the body. |


### Dribbble (`dribbble.json`)

| | |
|---|---|
| Can | OAuth 2 (`public upload`; Dribbble's tokens do not expire, `never_expires`); the user is the account; a shot with exactly one picture (png, jpeg, gif; at least 400x300; 8 MB), a title (required), an html description and the team to post as (choices from `/user`); the shot's id from the `Location` header of the 202 answer; delete; views as impressions, likes + comments as engagements; followers; the documented 60 requests a minute. Never twice: no idempotency header. After a lost answer the call reads the user's own shots for one with that title published since the publication began; Dribbble makes a shot asynchronously (202), so one not yet listed is not found and the call fails rather than sending again. |
| Cannot yet, and why | tags, scheduling (`scheduled_for`), low profile, rebounds; follow-ups (the API has no comments); the picture's description; a GIF's smaller limit. |
| Set up at the network | an application at dribbble.com/account/applications/new with the callback `{PUBLIC_URL}/api/connections/callback`; its id in the description, its secret under `client_secret`. |
| Unverified | `DELETE /shots/{id}` and `GET /shots/{id}` counts (`views_count`, `likes_count`, `comments_count`) and `team_id` on create; the size limits; `published_at` on `/user/shots`; that `headers.location` is lower-cased in the expression. The 202's `Location` is read as the platform stores it. |


### Whop (`whop.json`)

| | |
|---|---|
| Can | OAuth 2 with PKCE (a public client: no secret); the user is the account (`/oauth/userinfo`); company and forum are required settings (the forum's choices depend on the company picked); a forum post in markdown with an optional title and up to 10 files: a file record, the bytes `PUT` to the address the record gives (no credentials), then the file's status read every 2 seconds until `ready` (a `wait` step, failing on `failed`), and the post names the files; follow-ups and the first comment as replies (`parent_id`); delete; the post's address. Never twice: no idempotency header. After a lost answer the call reads the forum's ten newest posts for one with those words created since the publication began. |
| Cannot yet, and why | numbers (none known); mentions; edit; a file above 25 MB; follow-ups with files. (round five) The headers the file record names go with the bytes (`headers_from`). |
| Set up at the network | a Whop app (whop.com/dashboard/developer) with the redirect `{PUBLIC_URL}/api/connections/callback`; its client id in the description; connect as a member of the company who may post in the forum. |
| Unverified | nearly everything, from Postiz's code and my memory of Whop's v1 API: the token address taking a form (Postiz sends JSON); `error.message` as the error text; `created_at` and `content` on a forum post; `DELETE /forum_posts/{id}`; the scopes; that the `PUT` needs nothing but the signed address. |


### MeWe (`mewe.json`)

| | |
|---|---|
| Can | the user's `apiToken` as a bearer and the app's id and key as `X-App-Id` and `X-Api-Key` (all three connection secrets); the user is the account; two kinds, a post on the user's timeline and a post in a group (the group a required setting, choices from `/groups`); plain text up to 63206 characters; up to 10 photos (`/photo/upload`, then `uploadedPhotoIds`). |
| Cannot yet, and why | **sign in.** MeWe's sign-in is its own login-request flow: the browser goes to `mewe.com/login`, comes back with a `loginRequestToken`, which is exchanged by `GET /api/dev/token` carrying the app's headers for an `apiToken` that lasts about 30 days. The platform's `oauth` exchange is a POST of a code to a token address; it cannot make that call, so the `apiToken` has to be obtained elsewhere and put under `api_token`, and it must be renewed by hand. *Needed from the platform*: a sign-in whose exchange is an operation of the description (a GET with the callback's `loginRequestToken` and headers from secrets), keeping its `apiToken` and expiry. Also: an id (posts answer 204, so the id is `mewe-<moment>`: no delete, no numbers, no comments; the post's permalink); video (Postiz filters it out); a lost answer fails the call (no key, nothing to read back). |
| Set up at the network | a MeWe developer app (by request to MeWe), its id and key, and an `apiToken` for the user (via MeWe's login flow). |
| Unverified | everything: MeWe's host, paths, field names and the `Forbidden` / `Unauthorized` / 420 texts come from Postiz and have not been checked against MeWe's documentation, which I could not read. |


### Listmonk (`listmonk.json`)

| | |
|---|---|
| Can | basic auth (the API user's name in the description, its token under `password`; the instance address is the placeholder in `base` and `publish`'s address: two places); the instance is the account (its site name); an e-mail campaign with subject (required), list (required, choices) and template (choices, "Default" first), the html as the body; created as a draft and then started (`PUT /campaigns/{id}/status` to `running`); the campaign's address in the admin; views as impressions, clicks and sent as reach; delete. Never twice: no idempotency header, and an e-mail sent twice is the worst case here, so both writing steps read back. A lost answer on the creation: the campaigns are searched by subject for one created since the publication began and used as the answer. A lost answer on the start: the campaign is read and, if it is running, finished or scheduled, the start is not sent again. If neither is proved the call fails. |
| Cannot yet, and why | schedule at Listmonk (`send_at`; Social sends at the time itself), choose several lists, a from address, a plain-text alternative, a preview text (the hidden preheader Postiz adds is not reproduced), upload pictures (images are addresses in the html); follow-ups. |
| Set up at the network | an API user (Settings, Users) with campaign rights, its name in `auth.username` and its token under `password`; the instance's address in place of the placeholder (twice). |
| Unverified | all of it against a live instance; `created_at` and `subject` in the campaign search; `per_page=all`; that a draft with the same subject is not found as a duplicate by a person re-sending the same subject (it would be, within the publication's own window only). |


### Moltbook (`moltbook.json`)

| | |
|---|---|
| Can | an agent API key (bearer); the agent is the account (`/agents/me`); a post in a submolt (choices from `/submolts`, `general` by default) with a title (a setting; the first 100 characters of the text when empty, as Postiz does) and 300 characters of text; follow-ups and the first comment as comments (`/posts/{id}/comments`); delete of a post or a comment; the post's address; the documented 100 requests a minute. Never twice: no idempotency header and nothing that lists one's own posts that I could confirm: a lost answer fails the call ("may have been sent") and is not sent again. |
| Cannot yet, and why | nested replies (every comment is under the post; Postiz chains them by the last comment); media; votes or numbers (no call I could confirm, so no `metrics`); delete a first comment (Social keeps no id for it); the agent's registration and claim (`POST /agents/register`, the human must claim the agent at a link: done by hand at Moltbook, then the key is pasted under `api_key`); Moltbook's posting limits (reported in its answer: "once every 30 minutes"). |
| Set up at the network | register the agent at Moltbook, have its owner claim it, put its API key under `api_key`. |
| Unverified | `GET /submolts` and `DELETE /posts/{id}`, `/comments/{id}` (not in Postiz); the answer shapes (`success`, `post.id`, `comment.id`); that a failing answer carries `error`. |


### TikTok Business (`tiktok_business.json`)

| | |
|---|---|
| Can | the Business API (`business-api.tiktok.com/open_api/v1.3`) with a **session** in place of OAuth: the connection holds the app's `client_id` and `client_secret`, the business account's `business_id` (its `open_id`) and a first `refresh_token` (obtained once, by hand, through TikTok's authorize page); the platform exchanges it for an access token (sent as `Access-Token`) and renews it with the refresh token each answer gives (checked: the rotated one is used); the business account is the account (name, picture, followers); two kinds: a video (2200 characters, counted in UTF-16) and a photo carousel (1 to 35 pictures, title of 90, 4000 characters, who can see it); comments, duet, stitch, brand switches; publishing by address, then the status polled every 3 seconds (a `wait` step, up to 15 minutes) until `PUBLISH_COMPLETE`, whose public video id is the post's id; views as impressions, likes + comments + shares as engagements. |
| Cannot yet, and why | **sign in with OAuth**: the authorize address wants `client_key` and the token call a JSON body with `auth_code`, answered under `data`, a code exchange the platform does not describe (0.3.8 did not add it): the connection is given a refresh token by hand. Delete (the Business API has none); music and location search; a draft instead of a post (`upload_to_draft`); a lost answer on the publish call fails the call (no key, and nothing to read back before the status exists); more than ten photos. (round five) Videos and photos go from Social by addresses the description makes, which TikTok pulls; its refusals in a 200 (`code` not 0) are read in its words. |
| Set up at the network | a TikTok for Business developer app with the media domain verified; authorize once to get the first refresh token; put the four values under the connection's secrets. |
| Unverified | all of it against the live API: field names, `video_views`, the `filters` form of the video list, `post_ids`, that the refresh call also serves as the first login, `expires_in` in seconds, the UTF-16 counting. |


### Farcaster (`farcaster.json`)

| | |
|---|---|
| Can | Neynar's hosted-signer API: the API key (`x-api-key`) and a signer's uuid, already approved, as connection secrets; the signer's Farcaster user (fid) is the account (name, picture, followers); a cast of 320 bytes with up to two embedded addresses (a setting, comma-separated), an optional channel (search-as-you-type choices from `/channel/search`; one channel, as Neynar takes one); follow-ups as a thread of replies (`parent` the cast before; the channel only on the first); delete (`DELETE /cast`); likes + recasts + replies as engagements. Never twice: Neynar takes an `idem` key in the body to make a cast once; the description sends the call's key as it (`{{ key \| tid }}`) and names an `idempotency_header` (ignored by Neynar) only so that the platform may send again after a lost answer, as for Discord's nonce. A second send is answered with the first cast. |
| Cannot yet, and why | embed a stored picture (Neynar takes web addresses: *needed from Social*, `media_items[].url`; until then the addresses are typed into the setting); create or approve the signer (done once in Neynar's dashboard or API; the signer needs a Farcaster account's approval); 1024-byte casts of Pro accounts; several channels at once; mentions; edit. |
| Set up at the network | a Neynar app (API key), a managed signer approved by the Farcaster account, both under the connection's secrets. |
| Unverified | the 320-byte limit and its counting in bytes; the `idem` length limit (16 characters; the key's `tid` is 13); `DELETE /cast` with a body; the cast's address (not mapped: the id is the hash); Neynar's rate limits depend on the plan. |


### Nostr: no description

| | |
|---|---|
| Can | — |
| Cannot yet, and why | A Nostr post is an event the person's own key signs (a Schnorr signature over the event's hash, secp256k1), sent to relays over WebSocket. Neither can be done by a connection description: the platform has no signature of that kind (its `signed` auth is an HMAC-SHA256 of a body with a shared secret), no WebSocket, and no way to keep a person's private key that the platform itself would not be able to read. What it would need: (1) a **signer**: a connection `auth` type that signs with a key held under a secret (BIP-340) over an expression of the event (`id = sha256` of the serialised `[0, pubkey, created_at, kind, tags, content]`, already available as filters), or better, a remote signer protocol (NIP-46, bunker) so the key never reaches Ferrum; (2) a **relay call**: a request kind that opens a socket, sends `["EVENT", ...]` and reads the `OK`; or a relay's HTTP bridge. Postiz keeps the key in a signed (not encrypted) token and posts to five fixed relays. Recorded; not built. |
| Set up at the network | — |
| Unverified | — |


### Skool: no description

| | |
|---|---|
| Can | — |
| Cannot yet, and why | Skool has no public API; Postiz signs in with cookies taken from a browser extension. The lead has decided not to build it. Recorded. |
| Set up at the network | — |
| Unverified | — |


## Needed from the platform

(round five) The platform's follow-up (App API 0.3.8, BD-63 to BD-78) answered: public addresses for stored files,
menu rows, an approval hook, reads of any public host, refusals in a 2xx answer, the client's parameter name and the
scope separator, kept sign-in values in results, Meta's handshake, a limit per operation, an operation's token scheme,
a part for each item of a list, headers from an answer, date arithmetic, a sorted list, reading steps that read, and
the stack. What remains:

1. **Sign-ins that are not the code exchange the platform makes**: VK ID needs the callback's `device_id` in the
   exchange and every refresh, and `auth.*` in `refresh`; TikTok Business wants `client_key`, a JSON body with
   `auth_code` and tokens under `data`; MeWe exchanges a `loginRequestToken` by a GET with the app's headers. Wanted:
   `provider.callback_params`, `auth.*` in `refresh` and `after`, an exchange described as an operation.
2. **Public addresses for each item of a list**: `public_files` is a fixed set of names, so an item makes as many
   addresses as its network's most media (ten for Instagram and TikTok, twenty for Threads), and photo posts to TikTok
   are held to ten. Wanted: `public_files` with `each`, as a file part has.
3. **An inbound mapping that passes over items**: a Facebook page's `feed` webhook mixes comments with likes and posts;
   a mapping cannot skip the others, so page comments are not read. Wanted: `when` on a mapping (or `skip_if`); and
   `items_at` over every entry of a delivery (Meta may batch several; the first is read).
4. **A secret shown where it is public**: Bluesky signs in with its handle, so the handle is masked everywhere; the
   permalink names the DID.
5. Files sent in chunks or as one body stop at `FERRUM_CONNECTION_UPLOAD_MB` (25 MB unless the operator raises it):
   every video limit in the rules is at most that.
6. **Signing and sockets for Nostr** (see its verdict).
