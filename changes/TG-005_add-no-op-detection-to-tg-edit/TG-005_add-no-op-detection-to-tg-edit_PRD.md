# Change: Add No-Op Detection to `tg edit`

**Status:** Proposed
**Created:** 2026-02-24
**Author:** phase-golem (autonomous)

## Problem Statement

`tg edit` currently always rewrites `tasks.jsonl` and updates the `updated_at` timestamp, even when no fields actually change. Running `tg edit tg-abc12` with no mutation arguments (or with arguments that match the current values) still acquires the lock, loads all items, bumps `updated_at`, and atomically rewrites the entire file. This causes:

1. **Misleading timestamps** — `updated_at` changes without any meaningful modification, making it unreliable for agents or humans trying to determine when an item was last actually edited.
2. **Unnecessary I/O** — The full active store is rewritten atomically (tempfile → fsync → rename) even when nothing changed, wasting I/O on a no-op.
3. **Inconsistency with transitions** — The transition commands (`tg do`, `tg done`, `tg todo`, `tg block`) already have idempotent (same operation, same result, no side effects) detection via `TransitionResult::Idempotent`, returning `{"idempotent": true, ...}` in JSON mode and `"Already <status> (no change)"` in human mode. `tg edit` lacks this pattern.

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
- [ ] If ANY mutation results in a field change, the command returns Applied (full item output, `updated_at` bumped, file written). Idempotent is returned only when ALL fields remain unchanged after applying all mutations
- [ ] JSON mode (`--json`) returns `{"idempotent": true}` on no-op (note: no `previous_state` field, since edit has no single target state unlike transitions)
- [ ] Human mode prints `"No changes: <id> - <title>"` on no-op, paralleling the existing `"Updated item: <id> - <title>"` format
- [ ] Exit code is 0 for no-op (not an error)
- [ ] When at least one field changes, behavior is identical to current (file write, `updated_at` bump, normal output)
- [ ] If any mutation fails (e.g., cycle detection, invalid input), the entire edit is rejected with exit code 1 — no partial state is saved, and no-op detection does not run

### Should Have

- [ ] Integration tests covering the no-op cases (no args, same-value args, mixed no-op and change args where one field changes and another doesn't)
- [ ] Test verifying `updated_at` is NOT bumped on no-op

### Nice to Have

- [ ] No additional allocations or cloning beyond what's needed for comparison

## Scope

### In Scope

- No-op detection logic in `src/cli/commands/edit.rs`
- Comparison of item state before and after applying edit arguments
- Idempotent output format (JSON and human)
- Integration tests for no-op scenarios
- Adding `PartialEq` derive to `Item` struct if needed for comparison

### Out of Scope

- Changing the `TransitionResult` enum or sharing it with `edit.rs` (edit can use its own mechanism)
- Adding no-op detection to `tg add` or other commands
- Changing the JSONL store layer (this is purely a command-level change)
- Optimizing the store write path (e.g., diffing before write) — the fix is to skip the write entirely
- Clearing `description` to `None` (no mechanism exists for that today; not introduced by this change)
- Validation of existing data integrity (e.g., pre-existing corrupt dependency graphs)
- Extension nesting depth limits (orthogonal concern, not introduced by this change)

## Non-Functional Requirements

- **Performance:** No-op detection adds one `Item` clone (before mutations) and one field-by-field comparison. Given items are small structs (typically < 1KB serialized), this is negligible compared to the file I/O cost of a full rewrite.

## Constraints

- Must not change the public behavior for actual edits (exit code, output format, file write semantics)
- Single-file change in `edit.rs` (plus test file, and possibly adding `PartialEq` derive in `item.rs`)
- The entire sequence — load, clone, mutate, compare, conditional save — must happen within the `store.with_lock()` closure to maintain atomicity under concurrent access

## Dependencies

- **Depends On:** Nothing — `tg edit` and the idempotent pattern both exist today
- **Blocks:** Nothing

## Risks

- [ ] Comparing `Item` structs for equality requires `PartialEq`. The `Item` struct currently derives `Debug, Clone, Serialize, Deserialize` but not `PartialEq`. Mitigation: add `PartialEq` derive. The comparison happens before `updated_at = Utc::now()`, so `updated_at` is still at its original value and compares correctly. All contained types (`String`, `Vec<String>`, `Option<String>`, `DateTime<Utc>`, `Status`, `BTreeMap<String, serde_json::Value>`) already implement `PartialEq`.

## Open Questions

None — the problem, solution approach, and existing pattern are all well-understood.

## Assumptions

- **Mode:** Light — this is a small, clearly-scoped change with a known solution pattern already in the codebase.
- **Comparison strategy:** Clone the item before mutation, compare after mutation but before `updated_at` update. This is the simplest correct approach. Both the clone and comparison happen inside the `with_lock` closure, so there is no TOCTOU window.
- **Ordering semantics:** Dependencies (`Vec<String>`) and tags (`Vec<String>`) use ordered comparison via `PartialEq`. This is correct because the existing mutation logic preserves order: `--add-dep` appends (but skips if `contains()`), `--rm-dep` uses `retain()`, `--add-tag` appends (but skips if `contains()`), `--rm-tag` uses `retain()`. No reordering occurs unless actual changes happen.
- **Atomic write guarantee:** The store layer's existing tempfile → fsync → rename pattern ensures `tasks.jsonl` is never left in a partially-written state. The no-op optimization simply skips this write when not needed; it does not change the guarantee.
- **No human available:** PRD created autonomously based on triage assessment and codebase analysis. All decisions reflect existing codebase patterns.

## References

- Existing idempotent pattern: `src/cli/commands/transition.rs` (`TransitionResult::Idempotent`)
- Current edit implementation: `src/cli/commands/edit.rs`
- Existing edit tests: `tests/edit_test.rs`
- Existing idempotent tests: `tests/idempotent_test.rs`
