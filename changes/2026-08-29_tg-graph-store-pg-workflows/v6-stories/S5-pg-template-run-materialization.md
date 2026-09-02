# Change Story: PG template compilation and run materialization

**Contract:** hamy-change-story/v6
**Change:** 2026-08-29_tg-graph-store-pg-workflows
**Story ID:** S5
**Version:** 1.0
**Status:** Complete
**Foundation version:** 3.0
**Foundation fingerprint:** bccd4078bedbeba0cd7160ad4f35076c794e96cd527fea2b4bcd44396b7e373e
**RFC version:** 2.0
**RFC fingerprint:** c1b5994147a42f73db575631f14409e6a15518fb861f54854cd1ebbc062dbf2d

## Outcome

Phase Golem can compile its default or a user-defined template into a safe, snapshot-stable TG graph for a new workflow run.

## Scope

- **Phase Golem:** Extend the current configuration/template surface around `src/config.rs` to select a usable default or validate a user-defined template and compile its complete declared graph before writing.
- **Phase Golem:** Materialize one independently schedulable outcome or human decision per declared node through TG graph application, with PG-owned run identity, template node keys, provenance, logical executor profile, and public execution-policy snapshots in opaque metadata.
- **Phase Golem:** Keep credentials and secrets external to TG and template snapshots; preserve generic TG ownership of IDs, edges, metadata storage, and lifecycle.

## Applicable Designs

-

## Depends on

- S4 version 1.0

## Acceptance

- With no custom template, the preconfigured default compiles and applies; a valid user-defined template also compiles and applies; malformed or unresolved templates write no TG items or edges.
- The materialized graph contains the declared generic items and edges, each node has a distinct TG UUID, and opaque metadata records the PG run identity, repeated template-local node key, provenance, logical executor profile, and public execution policy without secrets.
- Editing the source template or PG defaults after one run does not change that run's snapshots, while a later run receives the edited values; multiple runs coexist with distinct PG run identities and TG UUID mappings.
