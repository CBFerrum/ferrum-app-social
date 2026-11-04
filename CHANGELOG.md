# Changelog

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
