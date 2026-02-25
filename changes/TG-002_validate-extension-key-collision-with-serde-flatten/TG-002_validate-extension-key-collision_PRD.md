# Change: Validate extension key collision with serde(flatten)

**Status:** Proposed
**Created:** 2026-02-24
**Author:** Claude (autonomous)

## Problem Statement

The `Item` struct uses `#[serde(flatten)]` — a serde attribute that merges a map's keys into the parent struct's JSON representation — to store extension fields (a `BTreeMap<String, serde_json::Value>`) at the top level of each JSON object. While the CLI's `--set` flag enforces an `x-` prefix on extension keys (via `extensions::parse_dot_path`, the CLI function that validates extension key format), no validation exists at the model or deserialization level. This creates two risks:

1. **Silent data loss on round-trip via programmatic construction**: If code directly inserts a key like `"id"` or `"status"` into the extensions BTreeMap, serialization with `serde(flatten)` produces duplicate JSON keys (one from the struct field, one from extensions). On re-deserialization, the duplicate is resolved by serde's parser — the extension entry is silently lost.

2. **Schema pollution on deserialization**: When reading externally-crafted or manually-edited JSONL (JSON Lines, newline-delimited JSON) files, any unknown key that doesn't match a known struct field is captured into extensions — even without the required `x-` prefix. This can accumulate garbage keys that persist through subsequent saves.

