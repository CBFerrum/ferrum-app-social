# Social

Social media management for [Ferrum](https://github.com/CBFerrum/Ferrum), on the platform's
connections:
- brands and their accounts on each network, each reached through a connection;
- posts approved by someone other than their author, and sent at their time to each account;
- the inbox read, assigned and answered;
- what each post did, read back from the networks.

| | |
|---|---|
| 📣 App | `social`, native, in the server |
| 🧩 Package | `social` |
| 🖥️ Interface | its own, at `/social/`: what waits for approval, posts coming by day, the inbox, and what each brand's posts did |
| 🔐 Roles | Social Author, Social Approver, Social Manager |
| 📜 Licence | AGPL-3.0-only |

## Highlights

- **Through the platform's connections.** Each account names the connection that reaches it
  (Administration, Connections). Credentials, retries, waiting when a network says to, dead letters
  and reading back are the platform's (Ferrum docs/connectors.md). A post goes out through its
  account's own connection and through no other.
- **What each network takes.** A post must fit every account it goes to: 280 characters on X (a
  link counting as 23), 300 on Bluesky, 500 on Mastodon and Threads, 3,000 on LinkedIn. Instagram
  takes a post with its picture; YouTube takes video, which posts here do not carry. The account's
  connection must say it can `publish_text`, a post with a picture needs `publish_image`, and
  replying needs `reply`.
- **Pictures.** A post may carry a picture stored here, with what it shows for people who cannot
  see it, which every picture needs. Approval covers both. Mastodon gets the picture as a media
  attachment with that description, and Bluesky as an embedded image with it as alt text, within
  its 1 MB.
- **Review.** Someone other than the author approves a post, or sends it back saying why. A post
  changed after its approval needs approving again; its headline is the team's own and changes
  freely. A post that went out stays as it went.
- **Sent at its time.** When an approved post's time comes, it becomes one publication per account,
  each sent through that account's connection. A publication is out once the network has given its
  id; the post is published when all of them are.
- **What posts did.** Reading a connection's `posts` brings each post's impressions, engagements and
  link to the publication it is, and posts made on the network itself as publications of no post of
  ours.
- **Day by day.** Networks show only a post's latest numbers, so each day a publication's numbers
  change is kept as a Daily Insight, one a day with the day's last numbers (by the workspace's day).
  The report Social Engagement by Day, linked from the home page, shows what each day added, for
  everything, a brand or an account: each day's numbers less the publication's day before, with
  every day of the span listed.
- **The inbox.** Comments, mentions and direct messages come to the account whose handle they name.
  A message assigned says to whom. A reply goes back through the message's account's connection to
  the network's own message, and the message is answered.
- **Mastodon, built in.** Social brings a connector for Mastodon, or any server speaking its API
  (provider `mastodon`): see below.

## Bluesky

Social also brings a connector for Bluesky, on bsky.social or any server speaking the AT
Protocol. A connection's config may name the account's server, `{"service": "https://bsky.social"}`
(the default). Its credentials hold the account's handle and an app password made for it,
`{"identifier": "brand.bsky.social", "app_password": "…"}`. A session is opened with them and kept
for an hour, since servers limit how often one is opened; one the server stops taking is opened
again.
- Posts go out as Bluesky posts, the link made a link facet so it is a link. Each is under a
  record key made from the delivery's key, so a send tried again finds the post already there
  rather than making a second.
- They are read back from the account's own posts, reposts left out, with likes, reposts, replies
  and quotes counted as engagements, and the post's bsky.app address. Bluesky counts no
  impressions.
- Replies to the account's posts are read as comments, and mentions and quotes as mentions. Likes
  are not messages. Direct messages are Bluesky's chat, which is not read.
- A reply goes out in the thread of what it answers.

## Mastodon

A connection with provider `mastodon`, config `{"instance": "https://mastodon.social"}` (the
account's server) and credentials `{"token": "…"}`: an access token of the account with the `read`
and `write` scopes, made under its Preferences, Development. Testing it answers with the account's
`@handle`. Then, with the mapping below:
- **posts** go out as public statuses, the link after the text. Each is sent with the delivery's
  key as Mastodon's `Idempotency-Key`, so a send tried again is one status; one already out is not
  sent again. Reading `posts` brings the account's own statuses, without boosts: favourites, boosts
  and replies counted as engagements, and the status's address. Mastodon counts no impressions.
- **messages** are the account's mentions: a comment when it answers one of the account's statuses,
  a direct message when its visibility is direct, a mention otherwise.
- **replies** go out as statuses answering the message's status, as open as it (a direct message is
  answered directly) and naming its author, so Mastodon delivers them.

