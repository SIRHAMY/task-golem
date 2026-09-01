# Change Story: Canonical UUIDv7 item identity

**Contract:** hamy-change-story/v6
**Change:** 2026-08-29_tg-graph-store-pg-workflows
**Story ID:** S1
**Version:** 1.0
**Status:** Ready
**Foundation version:** 3.0
**Foundation fingerprint:** bccd4078bedbeba0cd7160ad4f35076c794e96cd527fea2b4bcd44396b7e373e
**RFC version:** 2.0
**RFC fingerprint:** c1b5994147a42f73db575631f14409e6a15518fb861f54854cd1ebbc062dbf2d

## Outcome

Task Golem creates and resolves only canonical full UUIDv7 item identities while all existing item-level operations remain usable.

## Scope

- **Task Golem:** Replace legacy generation and prefix resolution in `src/model/id.rs` and remove `id_prefix`/`id_len` configuration and related CLI/library assumptions.
- **Task Golem:** Update item creation, lookup, display, mutation, transition, claim, query, archive, and remove paths to use exact full UUID validation and resolution.
- **Task Golem:** Preserve generic item fields, statuses, claims, events, extensions, and JSONL storage without migration or legacy compatibility.

## Applicable Designs

- CHANGE_DESIGN_TG_IDENTITY_GRAPH_APPLY.md version 1.0

## Depends on

-

## Acceptance

- Repeated item creation produces unique lowercase `8-4-4-4-12` UUIDv7 values with the required RFC variant, and full exact lookup works in active and archive scopes.
- Item-level create, read, update, status/claim transition, query, archive, and remove paths work with those IDs; a shortened, prefixed, uppercase, malformed, non-v7, or mixed-format identifier is rejected as `invalid_id` rather than resolved approximately.
- The Task Golem public library/CLI surface and configuration no longer expose custom-prefix or custom-length identity behavior, and focused tests prove no legacy identifier is generated or accepted.
