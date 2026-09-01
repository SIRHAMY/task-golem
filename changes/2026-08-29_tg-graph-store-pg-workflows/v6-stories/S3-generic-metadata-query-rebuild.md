# Change Story: Generic opaque metadata query and rebuild

**Contract:** hamy-change-story/v6
**Change:** 2026-08-29_tg-graph-store-pg-workflows
**Story ID:** S3
**Version:** 1.0
**Status:** Ready
**Foundation version:** 3.0
**Foundation fingerprint:** bccd4078bedbeba0cd7160ad4f35076c794e96cd527fea2b4bcd44396b7e373e
**RFC version:** 2.0
**RFC fingerprint:** c1b5994147a42f73db575631f14409e6a15518fb861f54854cd1ebbc062dbf2d

## Outcome

Task Golem consumers can query arbitrary opaque item metadata reliably, including after the derived query state is rebuilt.

## Scope

- **Task Golem:** Extend the generic cache projection and rebuild path in `src/cache/` to include `Item::extensions` without naming or interpreting Phase Golem fields.
- **Task Golem:** Preserve the existing read-only query contract, deterministic structured output, freshness detection, and cache replacement behavior while exposing metadata suitable for exact consumer-side run lookup.
- **Task Golem:** Keep JSONL authoritative so a missing, stale, or rebuilt cache returns metadata from current durable state rather than stale derived rows.

## Applicable Designs

-

## Depends on

- S2 version 1.0

## Acceptance

- A supported generic TG query returns opaque extension keys and nested values for active items without PG-specific branching or field interpretation.
- After metadata changes, cache deletion, or a stale cache stamp, the query/rebuild path returns the current metadata and preserves the S2 dependency-readiness projection.
- Focused library and `tg query` tests prove deterministic metadata output and that recovery queries do not depend on stale or manually maintained cache state.
