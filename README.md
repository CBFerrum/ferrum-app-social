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

## Not yet

- A connector for any real network: each takes its API and an approved account (Ferrum's
  FERRUM-CONNECT-002).
- Images and video, threads, polls, and first comments.
- Best times to post, and analytics over time rather than the latest numbers.

## Changelog

| Version | Highlights |
|---|---|
| **0.1.0** | Brands' accounts through the platform's connections, posts reviewed and sent at their time within each network's limits, the inbox answered, and what posts did (FERRUM-ECOSYSTEM-005). |

## License

AGPL-3.0-only. See [LICENSE](LICENSE).
