# SPEC: Optimize load_archive_ids to Avoid Full Deserialization

**ID:** TG-004
**Status:** Draft
**Created:** 2026-02-24
**PRD:** ./TG-004_PRD.md
**Execution Mode:** autonomous
**New Agent Per Phase:** no
**Max Review Attempts:** 3

## Context

Every CLI command performing ID resolution fully deserializes the entire `archive.jsonl` into 14-field `Item` structs — including `DateTime<Utc>`, `Vec<String>`, and `BTreeMap<String, serde_json::Value>` via `#[serde(flatten)]` — only to extract and discard everything except the `id` field. The archive grows unboundedly, so this cost increases linearly with project age. The fix is surgical: add a lightweight `IdOnly` projection struct and a dedicated `read_archive_ids()` function in `jsonl.rs`, then rewire the one-line call in `mod.rs`.

## Approach

Add a private `IdOnly` struct (single `id: String` field, `#[derive(Deserialize)]`, no `#[serde(flatten)]`) and a new `pub fn read_archive_ids()` function to `src/store/jsonl.rs`. The new function mirrors `read_archive()` structure — schema header validation, skip-and-warn on malformed lines — but deserializes into `IdOnly` and collects directly into `HashSet<String>`. Then rewire `Store::load_archive_ids()` in `src/store/mod.rs` to call the new function (single-line change). The public API is unchanged.

The primary performance win comes from removing `#[serde(flatten)]` — without it, serde uses `IgnoredAny` to discard unknown fields with zero allocation instead of buffering them into an intermediate `Content` enum. Secondary wins: no DateTime parsing, no Vec/BTreeMap allocation, no Option field handling.

**Patterns to follow:**

- `src/store/jsonl.rs:61-117` (`read_archive()`) — mirror the schema header validation, BufReader/lines iteration, and skip-and-warn error handling pattern
- `src/store/jsonl.rs:12-15` (`SchemaHeader`) — reuse for header validation
- `src/store/jsonl.rs:207-375` (existing tests) — follow test structure using `tempfile::tempdir()` and `make_item()`

**Implementation boundaries:**

- Do not modify: `src/model/item.rs`, any CLI command files, `read_archive()`, `read_active()`, `write_atomic()`, `append_to_archive()`
- Do not refactor: existing `read_archive()` to share a generic helper with `read_archive_ids()` — the ~10-line schema validation duplication is acceptable for two functions (extraction threshold: 3+)
- Do not add: `#[serde(flatten)]` to `IdOnly` (defeats the optimization), `#[serde(deny_unknown_fields)]` (would reject every archive line), new crate dependencies

## Phase Summary

| Phase | Name | Complexity | Description |
|-------|------|------------|-------------|
| 1 | IdOnly struct, read_archive_ids(), rewire, and tests | Low | Add IdOnly projection struct, read_archive_ids() function, rewire Store::load_archive_ids(), and unit tests |

**Ordering rationale:** Single phase — the entire change is small enough to implement atomically. The IdOnly struct, function, rewire, and tests are tightly coupled and there's no meaningful boundary to split on.

---

## Phases

### Phase 1: IdOnly struct, read_archive_ids(), rewire, and tests

> Add IdOnly projection struct, read_archive_ids() function, rewire Store::load_archive_ids(), and unit tests

**Phase Status:** not_started

**Complexity:** Low

**Goal:** Replace the full-deserialization path in `load_archive_ids()` with a lightweight `IdOnly`-based deserialization path that avoids DateTime parsing, Vec/BTreeMap allocation, and `#[serde(flatten)]` buffering overhead.

**Files:**

- `src/store/jsonl.rs` — modify — add `IdOnly` struct, `read_archive_ids()` function, `use std::collections::HashSet` import, and 9 unit tests
- `src/store/mod.rs` — modify — rewire `load_archive_ids()` body to call `jsonl::read_archive_ids()` (single-line change)

**Patterns:**

- Follow `src/store/jsonl.rs:61-117` (`read_archive()`) for schema header validation, BufReader iteration, and skip-and-warn error handling
- Follow `src/store/jsonl.rs:207-375` (existing test module) for test structure

**Tasks:**

- [ ] Add `use std::collections::HashSet;` import to `src/store/jsonl.rs`
- [ ] Add `IdOnly` struct (private, `#[derive(Deserialize)]`, single `id: String` field) with doc comment explaining the no-flatten invariant and referencing TG-004
- [ ] Implement `read_archive_ids(path: &Path) -> Result<HashSet<String>, TgError>` mirroring `read_archive()` structure:
  - Return empty `HashSet` if file doesn't exist
  - Open file, create BufReader, read lines iterator
  - Parse first line as `SchemaHeader`; return empty `HashSet` if file is empty
  - Validate schema version; return `TgError::SchemaVersionUnsupported` on mismatch, `TgError::StorageCorruption` on invalid header JSON
  - Iterate data lines: skip empty/whitespace; on I/O error warn with `"Warning: could not read archive line {line_num}: {error}"` (line_num = i + 2); on malformed JSON warn with `"Warning: skipping malformed archive line {line_num}: {error}"` (line_num = i + 2); on success insert `id` into `HashSet`
