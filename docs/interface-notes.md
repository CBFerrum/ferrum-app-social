# Social 0.8.0: what the screens are built from

The contract for the interface release that follows 0.8.0 (the composer, calendar, queue, accounts and library
screens of GAPS.md section U). Everything here is server behaviour that exists and is tested; the screens add no rule
of their own. Paths are the platform's (`docs/api.md` in Ferrum); `{id}` is a record's id. Every write goes through the
generic records API, so a screen never needs a route of Social's own.

## 1. Routes the screens use

| What | Route |
|---|---|
| Read, create, change a record | `GET/POST /api/v1/{Entity}`, `GET/PATCH/DELETE /api/v1/{Entity}/{id}` (child tables in the body: rows with an `id` are patched, rows without are added, missing rows are removed) |
| An action | `POST /api/v1/{Entity}/{id}/actions/{name}` with the parameters as the JSON body; the answer is the action's result below |
| The calendar's posts | `GET /api/v1/SocialPost?filters=[["scheduled_at","between",[from,to]]]&fields=...` (add `["brand","=",id]`, `["targets.account","in",[ids]]`, `["status","in",[...]]`, `["_tag","=",tag]`) |
| Drag to move | action `reschedule` (`{"at": rfc3339}`): keeps an approval. A `PATCH` of `scheduled_at` asks for approval again |
| Review decisions | `GET /api/v1/SocialPost/{id}/approval`, `POST …/approval/decide` `{outcome: "approve"\|"reject", comment}` (a reject needs a comment), `POST …/approval/recall`; the caller's to-decide list `GET /api/approvals` |
| Review links | action `review_link`; links of a post `GET /api/v1/SocialPost/{id}/links`; revoke `DELETE /api/links/{link id}`; the outside page is the platform's (`/api/public/{workspace}/links/{token}`) |
| Comments between members | `GET/POST /api/v1/SocialPost/{id}/comments` (platform) |
| Tags | `GET/PUT /api/v1/SocialPost/{id}/tags`, `GET /api/tags/SocialPost` (platform) |
| Media upload | `POST /api/files` → `{url: "/files/<id>", ...}`; put the `url` in `media[].file` (or `image`) |
| Connect an account | action `connect` → `{connect: {method, path}}`: POST that path from the browser (OAuth), or `{signed_in, state, note}` for a session |
| Bulk create | `POST /api/v1/SocialPost/_import` (JSON array, each row with its `targets`; Social Author has import) |
| Settings | `/apps/social/settings` (SocialSettings, generic) |
| Reports | `/r/social_engagement_by_day`, `/r/social_best_times`, `/r/social_by_account`, `/r/social_by_post`, `/r/social_by_tag` |

## 2. Entities and fields (new or changed in 0.8.0)

Types are the platform's. R/O = read-only (written by Social).

**SocialPost** (changed). The original's content: `text` (required), `link`, `image` + `image_alt` (0.7.0's one
picture; still works), `first_comment`. New:
- `targets` (table SocialTarget, required): `account` (link, required), `kind` (data: a kind name of the network's
  rules; empty = the account's `default_kind`, else the network's first), `text` (the account's own words: when
  non-empty the account has a **version**), `link` (the version's link), `settings` (json: answers keyed by the
  network's setting keys; merged over the account's `default_settings`).
