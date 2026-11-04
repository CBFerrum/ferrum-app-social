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
  and YouTube take media, which posts here do not carry yet. The account's connection must say it
  can `publish_text`, and replying needs `reply`.
- **Review.** Someone other than the author approves a post, or sends it back saying why. A post
  changed after its approval needs approving again; its headline is the team's own and changes
  freely. A post that went out stays as it went.
- **Sent at its time.** When an approved post's time comes, it becomes one publication per account,
  each sent through that account's connection. A publication is out once the network has given its
  id; the post is published when all of them are.
- **What posts did.** Reading a connection's `posts` brings each post's impressions, engagements and
  link to the publication it is, and posts made on the network itself as publications of no post of
  ours.
- **The inbox.** Comments, mentions and direct messages come to the account whose handle they name.
  A message assigned says to whom. A reply goes back through the message's account's connection to
  the network's own message, and the message is answered.
- **Mastodon, built in.** Social brings a connector for Mastodon, or any server speaking its API
  (provider `mastodon`): see below.

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
- **replies** go out as public statuses answering the message's status.

A refused token is the connection's error; Mastodon's rate limit makes the platform wait as long as
it says (`Retry-After`), or five minutes.

## A connection's mapping

```json
{
  "posts": {"entity": "SocialPublication", "fields": {"text": "text", "link": "link", "impressions": "impressions",
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
- what posts did, read back;
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
- Connectors for X, Bluesky, LinkedIn, Threads, Instagram and YouTube: each takes its API and an
  approved account (Ferrum's FERRUM-CONNECT-002).
- Mastodon's media, polls, content warnings and editing a status once out.
- Images and video, threads, polls, and first comments.
- Best times to post, and analytics over time rather than the latest numbers.

## Changelog

| Version | Highlights |
|---|---|
| **0.2.0** | A connector for Mastodon: posts sent as statuses once each, engagements read back, mentions and direct messages in the inbox, replies answering them. |
| **0.1.0** | Brands' accounts through the platform's connections, posts reviewed and sent at their time within each network's limits, the inbox answered, and what posts did (FERRUM-ECOSYSTEM-005). |

## License

AGPL-3.0-only. See [LICENSE](LICENSE).
