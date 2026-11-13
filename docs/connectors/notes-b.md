# Social networks, second batch: what each description does and cannot

Written as connection descriptions (JSON, no Rust) and proved against fakes of each network's documented API
(`tests/networks_b_it.rs`). **None of it has run against a real network**: the first connection to each is the first
time its real answers are read. Facts come from each network's public documentation and, where marked, the Postiz
provider code (AGPL, read only).

## Needed from Social

(see the end of this file for the full list; filled in as the networks were written)

## Needed from the platform

(same)

## How these descriptions work around two things in the contract

* **A medium a network takes inside the post call** (Telegram, Discord, Tumblr, Dribbble ...). Social calls
  `publish_media` first and hands `publish` only the returned ids. These descriptions make `publish_media` a read-only
  step (a cheap `getMe`-style call that also proves the token) whose result `id` is `inputs.file`, the stored file's id.
  `publish` then receives the stored file ids in `media` and `media_items[].id` and sends each as a multipart part.
  It works, costs one extra read per medium, and would go with a `file` field in `media_items`.
* **`delete` is given only `id`, `item`, `follow_up` and `root`**, no account id. Networks whose delete needs a chat or
  channel (Telegram, Discord, Slack) return an id of the form `<chat>:<message>` from `publish`.
* An operation made only of `steps` has no method, so the platform treats it as one that changes something: a reading
  one (`account`, `metrics`, `choices`) says `"changes": false`.
* Every writing call whose network has no idempotency header is a one-step operation (`steps`): a lost answer then fails
  the call ("may have been sent") instead of sending twice.

## Telegram (`telegram.json`)

