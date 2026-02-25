# Design: Optimize load_archive_ids to Avoid Full Deserialization

**ID:** TG-004
**Status:** Complete
**Created:** 2026-02-24
**PRD:** ./TG-004_PRD.md
**Tech Research:** ./TG-004_TECH_RESEARCH.md
**Mode:** Light

## Overview

Replace the current `load_archive_ids()` implementation — which fully deserializes every archive line into a 14-field `Item` struct (including `DateTime<Utc>`, `Vec<String>`, and `BTreeMap<String, serde_json::Value>` via `#[serde(flatten)]`) — with a dedicated `read_archive_ids()` function that deserializes into a lightweight `IdOnly { id: String }` projection struct. Serde's derive macro automatically uses `IgnoredAny` to skip unknown fields with zero allocation, and the absence of `#[serde(flatten)]` eliminates the expensive buffering that is the primary cost driver. The public API is unchanged.

---

## System Design

### High-Level Architecture

The change is surgical: a single new struct and a single new function in `src/store/jsonl.rs`, plus a one-line rewire in `src/store/mod.rs`. No new modules, no new files, no new dependencies.

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

**Concurrency model:** Archive reads do not acquire locks, matching the existing `load_active()` pattern for read-only operations. The archive file uses append-only writes with fsync, and OS-level guarantees ensure readers see consistent line-level data. If a reader races with a writer and encounters a partially-written line, the skip-and-warn pattern handles it gracefully (treated as malformed JSON, skipped with warning). No new synchronization mechanisms are needed.

### Component Breakdown

#### `IdOnly` Struct (new, private to `jsonl.rs`)

**Purpose:** Lightweight serde projection struct for extracting only the `id` field from archive JSON lines.

**Struct definition guidance:**
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

**Responsibilities:**
- Deserialize the `id` field from a JSON object
- Silently ignore all other fields (serde default behavior via `IgnoredAny`)

**Interfaces:**
- Input: JSON string (one archive line)
- Output: `IdOnly { id: String }`

**Dependencies:** `serde::Deserialize` (already in scope)

**Behavioral note:** `IdOnly` deserialization succeeds for any valid JSON object containing a string `id` field, regardless of the validity of other fields. This means lines with corrupted non-`id` fields (e.g., malformed timestamps, wrong types) are included — this is intentional for collision avoidance. See Tradeoffs section.

#### `read_archive_ids()` Function (new, `pub` in `jsonl.rs`)

**Purpose:** Read archive JSONL file and return only the set of IDs, using lightweight deserialization.

**Doc comment guidance:**
```rust
/// Read archive JSONL, extracting only IDs (fast path).
///
/// Uses a lightweight `IdOnly` projection struct instead of full `Item`
/// deserialization, avoiding DateTime parsing, Vec/BTreeMap allocation,
/// and `#[serde(flatten)]` buffering overhead.
///
/// Note: Lines with valid JSON containing a string `id` field are included
/// even if other fields are malformed. This diverges from full `Item`
/// deserialization (which would reject such lines) and is intentional:
/// for collision avoidance, an ID should be treated as "taken" even if
/// the full record cannot be recovered.
```

**Responsibilities:**
- Validate schema header (same pattern as `read_archive()`)
- Iterate lines, deserializing each into `IdOnly`
- Skip-and-warn on I/O errors and malformed JSON (same resilience pattern as `read_archive()`)
- Collect IDs into `HashSet<String>` directly (duplicate IDs silently deduplicated)
- Handle empty/missing files by returning empty `HashSet`

**Interfaces:**
- Input: `&Path` (archive file path)
- Output: `Result<HashSet<String>, TgError>`

**Dependencies:** `SchemaHeader`, `CURRENT_SCHEMA_VERSION`, `TgError` (all existing; no new imports needed beyond `std::collections::HashSet`)

**Error handling contract:**
- File does not exist → `Ok(HashSet::new())`
- File exists but cannot be opened (permission denied, etc.) → `Err(TgError::IoError(...))`
- Empty file (0 bytes) or header-only → `Ok(HashSet::new())`
- Invalid schema header JSON → `Err(TgError::StorageCorruption(...))`
- Unsupported schema version → `Err(TgError::SchemaVersionUnsupported { ... })`
- I/O error reading a data line → warn to stderr, skip line, continue
- Malformed JSON data line (including truncated lines) → warn to stderr, skip line, continue
- Valid JSON with `id` but corrupted other fields → **included** (no warning)
- `Ok(HashSet<String>)` is returned as long as the file can be opened and the header is valid. Errors on individual data lines are non-fatal; the returned set contains all successfully-deserialized IDs.

**Warning message format** (must match `read_archive()` exactly):
- I/O error: `"Warning: could not read archive line {line_num}: {error}"` where `line_num = i + 2` (header is line 1)
- Malformed JSON: `"Warning: skipping malformed archive line {line_num}: {error}"` where `line_num = i + 2`

### Data Flow

1. CLI command calls `Store::load_archive_ids()`
2. `Store` delegates to `jsonl::read_archive_ids(&self.archive_path())`
3. `read_archive_ids()` opens file, validates schema header
4. For each data line: deserialize into `IdOnly`, insert `id` into `HashSet`
5. Return `HashSet<String>` to caller

### Key Flows

#### Flow: Happy Path — Load Archive IDs

> Extract all IDs from the archive file using lightweight deserialization.

1. **Open file** — Return empty `HashSet` if file doesn't exist; propagate `TgError::IoError` if file exists but cannot be opened (permission denied, etc.)
2. **Read header** — Parse first line as `SchemaHeader`; return empty `HashSet` if file is empty (no lines)
3. **Validate version** — Return `TgError::SchemaVersionUnsupported` if version mismatch; return `TgError::StorageCorruption` if header is not valid JSON
4. **Iterate lines** — For each remaining line:
   - Skip empty/whitespace-only lines
   - On I/O error reading line: warn to stderr with line number, skip, continue
   - Deserialize as `IdOnly`: on success, insert `id` into `HashSet`; on failure, warn to stderr with line number, skip, continue
5. **Return** — `Ok(HashSet<String>)` containing all successfully-deserialized IDs

**Edge cases:**
- Missing file → empty `HashSet`
- Empty file (0 bytes) → empty `HashSet`
- Header-only file → empty `HashSet`
- File permission error → `Err(TgError::IoError(...))`
- I/O error reading a data line → warn to stderr, skip line, continue
- Malformed JSON data line → warn to stderr, skip line, continue
- Valid JSON missing `id` field → deserialization fails (serde requires the field), skipped with warning
- Valid JSON with non-string `id` → deserialization fails (type mismatch), skipped with warning
- Valid JSON with `id` field but corrupted other fields → **included** (intentional: collision avoidance)
- Truncated last line (crash recovery) → treated as malformed JSON, skipped with warning
- Duplicate IDs in archive → silently deduplicated by `HashSet::insert()`

---

## Technical Decisions

### Key Decisions

#### Decision: Derived `Deserialize` over Manual Implementation

**Context:** Two approaches exist for partial deserialization: derive `Deserialize` on a projection struct, or hand-implement `Deserialize` with explicit `IgnoredAny` handling.

**Decision:** Use `#[derive(Deserialize)]` on `IdOnly`.

