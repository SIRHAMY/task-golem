# Change: Add No-Op Detection to `tg edit`

**Status:** Proposed
**Created:** 2026-02-24
**Author:** phase-golem (autonomous)

## Problem Statement

`tg edit` currently always rewrites `tasks.jsonl` and updates the `updated_at` timestamp, even when no fields actually change. Running `tg edit tg-abc12` with no mutation arguments (or with arguments that match the current values) still acquires the lock, loads all items, bumps `updated_at`, and atomically rewrites the entire file. This causes:

1. **Misleading timestamps** — `updated_at` changes without any meaningful modification, making it unreliable for agents or humans trying to determine when an item was last actually edited.
2. **Unnecessary I/O** — The full active store is rewritten atomically (tempfile → fsync → rename) even when nothing changed, wasting I/O on a no-op.
3. **Inconsistency with transitions** — The transition commands (`tg do`, `tg done`, `tg todo`, `tg block`) already have idempotent detection via `TransitionResult::Idempotent`, returning `{"idempotent": true, ...}` in JSON mode and `"Already <status> (no change)"` in human mode. `tg edit` lacks this pattern.

## User Stories / Personas

- **AI Agent** — An autonomous agent calling `tg edit` programmatically needs to trust `updated_at` as a reliable signal of actual change. False positives (timestamp bumps with no data change) degrade the agent's ability to reason about task freshness and change history.
- **CLI User** — A developer running `tg edit` should get clear feedback when their edit didn't actually change anything, rather than a misleading "Updated item" message.

## Desired Outcome

When `tg edit` is invoked and the resulting item state is identical to the state before the edit, the command should:
- **Not** update `updated_at`
- **Not** rewrite `tasks.jsonl`
- Return an idempotent/no-op response indicating nothing changed
- Exit with code 0 (not an error)

When `tg edit` is invoked and at least one field actually changes, behavior should remain exactly as it is today — update `updated_at`, save the file, and return the updated item.

## Success Criteria

### Must Have

- [ ] `tg edit <id>` with no mutation flags produces a no-op (no file write, no `updated_at` change)
- [ ] `tg edit <id> --title <same-title>` (value identical to current) produces a no-op
- [ ] Same no-op behavior for all field types: `--priority`, `--description`, `--add-dep` (already present), `--rm-dep` (not present), `--add-tag` (already present), `--rm-tag` (not present), `--set` (same value)
- [ ] JSON mode (`--json`) returns `{"idempotent": true}` on no-op, consistent with transition commands
- [ ] Human mode prints a "no change" message on no-op (e.g., `"No changes: <id> - <title>"`)
- [ ] Exit code is 0 for no-op (not an error)
- [ ] When at least one field changes, behavior is identical to current (file write, `updated_at` bump, normal output)

### Should Have

- [ ] Integration tests covering the no-op cases (no args, same-value args, mixed no-op and change args)
- [ ] Test verifying `updated_at` is NOT bumped on no-op

### Nice to Have

- [ ] No additional allocations or cloning beyond what's needed for comparison

## Scope

### In Scope

- No-op detection logic in `src/cli/commands/edit.rs`
- Comparison of item state before and after applying edit arguments
- Idempotent output format (JSON and human) consistent with `TransitionResult::Idempotent`
- Integration tests for no-op scenarios

### Out of Scope

- Changing the `TransitionResult` enum or sharing it with `edit.rs` (edit can use its own mechanism)
- Adding no-op detection to `tg add` or other commands
- Changing the JSONL store layer (this is purely a command-level change)
- Optimizing the store write path (e.g., diffing before write) — the fix is to skip the write entirely

## Non-Functional Requirements

- **Performance:** No-op detection should add negligible overhead. A clone of the item before mutation for comparison is acceptable given items are small structs.

## Constraints

- Must follow the existing idempotent output pattern from `transition.rs` for JSON mode (`{"idempotent": true}`)
- Must not change the public behavior for actual edits (exit code, output format, file write semantics)
- Single-file change in `edit.rs` (plus test file)

## Dependencies

- **Depends On:** Nothing — `tg edit` and the idempotent pattern both exist today
- **Blocks:** Nothing

## Risks

- [ ] Comparing `Item` structs for equality requires either `PartialEq` derive or manual field comparison. Using `PartialEq` is cleaner but means `updated_at` (which we haven't changed yet at comparison time) must be excluded or the comparison must happen before the timestamp update. Mitigation: compare before the `updated_at = Utc::now()` line, which is the natural position.

## Open Questions

None — the problem, solution approach, and existing pattern are all well-understood.

## Assumptions

- **Mode:** Light — this is a small, clearly-scoped change with a known solution pattern already in the codebase.
- **Comparison strategy:** Clone the item before mutation, compare after mutation but before `updated_at` update. This is the simplest correct approach.
- **No human available:** PRD created autonomously based on triage assessment and codebase analysis. All decisions reflect existing codebase patterns.

## References

- Existing idempotent pattern: `src/cli/commands/transition.rs` (`TransitionResult::Idempotent`)
- Current edit implementation: `src/cli/commands/edit.rs`
- Existing edit tests: `tests/edit_test.rs`
- Existing idempotent tests: `tests/idempotent_test.rs`
