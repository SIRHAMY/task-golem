# SPEC: Validate Extension Key Collision with serde(flatten)

**ID:** TG-002
**Status:** Draft
**Created:** 2026-02-24
**PRD:** ./TG-002_validate-extension-key-collision_PRD.md
**Design:** ./TG-002_validate-extension-key-collision_DESIGN.md
**Execution Mode:** autonomous
**New Agent Per Phase:** yes
**Max Review Attempts:** 3

## Context

The `Item` struct uses `#[serde(flatten)]` to store extension fields as a `BTreeMap<String, serde_json::Value>`. The CLI enforces an `x-` prefix on extension keys via `extensions::parse_dot_path()`, but no validation exists at the model or store layer. This means externally-crafted or hand-edited JSONL files can introduce non-`x-` keys that persist silently, and programmatic construction could insert keys that collide with known field names.

This change adds a post-deserialization validation method `Item::validate_extensions()` that enforces two invariants: all extension keys must start with `x-`, and no extension key may match a known Item field name. The method is called at both JSONL deserialization sites — `read_active` (hard error) and `read_archive` (skip with warning).

## Approach

Post-deserialization validation following the existing `validate_title()` pattern on the `Item` struct. A `KNOWN_FIELD_NAMES` constant lists all serialized Item field names. The `validate_extensions()` method iterates extension keys, checking for known-field collision first, then `x-` prefix. Two call-site integrations wire validation into `read_active()` and `read_archive()`. A serialize-and-compare drift test ensures `KNOWN_FIELD_NAMES` stays in sync with the actual struct.

No new dependencies, files, or error variants are needed. All changes are additions to existing files (`item.rs` and `jsonl.rs`).

**Patterns to follow:**

- `src/model/item.rs:53-63` — `Item::validate_title()` for validation method structure (takes input, returns `Result<(), TgError>`)
- `src/store/jsonl.rs:51-53` — `read_active()` deserialization error handling with line-number context
- `src/store/jsonl.rs:104-113` — `read_archive()` skip-with-warning pattern
- `src/model/item.rs:150-158` — Existing serde round-trip test for drift test approach

**Implementation boundaries:**

- Do not modify: `src/model/extensions.rs` (CLI-level validation is separate)
- Do not modify: `src/cli/commands/add.rs`, `src/cli/commands/edit.rs` (write-path already enforces `x-` prefix)
- Do not modify: `src/errors.rs` (`TgError::StorageCorruption` already exists)
- Do not add: Write-path validation hooks (explicitly out of scope per design)

## Phase Summary

| Phase | Name | Complexity | Description |
|-------|------|------------|-------------|
| 1 | Core Validation | Low | Add `KNOWN_FIELD_NAMES` constant and `validate_extensions()` method with unit tests |
| 2 | Store Integration | Low | Wire validation into `read_active()` and `read_archive()` with integration tests |

**Ordering rationale:** Phase 2 depends on `Item::validate_extensions()` defined in Phase 1. The method must exist and be unit-tested before the store layer can call it.

---

## Phases

### Phase 1: Core Validation

> Add KNOWN_FIELD_NAMES constant and validate_extensions() method with unit tests

**Phase Status:** complete

**Complexity:** Low

**Goal:** Define the validation constant and method on `Item`, and verify them with comprehensive unit tests including a drift-detection test.

**Files:**

- `src/model/item.rs` — modify — Add `KNOWN_FIELD_NAMES` constant and `validate_extensions()` method, plus 7 unit tests

**Patterns:**

- Follow `Item::validate_title()` at line 53 for validation method structure
- Follow existing `#[cfg(test)] mod tests` block for test organization

**Tasks:**

- [x] Add `const KNOWN_FIELD_NAMES: &[&str]` at module level (above `impl Item`), listing all 13 serialized field names: `id`, `title`, `status`, `priority`, `description`, `tags`, `dependencies`, `created_at`, `updated_at`, `blocked_reason`, `blocked_from_status`, `claimed_by`, `claimed_at`. Note: the `extensions` field is NOT included — `serde(flatten)` never serializes it as a key called "extensions"; it merges the map's contents into the parent object.
- [x] Add `pub fn validate_extensions(&self) -> Result<(), TgError>` to `impl Item` that iterates `self.extensions` keys, checks known-field collision first (returns `StorageCorruption("Extension key '{key}' collides with a known Item field name.")`), then checks `x-` prefix (returns `StorageCorruption("Extension key '{key}' must start with 'x-' prefix. Rename to 'x-{key}' or remove it.")`)
- [x] Add test: `validate_extensions_valid_x_prefix_keys` — item with `x-`-prefixed keys passes. Assert `is_ok()`.
- [x] Add test: `validate_extensions_empty_extensions` — item with empty extensions passes. Assert `is_ok()`.
- [x] Add test: `validate_extensions_rejects_non_x_prefix_key` — key `"bogus"` returns error. Assert `matches!(err, TgError::StorageCorruption(_))` and error message contains `"must start with 'x-' prefix"` and the key name `"bogus"`.
- [x] Add test: `validate_extensions_rejects_known_field_name` — key `"status"` returns error. Assert `matches!(err, TgError::StorageCorruption(_))` and error message contains `"collides with a known Item field name"`.
- [x] Add test: `validate_extensions_fails_on_first_invalid_key` — item with two invalid keys (e.g., `"aaa-bad"` and `"zzz-bad"` — BTreeMap orders alphabetically) returns error mentioning only `"aaa-bad"`, confirming fail-fast behavior.
- [x] Add test: `known_fields_match_serialized_item` — serialize an Item with empty extensions to `serde_json::Value`, extract the JSON object's keys, collect into a sorted `Vec`, and assert it matches a sorted copy of `KNOWN_FIELD_NAMES`. Use order-independent comparison (sort both sides or use `BTreeSet`) since JSON key order is not guaranteed to match the constant's declaration order.

