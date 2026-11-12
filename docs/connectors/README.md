# Example connection descriptions

Social names no network. A network is a connection's description (Administration, Connections) whose operations are
named by what Social needs of them, plus a network profile record (Social, Network Profiles). Copy one of these,
replace what is marked `REPLACE`, register the app's client at the provider with the redirect address
`{PUBLIC_URL}/api/connections/callback`, put the client secret in the connection's secret `client_secret`, and
connect the account from its page in Social.

Operations Social calls: `publish` (inputs `text`, `link`, `text_with_link`, `media`, `media_id`, `alt`; result `id`,
`url`), `publish_media` (`name`, `content_type`, `alt`, `size`, `data` as base64; result `id`), `delete` (`id`), `reply`
(`text`, `in_reply_to`, `author`, `thread`; result `id`), `send_message` (`to`, `text`, `in_reply_to`, `thread`; result
`id`), `metrics` (`id`; result `impressions`, `reach`, `engagements`, `clicks`, or `likes`, `reposts`, `replies`,
`quotes`, `saves` that add up to engagements) and `account` (result `followers` and the same numbers for the day).
`comments` and `messages` are pulled by the description's own `pulls`, or arrive on its `inbound`, mapped onto
`SocialMessage` found by `external_id`; a field the network may leave out is read as
`has(item.x) ? item.x : null`. A description lists `"apps": ["social"]`.

Written from the networks' public documentation, pages read on 12 and 13 November 2026:
- Mastodon (docs.joinmastodon.org): methods/statuses, methods/notifications, methods/accounts, methods/media,
  methods/apps, methods/oauth, api/oauth-scopes, api/rate-limits.
- X (docs.x.com): x-api/posts/creation-of-a-post, delete-post, post-lookup-by-post-id, users/get-mentions,
  users/get-user-by-username, media/upload-media, direct-messages (get-dm-events, create-dm-message-by-participant-id),
  oauth-2-0 authorization-code and user-access-token, fundamentals/counting-characters.
- LinkedIn (learn.microsoft.com, Posts, Images, Comments, share and follower statistics, authorization code flow) was
  read and **no example is shipped**: a new post's id is in the `x-restli-id` response header, which a description
  cannot read, and an image is a binary PUT to an address the first call returns.
- Bluesky: docs.bsky.app returned nothing; atproto.com/specs/oauth and the lexicon JSON were read. **No example is
  shipped**: its OAuth needs DPoP and pushed authorization, and an app password gives a session token that expires and
  is not one of the authentications a connection has.

What these two cannot do: Mastodon takes media as multipart (no `publish_media`), registers each app per server, and
gives paging in a `Link` header (the pull pages by the newest id instead); X gives its errors in two shapes (`/detail`
is mapped) and no idempotency header is documented, so only the platform's own key keeps a post from going twice.
The handle, user id and the address of a Mastodon server are placeholders to replace.