**Rationale:** Serde's derive macro already generates optimal `IgnoredAny`-based skipping for structs without `#[serde(flatten)]`. A manual implementation would add ~30 lines of boilerplate for zero measurable benefit. The tech research confirmed this.

**Consequences:** Simple, idiomatic, minimal code to maintain. If early-termination is ever needed (stop after finding `id` field), a manual impl could be added later — but that's a future concern and the JSON tokenizer must walk the full line anyway.

#### Decision: Mirror `read_archive()` Structure (Not Extract Shared Helper)

**Context:** `read_archive_ids()` shares the schema-header-validation and skip-and-warn patterns with `read_archive()`. Could extract a shared helper.

**Decision:** Duplicate the ~10-line schema validation pattern.

**Rationale:** The duplication is minimal (~10 lines of schema header reading and validation), the functions have different return types and different deserialization targets, and extracting a generic iterator helper would add abstraction complexity disproportionate to the duplication saved. Tech research recommends this approach.

**Extraction threshold:** If a third archive-reading function is added in the future, extraction of a shared schema-validation helper becomes justified. Until then, keeping the pattern inline in each function is simpler and easier to review.

**Consequences:** Two functions with similar-but-not-identical structure. Easy to maintain, easy to review, consistent with existing codebase patterns.

#### Decision: `IdOnly` is Private to `jsonl.rs`

**Context:** `IdOnly` could be `pub(crate)` for potential reuse by future optimizations.

**Decision:** Keep it private (`struct IdOnly`, no `pub` modifier).

**Rationale:** It has no current external consumers. Promoting to `pub(crate)` later is a trivial one-word change if needed (e.g., for `load_archive_item` early termination, which is deferred to a separate follow-up).

**Consequences:** Minimal API surface. Clean encapsulation.

#### Decision: Return `HashSet<String>` Directly

**Context:** Could return `Vec<String>` and let the caller collect into `HashSet`.

**Decision:** Return `HashSet<String>` directly.

**Rationale:** The sole consumer (`Store::load_archive_ids()`) returns `HashSet<String>`. Collecting directly avoids an intermediate `Vec` allocation. Archive IDs are unique by design, so the deduplication property of `HashSet` is a natural fit. All known callers treat the result as an unordered set for ID collision checks.

**Consequences:** Slightly more opinionated function, but perfectly matched to its only use case.

### Tradeoffs Accepted

| Tradeoff | We're Accepting | In Exchange For | Why This Makes Sense |
|----------|-----------------|-----------------|---------------------|
| Code duplication | ~10 lines of schema validation duplicated between `read_archive` and `read_archive_ids` | Simplicity — no generic abstraction layer | Only two functions with this pattern; extraction justified when a third is added |
| Behavioral divergence | Lines with valid `id` but corrupted other fields are included (old path would skip them) | Conservative collision avoidance — IDs treated as "taken" even if record is partially corrupted | False positives (blocking an ID) are harmless; false negatives (allowing duplicate IDs) cause data corruption. This also affects `all_known_ids()`, which may see IDs that `load_all_archive()` cannot recover — this is correct behavior for collision avoidance. |
| Full JSON tokenization | serde_json still walks the entire token stream of each line | Consistency with serde-based codebase (no raw string scanning) | The win is in avoiding Rust-side allocations (DateTime, Vec, BTreeMap, flatten buffering), not JSON parsing. For typical archive line sizes, tokenization cost is small relative to allocation cost. |

