# Change Story: PG materialization duplicate prevention and recovery

**Contract:** hamy-change-story/v6
**Change:** 2026-08-29_tg-graph-store-pg-workflows
**Story ID:** S6
**Version:** 1.0
**Status:** Complete
**Foundation version:** 3.0
**Foundation fingerprint:** bccd4078bedbeba0cd7160ad4f35076c794e96cd527fea2b4bcd44396b7e373e
**RFC version:** 2.0
**RFC fingerprint:** c1b5994147a42f73db575631f14409e6a15518fb861f54854cd1ebbc062dbf2d

## Outcome

Phase Golem prevents duplicate materialization for one run and reconstructs a committed run after an uncertain TG graph-apply response.

## Scope

- **Phase Golem:** Add PG-owned coordination around the S5 materialization path so one run identity has one serialized materialization decision.
- **Phase Golem:** Query TG's generic opaque metadata contract before materialization and after an uncertain response, classify complete, absent, and inconsistent discovered state, and reconstruct the complete template-node-to-UUID mapping in PG.
- **Phase Golem:** Keep TG graph application non-idempotent and keep all duplicate prevention, retry decisions, and recovery interpretation in PG.

## Applicable Designs

-

## Depends on

- S3 version 1.0
- S5 version 1.0

## Acceptance

- A second materialization request for an already complete run returns the reconstructed existing mapping and does not invoke TG graph application or create another graph.
- A simulated uncertain response is resolved by a generic TG metadata query: complete state is reconstructed, absent state permits one safe application, and inconsistent or partial state stops with a deterministic recovery error rather than blindly retrying.
- Concurrent materialization decisions for one PG run are serialized, while separate runs may coexist; TG remains unaware of PG run identity, node keys, duplicate prevention, and recovery semantics.
