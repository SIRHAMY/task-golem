# Design: Validate Extension Key Collision with serde(flatten)

**ID:** TG-002
**Status:** Complete
**Created:** 2026-02-24
**PRD:** ./TG-002_validate-extension-key-collision_PRD.md
**Tech Research:** ./TG-002_validate-extension-key-collision_TECH_RESEARCH.md
**Mode:** Light

## Overview

Add a post-deserialization validation method `Item::validate_extensions()` that checks all extension map keys start with `x-` and none collide with known Item field names. The method follows the existing `validate_title()` pattern, uses a hard-coded `KNOWN_FIELD_NAMES` constant with a serialize-and-compare drift test, and is called at the two JSONL deserialization sites (`read_active` as hard error, `read_archive` as skip-with-warning).

---

## System Design

### High-Level Architecture

This is a validation addition, not a new architectural component. The change introduces:

1. A `KNOWN_FIELD_NAMES` constant on the `Item` struct — the list of field names serde serializes for known Item fields
2. An `Item::validate_extensions()` instance method — iterates extension keys, checks two invariants
3. Two call-site integrations — `read_active` and `read_archive` call validation after deserialization

```
  JSONL file
      │
      ▼
  serde::from_str  ──►  Item (deserialized)
      │                       │
      │                       ▼
      │              validate_extensions()
      │                  │          │
      │               Ok(())    Err(TgError)
      │                  │          │
      ▼                  ▼          ▼
  read_active:     push item    return StorageCorruption
  read_archive:    push item    eprintln! + skip
```

### Component Breakdown

#### `KNOWN_FIELD_NAMES` Constant

**Purpose:** Hard-coded list of Item struct field names as they appear in serialized JSON, used to detect extension keys that collide with known fields.

**Value:** `["id", "title", "status", "priority", "description", "tags", "dependencies", "created_at", "updated_at", "blocked_reason", "blocked_from_status", "claimed_by", "claimed_at"]`

**Location:** `src/model/item.rs`, as a module-level `const KNOWN_FIELD_NAMES: &[&str]` above the `impl Item` block. Module-level makes it accessible to both the validation method and the drift test without visibility complications.

#### `Item::validate_extensions()`

**Purpose:** Validates that all keys in `self.extensions` satisfy two invariants:
1. No key matches a known Item field name (collision check)
2. Every key starts with `x-` (prefix check)

**Signature:** `pub fn validate_extensions(&self) -> Result<(), TgError>`

**Algorithm:**
```
for each key in self.extensions:
    if key is in KNOWN_FIELD_NAMES:
        return Err(StorageCorruption("Extension key '{key}' collides with a known Item field name."))
    if not key.starts_with("x-"):
        return Err(StorageCorruption("Extension key '{key}' must start with 'x-' prefix. Rename to 'x-{key}' or remove it."))
return Ok(())
```

**Check ordering rationale:** Since no known field name starts with `x-`, the two checks are mutually exclusive — a key can only fail one check, never both. We check known-field collision first so the more specific error message appears first in the code, improving readability. The ordering has no behavioral impact.

**Scope limitation:** This method validates keys that are _in_ the extensions map. During deserialization, `serde(flatten)` routes known field names to their struct fields, so a JSON-level collision (e.g., a duplicate `"status"` key in the JSON) never reaches the extensions map. The known-field collision check therefore catches programmatic construction errors (code inserting `"status"` into extensions directly), not JSON-level duplicates. This is by design — the `x-` prefix check is the primary guard, and the collision check is defense-in-depth for programmatic use.

**Error type:** `TgError::StorageCorruption` because validation failures indicate data integrity issues in the stored JSONL, not user input errors. This is consistent with how `read_active` handles malformed lines.

**Fail-fast:** Returns on the first invalid key, consistent with existing error handling patterns. If an item has multiple bad keys, the user fixes them one at a time.

#### Integration in `jsonl::read_active()`

**Location:** After `serde_json::from_str::<Item>(&line)` succeeds (line ~53 in `jsonl.rs`).

**Behavior:** Call `item.validate_extensions()`. On error, wrap the validation error message with the line number using the same pattern as existing malformed-line errors: `TgError::StorageCorruption(format!("Invalid extensions on line {}: {}", i + 2, e))`. The `validate_extensions()` method itself returns errors without line numbers — the call site adds the context.

