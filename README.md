<div align="center">

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/assets/banner-dark.svg">
  <img alt="Ferrum Social: plan, approve and publish posts, and answer the inbox" src="docs/assets/banner-light.svg" width="100%">
</picture>

<br>

[![Version](https://img.shields.io/badge/version-0.8.0-db2777?style=flat-square)](CHANGELOG.md)
[![Licence: AGPL-3.0](https://img.shields.io/badge/licence-AGPL--3.0-blue?style=flat-square)](LICENSE)
[![Ferrum platform](https://img.shields.io/badge/runs%20on-Ferrum-2f5bea?style=flat-square)](https://github.com/CBFerrum/Ferrum)
![Rust](https://img.shields.io/badge/Rust-server-b7410e?style=flat-square&logo=rust)
![PostgreSQL](https://img.shields.io/badge/PostgreSQL-data-336791?style=flat-square&logo=postgresql&logoColor=white)
![React](https://img.shields.io/badge/React%20%2B%20TypeScript-interface-0ea5e9?style=flat-square&logo=react&logoColor=white)
![Tests](https://img.shields.io/badge/tests-28%20Rust%20%2B%2027%20UI%20files-16a34a?style=flat-square)

### Plan a post once, approve it, send it to each network within its limits, and answer what comes back.

Ferrum Social is for marketing teams and agencies that speak for brands on many networks. It replaces a
hosted scheduler with a shared calendar, an approval step, an inbox and per-post numbers, kept next to your
CRM, helpdesk and documents. Networks are descriptions, not code, so a new one needs no release.

[💡 Why](#-why-ferrum-social) · [📸 Screenshots](#-screenshots) · [✨ Features](#-features) · [📊 Compare](#-how-it-compares) · [📍 Roadmap](#-roadmap) · [🚀 Install](#-install) · [🔧 Develop](#-develop)

</div>

> [!IMPORTANT]
> The 33 network descriptions are tested against fakes of each network's documented API. **None has been run against the real network yet.** The first connection to each is the first time its real answers are read. [docs/connectors/README.md](docs/connectors/README.md) says, per network, what works, what does not yet, and what is unverified.

## 💡 Why Ferrum Social

- 🔌 **A network is a description.** Social names no network in code. Each account is a platform connection; what a network takes (length, media, threads, kinds of post, settings) is data you can read and edit.
- ✅ **Someone else approves.** A post is approved by a person other than its author, by the platform's approval policy you can change without code, and can be reviewed by people outside through a signed link.
- 🛡️ **Sent once, however often it is retried.** Each account's copy is one queued job under a key; a lost answer, two workers or a retry never posts twice where the network honours a key, and where it does not the call says it may have been sent.
- 🧬 **One platform.** The inbox can match a sender to a CRM record; erasure of a person reaches the messages written to them; roles, audit and REST and MCP come with Ferrum.
- 🏠 **Self-hosted and open source.** AGPL-3.0, with no per-seat or per-profile fee from Ferrum.
- 🤝 **Learned from open source, openly.** The composer follows behaviour seen in Postiz (AGPL-3.0) and Mixpost Lite (MIT); no code was copied ([docs/PROVENANCE.md](docs/PROVENANCE.md)).
- 🌍 **English, Arabic (right to left) and Hindi.**

## 🎯 At a glance

| | |
|---|---|
| **Version** | 0.8.0 (package `social` 0.1.0) |
| **Kind** | Its own interface at `/social/` (compose, calendar, posting times, accounts, media, inbox, numbers, library, automation, notices) plus Ferrum's standard lists and forms |
| **What it adds** | 31 entities (28 records and 3 line tables), 4 roles, 10 reports, 1 weekly report document and automation, 8 message templates, 33 network descriptions, 10 left-menu pages, 1 create shortcut |
| **Depends on** | Ferrum platform `>=0.3.8, <0.4`; the platform's connections |
| **Works with** | Platform connections and approvals, CRM (inbox sender matching), any app that holds handles |
| **Languages** | English, Arabic, Hindi |
| **Tests** | 28 Rust tests against a real PostgreSQL and a fake network, 27 interface test files |
| **Licence** | AGPL-3.0-only |

## 📸 Screenshots

<picture><source media="(prefers-color-scheme: dark)" srcset="docs/assets/screens/social-home-dark.webp"><img alt="Social — home" src="docs/assets/screens/social-home-light.webp" width="100%"></picture>

<p align="center"><b>🏠 Home</b><br><sub>See what waits for approval, which posts did not go out, and what is coming today.</sub></p>

<table>
<tr>
<td width="50%" valign="top">
<picture><source media="(prefers-color-scheme: dark)" srcset="docs/assets/screens/social-social-brand-list-dark.webp"><img alt="Brands — list" src="docs/assets/screens/social-social-brand-list-light.webp"></picture>
<p><b>🏷️ Brands</b><br><sub>Browse the brands the team speaks for and their accounts.</sub></p>
</td>
<td width="50%" valign="top">
<picture><source media="(prefers-color-scheme: dark)" srcset="docs/assets/screens/social-social-post-list-dark.webp"><img alt="Posts — list" src="docs/assets/screens/social-social-post-list-light.webp"></picture>
<p><b>📝 Posts</b><br><sub>Browse, search and filter posts by status, brand and time.</sub></p>
</td>
</tr>
<tr>
<td width="50%" valign="top">
<picture><source media="(prefers-color-scheme: dark)" srcset="docs/assets/screens/social-social-message-list-dark.webp"><img alt="Inbox — list" src="docs/assets/screens/social-social-message-list-light.webp"></picture>
<p><b>💬 Inbox messages</b><br><sub>Browse comments, mentions and direct messages as records.</sub></p>
</td>
<td width="50%" valign="top">
<picture><source media="(prefers-color-scheme: dark)" srcset="docs/assets/screens/social-social-brand-detail-dark.webp"><img alt="Brand — open record" src="docs/assets/screens/social-social-brand-detail-light.webp"></picture>
<p><b>📇 A brand</b><br><sub>Read and edit one brand, with its accounts, history and related records.</sub></p>
</td>
</tr>
<tr>
<td width="50%" valign="top">
<picture><source media="(prefers-color-scheme: dark)" srcset="docs/assets/screens/social-post-list-dark.webp"><img alt="Social — posts" src="docs/assets/screens/social-post-list-light.webp"></picture>
<p><b>🗓️ Posts</b><br><sub>Plan posts and see which wait for approval.</sub></p>
</td>
<td width="50%" valign="top">
<picture><source media="(prefers-color-scheme: dark)" srcset="docs/assets/screens/social-inbox-dark.webp"><img alt="Social — inbox" src="docs/assets/screens/social-inbox-light.webp"></picture>
<p><b>📥 Inbox</b><br><sub>Answer messages from every connected account.</sub></p>
</td>
</tr>
</table>

<table>
<tr>
<td align="center" valign="top">
<picture><source media="(prefers-color-scheme: dark)" srcset="docs/assets/screens/social-home-phone-dark.webp"><img alt="Social — home" src="docs/assets/screens/social-home-phone-light.webp" width="260"></picture>
<p><b>📱 Home (phone)</b><br><sub>See what waits for approval, which posts did not go out, and what is coming today.</sub></p>
</td>
<td align="center" valign="top">
<picture><source media="(prefers-color-scheme: dark)" srcset="docs/assets/screens/social-social-brand-list-phone-dark.webp"><img alt="Brands — list" src="docs/assets/screens/social-social-brand-list-phone-light.webp" width="260"></picture>
<p><b>📱 Brands (phone)</b><br><sub>Browse the brands the team speaks for and their accounts.</sub></p>
</td>
</tr>
</table>

## ✨ Features

### 🏷️ Brands and accounts

- **Brands.** A brand is what the team speaks for, with its accounts on each network.
- **Accounts on connections.** An account belongs to a brand and a network, and is reached through a platform connection: the workspace's sign-in at a provider, a person's own, or a key an administrator holds.
- **Account state.** Connected, Needs Reconnecting (the provider refused the sign-in; the owner is told), Not Connected, or Connection Off. Connect, Check (reads the name, handle and picture) and Disconnect (which revokes at the provider) are actions of the account.
- **Pick what a sign-in reaches.** After signing in, the pages, channels, boards or locations the sign-in reaches are listed and each pick becomes an account of its own.
- **Network profiles.** What a network takes in a post is a record: characters, how a link counts, pictures and their size, video, whether media is required, first comment, threads, link preview. Profiles are the defaults for an account whose connection's description does not say. They are seeded for X, Bluesky, Mastodon, Threads, LinkedIn, Facebook, Instagram and YouTube.
- **A brand's voice.** Each brand keeps a voice that writing help uses, and an address for its weekly report.

### 🌐 Networks (33 described)

Every description ships with the package, offered to a workspace turned off and without secrets. To add one: take it in Administration, Connections; set up the app at the network; give the connection its client id and secrets; turn it on; add the account in Social and Connect it.

| Kind | Networks |
|---|---|
| Large social networks | X, LinkedIn (member), LinkedIn page, Facebook Page, Instagram (through a page), Instagram (its own sign-in), Threads, TikTok, TikTok Business, Pinterest, YouTube, Reddit |
| Open networks | Mastodon (any server), Bluesky, Lemmy, Farcaster |
| Chat and community | Telegram, Discord, Slack, Twitch, Kick, MeWe, VK, Whop |
| Publishing platforms | WordPress, Medium, DEV, Hashnode, Tumblr, Dribbble, Listmonk (your own newsletter server) |
| Local and other | Google Business Profile, Moltbook |

What the descriptions can do varies by network and is written per network in the connector notes. Examples of what is known to be missing:

- **X.** A lost post is not resent (X honours no idempotency key); direct messages are not read; link clicks are not read; the limit is fixed at 280 characters.
- **LinkedIn member.** A lost post fails; mentions and hashtags are plain text; member numbers and comments are not read (they need partner access).
- **Facebook Page.** A page's comments are not read yet; videos over 25 MB.
- **Instagram.** Delete is not offered (the API has none); a lost story fails.
- **TikTok.** Delete and comments (no API); a lost chunk stops the call; no more than ten photos.
- **Pinterest and Reddit.** Pinterest comments (no API) and Reddit video posts are missing.
- **VK and TikTok Business** cannot sign in through the platform's code exchange yet; **MeWe's** sign-in is by hand.
- **Nostr and Skool** have no description: Nostr needs signing and sockets the platform does not offer, and Skool has no public API.

### ✍️ Composer

- **Compose once, for many accounts.** Choose accounts by brand; write the original text; add a version per account with its own words, link, media, follow-ups and answers.
- **Checked as you type.** Each account's final copy, its length as that network counts it (characters, graphemes, UTF-16, bytes, X's weights, links at a fixed length), and problems in plain words come from the server. **As Each Account Gets It** shows each copy's final words and lengths. A draft may be saved with what its networks would refuse, and is refused only when it leaves Draft.
- **Kinds of post.** Post, reel, story, short, article and more, each with its own rules for the network.
- **Threads and comments.** Follow-up items (the next post of a thread, or a comment under it, as the network takes them) with delays. A post that fails half-way resumes where it stopped.
- **Media.** Several pictures, videos or documents per item with descriptions, thumbnails and dimensions, checked against the network's rules. Media a network pulls by address goes by a short-lived address made for the stored file (the server's `PUBLIC_URL` is needed).
- **First comment** where the network takes one.
- **Preview per account.**
- **Link tracking.** Tracking parameters from the most specific Link Template (account, brand, then workspace), never doubled and never on excluded hosts; short links through a shortener (Dub and Kutt are described), once per address and publication, with their clicks read back.
- **Signatures and variables.** A signature is added at the end of a post that asks for it; `{{name}}` variables are filled when the post is checked and sent (an account's value before its brand's before the workspace's).
- **Writing help.** Through a connection to a model you choose (an example description for a language-model API ships): write, rewrite with presets, adapt for each account within its rules, hashtags, describe a picture, split into a thread, a picture kept as an asset, a blank start from a brand. Prompts are editable templates with the brand's voice; answers are proposals to accept and never saved over; each use is kept and erased with the person; the role Social Writing Help is needed.

### 🗓️ Scheduling and queue

- **When.** Draft, Publish Now, a chosen time, or Add to Queue.
- **Posting times.** Times posts go out at, per account or brand, by weekday, on the workspace's clock. Add to Queue takes the first free one; two posts queued at once never share a time.
- **Calendar.** Month, week and day views, an agenda on a phone; move a post by dragging or **Move to**, with a quick look. Moving keeps an approval.
- **Repeats.** Every N days, weeks or months, until a date or a count; each next post is a real post made once when the one before goes out; stopping a repeat tells the author.
- **Back to Draft, Duplicate, Unschedule.**
- **Save as Template.**

### ✅ Review and approval

- **Approval by someone else.** The platform's approval policy ships as one step by a Social Approver who is never the author; a workspace changes it in Customize (steps, approvers by brand, N of M, escalation) without code. A changed post needs approving again.
- **Decision shows at once** on the post.
- **Review links.** Someone outside the workspace gets a signed link (with expiry and uses) to read each account's copy, comment, approve or ask for changes. What they say is kept as Reviews and erased with them.
- **Statuses.** Draft, In Review, Approved, Rejected, Sending, Published, Failed.

### 🚀 Publishing

- **One publication and one queued job per account** at the post's time. The job uploads media first where the network needs it, then posts, each under a key; it keeps the network's id and address.
- **Once per account under retries and two workers.** The job is deduplicated by publication and takes a lock on its row; the platform hands a key that already went through its stored answer without calling the network.
- **Plain refusals.** A refusal shows in the network's own words per account; "later" (including `Retry-After`) is retried with backoff (eight tries); a post whose account cannot be reached waits, saying which and why. **Retry** sends again only to the accounts not reached, or only the items that did not go.
- **Delete on the network** takes every item down where the connection offers delete.
- **Failures are visible.** The home lists posts that did not go out, with the reason and Retry.

### 📥 Inbox

- **Comments, mentions and direct messages** arrive on the connection's inbound address or by a pull, are found by the network's id so each arrives once, and are routed to the account by the handle, the post they answer, or the one account the connection reaches.
- **Handle it.** New, Assigned, Answered, Closed. Assign to a person; tags (the platform's); saved replies; reply through the network, once each, or a direct message.
- **Keyboard desk.** `j`, `k`, `r` and `e` move and act.
- **Sender matching.** Social Settings can match a sender to a record of an entity you name (CRM, for example), where installed and a field holds handles.
- **Erasure** reaches handles written as addresses, message text and the replies written to them.

### 📈 Numbers and reports

- **Read-back through connections.** Once the hours in Social Settings have passed, each account's figures and each recent post's metrics are read into one row a day: impressions, reach, engagements, clicks, followers, whatever the network calls them.
- **Ten reports:** Engagement by Day, Best Times (the weekday and hour posts went out at, ranked by average engagement), by Account, by Post, by Tag, by Network, by Kind, Period Compared with the One Before, Followers by Day, and Top Posts.
- **The Numbers page** compares a period with the one before, by network, account and kind, and shows top posts and posts made on the network itself.
- **A weekly brand report.** A document generated each week and mailed to an address on the brand; edit or switch off the automation.

### 🗂️ Library and automation

- **Post Templates, Sets, Snippets, Signatures, Variables.** A set is a named selection of accounts with their kinds and answers and, if wanted, starting words; Use Set makes a draft from it.
- **Media library.** Pictures, videos, GIFs and documents with what they show, size, source and credit; stock pictures and GIFs searched and taken through sources named in Social Settings (Unsplash, Pexels and GIPHY are described); media from an address through a connection's guard.
- **Feeds.** A feed (RSS 2.0, RSS 1.0, Atom or JSON) turns new items into posts, once each, from a template or a set, then leaves them a draft, sends them for review, queues them or publishes now.
- **Plugs.** After a post does well (a measure reaches a number within hours) or a time passes: repost it, comment under it, or have other accounts like, repost or comment. Each run is kept as done, refused by the network, or skipped.
- **Notices.** Templates for: went out, did not, review, reconnect, repeat stopped, weekly digest, and the review link mail. Each person can turn each off; a digest day and time are settings.

### 🔐 Roles and data

- **Personal-data requests** reach message senders, message text and replies written to them. Handles that are not addresses are found by name for a person to judge, never erased by request.
- **Assistant.** It knows post, publication, insight and message states and suggests "Which posts wait for approval?" and "Which messages have not been answered?".

## 🧭 How it works

### From draft to published

```mermaid
flowchart LR
    DR["Draft: text, versions, media"] --> CK["Checked per account"]
    CK --> RV{"Approved by someone else"}
    RV -->|no| BK["Back to draft or rejected"]
    RV -->|yes| Q["Scheduled or queued"]
    Q --> J["One job per account, once"]
    J --> NW["Network (via a connection)"]
    NW --> PB["Published, id kept"]
    NW -->|refused| FL["Failed, reason shown, Retry"]
    PB --> NM["Numbers read back daily"]
    classDef app fill:#db2777,color:#fff,stroke:#db2777
    class DR,J,PB app
```

A post is checked against each account's network rules before it leaves Draft. An approver other than the author approves it. At its time every account gets its own job, and numbers are read back later.

### How a network is plugged in

```mermaid
flowchart LR
    SO["Social: posts, inbox, numbers"] --> OP["Operations: publish, reply, messages, metrics, account"]
    OP --> CN["Platform connection: sign-in, keys, limits"]
    CN --> DS["Description: meta.social rules"]
    DS --> NW["Any network's HTTP API"]
    NW -->|"webhook or pull"| IB["Inbox messages"]
    IB --> SO
```

Social asks for operations by what it needs (`publish`, `reply`, `messages`, `metrics`, `account` and others). The connection's description says how each network does them and what it takes in a post.

## 🧱 Data model

| Entity | What it is | Key fields or states |
|---|---|---|
| SocialBrand | What the team speaks for | Name, description, voice, report address |
| SocialAccount | A brand's account on a network | Network, connection, handle, state (Connected, Needs Reconnecting, Not Connected, Connection Off) |
| SocialProfile | What a network takes in a post | Length, link counting, media, threads |
| SocialPost (with SocialTarget, SocialItem, SocialMedia) | What a brand says, to one or more accounts | Status (Draft, In Review, Approved, Rejected, Sending, Published, Failed), versions, follow-ups, media, time, repeat |
| SocialPublication | A post as it went to one account | Network id, sent, failed, permalink, numbers |
| SocialInsight, SocialMetric | What a publication or account had by a day | Impressions, reach, engagements, clicks, followers |
| SocialMessage, SocialReply, SocialSavedReply | The inbox | Kind (Comment, Mention, Direct), status (New, Assigned, Answered, Closed) |
| SocialPostingTime | A time posts go out at | Weekday, time, account or brand |
| SocialTemplate, SocialSet, SocialSnippet, SocialSignature, SocialVariable | The library | |
| SocialReview | What an outside reviewer said | Comment, approval, changes asked for |
| SocialMediaAsset | A picture, video, GIF or document kept | Size, source, credit, tags |
| SocialLinkTemplate | Tracking parameters for links | Account, brand or workspace |
| SocialWritingPreset, SocialWritingUse | Writing help | A way to rewrite; each use kept |
| SocialPlug, SocialPlugRun | Reactions after a post does well | Trigger, measure, action; each run |
| SocialFeed, SocialFeedItem | Feeds that become posts | Address, then (Draft, Send for Review, Add to Queue, Publish Now) |
| SocialNoticeOptOut, SocialDigestRun | Notices | |
| SocialSettings | One settings record | Inbox matching, read-back hours, media sources, digest day, connections for writing help, shortener and feeds |

### 👥 Roles

| Role | What it can do |
|---|---|
| Social Author | Writes posts and answers the inbox; someone else approves the posts |
| Social Approver | Approves posts or sends them back, and sees what they did |
| Social Manager | Keeps the brands and their accounts |
| Social Writing Help | May ask writing help to write, rewrite, adapt, describe and draw; each use is kept |

## 🔌 Integrations

| With | How | Needs |
|---|---|---|
| Platform connections | Every account and every outside service (stock pictures, shortener, writing help, feeds) is a connection with a description | An administrator turns each on |
| The 33 networks | Developer app at each network, client id and secrets, redirect address `{PUBLIC_URL}/api/connections/callback` | Your own app, keys and approval at each network |
| Platform approvals | Review policy, editable in Customize | Platform |
| CRM and other apps | Inbox senders matched to a record by handle | The app installed and a field holding handles |
| Stock media services | Unsplash, Pexels, GIPHY descriptions | Your keys |
| Link shorteners | Dub and Kutt descriptions | Your account and key |
| A model for writing help | A language-model API description ships as an example; any compatible description works | Your key |
| REST, MCP, webhooks | Every record and action | API keys |

## 📊 How it compares

Compared with Hootsuite, Buffer, Sprout Social, Later and, as open-source options, Postiz and Mixpost.

| Capability | Ferrum Social | Hootsuite | Buffer | Sprout Social | Later | Postiz | Mixpost |
|---|:-:|:-:|:-:|:-:|:-:|:-:|:-:|
| Many networks from one composer | 🟡 [1] | ✅ | ✅ | ✅ | ✅ | ✅ | 🟡 |
| Publishing proven against the live networks | ❌ [1] | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| A version of the post per account | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| Threads and follow-up comments with delays | ✅ | 🟡 | ✅ | 🟡 | 🟡 | ✅ | 🟡 |
| Calendar with month, week and day views | ✅ [2] | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| Queue by posting times | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| Repeating posts | ✅ | 🟡 | 🟡 | 🟡 | 🟡 | ✅ | 🟡 |
| Approval before publishing | ✅ | ✅ | ✅ | ✅ | ✅ | 🟡 | 🟡 |
| Review link for people outside the team | ✅ | 🟡 | 🟡 | ✅ | ✅ | ✅ | 🟡 |
| Shared inbox for comments, mentions and messages | 🟡 [3] | ✅ | 🟡 | ✅ | ✅ | 🟡 | 🟡 |
| Numbers by post, account and network, with best times | 🟡 [3] | ✅ | ✅ | ✅ | ✅ | ✅ | 🟡 |
| Scheduled report mailed | ✅ | ✅ | 🟡 | ✅ | ✅ | 🟡 | 🟡 |
| Media library with stock pictures and GIFs | ✅ | ✅ | 🟡 | ✅ | ✅ | ✅ | ✅ |
| Link shortening and tracking parameters | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | 🟡 |
| AI writing help | ✅ [4] | ✅ | ✅ | ✅ | ✅ | ✅ | 🟡 |
| Templates, sets, snippets, signatures | ✅ | 🟡 | 🟡 | ✅ | 🟡 | ✅ | 🟡 |
| Feeds (RSS) turned into posts | ✅ | 🟡 | ✅ | 🟡 | ❌ | 🟡 | ❌ |
| Automatic repost or comment when a post does well | ✅ | ❌ | ❌ | ❌ | ❌ | ✅ | ❌ |
| Link-in-bio page | ❌ | ❌ | ✅ | ❌ | ✅ | ❌ | ❌ |
| Social listening and brand monitoring | ❌ | ✅ | ❌ | ✅ | 🟡 | ❌ | ❌ |
| Paid ads and boosting | ❌ | ✅ | ❌ | ✅ | 🟡 | ❌ | ❌ |
| Native mobile app | ❌ | ✅ | ✅ | ✅ | ✅ | 🟡 | ❌ |
| Inbox senders linked to CRM records | ✅ | 🟡 | ❌ | ✅ | ❌ | ❌ | ❌ |
| Public API and automation | ✅ | ✅ | ✅ | ✅ | 🟡 | ✅ | 🟡 |
| Posts, customers and tickets in one data set | ✅ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ |
| Self-hosted, open source | ✅ | ❌ | ❌ | ❌ | ❌ | ✅ | 🟡 [5] |
| No per-seat or per-profile fee from the vendor | ✅ | ❌ | ❌ | ❌ | ❌ | ✅ | 🟡 |

Notes. 🟡 for a leader means a higher plan, a paid edition, or partial or unconfirmed support; open-source columns are from each project's public documentation, so check them for the current state.

1. 33 networks are described and tested against fakes; none has run against a real network yet, so the first connection to each is the first live test. Until then this row stays below the leaders.
2. Month, week, day and an agenda on a phone; moving a post with **Move to** is tested, dragging on the calendar has not been tried in a browser.
3. Inbox and numbers work through each network's described operations; which exist differs by network (for example X does not read direct messages, and LinkedIn member numbers need partner access).
4. Through a connection to a model you configure; no model ships with the app.
5. Mixpost Lite is MIT; the paid editions are not open source.

### 🏆 Where Ferrum Social stands out

- **Networks as data.** 33 described, readable and editable, with what each can and cannot do written down, and a new network needs no release.
- **Never twice.** One keyed job per account, locks, stored answers and a plain statement where a network cannot be asked.
- **Approval you can shape,** with outside review links and no per-reviewer fee.
- **One data set.** Inbox senders match CRM records; erasure reaches replies; the same roles and audit as the rest of the business.
- **Open and self-hosted,** with the connection's keys held by the platform.

### 🚧 Where the leaders are still ahead

- **Proven live connections** to the large networks, and mobile apps.
- **Listening, ads and link-in-bio,** which Ferrum Social does not do.
- **A native mobile app** and a browser extension.
- **Deeper analytics** from networks that need partner access Ferrum has not requested.

> [!NOTE]
> Leader columns are from public product information as of November 2026 and may change. Plans and add-ons differ; check each vendor.

## 📍 Roadmap

### ✅ Recently shipped

- [x] **0.8.0** — The composer's server and screens: network descriptions with `meta.social` rules, versions per account, threads, several media, queue, repeats, templates, sets, snippets, signatures, variables, review policy and review links; media library with stock pictures and GIFs, links with tracking and short links, writing help, plugs, reports, notices and feeds; 33 networks described; the app's own pages in the left menu.
- [x] **0.7.0** — Accounts on connections with their state, network profiles as records, one queued call per account, the inbox through inbound addresses and pulls, numbers by day and reports.
- [x] **0.6.12** — The home lists posts that did not go out, with the reason and Retry.
- [x] **0.6.11** — A post that fails says so: a Failed status with the network's reason per account.
- [x] **0.6.0** — Best times to post.
- [x] **0.5.0** — Daily insights and the Engagement by Day report.

### 🔜 Next up

**Prove the networks**
- [ ] Run each network's description against the real network with your own keys and fix what its real answers show. This needs developer apps and accounts at each network.
- [ ] Try signing in to a real network, dragging on the calendar and uploading files in a browser.

**Networks that cannot sign in yet**
- [ ] VK and TikTok Business sign-in (needs extra callback parameters from the platform), and MeWe's sign-in without a manual step.
- [ ] Read a Facebook page's comments (the platform's inbound mapping must skip likes and posts).
- [ ] Videos over 25 MB (the platform's upload limit is configurable).

**Composer and queue**
- [ ] Pick assets from Ferrum Digital Assets, so an expired or editorial-only asset cannot be posted.
- [ ] Send native scheduling and link previews to the networks that offer them (recorded, not used).

### 🔭 Later

- [ ] Link-in-bio page.
- [ ] Social listening and brand monitoring.
- [ ] Paid ads and boosting.
- [ ] Messages into Helpdesk as tickets.
- [ ] A native mobile app.
- [ ] Nostr (needs signing and sockets) and other networks without public APIs.
- [ ] X: read direct messages, link clicks, and Premium's longer posts.
- [ ] LinkedIn member numbers and comments (needs partner access).
- [ ] The first pull of Mastodon mentions reads the history, not just the newest page.
- [ ] Per-item public addresses for media beyond ten items.
- [ ] Reading platform tables through platform calls rather than SQL where the platform adds them.

## 🚀 Install

Social is compiled into the Ferrum server image and installed from **Apps** (`/store/`) by a workspace administrator. Self-hosters who build their own image enable the `social` feature of the server. It needs Ferrum `>=0.3.8, <0.4`. Set the server's `PUBLIC_URL`: networks that pull media by address (Instagram, Threads, Pinterest, Google Business, TikTok) need it, and without it such a post says so before it leaves Draft.

After installing:

1. Give people the roles **Social Author**, **Social Approver**, **Social Manager** (and **Social Writing Help**).
2. In **Administration, Connections**, take each network you want. Create the developer app at the network with the redirect address `{PUBLIC_URL}/api/connections/callback`, give the connection its client id and secrets, and turn it on.
3. In Social, make a **Brand**, add its **Accounts**, **Connect** each and pick the pages or channels it reaches.
4. Fill **Social Settings**: how the inbox finds the person who wrote, how often numbers are read, media sources, digest day, and the connections for writing help, a shortener and feeds.
5. Set **Posting Times** per account or brand.
6. Check the review policy in **Customize**; it ships as one step by a Social Approver who is not the author.
7. Edit the notice templates and the weekly report if you use them.

## 🔧 Develop

Clone it next to the platform:

```text
work/
├── ferrum/            https://github.com/CBFerrum/Ferrum (with Ferrum-Frontend cloned into ferrum/web)
└── ferrum-app-social/
```

Point the platform dependencies at your checkout (this file is git-ignored) with a `[patch."https://github.com/CBFerrum/Ferrum"]` section for the platform crates your checkout needs (`ferrum-apps`, `ferrum-domain`, `ferrum-kernel`, `ferrum-meta`).

The tests need PostgreSQL with an ordinary (non-superuser) role, so row-level security applies, and two databases (one for the upgrade test):

```bash
createuser ferrum --pwprompt
createdb -O ferrum ferrum_social_test
createdb -O ferrum ferrum_social_upgrade_test
export FERRUM_SOCIAL_TEST_DATABASE_URL=postgres://ferrum:ferrum@localhost:5432/ferrum_social_test
export FERRUM_SOCIAL_UPGRADE_TEST_DATABASE_URL=postgres://ferrum:ferrum@localhost:5432/ferrum_social_upgrade_test
cargo test
```

`tests/social_it.rs` runs a fake network on 127.0.0.1 through real connections. `tests/networks_a_it.rs` and `tests/networks_b_it.rs` run each network's description against a fake of its documented API. `tests/connectors_it.rs` fails when a shipped description and its file in `docs/connectors/` differ, and `tests/upgrade_it.rs` serves a workspace still on 0.6.13 with this release.

The shipped network descriptions are written by `scripts/ship_connections.py` from `docs/connectors/`; run it after you change one.

The interface is in `ui/` and builds with Ferrum-Frontend's packages (checked out at `../ferrum/web`):

```bash
cd ui
npm run check
npm run typecheck
npm test
npm run build      # ui/dist
```

CI (`.github/workflows/ci.yml`) runs `cargo fmt -- --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` on PostgreSQL, then the interface check, typecheck and tests on Node 22. [docs/interface-notes.md](docs/interface-notes.md) lists every route, action and field the screens are built from.

## 📂 Project layout

```text
app.yaml               the manifest: id, version, requirements, left-menu pages, what it adds to Ferrum
packages/social/       entities, roles, messages, automation, the 33 shipped connection descriptions
src/                   Rust: accounts, posts, review, publishing, content checks, inbox, numbers, library, feeds, plugs
ui/                    its own interface (React and Vite), built into ui/dist
tests/                 integration tests against a real PostgreSQL and fake networks
docs/connectors/       one description and notes per network, and the contract Social calls
docs/interface-notes.md  what the screens are built from
scripts/               ship_connections.py
```

## 📜 Changelog

Every version is in [CHANGELOG.md](CHANGELOG.md). The latest is **0.8.0**.

## 🤝 Contributing

Issues and pull requests are welcome at [github.com/CBFerrum/ferrum-app-social](https://github.com/CBFerrum/ferrum-app-social/issues). Run the checks in the Develop section before you open one. Anyone with a developer account at a network can help most by running that network's description live and reporting what its real answers show.

## 📄 License

[AGPL-3.0-only](LICENSE), like the platform.

<div align="center">

<sub>Part of <a href="https://github.com/CBFerrum/Ferrum">Ferrum</a>, the open business platform · Made with 🦀 Rust and 🐘 PostgreSQL</sub>

</div>
