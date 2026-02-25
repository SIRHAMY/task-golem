# Change: Optimize load_archive_ids to avoid full deserialization

**Status:** Proposed
**Created:** 2026-02-24
**Author:** phase-golem (autonomous)

## Problem Statement

Every CLI command that performs ID resolution (`add`, `edit`, `next`, `ready`, `rm`, `show`, `transition`) fully deserializes the entire `archive.jsonl` file into complete `Item` structs -- parsing all 14 fields including `DateTime<Utc>`, `Vec<String>`, `Option<T>`, and `BTreeMap<String, serde_json::Value>` (flattened extensions, where arbitrary extension keys like `x-agent` are serialized as top-level JSON keys) -- only to extract and discard everything except the `id: String` field.

The doc comment on `load_archive_ids()` at `src/store/mod.rs:54` says "Scan archive line-by-line extracting only IDs (fast path)" but the implementation delegates to `read_archive()` which returns `Vec<Item>`, performing full deserialization including DateTime parsing, Vec/BTreeMap allocation, and extension field collection. The archive grows unboundedly as tasks are completed, meaning this cost increases linearly with project age and is paid on nearly every command invocation.

## User Stories / Personas

- **AI Agent** - Automated task orchestrators that run CLI commands in tight loops (check ready queue, pick task, transition to doing, complete, repeat). Each invocation pays the full deserialization cost. Over long-running projects with hundreds of completed tasks, this overhead accumulates as a constant tax on every command.

- **Human CLI User** - Uses `tg` interactively on long-lived projects. Experiences progressively slower response times as the archive grows, with no obvious cause or workaround.

## Desired Outcome

`load_archive_ids()` should deserialize only the `id` field from each archive line, avoiding the cost of DateTime parsing, Vec allocation, Option field handling, BTreeMap construction, and extension field collection. The public API remains identical -- callers continue receiving `HashSet<String>` -- but the per-line deserialization cost is reduced from 14 fields (including complex types) to 1 string field.

## Success Criteria

### Must Have

- [ ] `load_archive_ids()` uses a lightweight `IdOnly` struct (not the full `Item` struct) for deserialization
- [ ] Public API signature of `Store::load_archive_ids()` is unchanged (`Result<HashSet<String>, TgError>`)
- [ ] All existing callers (`add`, `edit`, `next`, `ready`, `rm`, `show`, `transition`, and `all_known_ids`) continue working without modification
- [ ] Invalid JSON lines are skipped with stderr warnings, matching current `read_archive` resilience semantics
- [ ] Lines where `IdOnly` deserialization succeeds (valid JSON with a string `id` field) but full `Item` deserialization would fail (corrupted other fields) are included in the returned ID set -- this is intentionally conservative for ID collision avoidance (prevents generating an ID that collides with a partially-corrupted archived item)
- [ ] Lines that are not valid JSON, or valid JSON missing the `id` field or with a non-string `id`, are skipped with warnings
- [ ] Schema version header is validated; unsupported versions return `TgError::SchemaVersionUnsupported` (matching existing behavior)
- [ ] Empty/missing archive files return empty `HashSet`
- [ ] All existing tests pass

### Should Have

- [ ] Unit tests for the new ID-only deserialization path covering: happy path, malformed JSON lines, truncated lines, empty/missing files, bad schema versions, and lines with valid `id` but corrupted other fields

### Nice to Have

- [ ] Optimize `load_archive_item(id)` with early termination (stop reading after finding the target ID instead of loading the entire archive) -- deferred to a separate follow-up item

## Scope

### In Scope

- New private `IdOnly` deserialization struct in `src/store/jsonl.rs` (only the `id: String` field, no `#[serde(flatten)]`)
- New `read_archive_ids()` function in `src/store/jsonl.rs` that mirrors `read_archive()` structure (schema header validation, skip-and-warn on malformed lines) but deserializes into `IdOnly`, returning `HashSet<String>` directly
- Rewiring `Store::load_archive_ids()` in `src/store/mod.rs` to call the new function
- Unit tests for the new code path

### Out of Scope