#### Integration in `jsonl::read_archive()`

**Location:** After `serde_json::from_str::<Item>(&line)` succeeds (line ~105 in `jsonl.rs`).

**Behavior:** Call `item.validate_extensions()`. On error, `eprintln!` a warning with the line number following the existing archive warning pattern: `"Warning: skipping archive item with invalid extensions on line {}: {}"`, then `continue` (skip the item). As with `read_active`, the validation method returns the error without line numbers and the call site adds context.

### Data Flow

1. JSONL line is read from file
2. `serde_json::from_str::<Item>` deserializes the line — serde routes known field names to struct fields, unknown keys go to `extensions`
3. `item.validate_extensions()` checks all extension keys
4. If valid, item is pushed to the result vector
5. If invalid in active store, error propagates up; in archive, warning printed and item skipped

### Key Flows

#### Flow: Active store loads file with invalid extension key

> Detect and reject items with non-`x-` or field-colliding extension keys during active store read.

1. **Read line** — `read_active` reads a JSONL line
2. **Deserialize** — `serde_json::from_str::<Item>` succeeds, unknown keys land in `extensions`
3. **Validate** — `item.validate_extensions()` detects invalid key
4. **Error** — Returns `TgError::StorageCorruption` with line number and key name

**Edge cases:**
- Item with no extensions — `validate_extensions()` iterates empty map, returns `Ok(())`
- Item with only valid `x-`-prefixed extensions — passes validation
- Item with multiple invalid keys — validation returns on the first invalid key (fail-fast); user fixes one, reloads, sees the next

#### Flow: Archive store loads file with invalid extension key

> Detect and skip items with invalid extension keys during archive read, without failing the entire load.

1. **Read line** — `read_archive` reads a JSONL line
2. **Deserialize** — succeeds
3. **Validate** — `validate_extensions()` detects invalid key
4. **Skip** — Print warning via `eprintln!`, continue to next line

---

## Technical Decisions

### Key Decisions

#### Decision: Post-deserialization validation (Pattern 1) vs. custom deserializer

**Context:** Need to validate extension keys. Could do it during deserialization (Pattern 2/3) or after.

**Decision:** Post-deserialization validation method.

**Rationale:** Simplest approach. Follows existing `validate_title()` pattern. Only 2 call sites to integrate. No custom serde machinery needed. Tech research confirmed this is the standard Rust ecosystem approach for this problem.

**Consequences:** Item struct can briefly exist with invalid extensions between deserialization and validation. Acceptable because validation is called immediately after deserialization at both sites.

#### Decision: `StorageCorruption` error type for validation failures

**Context:** Validation failures from deserialized data could use `InvalidInput` or `StorageCorruption`.

**Decision:** Use `StorageCorruption`.

**Rationale:** These failures indicate corrupted or malformed stored data, not user input errors. This is consistent with how `read_active` already handles malformed JSONL lines. `InvalidInput` is reserved for CLI-level validation errors (exit code 1), while `StorageCorruption` indicates system-level data issues (exit code 2).

**Consequences:** Exit code 2 (system error) for extension validation failures, signaling to callers that data needs repair, not user correction.

#### Decision: Hard-coded `KNOWN_FIELD_NAMES` with drift test

**Context:** Need a list of known field names to detect collisions. Could use proc macros, serde internals, or a hard-coded list.

**Decision:** Hard-coded `const` array with a unit test that serializes a default Item and compares JSON keys.

**Rationale:** No proc-macro dependency needed. Serialize-and-compare is the standard approach (tech research Pattern 4). The drift test catches field additions/removals at test time. No `#[serde(rename)]` attributes are used on Item, so field names match struct field names.

**Consequences:** If a developer adds a field to Item and forgets to update `KNOWN_FIELD_NAMES`, the drift test fails. Not compile-time, but sufficient for a project of this size.

#### Decision: Read-path only — no write-path validation

**Context:** The PRD's "Desired Outcome" mentions validation "before serialization or at construction time." Should we add `validate_extensions()` calls in the write path?

**Decision:** No write-path validation in this change. Read-path only.

