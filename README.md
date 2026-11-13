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

- **Networks (round four).** Thirty-three networks are described and shipped with the package, each offered to a
  workspace turned off: X, LinkedIn (member and page), Facebook Page, Instagram (through a page, and its own sign-in),
  Threads, YouTube, TikTok, TikTok Business, Pinterest, Reddit, Mastodon (any server), Bluesky, Google Business Profile,
  Telegram, Discord, Slack, WordPress, Medium, DEV, Hashnode, Tumblr, Lemmy, VK, Twitch, Kick, Dribbble, Whop, MeWe,
  Listmonk, Moltbook, Farcaster. **Each is tested against a fake of the network's documented API and has never run
  against the real network**: the first connection to each is the first time its real answers are read.
  docs/connectors/README.md says, per network, what it can, what it cannot yet and why, what to set up at the network,
  and what is unverified. Some cannot work yet: TikTok and VK cannot sign in, MeWe's sign-in is by hand, and pictures
  Instagram, Threads, Pinterest, Google Business and TikTok take only by a public address wait on the platform giving
  stored files one. To add a network: Administration, Connections, take the offered one; set up the app at the network;
  give the connection its client id and secrets; turn it on; in Social add the account, Connect it and, where offered,
  choose the pages or channels it reaches.
- **A network is a description, not code.** Social names no network. Each account is a connection
  (Administration, Connections): the workspace's sign-in at a provider, a person's own, or a key an
  administrator holds. Its description names operations by what Social needs: `publish`,
  `publish_media`, `delete`, `comments`, `reply`, `messages`, `send_message`, `metrics`, `account`.
  Working descriptions for Mastodon, Bluesky, LinkedIn and X are in `docs/connectors/`, with the network pages
  they were written from and what each still cannot express; Mastodon's and Bluesky's are also shipped in the
  package, so a workspace sees them offered and an account carried over from 0.6.x is given its connection (turned
  off) from them. What a network takes in a post is the description's `meta.capabilities`, else the account's profile.
- **Accounts show their state**: connected, needs reconnecting (the provider refused the sign-in:
  the platform tells the owner and the account shows it), not connected, or the connection off.
  Connect, Check and Disconnect (which revokes at the provider) are actions of the account.
- **What each network takes is a record** (Network Profiles): characters, how a link counts,
  pictures and their size, video, whether media is required, first comment, threads, link preview.
  A post is checked per account, in plain words, before it is scheduled and again when it is sent.
  The profiles the app used to hold in code are seeded as records (X, Bluesky, Mastodon, Threads,
  LinkedIn, Facebook, Instagram, YouTube) and an account is linked to the one named like its network; they are
  the defaults for an account whose connection's description does not say what its network takes.
- **Publishing.** At its time, a post becomes one publication and one queued job per account. The job
  uploads the picture first when the connection offers `publish_media`, then `publish`, each under the
  key `publish:<publication>`; it keeps the network's id and address. Once per account under retries
  and two workers: the job is deduplicated by publication, holds a lock on the publication's row and
  looks at it after taking it, and the platform hands a key that went through its stored answer without
  calling the network (and gives the network the key in the description's idempotency header).
  A refusal shows in the service's own words per account; "later" is retried with backoff (eight
  tries); a post whose account cannot be reached (not connected, sign-in refused, connection off) waits,
  saying which and why; Retry sends again to the accounts not reached. A first comment follows where the profile
  takes one; Delete on the network where the connection offers `delete`.
- **Review.** The platform's approval policy on the post's review, shipped as one step by a Social Approver who is not
  the author, editable per workspace; a changed post needs approving again. A review link lets someone outside read
  each account's copy, comment and approve or ask for changes.
- **Composer's server.** What a network takes and asks is `meta.social` on its description (docs/connectors/README.md):
  kinds, lengths as the network counts them, media rules, follow-ups and settings. A post has per-account versions,
  follow-ups (a thread or comments) with delays, several media; Publish Now, Add to Queue (Posting Times), Move, Back to
  Draft, Duplicate, repeats; Post Templates, Sets, Snippets, Signatures, Variables. docs/interface-notes.md lists every
  route, action and field the screens need.
- **The inbox.** Comments, mentions and direct messages arrive on the connection's inbound address or
  by a pull with a cursor, mapped by the description onto Messages found by the network's id, so each
  arrives once. They are routed to the account by the handle the item names, else by the post they
  answer, else by the one account the connection reaches (for a person's own connection, whose pull runs as
  that person, the account she connected). Replies go out through `reply` or, for a direct message, `send_message`, once each. Saved
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
pull; replies; matching and erasure; numbers by day and the reports. `tests/connectors_it.rs` loads the
examples and runs them against a fake of Mastodon's, Bluesky's and LinkedIn's documented shapes: a multipart
upload, a raw-body upload, an id in a header, a pull paged by the `Link` header, a `Retry-After`, a session
login and its refresh, a person's own pull. `tests/upgrade_it.rs` serves a workspace still on 0.6.13 with
this release: connections made from the shipped descriptions, administrators told once, a post waiting.

## Not yet

- What each network cannot do yet, and what the platform would need for it: docs/connectors/README.md ("Each network",
  "Needed from the platform").
- A short-lived public address for a stored file, for a network that pulls media by address (Instagram, Threads):
  the platform gives an app none (`public_url` is null).
- Reading platform tables by SQL where the platform has no call an app can use in its transaction (checked again
  against 0.3.7, which adds none): `conn` (a connection by id or name: `accounts::reach`, `outside::named`),
  `conn_token` (a sign-in's state: `accounts::state_of`), `app_user` (a person by address, the administrators:
  `accounts::person`, `upgrade`), `job` (a publication's job by its key: `posts::mark_sent`), `record_tag` (the
  report by tag), `tenant` (the active workspaces: the upkeep), and 0.6's `connection` (the upgrade). Each would go
  with a kernel call taking the caller's transaction.
- The screens for all of 0.8.0 (docs/interface-notes.md): the generic record forms show the new fields and actions.
- Video, documents and carousels go as far as a description's `publish` can upload them in its steps. Link preview and
  native scheduling are recorded but Social always sends at its time itself.
- The first pull of Mastodon's mentions reads the newest page, not the history.
- Handles that are not addresses are found by name for a person to judge, never erased by request.

## Changelog

| Version | Highlights |
|---|---|
| **0.8.0** | The composer's server: what each network takes and asks as one shape (`meta.social`); versions per account, threads and comments with delays, several media; publish now, queue by posting times, repeats; templates, sets, snippets, signatures, variables; review by the platform's approval policies and a review link for outside people; accounts picked after sign-in, mentions and choices looked up; a media library with stock sources, tracking and short links, writing help, plugs, more reports and a weekly report document, notices and a digest, feeds. |
| **0.7.0** | Accounts on connections, with their state; network profiles as records and a description's own `meta`; one queued call per account, once; the inbox through inbound addresses and pulls, as the person on a person's own connection; numbers by day and reports by account, post and tag. The built-in Mastodon and Bluesky connectors are gone: their descriptions ship as data, accounts are carried over (connection off, administrators told once) and posts wait for an account that cannot be reached. |
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
