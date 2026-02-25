# SPEC: Add No-Op Detection to `tg edit`

**ID:** TG-005
**Status:** Ready
**Created:** 2026-02-24
**PRD:** ./TG-005_add-no-op-detection-to-tg-edit_PRD.md
**Execution Mode:** autonomous
**New Agent Per Phase:** no
**Max Review Attempts:** 3

## Context

`tg edit` currently always rewrites `tasks.jsonl` and bumps `updated_at`, even when no fields actually change. The transition commands (`tg do`, `tg done`, `tg todo`, `tg block`) already have idempotent detection via `TransitionResult::Idempotent`. This change brings the same pattern to `tg edit` using a Snapshot-Compare approach: clone the item before mutations, compare after, and conditionally skip the save.

## Approach

Use derived `PartialEq` on `Item` to enable structural comparison. Add a local `EditResult` enum to `edit.rs` (paralleling `TransitionResult` in `transition.rs`). Inside the `store.with_lock()` closure, clone the item before mutations, apply all mutations, compare against the snapshot, and either return `Unchanged` (skip save) or `Applied` (bump `updated_at`, save, return item). Output handling matches on `EditResult` to produce the appropriate JSON or human output.

**Patterns to follow:**

- `src/cli/commands/transition.rs` — `TransitionResult` enum pattern (Applied vs Idempotent), `output_transition()` for dual-mode output
- `src/cli/commands/edit.rs` — existing mutation flow (title → priority → description → deps → tags → extensions), closure-based locking
- `tests/idempotent_test.rs` — test structure for idempotent JSON assertions (`{"idempotent": true}`)

**Implementation boundaries:**

- Do not modify: `src/store/` (no store-layer changes)
- Do not modify: `src/cli/commands/transition.rs` (separate concern, no shared enum)
- Do not refactor: existing mutation logic in `edit.rs` (leave as-is, wrap with snapshot/compare)

## Phase Summary

| Phase | Name | Complexity | Description |
|-------|------|------------|-------------|
| 1 | Core Logic | Low | Add `PartialEq` to `Item`, add `EditResult` enum, restructure `edit::run()` with snapshot-compare and conditional save |
| 2 | Integration Tests | Low | Add integration tests for no-op, applied, mixed, and error scenarios |

**Ordering rationale:** Phase 2 tests depend on the logic implemented in Phase 1. Tests are separated into their own phase because they are integration tests in a separate file, not unit tests inline with the implementation.

---

## Phases

---

### Phase 1: Core Logic

> Add `PartialEq` to `Item`, add `EditResult` enum, restructure `edit::run()` with snapshot-compare and conditional save

**Phase Status:** not_started

**Complexity:** Low

**Goal:** Implement no-op detection so that `tg edit` skips the file write and `updated_at` bump when no fields actually change, returning an idempotent response instead.

**Files:**

- `src/model/item.rs` — modify — add `PartialEq` to `#[derive(...)]` on `Item` struct (line 21)
- `src/cli/commands/edit.rs` — modify — add `EditResult` enum, add snapshot clone, add comparison logic, restructure save/output to be conditional

**Patterns:**

- Follow `src/cli/commands/transition.rs:9-14` for enum definition pattern (`TransitionResult`)
- Follow `src/cli/commands/transition.rs:16-36` for dual-mode output handling (`output_transition`)

**Tasks:**

- [ ] Add `PartialEq` to the derive list on `Item` struct in `src/model/item.rs:21` — change `#[derive(Debug, Clone, Serialize, Deserialize)]` to `#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]`
- [ ] Define `EditResult` enum at the top of `src/cli/commands/edit.rs` with variants `Applied(Item)` and `Unchanged { id: String, title: String }`
- [ ] Add snapshot clone inside the `with_lock` closure, after item lookup but before first mutation: `let snapshot = items[item_idx].clone();`
- [ ] Insert comparison after the last mutation (`extensions::apply_sets`) but BEFORE `updated_at = Utc::now()`: `if items[item_idx] == snapshot { return Ok(EditResult::Unchanged { ... }); }`
- [ ] Make `updated_at` bump, `save_active`, and item clone conditional on the "changed" branch, returning `EditResult::Applied(updated)`
- [ ] Update output handling after the closure to match on `EditResult`: `Applied` prints the item (existing behavior), `Unchanged` prints `"No changes: <id> - <title>"` (human) or `{"idempotent": true}` (JSON)
- [ ] Add inline comment at the comparison point: `// CRITICAL: Compare BEFORE updated_at bump — comparing after would always differ`

**Verification:**

- [ ] `cargo build` succeeds with no errors
- [ ] `cargo test` passes (all existing tests still pass — no regressions)
- [ ] Manual test: `tg edit <id>` with no flags → human output: `"No changes: <id> - <title>"`, JSON output: `{"idempotent": true}`, exit code 0
- [ ] Manual test: `tg edit <id> --title "New"` with a different title → full item output with bumped `updated_at`, exit code 0
- [ ] Manual test: `tg edit <id>` with no flags → verify `updated_at` timestamp is unchanged (compare `tg show` before and after)
- [ ] Code review passes — verify comparison is placed after all mutations but before `updated_at = Utc::now()`

