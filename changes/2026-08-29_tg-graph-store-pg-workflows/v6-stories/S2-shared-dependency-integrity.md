# Change Story: Shared dependency satisfaction and integrity

**Contract:** hamy-change-story/v6
**Change:** 2026-08-29_tg-graph-store-pg-workflows
**Story ID:** S2
**Version:** 1.0
**Status:** Ready
**Foundation version:** 3.0
**Foundation fingerprint:** bccd4078bedbeba0cd7160ad4f35076c794e96cd527fea2b4bcd44396b7e373e
**RFC version:** 2.0
**RFC fingerprint:** c1b5994147a42f73db575631f14409e6a15518fb861f54854cd1ebbc062dbf2d

## Outcome

Task Golem has one dependency-satisfaction and integrity rule used consistently by mutation, readiness, diagnostics, and downstream consumers.

## Scope

- **Task Golem:** Refactor `src/model/deps.rs` and store mutation boundaries so active `done` targets and archived targets with preserved completion evidence satisfy dependencies, while every other active target and every truly missing target is unmet.
- **Task Golem:** Apply the rule to item creation/edit/dependency mutation, removal with active dependents, ready/next computation, cache rebuild readiness, and `doctor` integrity reporting without changing parent-edge semantics.
- **Task Golem:** Expose the resulting generic readiness/integrity result through the existing library and in-process surfaces for Phase Golem consumption.

## Applicable Designs

-

## Depends on

- S1 version 1.0

## Acceptance

- A dependency on an active non-done item is unmet; a completed prerequisite remains satisfying after archival; a target absent from both active and archive is unmet and is never treated as archived.
- New missing dependency references and removal of a target with active dependents are rejected before durable mutation, while an unexpected persisted dangling dependency is unready and appears in structured `doctor` integrity diagnostics.
- The same results are observed through add/edit/dep mutation, ready/next or equivalent readiness queries, cache rebuild, `doctor`, and the consumer-facing TG API; parent validation and no-rollup behavior remain unchanged.
