# Networks, set A: what each description does, what it cannot, and why

Fourteen descriptions: `x`, `linkedin`, `linkedin_page`, `facebook`, `instagram` (through a Facebook page),
`instagram_standalone` (Instagram's own sign-in), `threads`, `youtube`, `tiktok`, `pinterest`, `reddit`, `mastodon`,
`bluesky`, `google_business`. Each is tested in `tests/networks_a_it.rs` against a fake of the network's documented
API (its shapes as the network's pages and Postiz's providers give them), with the calls made as Social makes them.
**None has been run against the real network**: the first connection to each is the first time its real answers are
read. Facts marked *(unverified)* below are ones neither the network's pages nor Postiz settled.

Conventions in every file:

* **Never twice.** A post is an operation of steps, so a lost answer is caught. It is sent again only where the network
  honours a key (Mastodon's `Idempotency-Key`), or after a read-back proves it did not happen (`confirm`: LinkedIn page,
  Facebook, Instagram, Threads, Pinterest, Reddit, Google Business; Bluesky's record key is made from the call's key, so
  the read-back finds it where it would be). Elsewhere (X, LinkedIn member, TikTok, Facebook stories) a lost answer
  **fails the call**, saying it may have been sent, and nothing is sent again. Steps that leave nothing public
  (registering an upload, putting bytes, setting a description) carry the key in `X-Ferrum-Upload-Key`, a header the
  networks ignore: it only tells the platform such a step may be sent again (at worst an unused upload).
* A medium that is posted with the post (YouTube, TikTok and Facebook videos, Instagram and Threads media) goes in
  `publish`: `publish_media` then only checks the account and gives back `file:<stored file>` or `url:<address>`, read
  by `publish`. A multi-step post resumes after its last finished step.
* `publish_media` reads the medium's kind from `content_type`: Social gives the **post's** kind as `kind` there.
* Settings answers come in `inputs.settings`; choices operations are reading ones (`changes: false` where a POST reads).

## Needed from Social

1. **`account_id` (and `handle`) on every `reply` call**, the first comment's (`finish` in src/posts.rs gives only
   `text, in_reply_to, root`) and the inbox's (src/inbox.rs). Facebook comments are made with the picked page's own
   token, chosen by `account_id`: without it a Facebook first comment or inbox answer fails. LinkedIn page reads the
   post's author instead (one more request).
2. **A public address for a stored file**, given to `publish_media` as `url` (signed, expiring in an hour or more): the
   networks that pull media by address cannot take bytes for it. Instagram pictures, Threads pictures and videos,
   TikTok photos, Pinterest pictures, Google Business pictures. The descriptions read `inputs.url` already and the
   tests give it; until Social does, those posts are refused by the network (Instagram: "The parameter image_url is
   required"). TikTok photos also need the address's domain verified at TikTok.
3. **The medium's own kind in `publish_media`**: `media_inputs` puts the post's common inputs over the medium's, so
   `kind` is the post's (`reel`, `story`, `document`). Name one of them differently (`post_kind`).
4. **The account's own posts read back** (Mastodon's 0.2.0 behaviour; also X, Threads, Bluesky author feeds): a pull can
   read them (`inputs: {"id": "auth.account_id"}`), but a `SocialPublication` cannot be found by its connection the way
   `SocialMessage` is (`via_connection`). Accept `via_connection` (and `external_id`) on `SocialPublication`.

## Needed from the platform

1. **The client's parameter name and the scope separator** on `auth.provider` (TikTok: `client_key`, scopes joined by
   commas). The authorisation address carries `client_key` through `params`, but the token and refresh requests send
   `client_id`, so TikTok's sign-in fails (tested: "did not give tokens: 400: invalid_request"). Smallest addition:
   `provider.client_id_param: "client_key"` and `provider.scope_separator: ","`.
2. **A refusal read from a 2xx answer, and codes put in words.** Reddit answers 200 with `json.errors` (a call fails
   only because the post has no id, without Reddit's words); TikTok and Meta refuse with codes whose message is
   sometimes empty. Smallest addition: `error_at` as a pick (an `expr`), read also on a 2xx when it gives a value.
3. **Limits per operation**, not per connection: a connection's `limit` counts every request, polls included, so the
   networks' posting caps (X 100 posts / 15 min, TikTok 6 initialisations / min, Instagram 100 posts / 24 h, LinkedIn
   150 member posts / day) are left to the network's own 429. Smallest addition: `limit` on an operation.