**Commit:** `[TG-005][P1] Feature: Add no-op detection to tg edit`

**Notes:**

Critical ordering: the comparison `items[item_idx] == snapshot` MUST happen AFTER all mutations but BEFORE `updated_at = Utc::now()`. If comparison happens after the timestamp bump, snapshot and mutated item always differ, and no-op detection never triggers.

**Followups:**

---

### Phase 2: Integration Tests

> Add integration tests covering no-op, applied, mixed, and error scenarios

**Phase Status:** not_started

**Complexity:** Low

**Goal:** Add comprehensive integration tests verifying no-op detection for all field types, mixed edits, timestamp behavior, and error paths.

**Files:**

- `tests/edit_noop_test.rs` — create — integration tests for edit no-op detection

**Patterns:**

- Follow `tests/idempotent_test.rs` for idempotent assertion patterns (`json["idempotent"]`, exit code checks, human output assertions)
- Follow `tests/edit_test.rs` for edit-specific test setup (`TestProject::new()`, `run_tg_json()`)

**Tasks:**

- [ ] Create `tests/edit_noop_test.rs` with `mod common;` and `use common::TestProject;`
- [ ] Test: no-op with no mutation flags — `tg edit <id>` returns `{"idempotent": true}` in JSON mode, exit code 0
- [ ] Test: no-op with same-value title — `tg edit <id> --title <same>` returns idempotent
- [ ] Test: no-op with same-value priority — `tg edit <id> --priority <same>` returns idempotent
- [ ] Test: no-op with same-value description — `tg edit <id> --description <same>` returns idempotent
- [ ] Test: no-op with add-dep already present — `tg edit <id> --add-dep <existing>` returns idempotent
- [ ] Test: no-op with rm-dep not present — `tg edit <id> --rm-dep <nonexistent>` returns idempotent
- [ ] Test: no-op with add-tag already present — `tg edit <id> --add-tag <existing>` returns idempotent
- [ ] Test: no-op with rm-tag not present — `tg edit <id> --rm-tag <nonexistent>` returns idempotent
- [ ] Test: no-op with same extension value — `tg edit <id> --set x-key=<same>` returns idempotent
- [ ] Test: applied when at least one field changes in mixed edit — `tg edit <id> --title <same> --priority <different>` returns full item
- [ ] Test: `updated_at` is NOT bumped on no-op (compare timestamps before and after)
- [ ] Test: human mode output contains `"No changes:"` on no-op
- [ ] Test: mutation error (cycle detection) still returns exit 1, no-op detection does not run

**Verification:**

- [ ] `cargo test` passes (all new and existing tests)
- [ ] No-op tests cover all field types listed in PRD success criteria
- [ ] Code review passes

**Commit:** `[TG-005][P2] Test: Add integration tests for edit no-op detection`

**Notes:**

**Followups:**

---

## Final Verification

- [ ] All phases complete
- [ ] All PRD success criteria met
- [ ] Tests pass
- [ ] No regressions introduced
- [ ] Code reviewed (if applicable)

## Execution Log

| Phase | Status | Commit | Notes |
|-------|--------|--------|-------|

## Followups Summary

### Critical

### High

### Medium

### Low

## Design Details

### Key Types

```rust
// Local to src/cli/commands/edit.rs
enum EditResult {
    /// Edit was applied — at least one field changed.
    Applied(Item),
    /// No fields changed — skip save, no updated_at bump.
    Unchanged { id: String, title: String },
}
```

### Architecture Details

The implementation uses the Snapshot-Compare pattern:

1. **Snapshot:** Clone the `Item` before any mutations — `let snapshot = items[item_idx].clone()`
2. **Mutate:** Apply all mutations in existing order (title → priority → description → deps → tags → extensions)
3. **Compare:** After all mutations, before `updated_at` bump — `items[item_idx] == snapshot`
4. **Branch:**
   - Equal → extract `id`/`title`, return `EditResult::Unchanged` (no save, no timestamp bump)
   - Different → bump `updated_at`, save, clone, return `EditResult::Applied`

All steps happen inside the `store.with_lock()` closure, maintaining atomicity under concurrent access.

### Design Rationale

- **`PartialEq` derive vs dirty flag:** Derived `PartialEq` is simpler, self-maintaining (new fields auto-participate), and the clone cost (~1KB) is negligible for a CLI command. A dirty flag would require manual tracking in every mutation site.
- **Local `EditResult` vs shared `TransitionResult`:** Edit idempotency is semantically different (no target state concept). Keeping enums separate avoids coupling.
- **`Unchanged` carries `id`+`title`, not full `Item`:** The no-op output needs only id and title. Avoids unnecessary full clone on the no-op path.
- **Comparison before `updated_at`:** The only correct ordering — comparing after would always show a difference due to the timestamp.

---

## Retrospective

[Fill in after completion]

### What worked well?

### What was harder than expected?

### What would we do differently next time?
