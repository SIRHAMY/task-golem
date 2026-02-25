# Design: Optimize load_archive_ids to Avoid Full Deserialization

**ID:** TG-004
**Status:** Initial
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

### Component Breakdown

#### `IdOnly` Struct (new, private to `jsonl.rs`)

**Purpose:** Lightweight serde projection struct for extracting only the `id` field from archive JSON lines.

**Responsibilities:**
- Deserialize the `id` field from a JSON object
- Silently ignore all other fields (serde default behavior via `IgnoredAny`)

**Interfaces:**
- Input: JSON string (one archive line)
- Output: `IdOnly { id: String }`

**Dependencies:** `serde::Deserialize` (already in scope)

#### `read_archive_ids()` Function (new, `pub` in `jsonl.rs`)

**Purpose:** Read archive JSONL file and return only the set of IDs, using lightweight deserialization.

**Responsibilities:**
- Validate schema header (same pattern as `read_archive()`)
- Iterate lines, deserializing each into `IdOnly`
- Skip-and-warn on I/O errors and malformed JSON (same resilience pattern as `read_archive()`)
- Collect IDs into `HashSet<String>` directly
- Handle empty/missing files by returning empty `HashSet`

**Interfaces:**
- Input: `&Path` (archive file path)
- Output: `Result<HashSet<String>, TgError>`

**Dependencies:** `SchemaHeader`, `CURRENT_SCHEMA_VERSION`, `TgError` (all existing)

### Data Flow

1. CLI command calls `Store::load_archive_ids()`
2. `Store` delegates to `jsonl::read_archive_ids(&self.archive_path())`
3. `read_archive_ids()` opens file, validates schema header
4. For each data line: deserialize into `IdOnly`, insert `id` into `HashSet`
5. Return `HashSet<String>` to caller

### Key Flows

#### Flow: Happy Path — Load Archive IDs

> Extract all IDs from the archive file using lightweight deserialization.

1. **Open file** — Return empty `HashSet` if file doesn't exist
2. **Read header** — Parse first line as `SchemaHeader`, return empty `HashSet` if file is empty
3. **Validate version** — Return `TgError::SchemaVersionUnsupported` if version mismatch
4. **Iterate lines** — For each remaining line:
   - Skip empty/whitespace-only lines
   - Deserialize as `IdOnly`
   - Insert `id` into `HashSet`
5. **Return** — `Ok(HashSet<String>)`

**Edge cases:**
- Missing file → empty `HashSet`
- Empty file (0 bytes) → empty `HashSet`
- Header-only file → empty `HashSet`
- I/O error reading a line → warn to stderr, skip line, continue
- Malformed JSON line → warn to stderr, skip line, continue
- Valid JSON with `id` field but corrupted other fields → **included** (intentional: collision avoidance)
- Truncated last line (crash recovery) → treated as malformed, skipped with warning

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

**Rationale:** The duplication is minimal, the functions have different return types and different deserialization targets, and extracting a generic iterator helper would add abstraction complexity disproportionate to the duplication saved. Tech research recommends this approach. A shared helper can be extracted if more archive-reading functions are added later.

**Consequences:** Two functions with similar-but-not-identical structure. Easy to maintain, easy to review, consistent with existing codebase patterns.

#### Decision: `IdOnly` is Private to `jsonl.rs`

**Context:** `IdOnly` could be `pub(crate)` for potential reuse by future optimizations.

**Decision:** Keep it private (`struct IdOnly`, no `pub` modifier).

**Rationale:** It has no current external consumers. Promoting to `pub(crate)` later is a trivial one-word change if needed (e.g., for `load_archive_item` early termination).

**Consequences:** Minimal API surface. Clean encapsulation.

#### Decision: Return `HashSet<String>` Directly

**Context:** Could return `Vec<String>` and let the caller collect into `HashSet`.

**Decision:** Return `HashSet<String>` directly.

**Rationale:** The sole consumer (`Store::load_archive_ids()`) returns `HashSet<String>`. Collecting directly avoids an intermediate `Vec` allocation. This matches the existing public API.

**Consequences:** Slightly more opinionated function, but perfectly matched to its only use case.

### Tradeoffs Accepted

| Tradeoff | We're Accepting | In Exchange For | Why This Makes Sense |
|----------|-----------------|-----------------|---------------------|
| Code duplication | ~10 lines of schema validation duplicated between `read_archive` and `read_archive_ids` | Simplicity — no generic abstraction layer | Only two functions with this pattern; extraction would be premature |
| Behavioral divergence | Lines with valid `id` but corrupted other fields are included (old path would skip them) | Conservative collision avoidance — IDs treated as "taken" even if record is partially corrupted | False positives (blocking an ID) are harmless; false negatives (allowing duplicate IDs) cause data corruption |
| Full JSON tokenization | serde_json still walks the entire token stream of each line | Consistency with serde-based codebase (no raw string scanning) | The win is in avoiding Rust-side allocations (DateTime, Vec, BTreeMap, flatten buffering), not JSON parsing |

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
| Future refactor adds `#[serde(flatten)]` to `IdOnly` | Eliminates the performance win silently | Low — `IdOnly` is a private, single-purpose struct | Code comment warning against `flatten`; the struct has no `extensions` field to flatten into |
| Warning message format diverges from `read_archive` | Inconsistent stderr output | Low | Mirror existing format strings exactly |

---

## Integration Points

### Existing Code Touchpoints

- `src/store/jsonl.rs` — Add `IdOnly` struct and `read_archive_ids()` function; add unit tests to existing `#[cfg(test)]` module
- `src/store/mod.rs:55-58` — Rewire `load_archive_ids()` to call `jsonl::read_archive_ids()` instead of `jsonl::read_archive()`

### External Dependencies

None. Pure internal change using existing crates (`serde`, `serde_json`, `std::collections::HashSet`).

---

## Open Questions

None. The design is fully resolved.

---

## Design Review Checklist

Before moving to SPEC:

- [x] Design addresses all PRD requirements — `IdOnly` struct, unchanged API, skip-and-warn, schema validation, behavioral divergence documented
- [x] Key flows are documented and make sense — happy path with all edge cases
- [x] Tradeoffs are explicitly documented and acceptable — code duplication, behavioral divergence, full tokenization
- [x] Integration points with existing code are identified — `jsonl.rs` (new code) and `mod.rs` (rewire)
- [x] No major open questions remain

---

## Design Log

| Date | Activity | Outcome |
|------|----------|---------|
| 2026-02-24 | Initial design draft | Straightforward design: `IdOnly` struct + `read_archive_ids()` mirroring `read_archive()` structure |