---

## Alternatives Considered

### Alternative: Manual `Deserialize` Implementation

**Summary:** Hand-implement `serde::Deserialize` for `IdOnly` using a `Visitor`, explicitly calling `map.next_value::<IgnoredAny>()` for non-id fields.

**How it would work:**
- Define a `Visitor` struct implementing `visit_map`
- Loop through map entries, match on `"id"` key, skip others with `IgnoredAny`

**Pros:**
- Maximum explicitness about what's happening
- Could be extended for early termination (though tokenizer must still walk full line)

**Cons:**
- ~30 lines of boilerplate
- No measurable performance benefit — derive generates equivalent `IgnoredAny` code
- More surface area for bugs

**Why not chosen:** Derive generates optimal code for this case. Complexity without benefit.

---

## Risks

| Risk | Impact | Likelihood | Mitigation |
|------|--------|------------|------------|
| Future refactor adds `#[serde(flatten)]` to `IdOnly` | Eliminates the performance win silently | Low — `IdOnly` is a private, single-purpose struct with no `extensions` field | Defensive code comment on struct explaining the invariant and referencing TG-004; the struct's single-field design makes adding `flatten` nonsensical |
| Warning message format diverges from `read_archive` | Inconsistent stderr output | Low | Copy exact format strings from `read_archive()` and use `i + 2` line numbering convention |

---

## Integration Points

### Existing Code Touchpoints

- `src/store/jsonl.rs` — Add `IdOnly` struct and `read_archive_ids()` function; add unit tests to existing `#[cfg(test)]` module
- `src/store/mod.rs:55-58` — Rewire `load_archive_ids()` body to: `jsonl::read_archive_ids(&self.archive_path())` (single-line change, removes `.map(|item| item.id).collect()`)

### External Dependencies

None. Pure internal change using existing crates (`serde`, `serde_json`, `std::collections::HashSet`). All required types are already imported or accessible within `jsonl.rs`.

---

## Test Cases

Unit tests should be added to the existing `#[cfg(test)]` module in `src/store/jsonl.rs`:

1. **Happy path** — Valid archive with multiple items; verify all IDs returned in `HashSet`
2. **Empty/missing file** — Nonexistent file returns empty `HashSet`
3. **Header-only file** — File with schema header but no data lines returns empty `HashSet`
4. **Malformed JSON line** — Line with invalid JSON is skipped; valid lines still included
5. **Truncated last line** — Incomplete JSON at end of file is skipped; earlier valid lines included
6. **Schema version mismatch** — Returns `TgError::SchemaVersionUnsupported`
7. **Valid `id` with corrupted other fields** — Line like `{"id":"tg-abc12","created_at":"INVALID","status":999}` is included (verifies the intentional behavioral divergence)
8. **Missing `id` field** — Valid JSON without `id` field (e.g., `{"title":"test"}`) is skipped with warning
9. **Duplicate IDs** — Same ID appearing multiple times results in single entry in `HashSet`

---

## Open Questions

None. The design is fully resolved.

---

## Design Review Checklist

Before moving to SPEC:

- [x] Design addresses all PRD requirements — `IdOnly` struct, unchanged API, skip-and-warn, schema validation, behavioral divergence documented
- [x] Key flows are documented and make sense — happy path with all edge cases, error handling contract specified
- [x] Tradeoffs are explicitly documented and acceptable — code duplication, behavioral divergence, full tokenization
- [x] Integration points with existing code are identified — `jsonl.rs` (new code) and `mod.rs` (rewire)
- [x] No major open questions remain

---

## Assumptions (autonomous mode)

- **Mode selection:** Light — this is a straightforward, well-understood optimization with a single clear approach. No directional decisions or competing alternatives needed human input.
- **Self-critique auto-fixes applied:** Strengthened error handling contract, added concurrency model note, specified warning message format, added explicit test cases list, documented `IdOnly` struct invariant, clarified behavioral divergence in function doc comment, added `HashSet` dedup note, documented code duplication extraction threshold.
- **No directional items surfaced:** All 7 critique agents found only documentation/specification improvements, not architectural concerns. No decisions required human input.

---

## Design Log

| Date | Activity | Outcome |
|------|----------|---------|
| 2026-02-24 | Initial design draft | Straightforward design: `IdOnly` struct + `read_archive_ids()` mirroring `read_archive()` structure |
| 2026-02-24 | Self-critique (7 agents) | 46 raw findings, 8 themes after dedup; all auto-fixable documentation improvements |
| 2026-02-24 | Auto-fixes applied, design finalized | Added: error contract, concurrency note, warning format, test cases, struct invariant doc, extraction threshold |