4. **The authorisation scheme of one operation.** Instagram's upload host documents `Authorization: OAuth <token>`; the
   platform sends `Bearer` *(unverified whether it takes Bearer)*. Smallest addition: `credentials: {"prefix": "OAuth "}`.
5. **A public value not masked.** Bluesky's handle is a secret of the connection, so it is masked wherever an answer
   shows it, and no permalink can be built (the post's URL needs the handle or the DID the session kept). Smallest
   addition: `auth.<kept>` in result expressions, or a secret marked not-masked.
6. **Meta's webhook handshake** (a `GET` answered with `hub.challenge`) on `inbound`: Facebook, Instagram and Threads
   comments arrive only by webhook, so no inbox for them.
7. **Date arithmetic in templates** (`now | days_ago(90)`): Pinterest's analytics and Google's insights want a window;
   the descriptions read lifetime numbers or start at a fixed date instead.
8. **Seen while testing**: in a step's `inputs`, `items | selectattr('content_type', 'startingwith', 'image/')` kept
   every item, while the same filter in a body template filtered; Bluesky picks its media in the template.
9. Files sent in chunks or as one body stop at 25 MB (`FERRUM_CONNECTION_UPLOAD_MB`): every video limit in the rules is
   at most that, though the networks take far more.

## Also for the lead

* `tests/connectors_it.rs` lists the four old files (`the_examples_are_descriptions_the_platform_takes`), expects
  `meta.capabilities` in each, and drives the old Mastodon, Bluesky and LinkedIn shapes against its own fake (Mastodon
  now registers its client by `POST /api/v1/apps`; LinkedIn reads the member after sign-in). `packages/social/package.yaml`
  carries copies of `mastodon.json` and `bluesky.json` that the package test compares with the files. Both need
  bringing in line; `docs/connectors/README.md`'s table and "What they still cannot do" are superseded by this page.

## Per network

### X
Does: OAuth 2 with PKCE, client by basic authentication, the user read after sign-in (`after: me`, kept `user_id`);
text (weighted counting, links as 23); up to 4 pictures or one video by INIT/APPEND/FINALIZE, processing waited for,
alt text set; threads as reply chains; first comment; delete, repost, like; direct messages sent; numbers (public
metrics), followers, name and picture; mentions lookup; mentions pulled forward by `since_id`, a reply told as a comment
on the post it answers. Cannot: a lost post is not resent (X honours no idempotency key; reading back would compare
text X rewrites with t.co links); direct messages are not read (a DM pull would read the account's own messages too, and
a mapping cannot tell them apart); link clicks (`non_public_metrics`) not read; Premium's longer posts (the limit is
fixed at 280 in the rules). Unverified: the v2 media endpoints `/2/media/upload/initialize|{id}/append|{id}/finalize`,
`/2/media/metadata`, the `STATUS` query. Set up: a developer app with OAuth 2.0 on, type "Web App" (confidential),
callback `{PUBLIC_URL}/api/connections/callback`, a plan that allows posting and media.

