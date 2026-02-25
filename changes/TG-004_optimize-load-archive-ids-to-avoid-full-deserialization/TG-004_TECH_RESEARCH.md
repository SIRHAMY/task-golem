# Tech Research: Optimize load_archive_ids to Avoid Full Deserialization

**ID:** TG-004
**Status:** Complete
**Created:** 2026-02-24
**PRD:** ./TG-004_PRD.md
**Mode:** Light

## Overview

Researching how to replace the current `load_archive_ids()` implementation — which fully deserializes every archive line into a 14-field `Item` struct (including `DateTime<Utc>`, `Vec<String>`, and `BTreeMap<String, serde_json::Value>` via `#[serde(flatten)]`) just to extract the `id` field — with a lightweight `IdOnly` projection struct that lets serde skip unknown fields cheaply.

## Research Questions

- [x] What is the idiomatic Rust/serde pattern for partial JSON deserialization? → Lightweight projection struct with derived `Deserialize`
- [x] Where does the current deserialization cost actually come from? → Primarily from `#[serde(flatten)]` forcing serde to buffer all unknown fields, plus DateTime parsing and Vec/BTreeMap allocation
- [x] What existing codebase patterns must the new function follow? → Schema validation + skip-and-warn, matching `read_archive()` structure

---

## External Research

### Landscape Overview

Serde provides several mechanisms for partial deserialization. The core insight: serde's default behavior for derived `Deserialize` implementations **silently ignores unknown fields** in self-describing formats like JSON. A struct with only `id: String` will successfully deserialize from JSON containing 14+ fields — serde parses the `id` value and uses `IgnoredAny` to cheaply skip everything else.

The critical performance variable is how unknown fields are handled: cheaply via `IgnoredAny` (default for simple structs) or expensively via buffering into `BTreeMap` (when `#[serde(flatten)]` is present).

### Common Patterns & Approaches

#### Pattern: Lightweight Projection Struct (Recommended)

**How it works:** Define a minimal struct containing only needed fields. Serde's derive macro automatically uses `IgnoredAny` for unknown fields on structs without `#[serde(flatten)]`.

```rust
#[derive(Deserialize)]
struct IdOnly {
    id: String,
}
```

**When to use:** When you need a subset of fields from a known JSON schema and don't need the remaining fields.

**Tradeoffs:**
- Pro: Zero dependencies, trivially simple, uses serde's built-in behavior
- Pro: No `#[serde(flatten)]` overhead — unknown fields discarded via `IgnoredAny` with no allocation
- Con: Still tokenizes the entire JSON line (savings come from avoiding Rust-side allocations, not JSON parsing)