- [ ] Rewire `Store::load_archive_ids()` in `src/store/mod.rs` to: `jsonl::read_archive_ids(&self.archive_path())`
- [ ] Add unit test: happy path — valid archive with multiple items, all IDs returned
- [ ] Add unit test: nonexistent file returns empty `HashSet`
- [ ] Add unit test: header-only file returns empty `HashSet`
- [ ] Add unit test: malformed JSON line skipped, valid lines included
- [ ] Add unit test: truncated last line skipped, earlier valid lines included
- [ ] Add unit test: schema version mismatch returns `TgError::SchemaVersionUnsupported`
- [ ] Add unit test: valid `id` with corrupted other fields (e.g., `{"id":"tg-abc12","created_at":"INVALID","status":999}`) is included
- [ ] Add unit test: missing `id` field (e.g., `{"title":"test"}`) is skipped
- [ ] Add unit test: duplicate IDs deduplicated by `HashSet`

**Verification:**

- [ ] `cargo build` succeeds without errors or new warnings
- [ ] `cargo test` passes — all existing tests + 9 new unit tests
- [ ] `cargo clippy` passes without new warnings
- [ ] `Store::load_archive_ids()` public API signature unchanged (`Result<HashSet<String>, TgError>`)
- [ ] New `read_archive_ids()` handles: missing file (empty set), empty file (empty set), header-only (empty set), valid archive (all IDs), malformed lines (skipped with warning), schema version mismatch (error), corrupted non-id fields (included)

**Commit:** `[TG-004][P1] Feature: Add IdOnly fast path for archive ID loading`

**Notes:**

- The `IdOnly` struct must never have `#[serde(flatten)]` added — this would force serde to buffer all unknown fields and eliminate the performance win. The doc comment explains this invariant.
- Behavioral divergence: lines with valid `id` but corrupted other fields are included by `IdOnly` deserialization but would have been skipped by full `Item` deserialization. This is intentional for collision avoidance (false positives harmless, false negatives cause data corruption).
- Warning message format must exactly match `read_archive()`: `"Warning: could not read archive line {}: {}"` for I/O errors and `"Warning: skipping malformed archive line {}: {}"` for JSON errors, with `i + 2` line numbering.

**Followups:**

---

## Final Verification

- [ ] All phases complete
- [ ] All PRD success criteria met:
  - [ ] `load_archive_ids()` uses lightweight `IdOnly` struct (not full `Item`)
  - [ ] Public API signature unchanged
  - [ ] All existing callers work without modification
  - [ ] Invalid JSON lines skipped with stderr warnings
  - [ ] Lines with valid `id` but corrupted other fields included
  - [ ] Lines that are not valid JSON or missing `id` skipped with warnings
  - [ ] Schema version header validated
  - [ ] Empty/missing archive files return empty `HashSet`
  - [ ] All existing tests pass
  - [ ] Unit tests cover new deserialization path
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

- [ ] Optimize `load_archive_item(id)` with early termination — deferred per PRD scope (separate follow-up item)
- [ ] Combine `show` command's two separate archive reads into single pass — deferred per PRD scope

## Design Details

### Key Types

```rust
/// Lightweight projection for extracting only the `id` field from archive JSON.
/// INVARIANT: Must remain a single-field struct without #[serde(flatten)].
/// Adding flatten would force serde to buffer all unknown fields into an
/// intermediate representation, eliminating the performance win. See TG-004.
#[derive(Deserialize)]
struct IdOnly {
    id: String,
}
```

### Architecture Details

```
CLI Commands (unchanged)
    │
    ▼
Store::load_archive_ids()  ── rewired ──▶  jsonl::read_archive_ids()  ── NEW
    │                                           │
    │ (was: jsonl::read_archive())              │ uses IdOnly { id: String }
    │                                           │ returns HashSet<String>
    ▼                                           ▼
Store::load_all_archive()  ── unchanged ──▶  jsonl::read_archive()
Store::load_archive_item() ── unchanged ──▶  jsonl::read_archive()
```

### Design Rationale

- **Derived `Deserialize` over manual impl:** Serde's derive macro generates optimal `IgnoredAny`-based skipping. Manual impl would be ~30 lines of boilerplate for zero benefit.
- **Mirror `read_archive()` over shared helper:** ~10 lines of schema validation duplication is acceptable for two functions with different return types and deserialization targets. Extraction justified at 3+ functions.
- **`IdOnly` private to `jsonl.rs`:** No current external consumers. Trivial to promote to `pub(crate)` later if needed.
- **Return `HashSet<String>` directly:** Sole consumer returns `HashSet<String>`. Avoids intermediate `Vec` allocation. Duplicate IDs deduplicated naturally.

---

## Retrospective

[Fill in after completion]

### What worked well?

### What was harder than expected?

### What would we do differently next time?