**Verification:**

- [x] All 7 new unit tests pass
- [x] Existing `item.rs` tests still pass (no regressions)
- [x] `cargo build` succeeds
- [x] Code review passes (`/code-review` → fix issues → repeat until pass)

**Commit:** `[TG-002][P1] Feature: Add validate_extensions() method and KNOWN_FIELD_NAMES constant`

**Notes:**

- Check ordering (collision before prefix) is for error message clarity only — the two checks are mutually exclusive since no known field starts with `x-`.
- The method is fail-fast: returns on the first invalid key. This is consistent with existing error patterns. The PRD uses singular "the offending key" in error message descriptions, supporting this approach.
- The drift test should use an Item with empty extensions and serialize to `serde_json::Value`, then compare the `as_object()` keys against `KNOWN_FIELD_NAMES`. Use order-independent comparison (sort both sides) since serde's output key ordering may differ from the constant's declaration order.
- The `extensions` field must NOT appear in `KNOWN_FIELD_NAMES` because `serde(flatten)` merges the map's contents into the parent JSON object rather than serializing it as a nested key.
- Both `KNOWN_FIELD_NAMES` and `validate_extensions()` are marked `#[allow(dead_code)]` until Phase 2 wires them into the store layer.

**Followups:**

---

### Phase 2: Store Integration

> Wire validation into read_active() and read_archive() with integration tests

**Phase Status:** not_started

**Complexity:** Low

**Goal:** Integrate `validate_extensions()` at both JSONL deserialization sites and verify with integration tests that invalid extensions are properly rejected (active) or skipped (archive).

**Files:**

- `src/store/jsonl.rs` — modify — Add validation calls in `read_active()` and `read_archive()`, plus 3 integration tests

**Patterns:**

- Follow `read_active()` line 51-53 for error wrapping with line numbers
- Follow `read_archive()` line 104-113 for skip-with-warning pattern
- Follow existing `jsonl.rs` test helpers (`make_item()`, temp file patterns)

**Tasks:**

- [ ] In `read_active()`: after `serde_json::from_str::<Item>(&line)` succeeds (line 53), call `item.validate_extensions()` before `items.push(item)`. On error, extract the inner message string (match on `TgError::StorageCorruption(msg)`) and re-wrap with line context: `TgError::StorageCorruption(format!("Invalid extensions on line {}: {}", i + 2, msg))`. This avoids double-nesting the "Storage corruption:" prefix that `TgError`'s `Display` impl would add. Use `map_err` with a closure that destructures the error.
- [ ] In `read_archive()`: expand the `Ok(item)` arm (line 105) into a block. Call `item.validate_extensions()`. On `Ok(())`, push the item. On `Err(e)`, extract the inner message (match on the error variant), print `eprintln!("Warning: skipping archive item with invalid extensions on line {}: {}", i + 2, msg)` and `continue`. Same double-wrap avoidance as `read_active`.
- [ ] Add test: `active_invalid_extension_key_fails` — write JSONL with a non-`x-` extension key, assert `read_active()` returns `StorageCorruption` with line number
- [ ] Add test: `archive_invalid_extension_key_skipped` — write JSONL with one good item and one with a non-`x-` extension key, assert `read_archive()` returns only the good item
- [ ] Add test: `active_valid_extensions_pass` — write JSONL with valid `x-`-prefixed extension keys, assert `read_active()` loads all items successfully

**Verification:**

- [ ] All 3 new integration tests pass
- [ ] Existing `jsonl.rs` tests still pass (no regressions)
- [ ] Full test suite passes: `cargo test`
- [ ] Code review passes (`/code-review` → fix issues → repeat until pass)

**Commit:** `[TG-002][P2] Feature: Integrate extension validation in read_active and read_archive`

**Notes:**