**Rationale:** The CLI write path already enforces `x-` prefix via `apply_sets()` in both `add` and `edit` commands. There are no programmatic write paths outside the CLI today. Adding write-path validation would be defense-in-depth but is not required by the PRD's Must Have criteria. The PRD's Out of Scope section explicitly lists "Pre-serialization validation hooks."

**Consequences:** If future code constructs Items with invalid extensions and writes them directly (bypassing the CLI), the invalid data would only be caught on the next read. Acceptable risk given no such code path exists today. Can be added as a follow-up if programmatic Item construction becomes a pattern.

### Tradeoffs Accepted

| Tradeoff | We're Accepting | In Exchange For | Why This Makes Sense |
|----------|-----------------|-----------------|---------------------|
| Brief invalid state | Item exists with invalid extensions between deser and validation | Implementation simplicity — no custom serde code | Validation is called immediately; invalid state never escapes the read functions |
| Test-time drift detection | Field list drift caught at test time, not compile time | No proc-macro dependency or serde internals hacking | Compile-time would require a proc macro; test-time is standard practice |
| Fail-fast on first error | Only one invalid key reported per load attempt | Simpler code, consistent with existing patterns | Users fix one key, reload, see next; typical workflow for data repair |

---

## Alternatives Considered

### Alternative: Custom `deserialize_with` on the flattened field (Pattern 2)

**Summary:** Use `#[serde(flatten, deserialize_with = "...")]` with a custom visitor that rejects non-`x-` keys during deserialization itself.

**How it would work:**
- Define a `ValidatedExtensions` visitor that checks each key as it's deserialized
- Invalid keys cause deserialization to fail

**Pros:**
- Invalid keys never enter the struct
- Stronger invariant enforcement

**Cons:**
- Significantly more complex (custom `Visitor`, `MapAccess` handling)
- Harder to provide rich, actionable error messages with line numbers
- Only sees keys serde didn't route to struct fields (can't detect all collision scenarios)
- PRD explicitly scopes custom serde impls as out of scope

**Why not chosen:** Over-engineered for 2 call sites. The post-deser approach is simpler, equally effective, and follows existing codebase patterns.

---

## Risks

| Risk | Impact | Likelihood | Mitigation |
|------|--------|------------|------------|
| Developer adds Item field, forgets to update `KNOWN_FIELD_NAMES` | Collision detection misses the new field | Low | Drift test (`known_fields_match_serialized_item`) fails, caught in CI |
| Existing JSONL files have non-`x-` extension keys | Files fail to load after upgrade | Very low | No known instances; all writes go through CLI which enforces `x-` prefix. PRD documents this as expected behavior. |

---

## Integration Points

### Existing Code Touchpoints

- `src/model/item.rs` — Add `KNOWN_FIELD_NAMES` constant and `validate_extensions()` method to `impl Item`
- `src/store/jsonl.rs:read_active()` — Add `item.validate_extensions()?` call after successful deserialization (line ~53)
- `src/store/jsonl.rs:read_archive()` — Add `item.validate_extensions()` call with match/warning after successful deserialization (line ~105)

### External Dependencies

None. Uses only existing `serde_json` for the drift test.

---

## Open Questions

None — all decisions resolved. See Assumptions section below.

---

## Design Review Checklist

Before moving to SPEC:

- [x] Design addresses all PRD requirements
- [x] Key flows are documented and make sense
- [x] Tradeoffs are explicitly documented and acceptable
- [x] Integration points with existing code are identified
- [x] No major open questions remain (or they're flagged for spec phase)

---

## Design Log

| Date | Activity | Outcome |
|------|----------|---------|
| 2026-02-24 | Initial design draft (autonomous, light mode) | Post-deser validation with KNOWN_FIELD_NAMES and drift test |
| 2026-02-24 | Self-critique and auto-fixes | Polished design ready for review |

## Assumptions

- **Autonomous design**: Created without human interview. Light mode chosen given small size and low complexity.
- **`StorageCorruption` over `InvalidInput`**: Post-deser validation failures indicate stored data issues, not user input. This matches existing `read_active` error handling and gives exit code 2. Documented in Technical Decisions.
- **Read-path only**: No write-path validation in this change. Documented in Technical Decisions.
