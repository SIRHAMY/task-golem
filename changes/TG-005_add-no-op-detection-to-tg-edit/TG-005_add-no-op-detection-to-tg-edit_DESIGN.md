# Design: Add No-Op Detection to `tg edit`

**ID:** TG-005
**Status:** Complete
**Created:** 2026-02-24
**PRD:** ./TG-005_add-no-op-detection-to-tg-edit_PRD.md
**Tech Research:** ./TG-005_add-no-op-detection-to-tg-edit_TECH_RESEARCH.md
**Mode:** Light

## Overview

Use the Snapshot-Compare pattern (clone before mutations, compare after) to detect when `tg edit` produces no actual change. When no change is detected, skip the file write and `updated_at` bump, and return an idempotent response. This mirrors the existing `TransitionResult::Idempotent` pattern in `transition.rs` but uses a local `EditResult` enum scoped to the edit command.

---

## System Design

### High-Level Architecture

Two files change:

1. **`src/model/item.rs`** — Add `PartialEq` to the `Item` derive list (single token addition).
2. **`src/cli/commands/edit.rs`** — Add `EditResult` enum, clone the item before mutations, compare after mutations but before `updated_at` bump, conditionally save or return unchanged.

No new files, no new dependencies, no store-layer changes.

### Component Breakdown

#### `EditResult` enum (new, in `edit.rs`)

**Purpose:** Distinguishes between an applied edit and a no-op, carrying enough data for output formatting.

**Definition:**
```rust
enum EditResult {
    Applied(Item),
    Unchanged { id: String, title: String },
}
```

**Design rationale:**
- `Applied` carries the full `Item` for JSON serialization and human output (matches current behavior).
- `Unchanged` carries only `id` and `title` — the minimum needed to format `"No changes: <id> - <title>"` and `{"idempotent": true}`. No full `Item` clone needed since we're not serializing the item on no-op.
- Parallels `TransitionResult` but is intentionally local — edit idempotency has different semantics (no "target state" concept).

#### `PartialEq` on `Item` (modification in `item.rs`)

**Purpose:** Enables structural equality comparison via `==`.

**Change:** Add `PartialEq` to the existing derive list: `#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]`.

**Why this works:** All 14 fields use types that already implement `PartialEq`: `String`, `i64`, `Option<String>`, `Vec<String>`, `DateTime<Utc>`, `Status` (already derives `PartialEq`), `Option<Status>`, `Option<DateTime<Utc>>`, `BTreeMap<String, serde_json::Value>`. No manual implementation needed.

### Data Flow

1. Validate title (if provided) — unchanged
2. Find project root, create store — unchanged
3. Acquire lock via `store.with_lock()` — unchanged
4. Load items, resolve ID, find item by index — unchanged
5. **Clone item as snapshot** — `let snapshot = items[item_idx].clone();`
6. Apply all mutations (title → priority → description → deps → tags → extensions) — unchanged
7. **Compare:** `if items[item_idx] == snapshot` — NEW
8. If equal → extract `id`/`title`, return `EditResult::Unchanged` — NEW
9. If different → bump `updated_at`, save, clone, return `EditResult::Applied` — existing logic, now conditional
10. Outside closure: match on `EditResult` for output — modified

### Key Flows

#### Flow: No-Op Edit (no flags or same-value flags)

> User runs `tg edit <id>` with no mutation arguments, or with arguments that match current values.

1. **Lock acquired** — `store.with_lock()` called
2. **Items loaded** — `store.load_active()`
3. **Item found** — resolve ID, locate by index
4. **Snapshot taken** — `let snapshot = items[item_idx].clone()`
5. **Mutations applied** — all are no-ops (no flags, or same values)
6. **Comparison** — `items[item_idx] == snapshot` → `true`
7. **Skip save** — no `updated_at` bump, no `store.save_active()`
8. **Return `Unchanged`** — `EditResult::Unchanged { id, title }`
9. **Output** — JSON: `{"idempotent": true}` / Human: `"No changes: <id> - <title>"`
10. **Exit 0**

**Edge cases:**
- `--add-dep <already-present>` — `push` is guarded by `contains()`, so no change occurs. Comparison catches it.
- `--rm-dep <not-present>` — `retain()` removes nothing. Comparison catches it.
- `--add-tag <already-present>` — same as deps.
- `--rm-tag <not-present>` — same as deps.
- `--set x-key=same-value` — `extensions::apply_sets()` overwrites with identical value. `BTreeMap::PartialEq` catches it.

#### Flow: Actual Edit

> User runs `tg edit <id> --title "New Title"` where the title actually changes.

1. **Snapshot taken** — clone before mutations
2. **Title mutated** — new value differs from snapshot
3. **Comparison** — `items[item_idx] == snapshot` → `false`
4. **Save path** — bump `updated_at`, `store.save_active()`, clone for return
5. **Return `Applied`** — `EditResult::Applied(item)`
6. **Output** — identical to current behavior

#### Flow: Mixed Edit (some fields change, some don't)

> User runs `tg edit <id> --title "Same" --priority 5` where title is unchanged but priority differs.

1. **Snapshot taken**
2. **Title mutation** — no change
3. **Priority mutation** — value changes
4. **Comparison** — `false` (priority differs)
5. **Full save path** — as designed, ANY field difference triggers Applied

#### Flow: Mutation Error

> User runs `tg edit <id> --add-dep <cycle>` causing `TgError::CycleDetected`.