**Does**: bot token as the connection's secret (in the URL path, as Telegram wants it); the chats the bot was added to
are what the sign-in reaches (`getUpdates`, `my_chat_member`), the chat id is the account's id on the network; two
kinds: `message` (HTML, 4096) and `media` (caption 1024, 1 to 10 photos, videos or files, one kind in a post); text,
photos, videos and documents (each medium its own message, the words as the first one's caption); follow-ups as replies
(`reply_parameters`), text only; delete (`deleteMessages`, every message of the item in one call); the account's
members (`getChatMemberCount`) and name.
**Cannot**: an album (`sendMediaGroup` needs every file as a part of one request; `form` cannot take a list of files:
platform, a `form` part repeated per item of a list); numbers for a post (the Bot API gives a bot no views or
reactions); mentions; files above 25 MB (Telegram takes 50 MB, the platform's part limit is 25 MB); a lost answer on a
send fails the call (no idempotency header and no way to read a message back).
**Unverified**: the token's `:` is sent as it is (the `url` form of an operation prints it unencoded; a path would
percent-encode it); an empty `caption` on the second and later pictures; that 429 carries a `Retry-After` header (the
API gives `parameters.retry_after` in the body, which the platform does not read, so the call is "later" with the
platform's own backoff); `characters` as the counting unit (the docs say "characters after entities parsing").
**Setup**: create the bot with @BotFather, put its token under the connection's `bot_token`, add the bot to the
group or channel as an administrator with the right to post, send one message there (a channel: make a post) so
that `getUpdates` shows it, then pick the chat in Social. Do not set a webhook on the bot (`getUpdates` stops working).

## Discord (`discord.json`)

**Does**: a bot token (`Authorization: Bot`); the servers the bot is in are what the sign-in reaches (server icon as the
picture), the server id is the account's id on the network, the channel is a required setting whose choices are the
server's text and announcement channels (categories and voice channels filtered out by the description); markdown text
up to 2000 characters; up to 10 files in one message (a `payload_json` part and `files[n]`, one operation for each count
1 to 10, because a `form` has a fixed set of parts), each with its description; follow-ups as a thread made from the
first message (made once, every later item posted in it); a first comment as a reply in the channel; delete by channel
and message; mentions (`<@id>`, member search); the server's name and member count.
**Never twice**: Discord has no idempotency header but takes a `nonce` with `enforce_nonce`: the description sends the
call's key as the nonce (`{{ key | tid }}`, 13 characters) so a second send is answered with the first message. The
operations name an `idempotency_header` (`X-Idempotency-Key`, ignored by Discord) only so that the platform may send
again after a lost answer; the protection is the nonce. A thread made twice is refused by Discord (160004); its step
reads the thread back (`confirm`) instead of sending again.
**Cannot**: numbers for a post (reactions cannot be summed in the platform's expression language); embeds, stickers,
polls, forum or voice channels, a thread name of its own (always "Replies"); files above 10 MiB (see unverified); the
message's permalink (the link needs the server id, which `publish` is not given: Social would have to pass `account_id`
into `url`, the description does not map one).
**Unverified**: the 10 MiB file limit (Discord's limit for bots has changed more than once, 8 / 25 MiB before); that a
new upload may be described in `attachments` by `id` and `description` without `filename`; that the member search needs
the privileged Server Members intent (it does in the docs I know); the `User-Agent` form.
**Setup**: a Discord application with a bot (discord.com/developers), the bot token under `bot_token`, the bot invited
with View Channel, Send Messages, Attach Files, Create Public Threads, Send Messages in Threads (and Manage Messages to
delete other people's), the Server Members intent for mentions.

## Slack (`slack.json`)

**Does**: OAuth v2 (the workspace's Slack app, bot token; the workspace is the account, picked from `auth.test`); the
channel is a required setting with choices from `conversations.list`; two kinds: `message` (4000 characters) and `images`
(3000, a section block and one image block for each of up to 10 pictures); pictures through Slack's external upload
flow (`files.getUploadURLExternal`, the bytes to the signed address with no credentials, `files.completeUploadExternal`);
follow-ups as replies in the first message's thread (`thread_ts`); delete (`chat.delete`, the id is `<channel>:<ts>`);
the message's permalink (`chat.getPermalink`); mentions (`<@id>`, from `users.list`); the workspace's name.
**Never twice**: Slack has no idempotency header. Each post carries the call's key in the message's `metadata`; if the
answer is lost the call reads the channel's recent messages (`conversations.history` with `include_all_metadata`) and
takes the message with that key as the answer. If it cannot prove it (the bot is not a member of the channel, so it cannot
read it) the call fails and is not sent again. The upload's completion is read back (`files.info`) the same way.
**Cannot**: say why Slack refused in Slack's words. Slack answers 200 with `{"ok": false, "error": "channel_not_found"}`
and the platform reads an `error_at` only on a failing status; the description makes the missing id fail the call
(`required`), so the person sees "the answer has no id". *Needed from the platform*: a way to say that a 2xx answer is a
refusal (an expression over the answer, e.g. `fail_if: "not answer.ok"`) and read `error_at` from it. Also: numbers
(Slack gives none for a bot's post); mrkdwn is shown as plain text in the composer (no `mrkdwn` text format in
`meta.social`); posts to a channel the bot is not in need `chat:write.public` and cannot be read back; user-scope
tokens; token rotation (`never_expires` is set; a workspace that turns rotation on must connect again).
**Unverified**: that the platform's space-separated `scope` is taken by Slack (its docs say comma-separated; the
platform's `params` may not set `scope`); that an image block's `slack_file` may name a file completed without a
`channel_id` (Slack's docs say the file must be visible to the bot; Postiz shares the file into the channel); that
`chat.getPermalink` needs no scope beyond `chat:write`; the `Retry-After` of a 429 (Slack sends it).
**Setup**: a Slack app (api.slack.com/apps) with the redirect `{PUBLIC_URL}/api/connections/callback`, the client id in
the description and the client secret under `client_secret`; the bot scopes the description lists; add the bot to
the channels it posts to (or leave `chat:write.public`, losing the read-back there).

## WordPress (`wordpress.json`)

**Does**: the site's REST API with an application password (basic auth; the user name is in the description, the
password under `application_password`, the site address is the placeholder in `base`, `accounts` and `account`: four
places); the site is the account; html text; settings: title (required), kind of content (`/types`, so pages and custom
types with `show_in_rest`), status (publish, draft, pending, private), categories and tags (choices, searchable), address
name; a cover picture (upload, then its description, then `featured_media`); the post's permalink; delete to the trash
(no `force`), the id is `<rest_base>:<id>`.
**Never twice**: WordPress has no idempotency header. The post step reads back after a lost answer: posts of that kind
with that exact title (`context=edit`, `status=any`) created after the publication's `created_at`. A medium's upload is a
one-step operation: a lost answer fails the call rather than uploading twice.
**Cannot**: new tags or categories (only existing ones are chosen); inline pictures (only the cover; Social has no
`html` of a medium); scheduled posts (`future` needs a date setting: add when Social passes the post's time); the
title as a first-class input (it is a setting); excerpt, comment status, sticky, formats; a different author;
follow-ups (none); numbers (WordPress core has none). The read-back would match a different post with the same
title made in the same moments by someone else.
**Unverified**: that `after` accepts the RFC 3339 `created_at` with a `Z` (WordPress compares in the site's time zone);
the `Content-Disposition` form with a quoted file name for names with quotes; no rate limit is described (hosts differ).
**Setup**: a user with the right to publish, an application password (Users, Profile), HTTPS and the REST API
reachable at `/wp-json` (pretty permalinks or `?rest_route=` not supported here); put the site address where the
placeholder is (four places) and the user name in `auth.username`.

## Medium (`medium.json`)

**Does**: an integration token (bearer); the token's user is the account (`/me`); a story in markdown with title (100),
tags (comma-separated text, trimmed, empty ones dropped), original address, visibility (public, draft, unlisted), licence,
"tell followers", and a publication (choices from `/users/{id}/publications`; the story then goes to
`/publications/{id}/posts`); a picture: uploaded to `/images` and written at the top of the story as `![alt](address)`;
the story's address.
**Cannot**: delete or edit (the API has neither: `delete_remote` says the connection does not offer it); numbers (none);
more than three tags (Medium ignores the rest; the setting does not count them); several pictures (one is the lead; more
would be inline, which needs Social to place media in the text); follow-ups (none); a lost answer on the story fails the
call ("may have been sent"): Medium offers no list of one's stories to read back, and no idempotency header.
**Unverified**: all of it against the live service: Medium stopped issuing integration tokens to new accounts in 2023
(existing tokens still work); the picture types; the 100-character title limit; `reject("equalto", "")` in the tags
template is minijinja's, checked only by the fake.
**Setup**: an integration token from Medium's settings (only an account that already has one), under
`integration_token`.

## DEV Community (`devto.json`)

**Does**: an API key (`api-key` header, the v1 `Accept` type); the key's user is the account; a markdown article with
title, tags (up to four, choices from `/tags`), cover picture by address, original address, summary, series, an
organisation (choices) and a draft switch; the article's address; "delete" unpublishes (`PUT /articles/{id}` with
`published: false`; the API has no delete).
**Never twice**: no idempotency header. After a lost answer the call reads the author's own articles (`/articles/me/all`)
for one with that title published since the publication's `created_at` (compared to the second); a draft cannot be
found that way, so a lost answer on a draft fails the call instead of being resent.
**Cannot**: upload a cover (the API has no upload: the cover is an address typed as a setting. *Needed from Social*: a
public address for a stored file, e.g. `media_items[].url`, to fill `main_image` from the composer's picture; and a
title as a first-class input); list all organisations (Forem has no call for a user's own: the choices are the
organisations of the articles already written, as Postiz does); edit; numbers (page views and reactions are in
`/articles/me/all`; they are not mapped here: `metrics` would need the article's own read); follow-ups (none).
**Unverified**: rate limits (the pages I could recall name none that I trust, so none is set); that `PUT /articles/{id}`
unpublishes with an API key for the author's own article; `published_timestamp` as the field to read back by; the
128-character title limit; a tag outside the list is refused (the choices only offer the list).
**Setup**: an API key (Settings, Extensions) under `api_key`.

## Hashnode (`hashnode.json`)

**Does**: GraphQL at `gql.hashnode.com` with a personal access token (the `Authorization` header, no prefix); the token's
user is the account; a markdown article with publication (required, choices from `me.publications`), title, subtitle,
tags (comma-separated names, sent as `{slug, name}`), cover picture by address, original address, address name;
the article's address; delete (`removePost`).
**Never twice**: no idempotency header. After a lost answer the call reads the publication's ten latest posts for the
same title published since the publication's `created_at` and takes that post as the answer; if it is not there the call
fails instead of sending again.
**Cannot**: say why Hashnode refused in its words. GraphQL answers 200 with `errors`; the platform reads `error_at` only
on a failing status, so the description makes the missing id fail the call (`required`: "the answer has no id"). *Needed
from the platform*: an expression that marks a 2xx answer as a refusal and a pointer for its text (the same need as
Slack's `ok: false`). Upload a cover (an address only; *needed from Social*: a public address for a stored file); choose
tags from Hashnode's own list (a text setting; the tag's slug is the lower-cased name, so a tag Hashnode knows under
another slug is created or refused); series, scheduling, newsletter switch, co-authors, drafts; numbers; follow-ups.
`check` works only because `account` is a one-step operation without a method: Social reads an account's name through
an operation that is a GET, and a GraphQL query is a POST (*needed from Social*: ask the operation's `changes`, not its
method, whether it reads).
**Unverified**: all of it against the live API (the mutation and field names are from Hashnode's public schema as I know
it; Postiz still calls the old `gql-beta` address); the mutation limit of 500 a minute that `limit` sets; that unknown
tag slugs are accepted; the 250-character title limit.
**Setup**: a personal access token (hashnode.com/settings/developer) under `access_token`.

## Tumblr (`tumblr.json`)

**Does**: OAuth 2 (the workspace's Tumblr app; scopes `basic write offline_access`, refreshed by the platform); the
blogs the sign-in reaches are the accounts to pick (the blog's name is its id); posts in NPF: the text split into a
text block for each paragraph (blank line), tags (text, commas) and a state (published, draft, queue, private); up
to 10 pictures and videos in one post (one operation for each count, the post as the `json` part and each file as a part
the blocks name by `identifier`), a picture's description as `alt_text`; the post's address; delete
(`post/delete`; the id is `<blog>:<post>`); the post's notes (`note_count`) as engagements; the blog's name and followers;
the documented limit of 1000 requests an hour.
**Never twice**: no idempotency header. After a lost answer the call reads the blog's ten latest posts (`npf=true`)
for one made since the publication began whose first text block is this post's first paragraph; a post without
text cannot be found that way, so a lost answer there fails the call.
**Cannot**: formatting (bold, links inside the text: NPF takes them as `formatting` ranges with offsets, which a template
cannot count; the text is plain and a link is written after it); reblogs, polls, audio, links as link blocks; the 250 posts
a day limit is Tumblr's, not counted here; a video above 25 MB (the platform's part limit; Tumblr takes 100 MB);
follow-ups (none); mentions; the account's picture is Tumblr's avatar address.
**Unverified**: that the `json` part needs no content type of its own; that the token address takes the client secret in
the body; the error text pointer (`/errors/0/detail`; an answer without it shows the raw body); `timestamp()` of the
publication's moment against a post's epoch `timestamp`.
**Setup**: a Tumblr app (tumblr.com/oauth/apps) with the redirect `{PUBLIC_URL}/api/connections/callback`; the consumer
key as `client_id` in the description and the consumer secret under `client_secret`.

## Lemmy (`lemmy.json`)

**Does**: a session from the account's user name and password (secrets `username` and `password`; `/user/login`, a jwt
as a bearer, signed in again by the platform when it is refused); the instance address is the placeholder in `base` and
`publish_media` (two places); the user is the account; the community is a required setting (the month's top communities
as choices, or a search when something is typed); a post with title, markdown body, not-safe-for-work switch, a link
(`url`), a picture uploaded to the instance's pictrs and used as the link's thumbnail (`custom_thumbnail`), or as the
post itself when there is no link, the picture's description (`alt_text`); follow-ups and the first comment as comments
(`/comment`); delete of a post or a comment (`/post/delete`, `/comment/delete`, the comment first); numbers (upvotes,
downvotes and comments of the post); the user's name and picture.
**Never twice**: no idempotency header. After a lost answer the call reads the community's ten newest posts (or the post's
newest comments) for one with that title (content) published since the publication began and takes it as the answer.
A lost answer on the picture's upload fails the call instead of uploading twice.
**Cannot**: say more than Lemmy's error code (`language_not_allowed`, `rate_limit_error`: the API gives codes, not
sentences); a language, a nested comment; edit; the instance's rate limits (set per instance: none is described);
mentions of users or communities; followers (a Lemmy user has none in the API); a v4 (Lemmy 1.0) instance (the paths
are `/api/v3`; edit the base).
**Unverified**: that `custom_thumbnail` and `alt_text` exist on the instance's version (0.19 and later); that pictrs takes
the bearer header (0.19 does; older versions read the cookie); `search` with a typed query; the 50000-character body.
**Setup**: an account on the instance (a bot account is polite), its user name and password under the connection's
secrets, the instance address in place of the placeholder (twice).

## VK (`vk.json`)

**Does**: VK ID OAuth 2 with PKCE (`id.vk.ru`, a public client: no client secret; scopes `vkid.personal_info wall photos
video`; `keep: ["device_id"]`, a `refresh_vk` operation that sends the kept device id); the user is the account; wall
posts with up to 10 attachments, a picture through `photos.getWallUploadServer`, the upload and `photos.saveWallPhoto`,
a clip through `video.save` and its upload; the attachments named `photo<owner>_<id>`, `video<owner>_<id>`; a
friends-only switch; follow-ups and the first comment as comments (`wall.createComment`); delete of a post or a comment
(the comment first); views as impressions, likes + reposts + comments as engagements (`wall.getById`); the user's
name, picture and followers; the documented 3 requests a second.
**Never twice**: no idempotency header. After a lost answer the call reads the wall's ten newest posts (`wall.get`) for
one with those words dated since the publication began. A lost answer on a comment or an upload step fails the call.
**Cannot, today**: **connect.** VK ID wants the callback's `device_id` (and `state`) in the code exchange, and in every
refresh; the platform's exchange sends neither and its `refresh` operation is not given `auth.*`. The description
is written for what the brief said (the device id in the token answer, `keep`) but I believe VK gives it only in the
redirect's query. *Needed from the platform*: (1) extra parameters of the redirect (`device_id`, `ext_id`) added to the
token request and kept with the tokens (e.g. `provider.callback_params`); (2) the kept fields (`auth.*`) available to the
`refresh` and `after` operations (they are `bare` now, so `{{ auth.device_id }}` is undefined and the refresh fails
with "a template failed"). The fake in the test takes the code without a device id and gives tokens that outlast the test, so the
exchange with a device id and the refresh are **not proved**. Also: say why VK refused in VK's words (VK answers 200 with `error`; the
description makes the missing id fail the call: *needed from the platform*, a refusal marker for a 2xx answer, as for Slack);
`publish_media` is given `kind` as the post's kind, not the medium's (the description reads `content_type`; *needed from
Social*: a separate name for the medium's kind, e.g. `media_kind`); posting as a community (a community's own token and
`owner_id` `-<group>`: a second account kind); stories, polls, albums; a post's address on a custom domain.
**Unverified**: the hosts (VK moved from vk.com to vk.ru; the upload servers' hosts vary and each must be in
`allowed_hosts`: the list has the ones I know); `v=5.251` (Postiz's); the bearer header on API methods (VK documents it;
Postiz puts the token in the query); the 16384-character message limit; that scopes are passed space-separated.
**Setup**: a VK ID application (id.vk.ru/business) with the redirect `{PUBLIC_URL}/api/connections/callback`; its id as
`client_id` in the description (twice: `provider` and `refresh_vk`).

## Twitch (`twitch.json`)

**Does**: OAuth 2 (the workspace's Twitch app; `Client-Id` header beside the bearer, so the client id appears twice);
the signed-in user's own channel is the account; two kinds: a chat message (500 characters, `/chat/messages`) and an
announcement (`/chat/announcements`, a colour setting); follow-ups as replies to the message before
(`reply_parent_message_id`); delete of a chat message (`/moderation/chat`, the answer first); followers of the channel as
the account's number; the documented 20 messages in 30 seconds.
**Never twice**: Twitch has no idempotency header and no call that reads a chat back, so a lost answer on a message or an
announcement fails the call ("may have been sent") and is not sent again.
**Cannot**: say why Twitch dropped a message in its words: it answers 200 with `is_sent: false` and a `drop_reason`; the
description makes the missing id fail the call (*needed from the platform*: a refusal marker for a 2xx answer, as for Slack
and VK); give an announcement an id (Twitch returns none: the id is `<channel>:announcement-<moment>` and deleting it is
refused by Twitch); post in another channel (the account's own only; chatting elsewhere would need the channel as a
setting and the moderator rights it implies); numbers for a message (none exist); media; mentions (typed `@name`).
**Unverified**: the scope list (`moderator:manage:chat_messages` for deleting, `moderator:read:followers` for the count);
the 20 messages in 30 seconds applies to an ordinary chatter (a moderator's is 100); `account_at: /user_id` (Twitch's
token answer has no user; the account then shows without a name until `check`); a 204 answer for an announcement.
**Setup**: a Twitch application (dev.twitch.tv/console) with the redirect `{PUBLIC_URL}/api/connections/callback`, a
confidential client; its id twice in the description, its secret under `client_secret`; connect as the broadcaster.

## Kick (`kick.json`)

**Does**: OAuth 2.1 with PKCE (`id.kick.com`; Kick's app is confidential: the client secret goes in the body too); the
signed-in user's channel is the account (`/users`, the user id is the broadcaster id); chat messages up to 500 characters
(`POST /chat`, `type: user`); follow-ups as replies (`reply_to_message_id`); delete (`DELETE /chat/{id}`).
**Never twice**: no idempotency header and no call that reads a chat back: a lost answer fails the call and is not sent again.
**Cannot**: say why Kick dropped a message (a 200 with `is_sent: false`: the call fails with "the answer has no id";
*needed from the platform*: a refusal marker for a 2xx answer); followers or numbers (I could not confirm a call for
them, so none is described); chat in another channel; announcements; media; rate limits (none published that I
could confirm, so no `limit`).
**Unverified**: all of it against the live API, including the scope names (`user:read channel:read chat:write
moderation:chat_message:manage`), that `type: user` with the broadcaster's own id sends as the signed-in user, and that
the token address takes the client secret in the body.
**Setup**: an app in Kick's developer settings (needs two-factor sign-in) with the redirect
`{PUBLIC_URL}/api/connections/callback`; its id in the description, its secret under `client_secret`.

## Dribbble (`dribbble.json`)

**Does**: OAuth 2 (`public upload`; Dribbble's tokens do not expire, `never_expires`); the user is the account; a shot
with exactly one picture (png, jpeg, gif; at least 400x300; 8 MB), a title (required), an html description and the team to
post as (choices from `/user`); the shot's id from the `Location` header of the 202 answer; delete; views as impressions,
likes + comments as engagements; followers; the documented 60 requests a minute.
**Never twice**: no idempotency header. After a lost answer the call reads the user's own shots for one with that title
published since the publication began; Dribbble makes a shot asynchronously (202), so one not yet listed is not found and
the call fails rather than sending again.
**Cannot**: tags, scheduling (`scheduled_for`), low profile, rebounds; follow-ups (the API has no comments); the picture's
description; a GIF's smaller limit; the media as a first-class thing for a design: the picture is sent as a part from the
stored file (the `publish_media` step is a read of the account that returns the file id, as for Telegram).
**Unverified**: `DELETE /shots/{id}` and `GET /shots/{id}` counts (`views_count`, `likes_count`, `comments_count`) and
`team_id` on create; the size limits; `published_at` on `/user/shots`; that `headers.location` is lower-cased in the
expression. The 202's `Location` is read as the platform stores it.
**Setup**: an application at dribbble.com/account/applications/new with the callback
`{PUBLIC_URL}/api/connections/callback`; its id in the description, its secret under `client_secret`.

## Whop (`whop.json`)

**Does**: OAuth 2 with PKCE (a public client: no secret); the user is the account (`/oauth/userinfo`); company and forum are
required settings (the forum's choices depend on the company picked); a forum post in markdown with an optional title and
up to 10 files: a file record, the bytes `PUT` to the address the record gives (no credentials), then the file's status
read every 2 seconds until `ready` (a `wait` step, failing on `failed`), and the post names the files; follow-ups and the
first comment as replies (`parent_id`); delete; the post's address.
**Never twice**: no idempotency header. After a lost answer the call reads the forum's ten newest posts for one with those
words created since the publication began.
**Cannot**: numbers (none known); mentions; edit; the file's own `upload_headers` (the record may name headers the `PUT`
must carry; header names cannot come from an answer: *needed from the platform*, headers from a list; Postiz sends them);
a file above 25 MB; follow-ups with files.
**Unverified**: nearly everything, from Postiz's code and my memory of Whop's v1 API: the token address taking a form
(Postiz sends JSON); `error.message` as the error text; `created_at` and `content` on a forum post; `DELETE
/forum_posts/{id}`; the scopes; that the `PUT` needs nothing but the signed address.
**Setup**: a Whop app (whop.com/dashboard/developer) with the redirect `{PUBLIC_URL}/api/connections/callback`;
its client id in the description; connect as a member of the company who may post in the forum.

## MeWe (`mewe.json`)

**Does** (from Postiz's code only: MeWe publishes no open documentation of its Dev API): the user's `apiToken` as a
bearer and the app's id and key as `X-App-Id` and `X-Api-Key` (all three connection secrets); the user is the account;
two kinds, a post on the user's timeline and a post in a group (the group a required setting, choices from `/groups`);
plain text up to 63206 characters; up to 10 photos (`/photo/upload`, then `uploadedPhotoIds`).
**Cannot**: **sign in.** MeWe's sign-in is its own login-request flow: the browser goes to `mewe.com/login`, comes back
with a `loginRequestToken`, which is exchanged by `GET /api/dev/token` carrying the app's headers for an `apiToken` that
lasts about 30 days. The platform's `oauth` exchange is a POST of a code to a token address; it cannot make that call, so
the `apiToken` has to be obtained elsewhere and put under `api_token`, and it must be renewed by hand. *Needed from the
platform*: a sign-in whose exchange is an operation of the description (a GET with the callback's `loginRequestToken` and
headers from secrets), keeping its `apiToken` and expiry. Also: an id (posts answer 204, so the id is `mewe-<moment>`:
no delete, no numbers, no comments; the post's permalink); video (Postiz filters it out); a lost answer fails the call
(no key, nothing to read back).
**Unverified**: everything: MeWe's host, paths, field names and the `Forbidden` / `Unauthorized` / 420 texts come from Postiz
and have not been checked against MeWe's documentation, which I could not read.
**Setup**: a MeWe developer app (by request to MeWe), its id and key, and an `apiToken` for the user (via MeWe's login flow).

## Listmonk (`listmonk.json`)

**Does**: basic auth (the API user's name in the description, its token under `password`; the instance address is the
placeholder in `base` and `publish`'s address: two places); the instance is the account (its site name); an e-mail campaign
with subject (required), list (required, choices) and template (choices, "Default" first), the html as the body; created as a
draft and then started (`PUT /campaigns/{id}/status` to `running`); the campaign's address in the admin; views as impressions,
clicks and sent as reach; delete.
**Never twice**: no idempotency header, and an e-mail sent twice is the worst case here, so both writing steps read back.
A lost answer on the creation: the campaigns are searched by subject for one created since the publication began and used
as the answer. A lost answer on the start: the campaign is read and, if it is running, finished or scheduled, the start
is not sent again. If neither is proved the call fails.
**Cannot**: schedule at Listmonk (`send_at`; Social sends at the time itself), choose several lists, a from address, a
plain-text alternative, a preview text (the hidden preheader Postiz adds is not reproduced), upload pictures (images are
addresses in the html); follow-ups.
**Unverified**: all of it against a live instance; `created_at` and `subject` in the campaign search; `per_page=all`;
that a draft with the same subject is not found as a duplicate by a person re-sending the same subject (it would be, within
the publication's own window only).
**Setup**: an API user (Settings, Users) with campaign rights, its name in `auth.username` and its token under `password`;
the instance's address in place of the placeholder (twice).