- Raw string scanning / regex-based ID extraction (fragile, assumes key ordering)
- Separate index/cache file for archive IDs (disproportionate complexity for current project scale)
- Changes to `load_all_archive()` or the full-deserialization path
- Changes to the `Item` struct or serialization format
- Changes to any CLI command files (they call `load_archive_ids()` which keeps its API)
- Optimizing `load_archive_item()` with early termination (separate follow-up)
- Combining `show` command's two separate archive reads (one for ID resolution, one to fetch the item) into a single pass (separate follow-up)
- Benchmarking infrastructure (performance improvement is verified by code review confirming the deserialization path avoids complex field parsing)

## Non-Functional Requirements

- **Performance:** Per-line deserialization cost of `load_archive_ids` should reflect only the `id` field, not the full `Item` struct complexity. Specifically, the `IdOnly` struct avoids: DateTime parsing (2 fields), Vec allocation (2 fields), Option handling (5 fields), BTreeMap construction, and extension field collection via `#[serde(flatten)]`.

## Constraints

- Must use serde for JSON parsing (no raw string scanning) to maintain consistency with the rest of the codebase
- The `IdOnly` struct must tolerate unknown fields in the JSON (serde's default behavior ignores unknown fields) since archive lines contain all `Item` fields plus arbitrary extension keys
- No new crate dependencies
- Archive reads are concurrent-safe without locks: the archive file uses append-only writes with fsync, and OS-level guarantees ensure readers see consistent line-level data. Multiple processes may call `load_archive_ids()` concurrently. (This matches the existing `load_active` pattern for read-only operations.)

## Dependencies

- **Depends On:** None -- this is a pure internal optimization with no external dependencies
- **Blocks:** Nothing directly, but improves performance for all commands that do ID resolution

## Risks

- [ ] **Behavioral divergence on corrupted lines:** A line with valid JSON containing a string `id` field but corrupted other fields (e.g., malformed `created_at` timestamp) would be included by `IdOnly` deserialization but skipped by full `Item` deserialization. This is intentional: for collision avoidance purposes, the ID should be treated as "taken" even if the full record cannot be recovered. This divergence is the correct trade-off -- false positives (blocking an ID) are harmless while false negatives (allowing a duplicate ID) would cause data corruption.

## Decisions

- **`IdOnly` visibility:** Private to `jsonl.rs`. It is a deserialization-only struct with no external consumers. Trivial to promote to `pub(crate)` later if a future optimization (e.g., `load_archive_item` early termination) reuses it.
- **`read_archive_ids` return type:** Returns `HashSet<String>` directly, matching the public API of `Store::load_archive_ids()` and avoiding an intermediate `Vec<String>` allocation.

## Assumptions (autonomous mode)

- **Mode:** Light/medium -- this is a straightforward, well-understood optimization. The problem is clear, the solution approach is standard, and scope is tightly bounded.
- **No discovery flag needed** -- the solution space is well-known (partial serde deserialization is a standard Rust pattern).
- **`IdOnly` without `#[serde(flatten)]`** is correct -- we want serde to skip unknown fields cheaply (ignoring them), not collect them into a BTreeMap like the full `Item` struct does.
- **The `id` field position in JSON output is not relied upon** -- the `IdOnly` struct works regardless of field ordering in the JSON line, since serde scans the full JSON object for the named field.
- **Performance verification via code review** -- Benchmarking infrastructure is out of scope. The optimization is verified correct by confirming the new deserialization path instantiates only `IdOnly { id: String }` and never touches DateTime/Vec/BTreeMap parsing.

## References

- `src/store/mod.rs:55-58` -- current `load_archive_ids` implementation
- `src/store/mod.rs:61-64` -- current `load_archive_item` implementation (related, deferred)
- `src/store/mod.rs:73-81` -- `all_known_ids` (calls `load_archive_ids`, benefits automatically)
- `src/store/jsonl.rs:61-117` -- current `read_archive` function (structure to mirror)
- `src/model/item.rs:21-50` -- `Item` struct definition (14 fields being avoided)
