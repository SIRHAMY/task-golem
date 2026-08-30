# Change Foundation: Task Golem graph store and Phase Golem workflows

**Contract:** hamy-change-foundation/v6
**ID:** 2026-08-29_tg-graph-store-pg-workflows
**Version:** 2.0
**Fingerprint:** 2d828118629c6ce9b874287be6de98555b79e375852f941923b8132b1e27cb11
**Status:** Confirmed
**Created:** 2026-08-29

## Outcome

Agent, API, and CLI consumers can durably manage generic task graphs in Task Golem, including item-level changes and atomic whole-graph application, while Phase Golem owns template-driven workflow compilation and execution policy over those graphs.

## Requirements

### R1: Task Golem consumers can manage durable generic task graphs

- **R1.1: Item-level operations remain available** [Must] Consumers can create, read, update, transition, query, and remove individual TG items without using whole-graph application.
- **R1.2: Generic graph state remains available** [Must] TG persists and exposes parent and dependency edges, generic metadata, events, claims, statuses, and queries without assigning workflow meaning to them.
- **R1.3: Item identities are canonical UUIDv7s** [Must] Every new TG item has one globally unique full canonical UUIDv7. Consumers may use UUIDv7 time ordering but otherwise treat IDs as opaque and never derive workflow meaning from them.
- **R1.4: Identity resolution is exact** [Must] TG accepts full canonical UUIDs only; it does not generate or resolve legacy prefixes, custom prefixes, shortened IDs, or mixed identifier formats.

### R2: Task Golem consumers can apply a complete graph safely

- **R2.1: Whole-graph application is generic** [Must] A consumer can submit all requested generic items, parent edges, dependency edges, and opaque metadata for one graph application without workflow-specific fields or semantics.
- **R2.2: Graph application is all-or-nothing** [Must] TG validates the complete request before committing it, and any validation or persistence failure leaves the durable graph unchanged.
- **R2.3: Generic graph invariants are enforced** [Must] TG rejects duplicate identities, invalid or missing references, self-references, parent cycles, dependency cycles, and malformed generic item state with machine-readable diagnostics.
- **R2.4: Safe replay is idempotent** [Must] Replaying the same successful logical application does not duplicate items or edges and returns the same durable identity mapping; an incompatible replay fails without mutation.
- **R2.5: Results are structured and deterministic** [Must] Successes and failures identify the operation outcome, affected or mapped item identities, and stable error categories and details through supported CLI and library contracts.

### R3: Agent, API, and CLI consumers can rely on TG without a human-oriented storage surface

- **R3.1: Supported contracts are automation-first** [Must] TG's CLI, library, and in-process API inputs, outputs, ordering, and errors are structured and deterministic enough for agent and programmatic consumers.
- **R3.2: Existing generic capabilities remain composable** [Should] Consumers can combine item CRUD, graph application, queries, claims, events, and status transitions without adopting a TG workflow abstraction.

### R4: Phase Golem template authors can materialize workflows as TG graphs

- **R4.1: PG owns workflow templates** [Must] Phase Golem accepts user-defined templates and may ship preconfigured templates, including a usable default when no custom template is selected.
- **R4.2: Templates compile to generic TG state** [Must] PG validates and compiles a selected template into generic TG items, parent and dependency edges, and opaque PG-owned metadata, then uses TG's atomic graph application to persist the result.
- **R4.3: Default and custom templates share the same boundary** [Must] Preconfigured and user-defined templates produce executable TG-backed graphs without adding template, Campaign, run, plugin, or agent semantics to TG.
- **R4.4: Runs remain a PG concept** [Must] PG may record run provenance or grouping in opaque TG metadata, but TG stores and queries that data without interpreting it.

### R5: Phase Golem operators can execute workflows over TG-owned state

- **R5.1: PG executes from TG** [Must] PG discovers graph work through TG, selects and claims items, supervises agent or plugin execution, and explicitly transitions each affected TG item.
- **R5.2: PG owns execution policy** [Must] PG determines context, retries, budgets, stop conditions, gates, concurrency, scheduling, and trigger behavior.
- **R5.3: Triggers wake PG** [Should] Operators can start PG in the foreground or manually and can arrange scheduled wakeups such as cron without requiring TG to schedule or execute work.
- **R5.4: No status rolls up automatically** [Must] Completing a child never changes a parent, container, or root item automatically; PG or another consumer transitions every item according to its own policy.
- **R5.5: Existing PG machinery is retained where generic** [Should] PG continues to provide reusable configuration, scheduling, agent execution, and supervision behavior while narrowing persistence and graph mutation around TG's generic contracts.

## Boundaries

- **Included:** TG item persistence and CRUD; canonical UUIDv7 identity; parent and dependency graph state; generic metadata, events, claims, statuses, and queries; generic atomic whole-graph application; structured CLI, library, and in-process API contracts; PG-owned templates, compilation, runs, work selection, agent supervision, execution policy, and trigger behavior; refactoring useful generic validation and atomic-apply mechanisms from TG-009 into the TG boundary; cross-repository TG and PG integration and regression coverage.
- **Excluded:** TG-owned workflow templates, Campaign or run semantics, plugin or agent execution, iteration, scheduling, retry, gate, trigger, context, budget, or concurrency policy; automatic parent, container, or root rollup; migration of existing IDs or persisted data; compatibility with legacy, prefixed, shortened, mixed-format, or custom-prefix IDs; preserving custom ID prefix or length configuration; workflow semantics encoded in or parsed from UUIDv7 IDs; a new network service or network API; human-readable storage files, a UI, or other human-first views; a broader persistence-platform rewrite.

