# Change: Migrate BACKLOG.yaml to task-golem

**Status:** Proposed
**Created:** 2026-04-15
**Author:** HAMY

## TL;DR

- **Problem:** The repo's backlog lives in `BACKLOG.yaml` (phase-golem format) from before task-golem could dogfood itself. Now that TG-006 ships the query layer, task-golem is feature-complete enough to own its own backlog.
- **Solution:** One-shot migration: init `.task-golem/` in this repo, copy live items from `BACKLOG.yaml` as fresh tg records (new content-hash IDs, phase-golem metadata preserved in `x-` extensions), prune done items, delete `BACKLOG.yaml`.
- **Key criteria:** (1) All non-done items carry over with title + original timestamps + provenance extensions; (2) `tg list` and `tg query` work against the new project immediately; (3) `BACKLOG.yaml` is removed from the repo.
- **Needs attention:** None — directional decisions locked in below.

## Problem Statement

This repo's backlog lives in `BACKLOG.yaml` (phase-golem's schema v3) and predates task-golem dogfooding. The file has 23 historical items (TG-001–TG-023 in phase-golem's numbering, distinct from the `changes/TG-NNN_` folder space). Five are done, three are in-flight, fifteen are new.

Now that TG-006 shipped the parent field + `tg query`, task-golem has enough surface (nesting, tags, deps, query) to own the backlog itself. Keeping two systems in parallel is friction.

## User Stories / Personas

- **Maintainer (HAMY)** — wants a single source of truth for "what's next on task-golem" using task-golem itself, so dogfooding stays honest.
- **Future agents working the backlog** — want a tg project they can query (`tg next`, `tg query "SELECT * FROM task_view WHERE is_ready = 1"`) instead of parsing YAML.

## Desired Outcome

After this change:

1. `tg init` has been run at the repo root, producing `.task-golem/tasks.jsonl`, `archive.jsonl`, `tasks.lock`, and `.gitignore`.
2. Every non-done item from `BACKLOG.yaml` exists as a tg record with a fresh content-hash ID. The old `TG-NNN` numeric ID is preserved as `x-legacy-id`. Phase-golem metadata (`size`, `complexity`, `risk`, `impact`, `origin`, `phase`, `pipeline_type`, `phase_pool`, `last_phase_commit`, `requires_human_review`) is preserved as `x-*` extensions.
3. Original `created` timestamps survive exactly. `updated_at` is refreshed to the migration run time (simpler than preserving; the original is retained in an `x-legacy-updated` extension if anyone needs it).
4. Done items are NOT migrated — they're dropped entirely.
5. `BACKLOG.yaml` is deleted from the repo in the same commit.
6. `phase-golem.toml` is updated or removed if its references to `BACKLOG.yaml` break.
7. `tg list` shows the migrated items; `tg doctor` reports clean.

## Success Criteria

### Must Have

- [ ] `.task-golem/` initialized at repo root with cache gitignore entries.
- [ ] All non-done items from `BACKLOG.yaml` present in `tasks.jsonl` with titles intact.
- [ ] Each migrated item carries `x-legacy-id` (the original `TG-NNN` string).
- [ ] Each migrated item carries `x-origin` and any other phase-golem metadata fields that were populated.
- [ ] Original `created` timestamps preserved exactly (not the migration run time).
- [ ] Status maps correctly: `new → todo`, `in_progress → doing`.
- [ ] `BACKLOG.yaml` deleted from the working tree.
- [ ] `tg list` and `tg doctor` succeed against the new project.

### Should Have

- [ ] Status mapping decision captured in PRD (done above: `new → todo`, `in_progress → doing`, done items dropped).
- [ ] Priority derivation rule captured (default `0` for all migrated items; `impact` extension is informational only).
- [ ] A short `MIGRATION_NOTES.md` or equivalent commit message explains how to map `x-legacy-id` back to the old YAML if needed.

### Nice to Have

- [ ] `phase-golem.toml` updated to remove the BACKLOG.yaml reference if it exists and is orphaned.
- [ ] Verify no references to `BACKLOG.yaml` remain in `.claude/`, `README.md`, or other docs.

## Scope

### In Scope

- Initializing `.task-golem/` at the repo root.
- Writing a one-shot migration script (Rust binary under `tools/` or a standalone shell/`jq` script — implementer's choice) that reads `BACKLOG.yaml` and emits valid tg JSONL.
- Dropping done items entirely.
- Preserving `TG-NNN` legacy IDs + phase-golem metadata as `x-` extensions.
- Deleting `BACKLOG.yaml`.
- Single commit for the migration (or two: tooling commit + data commit — implementer's judgment).

### Out of Scope

- Building a generic `tg import` subcommand. This is a one-shot migration, not infrastructure.
- Preserving `updated_at` exactly (retained as `x-legacy-updated` extension only).
- Cleaning up or consolidating the live `changes/TG-NNN_` folders. Those stay as-is; the `x-legacy-id` on migrated items points to the phase-golem YAML IDs, which were a separate numbering space.
- Renumbering or rewriting any `changes/` folder (the shipped TG-006 stays TG-006).
- Setting priorities based on `impact` — all migrated items default to priority `0`; reprioritization is a manual follow-up.

## Non-Functional Requirements

- **Correctness:** `tg doctor` must report zero issues post-migration. The tool passes all integrity checks on the produced JSONL.
- **Idempotence:** Running the migration twice must not double-insert. Simplest path: the migration refuses to run if `.task-golem/` already exists, forcing a clean rerun.

## Constraints

- Must use fresh tg content-hash IDs (user decision); legacy IDs survive only as `x-legacy-id`.
- Must preserve original `created` timestamps exactly — tg's `Item::created_at` field accepts any `DateTime<Utc>`, so this is supported.
- Extensions must be `x-` prefixed per `src/model/item.rs::validate_extensions` (enforced on every save).

## Dependencies

- **Depends On:** TG-006 (query layer + parent field) is shipped on `main` — done as of commit `a32a674`. Migration doesn't strictly need TG-006, but we want the migrated items queryable from day one.
- **Blocks:** Nothing currently. Future agent-driven backlog work benefits from this but isn't gated.

## Risks

- [ ] **Content-hash ID collisions with existing `changes/TG-NNN_` folders are impossible** (content hashes are base32, folder IDs are `TG-NNN`). No real risk here, noted for completeness.
- [ ] **Lost `updated_at` provenance** — mitigated by preserving as `x-legacy-updated`. Low severity.
- [ ] **Phase-golem workflow breaks if it still reads `BACKLOG.yaml`** — check `.phase-golem/` and `phase-golem.toml` references before deletion. If phase-golem still expects the YAML, document a transition plan or delay deletion.

## Open Questions

- [ ] Does `phase-golem` actively read `BACKLOG.yaml` today, or is it purely historical? The implementer should `grep -r BACKLOG.yaml` before deleting. If phase-golem still depends on it, the deletion becomes a follow-up instead.
- [ ] Should the migration tooling (script) be committed to the repo or discarded after the one-shot run? Suggested: commit to `tools/` so reviewers can verify the mapping.

## References

- Source: `BACKLOG.yaml` at repo root (schema_version 3, 23 items).
- Target schema: `src/model/item.rs::Item`.
- Extension validation rules: `src/model/item.rs::validate_extensions` (must be `x-` prefixed).
- Init flow: `src/cli/commands/init.rs`.
- TG-006 commit (parent + query shipped): `a32a674`.
