# Provenance

This app's design and code are its own. No code was copied from another project.

- Its shape follows common social media management practice: brands with accounts on several
  networks, a calendar of posts with approval, publishing at a time, a shared inbox, and per-post
  analytics.
- The networks' limits are their published ones. Everything that reaches a network goes through
  Ferrum's connections (docs/connectors.md).
- No network is named in code. Examples under docs/connectors/ are written from the networks' public API
  documentation (pages listed there; the package ships two of them as data); the code is this app's own.
- 0.8.0 took behaviour and network facts, not code, from two open-source products read at
  `referance/social/`: Postiz (AGPL-3.0; composer, threads and delays, time slots, repeat, sets, signatures, the
  preview link, and each provider's limits and settings as written in docs/connectors/examples/) and Mixpost Lite
  (MIT; versions per account, editor limits). X's weighted length follows its published counting rules (ranges and
  weights only). No code was copied.
