# Change Story: PG execution over TG-native lifecycle

**Contract:** hamy-change-story/v6
**Change:** 2026-08-29_tg-graph-store-pg-workflows
**Story ID:** S7
**Version:** 1.0
**Status:** Ready
**Foundation version:** 3.0
**Foundation fingerprint:** bccd4078bedbeba0cd7160ad4f35076c794e96cd527fea2b4bcd44396b7e373e
**RFC version:** 2.0
**RFC fingerprint:** c1b5994147a42f73db575631f14409e6a15518fb861f54854cd1ebbc062dbf2d

## Outcome

Phase Golem selects and transitions workflow work using only TG status, claims, dependencies, and TG-owned readiness.

## Scope

- **Phase Golem:** Remove the authoritative `New`/`Scoping`/`Ready`/`InProgress`/`Parked` and `x-pg-status` lifecycle from `src/pg_item.rs`, `src/types.rs`, `src/scheduler.rs`, `src/coordinator.rs`, and related CLI/config paths.
- **Phase Golem:** Consume the TG readiness/integrity result from S2 rather than treating absent dependencies as satisfied, and preserve reusable PG policy as metadata or events instead of a second task state machine.
- **Phase Golem:** Model planned human gates as ordinary `todo` PG-owned tasks, keep attempted-but-unable work as `blocked`, remove project-prefix identity behavior, and make every status transition explicit with no automatic rollup.

## Applicable Designs

-

## Depends on

- S2 version 1.0
- S5 version 1.0

## Acceptance

- A materialized or CRUD-created task is selected only when TG reports it ready, claimed through TG before execution, and transitioned explicitly among `todo`, `doing`, `blocked`, and `done`; PG has no authoritative parallel status field or project-prefix identity path.
- A planned human gate remains `todo`, is not claimed or executed, is reported as a distinct stop condition, and leaves dependents unready until an authorized actor completes it; an attempted execution failure becomes `blocked` with diagnostics.
- Completing a child through PG or direct TG operations leaves every parent, container, and root unchanged until an explicit transition, and repository tests/searches show no remaining PG parallel lifecycle or absence-as-completion logic.