**Important technical nuance:** With `serde(flatten)`, during deserialization serde routes known field names (e.g., `"id"`, `"status"`) to their struct fields, NOT to the extensions map. Therefore, post-deserialization validation of the extensions map can detect non-`x-`-prefixed keys (risk #2) but cannot detect known-field collisions that already happened in the JSON (risk #1). Risk #1 is prevented by validating extensions at construction/write time, while risk #2 is caught by post-deserialization validation.

Enforcing the `x-` prefix at the model level (not just the CLI boundary) provides defense-in-depth — validating at multiple layers to catch violations regardless of entry point.

## User Stories / Personas

- **AI Agent** — Sets extension fields via `--set` to store agent-specific metadata. Expects extension data to survive round-trips (serialize then deserialize) without silent corruption, even if another tool modifies the JSONL file.

- **Human Operator** — May hand-edit JSONL files or use external scripts to manipulate task data. Should get clear errors when introducing extension keys that violate the `x-` prefix convention.

- **Tool Author** — Builds tooling on top of task-golem's JSONL format. Needs the schema contract enforced at the data layer, not just the CLI layer.

## Desired Outcome

An `Item::validate_extensions()` method validates that all extension keys start with `x-` and that no extension key matches a known `Item` field name. This method is called:

1. **After deserialization** in the store layer — catches schema pollution from hand-edited files (non-`x-` keys)
2. **Before serialization** or at construction time — catches programmatic collision with known field names

Validation failures produce clear error messages. Rejecting invalid extensions prevents silent data loss and keeps the extensions map clean.

## Success Criteria

### Must Have

- [ ] `Item::validate_extensions()` method rejects (returns error) if the extensions map contains any key that does not start with `x-`
- [ ] The error message names the offending key and suggests the fix, e.g.: `Extension key 'bogus' must start with 'x-' prefix. Rename to 'x-bogus' or remove it.`
- [ ] The method also rejects extension keys matching known `Item` field names (`id`, `title`, `status`, `priority`, `description`, `tags`, `dependencies`, `created_at`, `updated_at`, `blocked_reason`, `blocked_from_status`, `claimed_by`, `claimed_at`) with an error like: `Extension key 'status' collides with a known Item field name.`
- [ ] Validation is called after deserialization in `jsonl::read_active()`. Failures are hard errors (consistent with existing malformed-line behavior in active store).
- [ ] Existing valid JSONL files (with only `x-`-prefixed extension keys or no extensions) continue to load without error
- [ ] Unit tests cover: (a) valid `x-`-prefixed keys pass, (b) non-`x-` key rejected, (c) known-field-name key rejected, (d) empty extensions pass
- [ ] A unit test asserts the known-field-name list matches the actual `Item` struct to prevent drift when fields are added or removed

### Should Have

- [ ] Validation is called after deserialization in `jsonl::read_archive()`, with failures treated as skip-with-warning (consistent with archive reader's lenient error handling)

### Nice to Have

- [ ] `tg doctor` reports items with non-`x-`-prefixed extension keys as warnings

## Scope

### In Scope

- `Item::validate_extensions()` method checking `x-` prefix and known-field collision
- Calling validation after deserialization in the store layer
- Hard-coded known-field-name list with a test to prevent drift
- Error reporting with actionable messages
- Unit tests for validation logic

### Out of Scope

- Custom serde `Deserialize` implementation (post-deserialization check is simpler)
- Changing the `x-` prefix convention itself
- Migrating existing data that might have non-`x-` extension keys (no known instances exist; if discovered, a separate migration task will be created)
- Validating extension *values* (only top-level keys are validated; nested extension object contents are not checked)
- Changes to the `--set` CLI path (already correctly enforces `x-` prefix)
- Pre-serialization validation hooks (the CLI write path already enforces `x-` via `apply_sets`)
- Validation of case variants (collision detection is case-sensitive, matching serde's exact field name matching)

## Non-Functional Requirements

- **Performance:** Validation is a single O(n) pass over extension keys per deserialized item, where n is the number of extension keys (typically 0-5)

## Constraints

- Must not break existing valid JSONL files
- Must use `serde(flatten)` — replacing the serialization strategy is out of scope
- Known field names must stay in sync with the `Item` struct definition; a test must verify the hard-coded list matches the actual struct fields
- Backwards compatibility applies to JSONL files with properly `x-`-prefixed extension keys. Files with non-`x-`-prefixed keys created outside the CLI are considered malformed and will fail to load after this change.

## Dependencies

- **Depends On:** None
- **Blocks:** None

## Risks

- [ ] The known-field-name list could drift from the `Item` struct if fields are added/removed. Mitigation: a unit test that serializes a default Item and compares the resulting JSON keys against the hard-coded list.

## Assumptions

- **Autonomous PRD**: Created without human interview. Decisions that would normally be discussed are documented here.
- **Post-deserialization validation for `x-` prefix**: A validation function called after deserialization is simpler than a custom serde `Deserialize` impl and sufficient for catching non-`x-` keys from external edits.
- **Hard error for all violations**: Both non-`x-` prefix and known-field collisions are hard errors (not warnings). The `x-` prefix is a documented contract; violating it indicates a data integrity issue that should be surfaced, not silently tolerated.
- **No existing bad data**: No production JSONL files currently have non-`x-` extension keys, since all writes go through the CLI's `--set` path which enforces the prefix.
- **Hard-coded list with test**: The known-field-name list will be a `const` array tested against the actual struct via a unit test. This is simpler than proc macros or reflection and catches drift at test time.
- **Validation lives on `Item`**: `Item::validate_extensions()` is the natural home — it validates the struct's own invariant and is accessible from all call sites.
- **Active store fails hard, archive skips with warning**: The active store (`read_active`) treats validation failures as errors (data must be correct). The archive reader (`read_archive`) skips invalid items with a warning (consistent with its existing lenient error handling).

## Open Questions

(None — all directional decisions resolved autonomously; see Assumptions.)

## References

- `src/model/item.rs:48` — `#[serde(flatten)]` on extensions field
- `src/model/extensions.rs:7-28` — `x-` prefix enforcement in `parse_dot_path`
- `src/cli/commands/add.rs:62-64` — Extension parsing in `add` command
- `src/cli/commands/edit.rs:102-103` — Extension parsing in `edit` command
- `src/store/jsonl.rs` — JSONL store with `read_active` and `read_archive` deserialization paths
