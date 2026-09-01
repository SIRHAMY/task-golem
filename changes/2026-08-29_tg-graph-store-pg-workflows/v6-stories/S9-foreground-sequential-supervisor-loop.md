# Change Story: Foreground sequential supervisor loop

**Contract:** hamy-change-story/v6
**Change:** 2026-08-29_tg-graph-store-pg-workflows
**Story ID:** S9
**Version:** 1.0
**Status:** Ready
**Foundation version:** 3.0
**Foundation fingerprint:** bccd4078bedbeba0cd7160ad4f35076c794e96cd527fea2b4bcd44396b7e373e
**RFC version:** 2.0
**RFC fingerprint:** c1b5994147a42f73db575631f14409e6a15518fb861f54854cd1ebbc062dbf2d

## Outcome

Phase Golem provides one finite foreground drain-and-exit loop that executes newly discovered work and reports explicit stop reasons.

## Scope

- **Phase Golem:** Replace the parallel scheduling path in `src/scheduler.rs` and its `run` entrypoint with one sequential cycle of TG snapshot, ready selection, claim, supervised execution, explicit transition, and repeat.
- **Phase Golem:** Handle idle, ready human gate, declared budget/cap, unrecoverable failure, shutdown, and completed-scope exits deterministically, while consuming CRUD-created discovered work on a later cycle.
- **Phase Golem:** Route manual invocation and external scheduled wakeups through the same finite loop; do not add a daemon, internal scheduler, worker pool, remote-control surface, or concurrency coordination.

## Applicable Designs

-

## Depends on

- S8 version 1.0

## Acceptance

- With a fake executor and representative TG graphs, at most one task is executing at a time, dependency readiness gates selection, discovered CRUD-created work is picked up, and every selected task follows claim, execution, verification, and explicit transition.
- Focused loop tests produce stable stop reasons for idle, human gate, budget/cap, unrecoverable failure, shutdown, and completed selected scope, without changing parent/root statuses automatically.
- The manual command and an externally scheduled wakeup invoke the same finite supervisor behavior, and the implementation contains no daemon, internal scheduler, parallel worker pool, or remote-control path.
