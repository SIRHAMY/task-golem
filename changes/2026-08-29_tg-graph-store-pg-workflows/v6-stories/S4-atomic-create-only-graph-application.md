# Change Story: Atomic create-only graph application

**Contract:** hamy-change-story/v6
**Change:** 2026-08-29_tg-graph-store-pg-workflows
**Story ID:** S4
**Version:** 1.0
**Status:** Ready
**Foundation version:** 3.0
**Foundation fingerprint:** bccd4078bedbeba0cd7160ad4f35076c794e96cd527fea2b4bcd44396b7e373e
**RFC version:** 2.0
**RFC fingerprint:** c1b5994147a42f73db575631f14409e6a15518fb861f54854cd1ebbc062dbf2d

## Outcome

Task Golem can atomically create a complete generic graph from symbolic request references and existing read-only UUID anchors.

## Scope

- **Task Golem:** Implement the approved `Store::apply_graph(GraphApplyRequest)` library/in-process operation and the structured `tg apply` JSON stdin/stdout contract using `GraphApplyItem`, `GraphRef`, and `GraphApplyResult`.
- **Task Golem:** Generate fresh canonical UUIDv7 identities, resolve local and existing references, validate generic item state plus parent/dependency references and cycles, and return deterministic mappings and diagnostics.
- **Task Golem:** Hold the existing store lock through validation and the single atomic JSONL commit, invalidate only derived cache freshness, leave existing anchors/events/archive unchanged, and keep item CRUD as the update path.

## Applicable Designs

- CHANGE_DESIGN_TG_IDENTITY_GRAPH_APPLY.md version 1.0

## Depends on

- S1 version 1.0
- S2 version 1.0

## Acceptance

- A valid multi-level request with local parent/dependency references, existing active and archived dependency anchors, tags, and opaque extensions creates a complete graph and returns a lexicographically ordered reference-to-UUID mapping; stored edges contain only canonical UUIDs and new items are `todo`, unclaimed, and unblocked.
- Duplicate/missing references, invalid IDs or item state, self-references, duplicate dependencies, parent cycles, dependency cycles, invalid durable anchors, and injected atomic-write failures return stable structured categories/diagnostics and leave tasks, archive, events, and related durable state unchanged.
- Repeating an identical valid request creates a disjoint fresh graph, never updates an existing anchor, and exposes no application key, receipt, request digest, or durable request-reference lookup; item CRUD still performs later updates.
- Library, in-process, and structured CLI success/error results have the same deterministic semantic fields, and a successful task write makes the derived cache stale for the normal rebuild path.
