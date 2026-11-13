# Changelog

## 0.8.0

The composer's server. Everything a network takes and asks is one declarative shape, `meta.social` on its connection's
description (or the same shape in a Network Profile's new Rules field, else the profile's fields as before): text
limits and how length is counted (characters, graphemes, UTF-16, bytes, X's weights, links at a fixed length), text
format, media kinds, counts, mixes, sizes, types, dimensions, duration, descriptions and thumbnails, kinds of post
(post, reel, story, short, article... each with its own rules), what a follow-up is (a thread, comments, none), the
settings the composer asks (fetched from the network where they are: boards, subreddits, channels), the accounts a
sign-in reaches and how mentions are looked up. One generic check proves it can express YouTube, Reddit, Pinterest,
TikTok, Instagram, LinkedIn pages, Discord, Google Business Profile and WordPress (docs/connectors/examples/, their
operations not written yet). A setting the shape does not declare is refused; a required one missing names the account
and the field.

A post has an original and per-account versions (an account's own words, link, media and follow-ups), follow-up items
with delays, and several media per item with descriptions, thumbnails and dimensions. It is sent item by item under a
key each (`publish:<publication>` for the first, as 0.7.0, `publish:<publication>:<n>` after), each item's network id
and address kept as it goes; a thread that fails half way resumes where it stopped and Retry sends only what did not go;
Delete on the network takes every item down. When: Publish Now, Move (keeping an approval), Back to Draft, Duplicate,
Add to Queue (Posting Times per account or brand, by weekday, on the workspace's clock; two posts queued at once never
share a time), and repeats every N days, weeks or months until a date or a count, each next one a real post made once
when the one before goes out. Library: Post Templates, Sets, Snippets, Signatures (one default per account, brand or
workspace, never added twice) and Variables (`{{name}}`, an account's value before its brand's). Accounts: what a
sign-in reaches is listed and picked (each pick an account of its own), Check reads the name, handle and picture, and
mentions and a setting's choices are looked up; usual kind and answers, an order. As Each Account Gets It shows each
copy's final words and lengths.

Review is the platform's approval policy: the package ships 0.7.0's rule (one step, a Social Approver, never the
author) for a workspace to change in Customize (steps, approvers by brand, N of M, escalation) without code; approvers
decide in the platform's approvals, and Approve It still works where no policy can open (nobody holds the role) or a
post was In Review before the upgrade. A Review Link gives someone outside a signed link (expiry, uses) to read each
account's copy, comment, and approve or ask for changes; what they say is kept as Reviews and erased with them.

Every writing operation of the four descriptions carries an idempotency header. RetryAfter from the platform is
treated as "later". New: Posting Time, Post Template, Set, Snippet, Signature, Variable, Review; the message
`social.review_link`; Social Author may import posts. The interface is not rebuilt: docs/interface-notes.md is what the
screens are built from.

**Round two.** The media library (assets with description, size, source and credit; stock pictures and GIFs searched
and taken through sources named in Social Settings, the source told of a download where it asks; media from an
address through a connection's guard; a post's media line filled from an asset). Links: tracking parameters from the
most specific Link Template, never doubled nor on excluded hosts; short links through a shortener, once per address and
publication, their clicks read back. Writing help through a connection (write, rewrite with presets, adapt for each
account within its rules, hashtags, describe a picture, split into a thread, a picture kept as an asset, a blank start
from a brand), prompts as editable templates with the brand's voice, answers given back never saved over, each use
kept and erased with the person, the role Social Writing Help. Plugs (repost, comment, other accounts like, repost or
comment, once a number is reached or a time has passed), each run kept. Reports by network, by kind, against the period
before, followers by day, top posts; each brand's week kept on it, a document template and a weekly automation to the
address it names. Notices as templates (went out, did not, review, reconnect, repeat stopped, a weekly digest), each
turned off per person. Feeds whose new items become posts, once each, read through a connection answering JSON (XML
waits on the platform). A draft may be saved with what its networks would refuse, refused when it leaves Draft; a body
sent to `_calc` with `want_checks` answers what `copies` does without saving; `fold` in a network's text rules; the
bar's New Social Post opens `social/compose`. Example descriptions for Unsplash, Pexels, GIPHY, Dub, Kutt, the Claude
Messages API and a JSON feed.

**Round three.** Requires Ferrum's App API 0.3.7 (connections part three). A network's `Retry-After` puts a post's or
a reply's job back for the seconds it said (at most an hour), and the upkeep's own calls (numbers, plugs, clicks,
writing help, feeds) leave that connection alone until then. A message's network id is read-only again: the shipped
descriptions' mappings write as the system (`as: system`), and people no longer create messages. Feeds read RSS 2.0,
RSS 1.0 and Atom through one shipped `feeds` description (XML answers); a feed is an address. A `publish` made of steps
uploads its own media (no `publish_media` needed; media come as stored files), and each item gets the root's and the
previous item's whole results. The account's own posts can be read back as publications without a post, their numbers
read per item; a mention answering one of the brand's posts is a comment. A notice opt-out is its person's.

**Upgrading from 0.6.13 or 0.7.0.** No step for an administrator. Posts, approvals and publications read and send as
before (an approval given before keeps holding: its fingerprint is unchanged for a post with only 0.7.0's content); a
publication under way keeps its key. The review policy applies at once; a workspace where nobody holds Social Approver
keeps 0.7.0's Approve It until someone does. A connection a workspace copied from a description of before, whose pull
runs on each person's own sign-in without `as: system`, has its pull refused plainly (people no longer create messages;
the platform tells the administrators): add `"as": "system"` to its mapping. Pulls on the workspace's own sign-in, and
inbound addresses, already wrote as the system and go on as they were.

## 0.7.0

Social reaches every network through the platform's connections (Ferrum 0.3.5) and names none in code. A network is a connection's description; `docs/connectors/` has working ones for Mastodon and Bluesky (what the earlier built-in connectors did: text posts, a picture, delete, replies, mentions and notifications, numbers, followers), LinkedIn (a post's id read from a response header) and X, each tested end to end against a fake of the network's documented shapes, and the package ships Mastodon's and Bluesky's as data. What a network takes in a post is `meta.capabilities` on its description when it says (a server's own limit lives there), else the account's Network Profile: the seeded profiles (X, Bluesky, Mastodon, Threads, LinkedIn, Facebook, Instagram, YouTube) are the defaults.

**Upgrading from 0.6.x.** The built-in Mastodon and Bluesky connectors are removed and tokens are never copied. The upkeep makes, for each account of those two networks, a connection from the shipped description, turned off, with the server the old connection spoke to, links the account to it, and tells the workspace's administrators once, in the app, which accounts need connecting again and where (add the connection's secrets, turn it on, Connect the account). A post due for an account that cannot be reached (not connected, its sign-in refused, or its connection off) waits, with Waiting For saying which and why, and goes when the account can be reached; it is not failed. An account of another network, or one whose connection is not here, keeps its record, posts and messages and shows Not Connected. The old connections of the hub stay where they are, with their tokens: delete them in Administration once the new ones work.

Accounts are connections (the workspace's sign-in, a person's own, a key, or a session from an app password) with a state (Connected, Needs Reconnecting, Not Connected, Connection Off) and actions Check, Connect and Disconnect. A post is checked per account in plain words before it is scheduled and again when sent, and is one queued job per account, picture first, under keys, once under retries and two workers; the network's id and address are kept, refusals shown in the service's words, Retry, first comment, Delete on the network. The inbox takes comments, mentions and direct messages from an inbound address or a pull (on a person's own connection, run as that person; mapped writes marked as fetched from outside, so nothing is sent back), once each, routed to the account by handle, by the post answered, or by the one account the connection reaches; replies through `reply` or `send_message`; saved replies; optional matching of a sender to a record; erasure of handles and text. Metrics are read into a row a day (a network's likes, boosts and replies added by the description's expression); reports by account, by post and by tag. Social Author, Approver and Manager may now create a message (the pull of a person's own connection writes as her). New: Network Profile, Metric, Saved Reply and Social Settings records, Waiting For on a post, the message `social.reconnect`; the account's own screens need Connect, Check and Disconnect buttons (the interface is not rebuilt in this release). Requires platform 0.3.5.

## 0.6.13

Rebuilt with Ferrum-Frontend 45dad7f, the design system as tuned after the landed build was reviewed. The left menu's icons are chosen so that none repeats on more than three rows and each fits its row; their drawings load beside the first page instead of inside it, so the page starts lighter; a row that scrolls sideways fades at the edge where there is more, in every browser and with reduced motion; a printed record leaves out empty fields and keeps its totals with the table; menus and popovers sit inside the page's landmarks for assistive technology; a selection bar folds what does not fit into More on a narrow screen. Nothing else changes.

## 0.6.12

The home shows posts that did not go out: a "Did not go out" queue first, in the attention tone, each post with the network's reason (one line per account) and a Retry that sends it again only to the accounts that did not get it. Failed has its red status colour wherever the home lists a post. Its labels, the Retry action and the failure texts have names in Arabic and Hindi. Rebuilt with Ferrum-Frontend 0a47157.

## 0.6.11

A post that could not be sent now says so. It has a Failed status, with the network's reason kept on the post (one line per account) and on each publication, so one network can have taken it while another did not. The platform's delivery already retries what is temporary (a timeout, a rate limit, a 5xx: eight tries) and ends what is permanent (refused text, revoked access) at once; the post follows the outcome instead of staying in Sending for ever. A Retry action on a failed post sends it again, only to the accounts it did not reach. A post left in Sending from before, whose delivery has left no trace in the job list for a day, is marked failed too. The interface still shows Failed in the default colour and the home page's coming list leaves a failed post out (ui/src/Home.tsx names statuses): to follow in an interface release.

## 0.6.10

The interface follows the upgraded design system, rebuilt with Ferrum-Frontend 0a47157. Text uses the kit's type roles (nothing under 12px) and controls its height tokens; loading shows skeletons shaped like what is coming; empty, no matches, denied, offline and failed are told apart; status colours come from stored state ids. The interface check now runs the UI lint against an empty baseline. The home is rebuilt on the kit's work-home pattern: one page title, the main action as a button, queues with their counts, status at its natural width; its other own pages follow the same anatomy.

## 0.6.9

The reports it defines in code (their titles and parameters) have names in Arabic and Hindi too, with its models' and manifest's (106 each in ui/src/locales/names). Translated by the model, to be reviewed by native speakers.

## 0.6.8

Rebuilt with Ferrum-Frontend c59c9a2: the names of its models, fields and manifest in Arabic and Hindi (102 each in ui/src/locales/names, which the platform serves to every page of the workspace, so a record's labels, the launcher and the bar read in the person's language). Translated by the model, to be reviewed by native speakers.

## 0.6.7

The assistant knows what its records mean in its pages (contributes.shell.assistant): their states, what counts and where figures come from, with questions it suggests asking there.

## 0.6.6

Rebuilt with Ferrum-Frontend 5b22e7b: its own words carry built-in Arabic and Hindi (19 words each, ui/src/locales, translated by the model and to be reviewed by native speakers), fetched for the page's language only. The labels it keeps in objects, its dialogs, its page titles and the words it keeps in variables go through t() as well, and the names and sizes it shows as they are keep their order right to left. Its pages are drawn once their words are in, and its entry carries only the parts of the kit it uses.

## 0.6.5

Rebuilt with Ferrum-Frontend 6537d65: an action's link, code or token shown once is kept in a dialog to copy, and on a page that reads right to left, English left untranslated keeps its own order. Its own words go through the kit's t() (23 places), so a workspace's own words for its language reach them, and Admin, Interface words lists them under it (dist/words.json).

## 0.6.4

Rebuilt with Ferrum-Frontend d5d98f2: a workspace's own words for its interface (Admin, Interface words) reach its pages as they load; an attachment field that holds a picture shows it; and a value another record already has is said so, where the form said the record had been changed by someone else.

## 0.6.3

Rebuilt with Ferrum-Frontend 63280d3: right to left, its pages mirror as they read (icons that point along the line, margins and alignment by the line's start and end, arrow keys along it), and the records pages' words are in Arabic and Hindi.

## 0.6.2

Rebuilt with Ferrum-Frontend a64416a: a new record's box whose default is 1 shows ticked, as it is saved. An action's dialog says every refusal, the errors on what the action made (another record's rows) among them, and an action with parameters and a confirmation asks once. A new account's Active box is ticked, as it is saved: its default was the number 1, which the form showed unticked.

## 0.6.1

Rebuilt with Ferrum-Frontend a76295e: its pages show times on a person's own clock when they chose one on their Account page, while today and due dates stay on the workspace's day.

## 0.6.0

Best times to post (FERRUM-ECOSYSTEM-005).

- **Social Best Times to Post** (report `social_best_times`) ranks the weekday and hour posts went
  out at, on the workspace's clock, by their engagements on average. Each slot shows its posts,
  its engagements and impressions a post, and its engagement rate. It covers the last 90 days
  unless a span is given, for everything, a brand or an account.
- The home page links to it beside Day by day.

## 0.5.1

Rebuilt with Ferrum-Frontend 24c3951: its pages show times on the workspace's clock, naming the zone when it is not the browser's, a draft numbered on submit is headed as a draft, and a record of an app that is not installed says so.

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