### LinkedIn (member) and LinkedIn page
Does: sign-in (client in the body), the member read from `/v2/userinfo`; the page picked from the organisations the
member administers; text escaped as LinkedIn's little format wants (as Postiz does); pictures, PDF documents (title
from the settings) and video by initialise → put (a video in the parts LinkedIn names, finalised with their ETags) →
post, videos waited for (a page's pictures and documents too); several pictures as `multiImage`; comments as
follow-ups and first comment (a page's as the organisation); delete of posts and comments; a page's numbers
(impressions, reach, clicks, likes + comments + shares) and followers. A page's lost post is found among its recent
posts by its commentary. Cannot: a member's lost post fails (reading a member's posts needs `r_member_social`, which
LinkedIn grants to partners); mentions and hashtags are escaped to plain text (a template cannot leave
`@[name](urn:…)` unescaped while escaping the rest); member numbers (`r_member_postAnalytics`); comments are not read
(webhooks for approved partners); a member's token is not refreshed unless the app has programmatic refresh.
Unverified: `Linkedin-Version: 202610`; a comment's id in the answer body (`id`) else `x-restli-id`; deleting a comment
without `actor`; `organizationalEntityShareStatistics` with `shares=List(…)` for a Posts API URN. Set up: an app with
"Share on LinkedIn" and "Sign In with LinkedIn using OpenID Connect" (member), "Community Management API" (page, a
review), the redirect address, and the page's admin role for the signing member.

### Facebook Page
Does: sign-in, the short token exchanged for a long-lived one, and every page's own token kept **sealed with the
sign-in** (`keep: pages`), never in an answer (tested: it rests nowhere); a page's calls are made with its own token;
the long-lived token renewed by exchanging it again; the page picked; text with the link as its card; up to 10 photos
uploaded unpublished then attached; a video post (bytes to `graph-video`); reels and video stories by start → bytes to
`rupload` → wait → finish; photo stories; comments and first comment; delete, like; numbers (`post_media_view`,
`post_total_media_view_unique`, `post_clicks`, reactions + comments + shares) and followers. A lost feed post is found
among the page's latest by its message; a lost reel by its publishing status. Cannot: a lost story fails; a page added
after sign-in needs signing in again (its token is kept then); comments are not read (webhook handshake); first comment
and inbox answers need `account_id` from Social (above); videos over 25 MB. Unverified: Graph `v25.0`; the
`post_media_view` / `post_total_media_view_unique` metric names (Meta replaced impressions in 2025); `rupload` taking
`offset`/`file_size` headers with the bytes. Set up: a Meta app (Business type) with Facebook Login for Business, the
permissions `pages_show_list pages_manage_posts pages_read_engagement pages_manage_engagement read_insights
business_management` through App Review, the redirect address, the app live.

### Instagram (through a Facebook page) and Instagram (its own sign-in)
Does: the account picked from the pages' Instagram accounts (or the signed-in one, `/me`); a picture by address, a
video by its bytes to the upload host (`upload_type=resumable`); carousels (children made, a video child's bytes sent,
all waited for together with `?ids=`), reels (trial reels), stories; collaborators; published once, a lost answer found
among the account's media by its caption, then its permalink read; first comment and comments; numbers (`views`,
`reach`, `total_interactions`) and followers; long-lived tokens exchanged and renewed (`fb_exchange_token`;
`ig_exchange_token` and `ig_refresh_token`). Cannot: pictures need Social's public address (above); delete (the
Instagram API has none); a lost story fails (no caption to find it by); comments are not read (webhooks); location and
user tags not offered. Unverified: multi-id status reads (`/?ids=`) on graph.instagram.com; `views` as the impressions
metric; the upload host taking `Bearer`. Set up: a Meta app with "Instagram API with Facebook Login" (or "with
Instagram Login"), `instagram_content_publish` (resp. `instagram_business_content_publish`) and the rest through App
Review, the account a Business or Creator account (linked to a page for the first), the redirect address.

### Threads
Does: sign-in, long-lived token exchanged (`th_exchange_token`) and renewed (`th_refresh_token`); text with the link
as `link_attachment`; pictures and videos by address; carousels with children waited for together; reply chains;
first comment; who may reply; topic tag; published once (a lost answer found among the account's posts by its text);
permalink; repost; delete; numbers (views; likes + replies + reposts + quotes + shares) and followers; mentions pulled,
a mention answering a post as a comment on it. Cannot: any media without Social's public address; replies to the
account's posts other than mentions are not read (per-post `/replies`). Unverified: `?ids=` status reads on
graph.threads.net; deleting by `DELETE /{id}`; `topic_tag`. Set up: a Meta app with the Threads use case, the
`threads_*` permissions, the redirect address.

### YouTube
Does: Google sign-in with `access_type=offline`, the channel kept; the channel picked; a video's resumable upload
started with title, description, tags, category (choices from `videoCategories`), visibility and made-for-children,
sent 8 MiB at a time; a lost chunk settled by asking the session what it holds (a complete upload is never sent
again); thumbnail set; shorts (their URL); first comment (a new thread) and inbox answers (a reply to a comment);
comments pulled for the channel; numbers (views, likes + comments), subscribers, like, delete. Cannot: videos over
25 MB; impressions (YouTube Analytics, another API) — views stand in; region of the category list fixed to `US`.
Unverified: `allThreadsRelatedToChannelId` still served. Set up: a Google Cloud project with the YouTube Data API v3,
an OAuth client (web), the consent screen (verification for `youtube.upload`/`youtube.force-ssl`), the redirect
address; thumbnails need a verified channel.

### TikTok
Does (once it can sign in): privacy options from the creator's own as choices; comment/duet/stitch and branded
settings; a video initialised with its chunk size and count and sent in chunks (the remainder in the last), its
publishing waited for, the public post's id kept; photo carousels by address; numbers (views; likes + comments +
shares) and followers. Cannot: sign in (platform, item 1); a lost chunk stops the call (TikTok publishes an upload once
whole and offers no way to ask what it holds, so restarting could post twice); delete and comments (no API);
photos without Social's address and a verified domain; videos over 25 MB. Unverified: `publicaly_available_post_id`
being the id `video/query` takes. Set up: a TikTok developer app with Login Kit and Content Posting API (Direct Post
needs an audit; unaudited apps post privately), the redirect address, `user.info.stats` and `video.list` scopes.

### Pinterest
Does: sign-in with basic client authentication and refresh; boards as choices; a pin of one picture or several
(carousel) by address, or of a video registered, sent to the storage address with the fields the answer names, and
waited for; title, link, alt text, colour; a lost pin found on its board by its description; numbers (impressions,
pin + outbound clicks, saves + reactions + comments), followers, delete. Cannot: pictures without Social's address;
a video's own cover (key frame 0 is used); comments (no API). Unverified: `pin_metrics.lifetime_metrics` field names.
Set up: a Pinterest app with Standard access (trial access posts only to sandbox), the redirect address.

### Reddit
Does: a permanent sign-in, the user kept; subreddits searched and their flairs as choices; text, link and picture posts
(an upload lease, the file to storage with the lease's fields) and galleries; NSFW, spoiler; a lost post found among
the account's submissions by its title; first comment and comments; the inbox (replies as comments, mentions,
private messages); private messages sent; votes; numbers (score + comments), delete; one request at a time. Cannot:
Reddit's refusals are not read in its words (platform, item 2); video posts (Reddit wants a poster picture and a
websocket for processing). Unverified: `/api/subreddit_autocomplete_v2`; `submit_gallery_post.json`'s shape. Set up: a
"web app" at reddit.com/prefs/apps, the redirect address, and a User-Agent naming the app's Reddit user (replace the
placeholder in the file's headers).

