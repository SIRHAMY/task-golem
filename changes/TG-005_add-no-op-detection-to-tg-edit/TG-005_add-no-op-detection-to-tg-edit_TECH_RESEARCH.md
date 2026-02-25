# Tech Research: Add No-Op Detection to `tg edit`

**ID:** TG-005
**Status:** Complete
**Created:** 2026-02-24
**PRD:** ./TG-005_add-no-op-detection-to-tg-edit_PRD.md
**Mode:** Light

## Overview

Researching patterns for detecting when a `tg edit` invocation results in no actual change to an item, so we can skip the file write, preserve `updated_at`, and return an idempotent signal. The codebase already has this pattern for transition commands — this research validates the approach for the multi-field edit case.

## Research Questions

- [x] What is the standard pattern for no-op detection in mutation commands? → Snapshot-Compare (clone-mutate-compare)
- [x] Does the existing codebase have patterns we should follow? → Yes, `TransitionResult::Idempotent` in transition.rs
- [x] Can `PartialEq` be derived on `Item` without issues? → Yes, all field types already implement `PartialEq`

---

## External Research

### Landscape Overview

No-op detection (idempotency detection) is a well-established pattern in software engineering. The core idea: before committing a mutation to persistent storage, compare the post-mutation state against the pre-mutation state and skip the write if nothing actually changed. This pattern appears across CLI tools (Ansible's `changed_when`), configuration management, and UI frameworks.

In Rust, the approach maps cleanly to `Clone` + `PartialEq`: clone before mutation, mutate, compare with `==`, and conditionally skip the write.

### Common Patterns & Approaches

#### Pattern: Snapshot-Compare (Clone-Mutate-Compare)

**How it works:** Clone the object before mutation. Apply all mutations. Compare the mutated object to the snapshot using structural equality (`PartialEq`). If equal, skip the write; if different, proceed.

**When to use:** When mutation logic is complex (multiple independent fields) and you want a single, clean comparison point rather than tracking individual field changes.

**Tradeoffs:**
- Pro: Simple, correct, easy to audit. New fields automatically participate in comparison via derived `PartialEq`.
- Pro: Comparison happens at one point in the code, not scattered across mutation branches.
- Con: Requires one extra `Clone` per invocation (negligible for small structs).

**References:**
- [PartialEq in std::cmp - Rust](https://doc.rust-lang.org/std/cmp/trait.PartialEq.html) — Canonical reference for Rust equality traits
- [Effective Rust - Item 10: Standard Traits](https://effective-rust.com/std-traits.html) — Best practices for derive vs manual PartialEq

#### Pattern: Dirty Flag

**How it works:** Maintain a boolean that gets set to `true` when any mutation modifies a value. Check flag after all mutations.

**When to use:** Long-lived, frequently-mutated objects (game entities, UI state) where cloning is expensive.

**Tradeoffs:**
- Pro: No clone overhead
- Con: Every mutation site must correctly set the flag — easy to forget when adding new fields
- Con: Threading the flag through all mutation logic increases complexity
- Con: For a one-shot edit command, the overhead of managing flags exceeds the cost of a single clone

**References:**
- [Dirty Flag - Game Programming Patterns](https://gameprogrammingpatterns.com/dirty-flag.html) — When dirty flags make sense

### Technologies & Tools

| Technology | Purpose | Pros | Cons |
|---|---|---|---|
| `#[derive(PartialEq)]` (std) | Structural equality | Zero-dependency, auto-includes new fields | All contained types must impl PartialEq |
| `#[derive(Clone)]` (std) | Pre-mutation snapshot | Already derived on `Item` | One allocation per edit (negligible) |
| `serde_json::Value` | Extension field values | Already implements `PartialEq`; NaN→Null safe | None relevant |

### Standards & Best Practices

1. **Idempotent CLI commands should exit 0 on no-op.** A no-op is not an error; the system is already in the desired state. Follows Unix convention (`mkdir -p`, `touch`, `chmod`).
2. **Signal no-op to callers.** In JSON mode, include `{"idempotent": true}` so programmatic consumers can distinguish applied vs no-op without parsing human messages.
3. **Prefer derived `PartialEq`** for data structs where all fields are semantically significant. Manual implementations risk drift when fields are added.
4. **All-or-nothing mutation semantics.** If any mutation fails, reject the entire edit before comparison runs.

### Common Pitfalls

| Pitfall | Why It's a Problem | How to Avoid |
|---------|-------------------|--------------|
| Comparing after `updated_at` bump | Snapshot and mutated item always differ (timestamp changed), no-op never triggers | Compare BEFORE setting `updated_at = Utc::now()` |
| Order-dependent Vec comparison | If mutation logic reorders elements, semantically-equal vecs compare as unequal | Don't introduce sorting in mutation path; existing logic preserves order |
| Forgetting new fields in comparison | Manual PartialEq or dirty flags miss new fields | Use `#[derive(PartialEq)]` — new fields auto-included |
| TOCTOU window | Another process modifies file between check and write | Execute clone-compare-save inside `with_lock()` closure |

### Key Learnings

- The Snapshot-Compare pattern is the clear standard for one-shot mutation commands
- Rust's derive system makes this pattern essentially maintenance-free
- The existing codebase already uses this conceptual pattern for transitions

---

## Internal Research

### Existing Codebase State

**Relevant files/modules:**

- `src/cli/commands/edit.rs` (125 lines) — Current edit command. Loads items, applies mutations (title → priority → description → deps → tags → extensions), always updates `updated_at` and saves. Returns the item.
- `src/cli/commands/transition.rs` (302 lines) — Idempotent pattern via `TransitionResult::Idempotent`. Each transition command checks state before mutation and returns Applied or Idempotent.
- `src/model/item.rs` (374 lines) — `Item` struct derives `Debug, Clone, Serialize, Deserialize` but NOT `PartialEq`. Fields: id, title, status, priority, description, tags, dependencies, created_at, updated_at, blocked_reason, blocked_from_status, claimed_by, claimed_at, extensions (BTreeMap).
- `src/store/mod.rs` (88 lines) — `Store::with_lock()` closure pattern. Lock held for entire closure.
- `src/store/lock.rs` (140 lines) — `fd_lock::RwLock::try_write()` with exponential backoff (10ms→500ms cap, 5s total timeout). RAII-managed.
- `src/cli/output.rs` (148 lines) — `print_json<T: Serialize>()` and `print_human()` functions.
- `src/model/extensions.rs` (356 lines) — `apply_sets()` for extension mutations.
- `tests/edit_test.rs` (137 lines) — Existing edit tests (title, priority, description, deps, tags, extensions).
- `tests/idempotent_test.rs` (116 lines) — Existing idempotent tests for transition commands.
- `tests/common/mod.rs` (70 lines) — `TestProject` helper for integration tests.

**Existing patterns in use:**

- Closure-based locking: `store.with_lock(|store| { ... })?`
- Load → mutate → save inside lock closure
- Item cloning before returning from closure
- `TransitionResult` enum for Applied vs Idempotent
- Two-output-mode handling (JSON + human) at the output stage

### Reusable Components

- **`output::print_json()`** — Directly reusable for idempotent JSON output
- **`output::print_human()`** — Directly reusable for idempotent human output
- **`store.with_lock()` pattern** — No changes needed; conditional save goes inside existing closure
- **`TransitionResult` as conceptual template** — Edit should define its own enum following the same pattern

### Constraints from Existing Code

- `Item` needs `PartialEq` derive added (all field types already support it)
- Edit command currently returns `Item` from closure; needs to return an enum or tuple instead
- All mutations, comparison, and conditional save must happen inside `with_lock()` closure
- Cannot reuse `TransitionResult` directly (PRD scopes it as edit-local)

---

## PRD Concerns

| PRD Assumption | Research Finding | Implication |
|----------------|------------------|-------------|
| Clone-compare is sufficient | Confirmed as the standard pattern | No changes needed — approach is correct |
| `PartialEq` can be derived on `Item` | All 14+ field types already implement `PartialEq` | Single-line change in item.rs derive list |
| Vec ordering preserved by mutations | Confirmed: `push` for add, `retain` for remove | Ordered `PartialEq` on Vec is correct |
| Everything fits inside `with_lock()` | Confirmed: closure pattern supports this cleanly | No architectural changes needed |
| JSON output: `{"idempotent": true}` only | Consistent with transition pattern minus `previous_state` | Correct — edit has no single "target state" |

No concerns found. The PRD is well-aligned with both external patterns and internal codebase state.

---

## Critical Areas

### `updated_at` Ordering

**Why it's critical:** If comparison happens after `updated_at = Utc::now()`, the snapshot and mutated item will always differ, and no-op detection never triggers.

**Why it's easy to miss:** The current code sets `updated_at` as the second-to-last step. It's tempting to add the comparison at the very end rather than inserting it before the timestamp bump.

**What to watch for:** The comparison must be inserted BETWEEN the last mutation and the `updated_at = Utc::now()` line.

### EditResult Enum Design

**Why it's critical:** The Idempotent variant needs enough info to format both JSON and human output (needs the item's id and title for the human message).

**Why it's easy to miss:** Simply returning `Idempotent` with no data means the output handler can't format the "No changes: <id> - <title>" message.

**What to watch for:** The Idempotent variant should carry the item reference or at minimum the id and title strings.

---

## Deep Dives

_No deep dives conducted — light mode research with no outstanding questions._

---

## Synthesis

### Open Questions

None. All research questions are resolved. The PRD's approach is validated by both external patterns and internal codebase analysis.

### Recommended Approaches

#### Comparison Strategy

| Approach | Pros | Cons | Best When |
|----------|------|------|-----------|
| Snapshot-Compare (derive PartialEq) | Simple, self-maintaining, zero dependencies | One clone per invocation (~1KB) | One-shot mutation commands (this case) |
| Dirty Flag | No clone overhead | Error-prone, complex, must update for new fields | Long-lived frequently-mutated objects |

**Initial recommendation:** Snapshot-Compare. It is the clear winner — simpler, self-maintaining, and the clone cost is negligible.

#### Result Type

| Approach | Pros | Cons | Best When |
|----------|------|------|-----------|
| Local `EditResult` enum | Consistent with `TransitionResult` pattern, type-safe | Slightly more code | Structured output needed (this case) |
| `(Item, bool)` tuple | Minimal code | Less expressive, requires comments | Quick prototypes |

**Initial recommendation:** Local `EditResult` enum for consistency with the codebase's established pattern.

### Key References

| Reference | Type | Why It's Useful |
|-----------|------|-----------------|
| [PartialEq - Rust docs](https://doc.rust-lang.org/std/cmp/trait.PartialEq.html) | Official docs | Canonical reference for derive behavior |
| [Effective Rust - Standard Traits](https://effective-rust.com/std-traits.html) | Best practices | When to derive vs implement manually |
| [Dirty Flag - Game Programming Patterns](https://gameprogrammingpatterns.com/dirty-flag.html) | Article | Why dirty flags are NOT the right choice here |
| [Idempotent Command Handling - Event-Driven.io](https://event-driven.io/en/idempotent_command_handling/) | Article | Patterns for idempotency in command processing |
| [On Idempotence and Unix Commands - Peter Lyons](https://peterlyons.com/problog/2010/05/on-idempotence-intention-and-unix-commands/) | Article | Philosophy of idempotent CLI design and exit codes |
| [serde_json::Value PartialEq - serde-rs/json#638](https://github.com/serde-rs/json/issues/638) | GitHub issue | Confirms Value equality semantics (NaN-safe) |

---

## Research Log

| Date | Activity | Outcome |
|------|----------|---------|
| 2026-02-24 | External research: no-op detection patterns | Identified Snapshot-Compare as standard; validated PRD approach |
| 2026-02-24 | Internal research: codebase exploration | Mapped all relevant files; confirmed PartialEq derivability; documented integration points |
| 2026-02-24 | PRD analysis | No concerns — PRD is well-aligned with findings |

## Assumptions

- **Mode selection:** Used light mode as specified in the PRD. The problem space is well-understood and the codebase already has the pattern.
- **No human available:** All research decisions made autonomously. No questions arose that required human input.
- **EditResult naming:** Assumed the enum should be called `EditResult` to parallel `TransitionResult`. Design phase will finalize.