A refused token is the connection's error; Mastodon's rate limit makes the platform wait as long as
it says (`Retry-After`), or five minutes.

## A connection's mapping

```json
{
  "posts": {"entity": "SocialPublication", "fields": {"text": "text", "link": "link", "image": "image", "image_alt": "image_alt", "impressions": "impressions",
            "engagements": "engagements", "permalink": "permalink"}, "conflict": "source"},
  "messages": {"entity": "SocialMessage", "fields": {"account": "account_handle", "kind": "kind", "author": "author",
               "text": "text", "post": "post_external_id"}, "values": {"kind": {"comment": "Comment", "mention": "Mention", "direct": "Direct"}}},
  "replies": {"entity": "SocialReply", "fields": {"text": "text", "in_reply_to": "in_reply_to"}}
}
```

None of these streams pushes by itself: Social sends each publication and reply through the one
connection it is for.

## Develop

```bash
cargo test                       # needs FERRUM_SOCIAL_TEST_DATABASE_URL's database
cd ui && npm run build && npm run check
```

`tests/social_it.rs` runs a brand on a stand-in network through the platform's connections:
- posts refused where a network would not take them;
- review, a post sent back, and one changed after approval;
- sending at its time, through each account's own connection;
- what posts did, read back, then kept day by day (one row a day, its last numbers), and what each
  day added, for everything, an account or a brand;
- the inbox routed, assigned and answered;
- who may do what.

`tests/mastodon_it.rs` (`FERRUM_MASTODON_TEST_DATABASE_URL`) runs the Mastodon connector against a
stand-in server speaking the parts of Mastodon's API it uses, over HTTP:
- a wrong token refused, the right one answering with the handle;
- a post sent once though its first answer was lost, with its link;
- engagements read back, boosts left out, and nothing sent again once out;
- a comment, a mention and a direct message routed to the account, and a reply answering the comment.

## Not yet

- Mastodon against a real server: the connector was tested against a stand-in of its API, and a
  real one needs an account's token.
- Bluesky against a real server: its connector was tested against a stand-in of the parts of the
  AT Protocol it uses, and a real one needs an account's app password. Bluesky's direct messages
  (its chat) are not read.
- Connectors for X, LinkedIn, Threads, Instagram and YouTube: each takes its API and an approved
  account (Ferrum's FERRUM-CONNECT-002).
- Mastodon's polls, content warnings, and editing a status once out.
- Video, more than one picture, threads, polls, and first comments.
- Best times to post. The history starts when Social first reads a post, and a day's numbers are
  whatever the last reading that day found, so a day nobody read shows nothing.

## Changelog

| Version | Highlights |
|---|---|
| **0.5.1** | Rebuilt with Ferrum-Frontend 24c3951: times on the workspace's clock; drafts headed as drafts. |
| **0.5.0** | What posts did, day by day: Daily Insights keep each day's numbers, and the report Social Engagement by Day shows what each day added. |
| **0.4.0** | Pictures on posts, with what they show: Mastodon's media and Bluesky's embedded images; Instagram takes a post with its picture. Rebuilt with Ferrum-Frontend bb60115. |
| **0.3.0** | A connector for Bluesky: posts with their links, read back with their engagements, replies, mentions and quotes, and replies in their threads. |
| **0.2.5** | Rebuilt with Ferrum-Frontend 00dea07: a workflow state's kept fields are read-only, its needed ones required. |
| **0.2.4** | Rebuilt with Ferrum-Frontend daeadda: actions say what they did. |
| **0.2.3** | Rebuilt with Ferrum-Frontend 80ff179: labels name their fields, and linked titles show their titles. |
| **0.2.2** | A reply on Mastodon is as open as what it answers (a direct message is answered directly) and names who it answers; Reply's box starts empty. |
| **0.2.1** | An account names its connection by name, and a message is answered from the message (Reply). |
| **0.2.0** | A connector for Mastodon: posts sent as statuses once each, engagements read back, mentions and direct messages in the inbox, replies answering them. |
| **0.1.0** | Brands' accounts through the platform's connections, posts reviewed and sent at their time within each network's limits, the inbox answered, and what posts did (FERRUM-ECOSYSTEM-005). |

## License

AGPL-3.0-only. See [LICENSE](LICENSE).