### Mastodon (any server)
Does: the platform's client registered at the server by `POST /api/v1/apps` (once), a token that never expires, the
account's id kept after sign-in; visibility, content warning, language, polls; up to 4 media with descriptions,
waited for while processing; threads; first comment; comments told from mentions again (a pull asks for the account
with each page, and a reply to the account's own status is a comment); direct messages answered directly; favourite,
boost, delete; numbers and followers; mentions to the composer. A lost answer is sent again under the same
`Idempotency-Key`, which Mastodon honours. Cannot: the account's own native posts read back (Social, item 4); the
server's own character limit (500 is assumed). Unverified: none beyond the docs read for 0.7.0. Set up: nothing at the
server (registered on first sign-in); the connection's address is the server's (four places in the file).

### Bluesky
Does: a session from the handle and an app password, refreshed; the record key made from the call's key (`key | tid`),
so a lost answer is found where the record would be and never made twice; links, tags and mentions as facets at their
byte offsets (mentions resolved to DIDs; an unknown handle stays text); link cards; up to 4 pictures with descriptions
and shapes, or a video; threads and the first comment with the root's and the parent's references; likes, reposts;
direct messages through the chat service; languages; notifications pulled as before; numbers and followers. Cannot:
a permalink (platform, item 5); link cards carry no title, description or picture (fetching the page is not a
network call). Unverified: a video by `uploadBlob` and `app.bsky.embed.video` without the video service. Set up: an app
password (with direct messages allowed, for DMs).

### Google Business Profile
Does: Google sign-in with refresh; the location picked from the first account's; updates, events and offers with
their button, dates, coupon, redeem address and terms; one picture by address; a lost post found among the
location's by its words; numbers (search views, button clicks), the location's name, delete. Cannot: pictures without
Social's address; locations of a second account (the picker reads the first); no comments (posts have none).
Unverified: `localPosts:reportInsights` still served, and from a fixed start date. Set up: a Google Cloud project with
the Business Profile APIs (access by application to Google), an OAuth client, the redirect address.
