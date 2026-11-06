# Changelog

## 0.5.0

What posts did, day by day (FERRUM-ECOSYSTEM-005).

- **Daily Insights:** whenever reading a publication back changes its impressions, engagements or
  clicks, the day's row (by the workspace's day) takes the new numbers. That gives one row a day
  per publication, with its account and post, and a history networks do not keep.
- **Social Engagement by Day:** a report of what each day added, for everything, one brand or one
  account. A day's numbers count less the publication's day before, even when that day falls
  before the span, and every day of the span (30 days by default, a year at most) is listed with
  the publications read that day. The home page links to it.
- Authors, approvers and managers read the insights; approvers and managers export them.

## 0.4.0

Pictures on posts (FERRUM-ECOSYSTEM-005).

- A post may carry a picture stored here, with what it shows (required with a picture, for people
  who cannot see it). Approval covers both, and publications carry them.
- Mastodon uploads it as a media attachment (`POST /api/v2/media`) with that description, and the
  status carries it. Bluesky uploads it as a blob and embeds it as an image with that alt text,
  within its 1 MB. Both connectors now offer `publish_image`.
- A post with a picture goes only to accounts whose connection publishes pictures. Instagram takes
  a post with its picture; YouTube still takes video, which posts do not carry.
- Rebuilt with Ferrum-Frontend bb60115: empty numbers read as the server does, and settings open as
  their one record.

## 0.3.0

A connector for Bluesky (FERRUM-ECOSYSTEM-005).

- **Connection:** a server (bsky.social by default) and the account's handle and app password. A
  session is kept for an hour and opened again when the server stops taking it.
- **Posts:** sent as `app.bsky.feed.post` records, the link made a link facet, under a record key
  made from the delivery's key. A send tried again finds the post already there instead of making a
  second.
- **Reading back:** posts come back from the account's own posts, reposts left out, with likes,
  reposts, replies and quotes as engagements. Replies are read as comments, and mentions and quotes
  as mentions; likes are left out.
- **Replies:** sent in the thread of what they answer.
- **Refusals:** the server's become the platform's: its rate limit means waiting, an expired session
  means opening another.

## 0.2.5

Rebuilt with Ferrum-Frontend 00dea07: in a workflow state that names what may change, the other fields show read-only, and the fields the state needs show required.

## 0.2.4

Rebuilt with Ferrum-Frontend daeadda: after an action runs, a record page shows what the action said it did rather than only that it is done.

## 0.2.3

Rebuilt with Ferrum-Frontend 80ff179: a field's label names its control for assistive technology, and a record whose title is a link shows that record's title, not its id.

## 0.2.2

From the same browser walk, answering a direct message on Mastodon.

- A reply goes out as open as what it answers, never more: a direct message is answered directly, not
  in public, as 0.2.1 did. It names the one it answers, unless it does already, so Mastodon delivers
  it to them.
- Reply's box starts empty: it was filled with the message's own text.

## 0.2.1

From a browser walk of Social with a Mastodon account.

- An account names its connection by the name Administration, Connections shows (or its id); a name two live connections share is refused, and a disconnected one does not count.
- A message is answered from the message itself (Reply), as a reply of the one answering.

## 0.2.0

A connector for Mastodon, or any server speaking its API (provider `mastodon`).

- Posts sent as public statuses with their link, once each: the delivery's key is Mastodon's
  `Idempotency-Key`, and a status already out is not sent again.
- The account's own statuses read back without boosts, with favourites, boosts and replies as
  engagements.
- Mentions in the inbox: comments, mentions and direct messages; replies sent as statuses answering
  them.
- A refused token, a missing status, a refusal and Mastodon's rate limit each become the platform's
  error or waiting.

## 0.1.0

Social media management on the platform's connections (FERRUM-ECOSYSTEM-005).

- Brands and their accounts, each reached through a connection.
- Posts that fit each network, approved by someone other than their author, approved again when
  changed, and fixed once out.
- Sending at their time, one publication per account through its own connection.
- Impressions and engagements read back with the networks' posts.
- The inbox routed by handle, assigned, and answered through the right connection.
- Its own interface at `/social/`.