**References:**
- [Serde Container Attributes](https://serde.rs/container-attrs.html) — documents default unknown-field behavior
- [Serde Field Attributes](https://serde.rs/field-attrs.html) — per-field control

#### Pattern: Manual `Deserialize` with `IgnoredAny`

**How it works:** Hand-implement `Deserialize` using `serde::de::Visitor`, explicitly calling `map.next_value::<IgnoredAny>()` for non-id fields.

**When to use:** When derive cannot express the needed behavior or maximum control is required.

**Tradeoffs:**
- Pro: Maximum explicitness, potential early-termination extension point
- Con: ~30 lines of boilerplate for no measurable benefit over derived version (derive already generates `IgnoredAny` skipping)

**References:**
- [Serde IgnoredAny](https://docs.rs/serde/latest/serde/de/struct.IgnoredAny.html) — API reference
- [Serde - Discarding data](https://serde.rs/ignored-any.html) — usage guide
- [Serde - Manually deserialize struct](https://serde.rs/deserialize-struct.html) — manual implementation guide

### Technologies & Tools

| Technology | Purpose | Pros | Cons |
|---|---|---|---|
| serde + serde_json (in use) | JSON deserialization | Standard, well-tested, derive handles unknown fields | Full JSON token stream still walked |
| serde::de::IgnoredAny (built-in) | Efficient discard of unwanted values | No allocation, just walks tokens | Used automatically by derive — manual use adds no benefit |

No new dependencies needed.

### Common Pitfalls

| Pitfall | Why It's a Problem | How to Avoid |
|---------|-------------------|--------------|
| Adding `#[serde(flatten)]` to `IdOnly` | Forces serde to buffer all unknown fields into intermediate `Content` enum — eliminates the performance win | Do not use `flatten` on the projection struct |
| Adding `#[serde(deny_unknown_fields)]` | Would reject every archive line since they contain fields beyond `id` | Rely on serde's default behavior (ignore unknown fields) |
| Assuming JSON tokenization is skipped | serde_json must still walk the full token stream for well-formedness; savings are in Rust-side allocations | Understand that the win is avoiding DateTime parsing, Vec/BTreeMap allocation — not JSON parsing |

### Key Learnings

- The biggest performance win comes from removing `#[serde(flatten)]`, not just reducing field count. With `flatten`, serde buffers all unknown fields into an intermediate representation before collecting them into a map. Without it, `IgnoredAny` discards them with zero allocation.
- Serde's derive macro already generates optimal code for simple structs — manual `Deserialize` implementation adds boilerplate for no measurable benefit.
- Field ordering in JSON doesn't matter — serde scans the full object for declared field names.

---

## Internal Research

### Existing Codebase State

Single-binary Rust CLI application using JSONL backing store. Two JSONL files: `tasks.jsonl` (active items) and `archive.jsonl` (completed items). Both start with a JSON header line `{"schema_version": 1}`. Archive is append-only with fsync for crash-safety.

**Relevant files/modules:**

- `src/store/mod.rs:55-58` — Current `load_archive_ids()`: delegates to `read_archive()` then extracts IDs via `.map(|item| item.id).collect()`
- `src/store/mod.rs:61-64` — `load_archive_item()`: full archive read to find one item (deferred optimization)
- `src/store/mod.rs:73-81` — `all_known_ids()`: calls `load_archive_ids()`, benefits automatically
- `src/store/jsonl.rs:12-15` — `SchemaHeader` struct: reusable for version validation
- `src/store/jsonl.rs:61-117` — `read_archive()`: template function to mirror (schema validation + skip-and-warn pattern)
- `src/model/item.rs:21-50` — `Item` struct: 14 fields including `#[serde(flatten)]` extensions
- `src/errors.rs:48-49` — `TgError::SchemaVersionUnsupported`: existing error variant to reuse

**Callers (all unchanged by this optimization):**
- `src/cli/commands/add.rs:33`, `show.rs:12`, `edit.rs:34`, `rm.rs:27`, `ready.rs:25`, `next.rs:16`, `transition.rs:101`

**Existing patterns in use:**
- Schema validation: read first line → parse as `SchemaHeader` → check version → return `SchemaVersionUnsupported` if mismatch
- Skip-and-warn: malformed lines emit stderr warning with line number and continue; line numbering uses `i + 2` (header is line 1)
- Error messages: `"Warning: could not read archive line {}: {}"` (I/O) and `"Warning: skipping malformed archive line {}: {}"` (JSON)

### Reusable Components

- `SchemaHeader` struct — already defined, exact same version check needed
- `TgError::SchemaVersionUnsupported` — existing error variant
- `CURRENT_SCHEMA_VERSION` constant — existing constant for version comparison
- Test utilities: `make_item()` function in test module for constructing test data

### Constraints from Existing Code

- Must use serde for JSON parsing (consistency with codebase)
- `IdOnly` must not use `#[serde(flatten)]` — defeats the optimization
- Public API signature unchanged: `Result<HashSet<String>, TgError>`
- Warning message format must match existing patterns for consistency
- No new crate dependencies allowed
- Concurrent read safety: archive reads don't acquire locks (same as `load_active()`)

---

## PRD Concerns

| PRD Assumption | Research Finding | Implication |
|----------------|------------------|-------------|
| "Per-line cost reduced from 14 fields to 1 field" | JSON tokenization still walks all tokens; savings are in Rust-side allocations (DateTime, Vec, BTreeMap) | Wording nuance only — the optimization is real and substantial, especially due to `#[serde(flatten)]` removal. No design impact. |

No significant concerns. The PRD is well-researched, the approach is standard, and the scope is correctly bounded.

---

## Critical Areas

### `#[serde(flatten)]` Removal Is the Key Win

**Why it's critical:** The `#[serde(flatten)]` attribute on `Item.extensions` fundamentally changes how serde handles unknown fields — from cheap `IgnoredAny` discard to expensive buffering into an intermediate `Content` enum. Removing this (by not including it on `IdOnly`) is where the bulk of per-line savings comes from.

**Why it's easy to miss:** Someone might think "fewer fields = faster" when the real story is "no flatten = fundamentally different deserialization strategy."

**What to watch for:** Ensure `IdOnly` never gets `#[serde(flatten)]` added, even in future refactoring.

### Behavioral Divergence on Corrupted Lines

**Why it's critical:** Lines with valid JSON containing a string `id` but corrupted other fields (e.g., malformed timestamps) will be included by `IdOnly` but would have been skipped by full `Item` deserialization. This is intentional for collision avoidance.

**Why it's easy to miss:** It's a subtle semantic difference between the old and new code paths that could be perceived as a bug.

**What to watch for:** Document this divergence in code comments. The PRD correctly identifies this as the right trade-off (false positives harmless, false negatives cause data corruption).

---

## Deep Dives

*No deep dives needed — the problem space is well-understood and the solution approach is standard.*

---

## Synthesis

### Open Questions

*None — all research questions have been answered. The approach is clear and well-supported by both external patterns and internal codebase analysis.*

### Recommended Approaches

#### Deserialization Strategy

| Approach | Pros | Cons | Best When |
|----------|------|------|-----------|
| Derived projection struct (`IdOnly`) | Simple, idiomatic, zero boilerplate, serde generates optimal `IgnoredAny` code | Still tokenizes full JSON line | Standard partial deserialization (this use case) |
| Manual `Deserialize` impl | Maximum control, explicit `IgnoredAny` | ~30 lines boilerplate, no measurable benefit over derive | Derive cannot express needed behavior (not this case) |
| Raw string scanning / regex | Avoids JSON parsing entirely | Fragile, assumes key ordering, PRD explicitly excludes | Never for this use case |

**Initial recommendation:** Derived projection struct. It's the idiomatic, simplest, and most maintainable approach. Serde's derive macro already generates optimal code for this case.

#### Function Structure

| Approach | Pros | Cons | Best When |
|----------|------|------|-----------|
| Mirror `read_archive()` structure | Consistent with existing code, easy to review, reuses same error patterns | Some code duplication with `read_archive()` | Functions have different return types and semantics (this case) |
| Extract shared helper for schema validation | Reduces duplication | Over-engineering for 5-line pattern; couples two functions | Many archive-reading functions exist (not yet the case) |

**Initial recommendation:** Mirror `read_archive()` structure. The duplication is minimal (schema header validation is ~10 lines) and consistency is more valuable than DRY for two functions. A shared helper can be extracted later if more archive-reading functions are added.

### Key References

| Reference | Type | Why It's Useful |
|-----------|------|-----------------|
| [Serde Container Attributes](https://serde.rs/container-attrs.html) | Docs | Documents default unknown-field behavior and `deny_unknown_fields` |
| [Serde IgnoredAny](https://docs.rs/serde/latest/serde/de/struct.IgnoredAny.html) | API Docs | How serde efficiently discards unwanted values |
| [Serde Flatten](https://serde.rs/attr-flatten.html) | Docs | Documents flatten behavior, limitations, and performance implications |
| [serde_fast_flatten benchmarks](https://github.com/maartendeprez/serde_fast_flatten) | Code/Benchmarks | Demonstrates ~2x deserialization overhead from `#[serde(flatten)]` |

---

## Research Log

| Date | Activity | Outcome |
|------|----------|---------|
| 2026-02-24 | External research: partial serde deserialization patterns | Identified projection struct as clear winner; documented `flatten` as key cost driver |
| 2026-02-24 | Internal research: codebase exploration | Mapped all relevant files, patterns, integration points, and callers |
| 2026-02-24 | PRD analysis | No significant concerns; PRD well-aligned with research findings |
| 2026-02-24 | Research finalized | All questions answered, clear recommendation for design phase |