- `items` (table SocialItem): follow-ups. `account` (link; empty = the original's, else that account's version's),
  `text`, `link`, `delay_minutes` (int ≥ 0, after the item before). Order = row order.
- `media` (table SocialMedia): `account` (empty = the original's), `item` (int ≥ 1; 1 = the post itself, 2 = the first
  follow-up …, counted within that account's version or the original), `file` (attach, required), `kind` (data; empty =
  from the file's type: image/* image, video/* video, application/pdf document), `alt`, `thumbnail` (attach), `width`,
  `height` (int px), `seconds` (decimal). Unknown dimensions are not checked; send them when the browser knows them.
- `scheduled_at` is no longer required (a draft or a post to queue has none).
- `add_signature` (check), `signature` (link SocialSignature).
- Repeat: `repeat_every` (int ≥ 1), `repeat_unit` (select Days, Weeks, Months; default Days), `repeat_until` (date),
  `repeat_count` (int ≥ 2, times in all). R/O: `repeat_of` (link: the first), `repeat_index` (1 = the first),
  `repeat_next` (link), `repeat_note` (why the next was not made), `repeat_from`, `repeat_key` (hidden).
- R/O: `origin` (link: duplicated from), `approval_request` (hidden: the platform request asked last), `preview` (text:
  each account's copy as plain text, made when a review link is given).
- Unchanged statuses: Draft, In Review, Approved, Rejected, Sending, Published, Failed.

A **version** is the whole post for its account: its `targets[].text` and `link`, the `media` rows naming the
account, the `items` rows naming it; nothing of the original is mixed in. Without `targets[].text` the account gets the
original (`text`, `link`, `image`, the `media` and `items` rows with no account). The composer copies the original
into a version when the person customises an account, and deletes the account's rows to drop the version.

Minimal post (the generic form still works): `{"headline", "brand", "text", "image", "image_alt", "scheduled_at",
"targets": [{"account": id}]}`. A full one:

```json
{"headline": "Spring", "brand": "<brand>", "text": "Spring is here", "add_signature": true,
 "targets": [{"account": "<x>"}, {"account": "<pins>", "kind": "pin", "settings": {"board": "b-1"}, "text": "Spring, pinned"}],
 "items": [{"text": "Two of three"}, {"text": "Three of three", "delay_minutes": 30}, {"account": "<pins>", "text": "unused: pins take no follow-ups"}],
 "media": [{"item": 1, "file": "/files/<id>", "alt": "A cat"}, {"account": "<pins>", "item": 1, "file": "/files/<id>", "alt": "A cat", "width": 1000, "height": 1500}],
 "scheduled_at": null}
```

**SocialPublication** (changed): new R/O `sent_items` (json: `[{item, id, url, media: [ids], sent_at, answer}]`, item
from 0), `next_at` (datetime: a thread waiting its delay), `follow_ups` (hidden: thread | comment). `external_id`,
`permalink`, `media_ref` are the first item's, as before.

**SocialAccount** (changed): new R/O `network_id` (its id on the network, picked after sign-in; sent as `account_id`),
`display_name`, `picture` (an address). New editable `default_kind` (data), `default_settings` (json), `sort_order`
(int, default 0: the order the composer lists accounts in, lowest first; the generic list still sorts by name).

**SocialProfile** (changed): new `social` (json, label Rules): the `meta.social` shape below; overrides the fields.

New entities (all `master`, in the Social module):

| Entity | Fields | Notes |
|---|---|---|
| SocialPostingTime "Posting Time" | `brand` (link; filled from the account), `account` (link; empty = the brand's), `weekday` (select Every Day, Monday … Sunday; default Every Day), `time` (data `HH:MM`, required), `active` (check, default yes) | On the workspace's clock. An account with times of its own uses only those |
| SocialTemplate "Post Template" | `template_name` (unique), `brand`, `headline`, `text`, `link`, `image`, `image_alt`, `first_comment`, `targets`, `items`, `media`, `add_signature`, `signature` | Same child tables as a post |
| SocialSet "Set" | `set_name` (unique), `brand` (required), `targets` (required), `text`, `link`, `first_comment`, `items`, `media` | |
| SocialSnippet "Snippet" | `snippet_name` (unique), `kind` (select Hashtags, Caption, Other), `brand`, `text` | The composer inserts `text` |
| SocialSignature "Signature" | `signature_name` (unique), `text`, `brand`, `account`, `is_default` | Saving a default clears the other default of the same account, brand, or workspace (no brand, no account) |
| SocialVariable "Variable" | `variable_name` (letters, digits, `_`), `value`, `brand`, `account` | One value per name and scope; `{{name}}` in words. Account's value before brand's before the workspace's |
| SocialReview "Review" | R/O `post`, `reviewer` (address), `reviewer_name`, `outcome` (Comment, Approved, Changes Asked), `text`, `at` | Written by review links |

## 3. Actions

All `POST /api/v1/{Entity}/{id}/actions/{name}`. "Who" is the roles' permission the action needs (write: Social
Author and Approver on posts; Manager has read only on posts).

**SocialPost**

| Action | When (condition) | Params | Result |
|---|---|---|---|
| `send_for_review` | Draft, Rejected | — | `{status: "In Review", approval: <request>}`; `approval` absent when a request was pending already (it goes on); `approval: null, approval_refused: "<why>"` when the policy cannot open (then Approve It decides); `{status: "Approved"\|"Sending"}` when the policy skipped every step |
| `approve` / `reject` | In Review | reject: `note` (required) | `{status}`. Refused "decided in its approval" while a platform request is pending: the screen uses `approval/decide` instead and shows these buttons only when `GET …/approval` is not pending |
| `review` | hidden | — | run by the platform when the policy is satisfied; not for screens |
| `publish_now` | Draft, Rejected, Approved | — | `{status: "Sending"\|"Approved"\|"In Review", scheduled_at}`: an approved post starts at once (Approved = it waits for an account, see `waiting_for`); a draft is sent for review with the time now |
| `add_to_queue` | Draft, Rejected, In Review, Approved | — | `{scheduled_at, status}`; refused "no posting times are set …" or "every posting time … taken" |
| `reschedule` | Draft, Rejected, In Review, Approved | `at` (rfc3339, required) | `{scheduled_at, status}` (approval kept) |
| `unschedule` | In Review, Approved | — | `{status: "Draft"}` (time and approval cleared) |
| `duplicate` | any | — | `{entity: "SocialPost", id}` (a draft, no time, `origin` set) |
| `stop_repeat` | `repeat_every` set | — | `{stopped: true}` (this post and its next, if not out) |
| `copies` | any (read only) | — | `{copies: [{account, account_name, network, kind, follow_ups: "thread"\|"comment"\|"none", items: [{text, link, delay_minutes, length, max, media: [{file, kind, alt, name}]}], first_comment, settings, problems: [words]}]}`: final words (variables, signature), lengths as the network counts |
| `save_as_template` | any | `name` (required) | `{entity: "SocialTemplate", id}` |
| `review_link` | Draft, In Review, Approved, Rejected | `email`, `name`, `decide` (bool: they may approve or ask for changes), `days` (1-90, default 7), `uses` (1-100, default 20) | `{id, url, expires_at, sent_to}`: `url` shown once; mailed with template `social.review_link` when `email` is given |
| `retry` | Failed | — | `{status: "Sending"}`: only publications not out, and only their items not out |

**SocialPublication**: `delete_remote` (when out, not deleted) → `{deleted: <items deleted>}`.

**SocialAccount**

| Action | Params | Result |
|---|---|---|
| `check` | — | `{state, note, read: {display_name?, handle?, picture?}}` |
| `rules` (read only) | — | `{rules: Rules}`: what its network takes and asks (section 5), defaults filled; `{}`-like defaults when nothing says |
| `connect`, `disconnect` | — | as 0.7.0 |
| `reachable` (read only) | — | `{accounts: [{id, name, handle, picture, kind, account}]}`; `account` = the Social account that has it, or null |
| `choose` (Manager) | `picked`: list of ids (or text, comma or line separated) | `{accounts: [{id, account_name, network_id, created}]}`; the first pick fills this account when it has none, the rest become accounts of their own (same brand, connection, profile) or update the one that has it |
| `mentions` (read only) | `query` | `{mentions: [{value, label, picture, text}]}`; `text` is what to insert |
| `choices` (read only) | `setting` (key), `query`, `settings` (current answers, for choices that depend on them) | `{choices: [{value, label}]}` |

**SocialTemplate** `use_template` (params `brand`, `set` (a set's id: its accounts), `accounts` (list of ids),
`headline`) → `{entity: "SocialPost", id}` (a draft). **SocialSet** `use_set` (`headline`) → `{entity: "SocialPost",
id}`. Both refuse when no account results.

Outside (through a review link only, hidden): `review_comment` (`comment` required), `review_approve` (`comment`),
`review_changes` (`comment` required; sends an In Review or Approved post back to Rejected with "<name>: <comment>").

## 4. Review

- `GET …/approval` → `{id, action: "review", status: pending|approved|rejected|recalled|cancelled|failed, requester,
  steps: [{label, state, need, approvers, …}], decisions: [{step, decider, outcome, comment, at, void}]}`. Show the
  steps and decisions on the post; offer Approve/Send back (decide) to whoever `GET /api/approvals` lists it for.
- A platform rejection reaches the post at the next upkeep (status Rejected, `review_note` = the comment), and who
  approved fills `approved_by` then too: show the request's own status meanwhile.
- The shipped policy: one step "Social Approver" (role), never the requester (Social asks as the author),
  `reset_on` [text, link, image, image_alt, first_comment, targets, items, media, scheduled_at, add_signature,
  signature]. Workspaces change it in Customize (`SocialPost`, approvals, `action:review`).
- Reviews (SocialReview) list under the post (`GET /api/v1/SocialReview?filters=[["post","=",id]]`).

## 5. `meta.social` (exact)

The shape is defined in `docs/connectors/README.md` ("meta.social") and `src/rules.rs` (`Rules`); examples in
`docs/connectors/examples/`. Where the screens find it: the account's connection's description `meta.social` (an
administrator's `GET /api/admin/connections`, not open to Authors), else its profile's `social`, else the profile's
fields. Every role that reads accounts gets it, defaults filled, from the account's `rules` action.

```text
Rules     { name?, text: Text, media: Media, kinds: [Kind], follow_ups: FollowUps, settings: [Setting],
            operations: {name: name}, accounts?: Lookup, mentions?: Lookup, first_comment: bool,
            link_preview: bool, scheduling: bool, inbox_format?: "html" }
Text      { max?, min?, count: "characters"|"graphemes"|"utf16"|"bytes"|"weighted", link_counts?, format?: "plain"|"markdown"|"html" }
Media     { kinds: [string], required: bool, min?, max?, mixes: [{kind: n}], image?: Medium, video?: Medium,
            document?: Medium, other: {kind: Medium}, alt: bool (default true), alt_required: bool, alt_max?, thumbnail: bool }
Medium    { max_bytes?, types: [string], min_width?, max_width?, min_height?, max_height?, min_ratio?, max_ratio?,
            min_seconds?, max_seconds? }
Kind      { name, label?, text?: Text, media?: Media, follow_ups?: FollowUps, first_comment?: bool }
FollowUps { as: "thread"|"comment"|"none" (default none), media: bool, max?, text?: Text }
Setting   { key, label, type: "text"|"long"|"number"|"bool"|"select"|"multi"|"date", options: [string | {value, label}],
            choices?: Lookup, required: bool, default?, when: {kind?: [names], <key>: value | [values]}, help?, min?, max? }
Lookup    { operation, inputs: {name: literal | "query" | "account.id" | "account.handle" | "settings.<key>"},
            items_at: pointer ("" = the answer), value_at: pointer, label_at?, handle_at?, picture_at?, kind_at?,
            format?: "…{{ value }}…{{ label }}…" }
```

Counting for live counters (the server counts the same way, `src/rules.rs` `length`): `characters` = Unicode scalar
values; `graphemes` = what a reader sees as one (an emoji with joiners and skin tone, a flag pair, a letter with its
accents count 1); `utf16` = UTF-16 units; `bytes` = UTF-8 bytes; `weighted` = 1 for each character in U+0000–U+10FF,
U+2000–U+200D, U+2010–U+201F, U+2032–U+2037, else 2 (an emoji 2). With `link_counts`, every `http(s)://` address in
the words counts that much. The post's `link` adds a space and its length (or `link_counts`). A setting is asked when
`when` holds; answers to settings that do not apply are dropped, not refused; an answer to an undeclared key is refused.

## 6. Wording, messages, names

Message template new: `social.review_link` (email; values `url` (secret), `expires_at`, `from`, `name`; the post's
fields as `doc`). Refusals are the server's words (shown as they come).

New names for `ui/src/locales/names/{ar,hi}.json` (left for the interface release): entities Posting Time(s), Post
Template(s), Set(s), Snippet(s), Signature(s), Variable(s), Review(s), Follow-up, Medium; fields Its Own Words, Its Own
Link, Answers, Kind, Wait (Minutes), Item, File, Description, Thumbnail, Width, Height, Length (Seconds), Follow-ups,
Media, Repeat Every, Repeat Unit (Days, Weeks, Months), Repeat Until, Times in All, Repeat Of, Occurrence, Next
Occurrence, Not Repeated, Copied From, Add the Signature, Signature, As It Goes Out, Items Out, Next Item At, Its Id on
the Network, Name on the Network, Picture Address, Kind of Post, Usual Answers, Order, Rules, Weekday (Every Day,
Monday … Sunday), Time, Template Name, Set Name, Snippet Name (Hashtags, Caption, Other), Signature Name, Default,
Name, Value, Reviewer, Outcome (Comment, Approved, Changes Asked), At; actions Publish Now, Add to Queue, Move, Back to
Draft, Duplicate, Stop Repeating, As Each Account Gets It, Save as Template, Review Link, Comment, Approve, Ask for
Changes, What Its Network Takes, What the Sign-in Reaches, Choose Accounts, Look Up Mentions, Choices of a Setting, Use Template, Use Set, and
their parameters (Goes Out At, Send It To, Their Name, They May Approve or Ask for Changes, Works For (Days), Uses, Your
Comment, Anything to Add, What to Change, Ids Picked, Who, Setting, Search, Brand, Accounts of a Set, Or These
Accounts, Headline).

## 7. Screens that need a change

- **Home** (`ui/src/Home.tsx`): Waiting for approval should come from `GET /api/approvals` as well as status In
  Review; the coming list should show drafts with no time apart; Waiting For and account state (0.7.0's debt).
- **Composer** (new; replaces the generic post form): accounts by brand (sort by `sort_order`), each account's `rules`,
  original + one tab per account (version), items editor with delays, media per item with alt/thumbnail/dimensions, settings form drawn from
  the rules (`choices` for fetched options, `when` for visibility), kind picker, counters and `copies` problems,
  signature/variables/snippets pickers, when: now / time / queue / repeat, templates and sets, review link.
- **Calendar**: list of posts by `scheduled_at`; drag = `reschedule`; posting times (SocialPostingTime) as free
  slots; repeats show their next occurrence as a real post.
- **Accounts**: connect, then `reachable` + `choose` when the description has an `accounts` operation; Check shows
  `display_name`/`picture`.
- **Review**: the request's steps and decisions, decide buttons, Reviews list, review link dialog (url shown once).

## 8. What the server does not give the screens yet

- No server check of an unsaved post: a post is checked when saved (`422` with the field and words). The composer
  counts client-side from the rules for live feedback.
- The review link page cannot show pictures (the platform's link page shows no files); it shows `preview` text with
  each medium's description.
