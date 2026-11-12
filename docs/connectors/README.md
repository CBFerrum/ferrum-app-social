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

* `publish` (inputs `text`, `link`, `text_with_link`, `media`, `media_id`, `alt`, `created_at`, `picture_type`,
  `picture_size`; result `id`, `url`). `created_at` is the publication's own moment, the same on every attempt.
* `publish_media` (inputs `file`, the stored file's id, for a part or a body; `name`, `content_type`, `alt`, `size`; and
  `data`, base64, only when the description lists it among its `inputs`; result `id`), called first when a post has a
  picture, under a key of its own.
* `delete` (`id`), `reply` (`text`, `in_reply_to`, `author`, `thread`, `created_at`; result `id`), `send_message` (`to`,
  `author`, `text`, `in_reply_to`, `thread`; result `id`).
* `metrics` (`id`) and `account`: results named `impressions`, `reach`, `engagements`, `clicks`, `followers`. A network
  that counts likes, boosts and replies apart adds them in the description: `{"expr": "answer.favourites_count +
  answer.reblogs_count"}` under `engagements`.
* `messages` is pulled by the description's `pulls`, or its `inbound` takes them, mapped onto `SocialMessage` found by
  `external_id`, with `"outside": true` (what is read is not sent back out). A field the network may leave out is a
  pointer (`/status/in_reply_to_id`, null when it is not there). `via_connection: "connection.id"` lets Social find the
  account by the connection when the item names neither a handle nor a post (a person's own connection is read as her, so
  the account she connected). `thread` is what the answer needs from the item: Mastodon's keeps the status's openness (so
  an answer is never more open), Bluesky's the thread's root and the comment's own reference.
* `meta.capabilities`: `max_text`, `link_length`, `images`, `videos`, `media_required`, `max_media`, `max_image_bytes`,
  `alt_text`, `first_comment` (as a network profile's fields), `name`, and `html_text` for a server that sends its words
  as HTML. It says what the network takes for this connection (a server's own limit among them) and is read before the
  account's network profile; the seeded profiles are the defaults for an account whose connection says nothing.

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
* **All**: a pick of a pull cannot read the clock, so the numbers a day are Social's; Retry-After is honoured for pulls
  and for the platform's own queued calls but an app's own call (Social's publish job) sees only "later" and is asked
  again by the jobs' backoff.