## Appetite

- **Effort:** A bounded cross-repository refactor delivered through several focused build cycles, not a platform rewrite.
- **Surface:** Task Golem identity, model, graph validation, persistence, item CRUD, graph-application contracts, CLI/library output, TG-009 workflow code, and tests; Phase Golem template/configuration, TG adapter/coordinator integration, scheduler/executor boundaries, and tests.
- **Escalation:** Return to the human before changing the TG/PG ownership boundary, adding migration or legacy compatibility, changing UUIDv7 identity semantics, requiring a network service, weakening atomicity or idempotency, or expanding into a general workflow-platform rewrite.

## Verification

- Exercise TG CLI and library item-level create, read, update, status/claim transition, query, and remove paths after the refactor; prove they work independently of graph application and return deterministic structured output where supported.
- Apply a valid multi-level graph containing parent edges, dependency edges, and opaque metadata; prove all requested state appears together and the structured result maps every requested node to a canonical full UUID.
- Exercise invalid missing references, self-references, duplicate identities, parent cycles, and dependency cycles; prove each returns a stable machine-readable error and leaves active, archived, event, and related durable state byte-for-byte or semantically unchanged as appropriate.
- Exercise a failure during graph persistence; prove no partial item, edge, metadata, or event mutation remains.
- Replay the same successful graph application; prove no duplicate state is created and the same identity mapping is returned. Replay an incompatible application under the same logical identity; prove it fails without mutation.
- Prove generated IDs are unique full canonical UUIDv7s with valid version and variant fields, exact full UUID lookup succeeds, and shortened, prefixed, legacy, malformed, non-v7, and mixed-format IDs are rejected. Prove time ordering is available without workflow parsing, and TG `id_prefix`/`id_len` plus PG project-prefix configuration and behavior are removed rather than ignored or retained as compatibility aliases.
- Compile and apply a PG preconfigured default template and at least one user-defined template; prove both become only generic TG items/edges plus opaque metadata, malformed templates write nothing, and repeated compilation/application is safely idempotent.
- Run representative PG work from TG through selection, claim, agent execution, retries or gates, and explicit status transitions; prove PG remains the interpreter of templates, runs, provenance, and execution policy while TG does not branch on PG metadata.
- Complete child work and prove parent, container, and root statuses do not change until PG explicitly transitions each one; cover the same no-rollup behavior through direct TG item operations.
- Prove foreground/manual PG execution and a scheduled-wakeup path can invoke the same PG-owned workflow behavior without adding scheduling behavior to TG.
- Remove or relocate TG-009 workflow policy so repository searches and tests show no TG-owned template, Campaign/run, plugin/agent, scheduling/retry/gate, or rollup behavior remains, while reusable generic graph validation and atomic application behavior remains covered.
- Run `just check` in Task Golem; run Phase Golem formatting, clippy with warnings denied, and full tests; run `git diff --check` in both repositories.

## Context

- TG's durable generic item shape is `src/model/item.rs::Item`; it already carries status, claims, dependencies, parent identity, and flattened `x-*` metadata, with transitions implemented by `Item::apply_do`, `apply_done`, `apply_block`, `apply_unblock`, and `apply_todo`.
- Generic graph mechanics already exist in `src/model/deps.rs::{would_create_cycle, detect_all_cycles, would_create_parent_cycle, detect_all_parent_cycles, validate_parent}` and `src/model/parent.rs::reparent`.
- TG persistence and locking currently center on `src/store/mod.rs::Store::{with_lock, load_active, save_active, commit_status_change, commit_done}`; item-level CLI behavior is under `src/cli/commands/` and structured rendering under `src/cli/output.rs`.
- Legacy identity behavior is concentrated in `src/model/id.rs::{generate_id_with_prefix, resolve_id}` and `src/store/config.rs::Config::{id_prefix, id_len}`. This Change intentionally replaces it rather than migrating or supporting both forms.
- Historical TG-009 code is implementation input, not product authority: `src/workflow/template.rs::load_workflow_definition`, `src/workflow/instantiate.rs::instantiate_workflow`, and `src/workflow/runner.rs::run_campaign_state_shell` contain reusable validation/application ideas alongside TG-owned template, plugin, Campaign, execution, and `close_eligible_containers` rollup policy that now belongs in PG or is removed.
- PG already persists through TG: `src/pg_item.rs::PgItem` wraps `task_golem::model::item::Item`, and `src/coordinator.rs::CoordinatorState` owns a `task_golem::store::Store`. `changes/WRK-076_migrate-storage-to-task-golem/WRK-076_migrate-storage-to-task-golem_SPEC.md` records the completed storage migration and its no-migration precedent.
- Reusable PG policy machinery already exists in `src/config.rs::{PhaseGolemConfig, default_feature_pipeline}`, `src/scheduler.rs::{select_actions, run_scheduler}`, and `src/executor.rs::{execute_phase, resolve_transition}`. These are the current homes for workflow selection, retries, concurrency, gates, and agent execution.
- Human-confirmed direction states there are no external consumers or persisted identifiers requiring compatibility. No data migration, mixed-format period, custom-prefix preservation, or external rollout is required.
- Human-confirmed identity direction is a clean break to full canonical UUIDv7. UUIDv7 time ordering may be used, but consumers must not parse workflow meaning from IDs.
