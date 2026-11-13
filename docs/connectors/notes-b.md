# Social networks, second batch: what each description does and cannot

Written as connection descriptions (JSON, no Rust) and proved against fakes of each network's documented API
(`tests/networks_b_it.rs`). **None of it has run against a real network**: the first connection to each is the first
time its real answers are read. Facts come from each network's public documentation and, where marked, the Postiz
provider code (AGPL, read only).

## Needed from Social

(see the end of this file for the full list; filled in as the networks were written)

## Needed from the platform

(same)

## How these descriptions work around two things in the contract

* **A medium a network takes inside the post call** (Telegram, Discord, Tumblr, Dribbble ...). Social calls
  `publish_media` first and hands `publish` only the returned ids. These descriptions make `publish_media` a read-only
  step (a cheap `getMe`-style call that also proves the token) whose result `id` is `inputs.file`, the stored file's id.
  `publish` then receives the stored file ids in `media` and `media_items[].id` and sends each as a multipart part.
  It works, costs one extra read per medium, and would go with a `file` field in `media_items`.
* **`delete` is given only `id`, `item`, `follow_up` and `root`**, no account id. Networks whose delete needs a chat or
  channel (Telegram, Discord, Slack) return an id of the form `<chat>:<message>` from `publish`.
* An operation made only of `steps` has no method, so the platform treats it as one that changes something: a reading
  one (`account`, `metrics`, `choices`) says `"changes": false`.
* Every writing call whose network has no idempotency header is a one-step operation (`steps`): a lost answer then fails
  the call ("may have been sent") instead of sending twice.

## Telegram (`telegram.json`)

**Does**: bot token as the connection's secret (in the URL path, as Telegram wants it); the chats the bot was added to
are what the sign-in reaches (`getUpdates`, `my_chat_member`), the chat id is the account's id on the network; two
kinds: `message` (HTML, 4096) and `media` (caption 1024, 1 to 10 photos, videos or files, one kind in a post); text,
photos, videos and documents (each medium its own message, the words as the first one's caption); follow-ups as replies
(`reply_parameters`), text only; delete (`deleteMessages`, every message of the item in one call); the account's
members (`getChatMemberCount`) and name.
**Cannot**: an album (`sendMediaGroup` needs every file as a part of one request; `form` cannot take a list of files:
platform, a `form` part repeated per item of a list); numbers for a post (the Bot API gives a bot no views or
reactions); mentions; files above 25 MB (Telegram takes 50 MB, the platform's part limit is 25 MB); a lost answer on a
send fails the call (no idempotency header and no way to read a message back).
**Unverified**: the token's `:` is sent as it is (the `url` form of an operation prints it unencoded; a path would
percent-encode it); an empty `caption` on the second and later pictures; that 429 carries a `Retry-After` header (the
API gives `parameters.retry_after` in the body, which the platform does not read, so the call is "later" with the
platform's own backoff); `characters` as the counting unit (the docs say "characters after entities parsing").
**Setup**: create the bot with @BotFather, put its token under the connection's `bot_token`, add the bot to the
group or channel as an administrator with the right to post, send one message there (a channel: make a post) so
that `getUpdates` shows it, then pick the chat in Social. Do not set a webhook on the bot (`getUpdates` stops working).
