# Change Story: Trusted PG executor supervision

**Contract:** hamy-change-story/v6
**Change:** 2026-08-29_tg-graph-store-pg-workflows
**Story ID:** S8
**Version:** 1.0
**Status:** Complete
**Foundation version:** 3.0
**Foundation fingerprint:** bccd4078bedbeba0cd7160ad4f35076c794e96cd527fea2b4bcd44396b7e373e
**RFC version:** 2.0
**RFC fingerprint:** c1b5994147a42f73db575631f14409e6a15518fb861f54854cd1ebbc062dbf2d

## Outcome

Phase Golem supervises trusted local executors from claim through verification and applies the resulting TG transition itself.

## Scope

- **Phase Golem:** Evolve `src/config.rs`, `src/agent.rs`, `src/executor.rs`, and coordinator boundaries so templates select logical executor profiles while trusted local adapters/commands and runtime credentials remain PG configuration.
- **Phase Golem:** Claim each selected task through TG before invocation, pass the snapshotted public policy, accept structured `complete` or `blocked` results with summary and evidence references, and run required deterministic verification.
- **Phase Golem:** Record attempt evidence in PG-owned metadata/events and make only the PG supervisor perform TG status transitions; executors receive no direct TG mutation capability.

## Applicable Designs

-

## Depends on

- S5 version 1.0
- S7 version 1.0

## Acceptance

- A fake trusted adapter proves the sequence claim, invoke, structured result validation, deterministic verification, attempt evidence, and one PG-owned TG transition for both complete and blocked outcomes.
- Logical profile resolution works from a materialized snapshot, runtime credentials do not appear in TG metadata or snapshots, and malformed or identity-mismatched executor results are rejected without an unauthorized transition.
- Retry, gate, diagnostic, and provenance information remains PG policy/evidence; an executor cannot call the TG mutation surface directly, and existing reusable agent/process supervision remains behind the trusted PG boundary.
