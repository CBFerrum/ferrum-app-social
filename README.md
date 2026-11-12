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

- **A network is a description, not code.** Social names no network. Each account is a connection
  (Administration, Connections): the workspace's sign-in at a provider, a person's own, or a key an
  administrator holds. Its description names operations by what Social needs: `publish`,
  `publish_media`, `delete`, `comments`, `reply`, `messages`, `send_message`, `metrics`, `account`.
  Examples for Mastodon and X are in `docs/connectors/`, with what each cannot express.
- **Accounts show their state**: connected, needs reconnecting (the provider refused the sign-in:
  the platform tells the owner and the account shows it), not connected, or the connection off.
  Connect, Check and Disconnect (which revokes at the provider) are actions of the account.
- **What each network takes is a record** (Network Profiles): characters, how a link counts,
  pictures and their size, video, whether media is required, first comment, threads, link preview.
  A post is checked per account, in plain words, before it is scheduled and again when it is sent.
  The profiles the app used to hold in code are seeded as records (X, Bluesky, Mastodon, Threads,
  LinkedIn, Facebook, Instagram, YouTube) and an account is linked to the one named like its network.
- **Publishing.** At its time, a post becomes one publication and one queued job per account. The job
  uploads the picture first when the connection offers `publish_media`, then `publish`, each under the
  key `publish:<publication>`; it keeps the network's id and address. Once per account under retries
  and two workers: the job is deduplicated by publication, holds a lock on the publication's row and
  looks at it after taking it, and the platform hands a key that went through its stored answer without
  calling the network (and gives the network the key in the description's idempotency header).
  A refusal shows in the service's own words per account; "later" is retried with backoff (eight
  tries); Retry sends again to the accounts not reached. A first comment follows where the profile
  takes one; Delete on the network where the connection offers `delete`.
- **Review.** Someone other than the author approves; a changed post needs approving again.
- **The inbox.** Comments, mentions and direct messages arrive on the connection's inbound address or
  by a pull with a cursor, mapped by the description onto Messages found by the network's id, so each
  arrives once. They are routed to the account by the handle the item names, else by the post they
  answer. Replies go out through `reply` or, for a direct message, `send_message`, once each. Saved
  replies, tags (the platform's), assignment and status. Social Settings can match the sender to a
  record of an entity the workspace names (installed, with a field holding handles). Erasure reaches
  handles written as addresses, message text and the replies written to them.
- **Numbers.** Once the hours in Social Settings have passed, each account's `account` and each recent
  post's `metrics` are read into one Metric row a day: impressions, reach, engagements, clicks,
  followers, whatever the network calls them. Reports: by account, by post, by tag, best times and
  what each day added.

## Develop

```bash
cargo test                       # needs the databases named in .github/workflows/ci.yml
cd ui && npm run build && npm run check
```

`tests/social_it.rs` runs a fake network on 127.0.0.1 through real connections: sign-in, reconnecting
and revoking; posts refused in plain words; one queued call per account, picture first, once under a
lost answer, "later" and two workers; refusals shown and retried; the inbox by inbound address and by
pull; replies; matching and erasure; numbers by day and the reports. `tests/upgrade_it.rs` serves a
workspace still on 0.6.13 with this release. `tests/connectors_it.rs` loads the examples.

## Not yet

- Media that a network takes only as multipart or a binary upload, and networks whose sign-in or
  session a description cannot express (see docs/connectors/README.md).
- Video, several pictures, threads, polls. Link preview and native scheduling are recorded in a
  profile but Social always sends at its time itself.
- A mapping cannot name the connection a message came through, so an item must name its account by
  handle or by the post it answers.
- Handles that are not addresses are found by name for a person to judge, never erased by request.

## Changelog

| Version | Highlights |
|---|---|
| **0.7.0** | Accounts on connections, with their state; network profiles as records; one queued call per account, once; the inbox through inbound addresses and pulls; numbers by day and reports by account, post and tag. The built-in Mastodon and Bluesky connectors are gone: examples are descriptions. |
| **0.6.10** | The redesigned interface: the upgraded design system, shaped loading, told-apart states. |
| **0.6.12** | The home lists posts that did not go out, with the reason and Retry. |
| **0.6.13** | Rebuilt on the tuned design system: better menu icons, a lighter first page, a visible scroll cue, cleaner print. |
| **0.6.11** | A post that fails says so: a Failed status with the network's reason per account, and Retry. |
| **0.6.9** | Its reports' names in Arabic and Hindi. |
| **0.6.8** | Its names in Arabic and Hindi. |
| **0.6.7** | The assistant knows its records. |
| **0.6.6** | Rebuilt with Ferrum-Frontend 5b22e7b: its own words in Arabic and Hindi. |
| **0.6.5** | Rebuilt with Ferrum-Frontend 6537d65: once-shown answers kept; English isolated right to left; its own words translatable by a workspace. |
| **0.6.4** | Rebuilt with Ferrum-Frontend d5d98f2: a workspace's own interface words; pictures in attachment fields; a duplicate value said so. |
| **0.6.3** | Rebuilt with Ferrum-Frontend 63280d3: right to left, pages mirror; the records pages' words in Arabic and Hindi. |
| **0.6.2** | Rebuilt with Ferrum-Frontend a64416a: a box whose default is 1 shows ticked; action dialogs say every refusal and ask once; a new account shows Active ticked. |
| **0.6.1** | Rebuilt with Ferrum-Frontend a76295e: times on a person's own clock when they chose one. |
| **0.6.0** | Best times to post: the weekday and hour posts went out at, ranked by their engagements on average. |
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