1. **Snapshot taken** (clone happens before mutations)
2. **Dep addition fails** — returns `Err(TgError::CycleDetected(...))`
3. **Error propagates** — closure returns `Err`, comparison never runs
4. **Exit 1** — unchanged from current behavior

---

## Technical Decisions

### Key Decisions

#### Decision: Local `EditResult` enum vs reusing `TransitionResult`

**Context:** The codebase already has `TransitionResult` with `Applied(Box<Item>)` and `Idempotent(Status)`. We could reuse it.

**Decision:** Create a local `EditResult` enum in `edit.rs`.

**Rationale:** Edit idempotency is semantically different from transition idempotency. Transitions have a target state (the `Status`), and their `Idempotent` variant carries the current status. Edits have no single target state — they modify arbitrary fields. The PRD explicitly scopes this as edit-local. Keeping them separate avoids coupling and allows each to evolve independently.

**Consequences:** A small amount of structural duplication (two similar enums). This is acceptable given the semantic difference.

#### Decision: `Unchanged` variant carries `id` + `title`, not full `Item`

**Context:** The `Idempotent` output needs the item's id and title for the human-readable message.

**Decision:** Store `id: String` and `title: String` in the `Unchanged` variant rather than a full `Item`.

**Rationale:** The no-op JSON output is `{"idempotent": true}` (no item data). The human output is `"No changes: <id> - <title>"`. Neither needs the full item. Cloning just id/title from the snapshot avoids an unnecessary full clone.

**Consequences:** If future requirements add item data to no-op output, the variant would need to change. This is acceptable — YAGNI applies.

#### Decision: Compare BEFORE `updated_at` bump

**Context:** The comparison must use `PartialEq` on the full struct. If `updated_at` is bumped first, snapshot and mutated item always differ.

**Decision:** Insert the comparison between the last mutation (extensions) and the `updated_at = Utc::now()` line.

**Rationale:** This is the only correct ordering. The tech research identified this as a critical pitfall.

**Consequences:** The code must be structured so that `updated_at` is only set on the "changed" branch.

### Tradeoffs Accepted

| Tradeoff | We're Accepting | In Exchange For | Why This Makes Sense |
|----------|-----------------|-----------------|---------------------|
| Clone cost | One extra `Item::clone()` per edit (~1KB) | Simple, self-maintaining comparison via derived `PartialEq` | Clone cost is negligible vs file I/O saved on no-ops |
| Local enum | Structural similarity with `TransitionResult` (not shared) | Clean separation of semantically different concepts | Avoids coupling; each enum can evolve independently |
| Ordered Vec comparison | If mutation logic ever reorders elements, false-positives possible | No need for sorted comparison logic | Existing mutation code preserves order; no reordering paths exist |

---

## Alternatives Considered

### Alternative: Dirty Flag

**Summary:** Maintain a boolean `changed` flag, set it to `true` in each mutation branch that actually modifies a value.

**How it would work:**
- Before each mutation, check if the new value differs from current
- Set `changed = true` if it does
- After all mutations, check `changed` to decide whether to save

**Pros:**
- No clone overhead

**Cons:**
- Every mutation site must correctly check and set the flag — easy to forget when adding new fields or mutation types
- Threading the flag through all mutation branches increases complexity
- For extensions (`apply_sets`), the flag would need to be threaded through or returned from the function
- More code, more places to get wrong

**Why not chosen:** The clone cost (~1KB) is negligible for a CLI command. The dirty flag adds complexity and maintenance burden disproportionate to the trivial cost it saves. The Snapshot-Compare pattern is simpler, self-maintaining (new fields auto-participate via `derive(PartialEq)`), and less error-prone.

---

## Risks

| Risk | Impact | Likelihood | Mitigation |
|------|--------|------------|------------|
| `PartialEq` comparison includes `updated_at` | If comparison happens after `updated_at` bump, no-op never triggers | Low — design explicitly orders comparison before bump | Code review; the design doc and tech research both flag this ordering |
| New `Item` field added without `PartialEq` | Compilation error if field type doesn't impl `PartialEq` | Very Low — standard types all impl it | Compiler will catch it; `derive(PartialEq)` fails at compile time |
| Vec ordering changes in future mutation logic | Semantically-equal items compare as unequal (false negative — edit applied unnecessarily) | Very Low — benign failure mode | False negative just means an unnecessary save, not data corruption |

---

## Integration Points

### Existing Code Touchpoints

- `src/model/item.rs:21` — Add `PartialEq` to `#[derive(Debug, Clone, Serialize, Deserialize)]`
- `src/cli/commands/edit.rs` — Add `EditResult` enum (top of file), restructure `run()` to clone-compare-conditionally-save, update output handling to match on `EditResult`

### External Dependencies

None. All types (`PartialEq`, `Clone`) are from `std`.

---

## Open Questions

None. The design is fully specified and ready for SPEC.

---

## Design Review Checklist

Before moving to SPEC:

- [x] Design addresses all PRD requirements
- [x] Key flows are documented and make sense
- [x] Tradeoffs are explicitly documented and acceptable
- [x] Integration points with existing code are identified
- [x] No major open questions remain

---

## Design Log

| Date | Activity | Outcome |
|------|----------|---------|
| 2026-02-24 | Initial design draft | Snapshot-Compare with local EditResult enum; all flows documented |
| 2026-02-24 | Self-critique (7 agents) | No issues found — design is minimal, well-aligned, and complete |
