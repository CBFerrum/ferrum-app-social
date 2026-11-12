# Changelog

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
