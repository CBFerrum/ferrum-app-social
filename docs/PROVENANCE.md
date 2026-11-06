# Provenance

This app's design and code are its own. No code was copied from another project.

- Its shape follows common social media management practice: brands with accounts on several
  networks, a calendar of posts with approval, publishing at a time, a shared inbox, and per-post
  analytics.
- The networks' limits are their published ones. Everything that reaches a network goes through
  Ferrum's connections (docs/connectors.md).
- The Bluesky connector speaks the AT Protocol's published XRPC methods (docs.bsky.app):
  `com.atproto.server.createSession`, `com.atproto.repo.getRecord` and `createRecord`,
  `app.bsky.feed.getAuthorFeed` and `getPosts`, and `app.bsky.notification.listNotifications`, with
  record keys in the protocol's TID form and links as its rich-text facets. The code is this app's
  own.