- For integration tests, construct JSONL content manually (schema header + hand-crafted JSON lines) rather than using `write_atomic`, since `write_atomic` would serialize valid Items. Approach: serialize a valid Item with `serde_json::to_string`, then inject the invalid extension key via string manipulation or manual JSON construction.
- A known-field-name collision test at the jsonl level is omitted because serde routes known field names to struct fields during deserialization, so they never reach the extensions map. The collision check is tested at the unit level (Phase 1).
- The `read_archive` test verifies the skip behavior but does not assert on stderr output (testing stderr capture adds complexity for minimal benefit).
- Line references in Phase 2 tasks refer to `jsonl.rs` as it exists before Phase 1 modifications. Phase 1 only modifies `item.rs`, so `jsonl.rs` line numbers remain stable.

**Followups:**

---

## Final Verification

- [ ] All phases complete
- [ ] All PRD success criteria met:
  - [ ] `validate_extensions()` rejects non-`x-` keys
  - [ ] Error messages name the offending key and suggest a fix
  - [ ] Collision with known field names is rejected
  - [ ] `read_active()` treats validation failure as hard error
  - [ ] `read_archive()` skips invalid items with warning
  - [ ] Existing valid JSONL files load without error
  - [ ] Unit tests cover all specified scenarios
  - [ ] Drift test prevents `KNOWN_FIELD_NAMES` from going stale
- [ ] Tests pass (`cargo test`)
- [ ] No clippy warnings (`cargo clippy -- -D warnings`)
- [ ] No regressions introduced
- [ ] Code reviewed (if applicable)

## Execution Log

| Phase | Status | Commit | Notes |
|-------|--------|--------|-------|
| 1 — Core Validation | complete | `[TG-002][P1] Feature: Add validate_extensions() method and KNOWN_FIELD_NAMES constant` | All 7 tests pass, clippy clean, code review passed |

## Followups Summary

### Critical

### High

### Medium

### Low

- `tg doctor` integration: Report items with non-`x-`-prefixed extension keys as warnings (PRD Nice-to-Have)
- Write-path validation: Add `validate_extensions()` calls before serialization for defense-in-depth against programmatic misuse (deferred per Design "read-path only" decision; no programmatic write paths exist today)

## Design Details

### Key Types

No new types introduced. The change adds:

```rust
// Module-level constant in src/model/item.rs
const KNOWN_FIELD_NAMES: &[&str] = &[
    "id", "title", "status", "priority", "description",
    "tags", "dependencies", "created_at", "updated_at",
    "blocked_reason", "blocked_from_status", "claimed_by", "claimed_at",
];
```

```rust
// New method on impl Item
pub fn validate_extensions(&self) -> Result<(), TgError> {
    for key in self.extensions.keys() {
        if KNOWN_FIELD_NAMES.contains(&key.as_str()) {
            return Err(TgError::StorageCorruption(
                format!("Extension key '{}' collides with a known Item field name.", key),
            ));
        }
        if !key.starts_with("x-") {
            return Err(TgError::StorageCorruption(
                format!("Extension key '{}' must start with 'x-' prefix. Rename to 'x-{}' or remove it.", key, key),
            ));
        }
    }
    Ok(())
}
```

### Design Rationale

See `./TG-002_validate-extension-key-collision_DESIGN.md` for full rationale. Key decisions:

- **Post-deserialization validation** over custom serde deserializer — simpler, follows existing `validate_title()` pattern, only 2 call sites
- **`StorageCorruption` error type** — validation failures indicate stored data integrity issues, not user input errors (exit code 2)
- **Hard-coded constant with drift test** — no proc-macro dependency; serialize-and-compare catches field additions/removals at test time
- **Read-path only** — CLI write path already enforces `x-` prefix; no programmatic write paths exist today
- **Fail-fast on first error** — consistent with existing error handling; users fix one key at a time

## Assumptions

- **Autonomous SPEC**: Created without human interview. Light mode chosen given small size, low complexity, and clear design.
- **Two phases**: Separated core validation (item.rs) from store integration (jsonl.rs) for clean dependency ordering and testability. The work is small enough to merge into one phase, but two phases provide cleaner separation for autonomous execution with separate agents per phase.
- **No stderr assertion in archive test**: Testing stderr capture adds complexity for minimal benefit; the skip behavior is verified by checking returned items.
- **Fail-fast on first error**: The PRD uses singular "the offending key" in error message descriptions. Fail-fast is consistent with existing error patterns and simpler than collect-all-errors.
- **No CLI-level end-to-end test**: Integration tests in `jsonl.rs` cover the full read path. CLI commands delegate directly to `read_active`/`read_archive`, so a separate CLI test would add minimal coverage.
- **Breaking change for malformed files accepted**: Per PRD, files with non-`x-`-prefixed extension keys created outside the CLI are considered malformed and will fail to load after this change. No migration path is included because no known instances exist (all writes go through the CLI which enforces `x-` prefix).
