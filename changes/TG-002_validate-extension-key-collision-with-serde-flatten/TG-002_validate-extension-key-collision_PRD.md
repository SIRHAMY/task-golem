# Change: Validate extension key collision with serde(flatten)

**Status:** Proposed
**Created:** 2026-02-24
**Author:** Claude (autonomous)

## Problem Statement

The `Item` struct uses `#[serde(flatten)]` to merge extension fields (a `BTreeMap<String, serde_json::Value>`) into the top-level JSON representation. While the CLI's `--set` flag enforces an `x-` prefix on extension keys (via `parse_dot_path`), no validation exists at the model or deserialization level. This creates two risks:

1. **Silent data loss on round-trip**: If an extension key collides with a known field name (e.g., `"id"`, `"status"`), serialization produces duplicate JSON keys. On re-deserialization, serde assigns the value to the known struct field and silently discards the extension entry, losing user data.

2. **Schema pollution on deserialization**: When reading externally-crafted or manually-edited JSONL files, any unknown key that doesn't match a known field is captured into extensions — even if it doesn't have the required `x-` prefix. This can accumulate garbage keys that persist through subsequent saves.

The `x-` prefix convention is only enforced at the CLI boundary (`extensions::parse_dot_path`). A defense-in-depth approach requires validation at the model level to catch violations regardless of entry point.

## User Stories / Personas

- **AI Agent** — Sets extension fields via `--set` to store agent-specific metadata. Expects extension data to survive round-trips without silent corruption, even if another tool modifies the JSONL file.

- **Human Operator** — May hand-edit JSONL files or use external scripts to manipulate task data. Should get clear errors when introducing extension keys that would collide with known fields.

- **Tool Author** — Builds tooling on top of task-golem's JSONL format. Needs the schema contract enforced at the data layer, not just the CLI layer.

## Desired Outcome

When an `Item` is deserialized or constructed, any extension key that collides with a known `Item` field name is rejected with a clear error. Extension keys that lack the `x-` prefix are either rejected or stripped (with a warning), depending on the context. This prevents silent data loss during round-trips and keeps the extensions map clean.

## Success Criteria

### Must Have

- [ ] Deserialization of an `Item` rejects (returns error) if the extensions map contains any key matching a known `Item` field name (`id`, `title`, `status`, `priority`, `description`, `tags`, `dependencies`, `created_at`, `updated_at`, `blocked_reason`, `blocked_from_status`, `claimed_by`, `claimed_at`)
- [ ] The rejection produces a clear, actionable error message naming the offending key
- [ ] Existing valid JSONL files (with only `x-`-prefixed extension keys or no extensions) continue to deserialize without error
- [ ] Unit tests cover collision detection for at least 3 known field names

### Should Have

- [ ] Deserialization warns or rejects extension keys that lack the `x-` prefix (non-colliding but schema-violating)
- [ ] The known-field-name list is derived from code rather than a hand-maintained constant, or is tested against the actual struct fields to prevent drift

### Nice to Have

- [ ] `tg doctor` reports items with non-`x-`-prefixed extension keys as warnings

## Scope

### In Scope

- Post-deserialization validation of extension keys against known `Item` field names
- Error reporting for colliding keys
- Unit tests for validation logic

### Out of Scope

- Custom serde `Deserialize` implementation (a post-deserialization check is simpler and sufficient)
- Changing the `x-` prefix convention itself
- Migrating existing data that might have non-`x-` extension keys (no known instances exist)
- Validating extension *values* (only keys are validated)
- Changes to the `--set` CLI path (already correctly enforces `x-` prefix)

## Non-Functional Requirements

- **Performance:** Validation adds negligible overhead — a single pass over extension keys per deserialized item

## Constraints

- Must not break existing valid JSONL files
- Must use `serde(flatten)` — replacing the serialization strategy is out of scope
- Known field names must stay in sync with the `Item` struct definition; any approach should minimize the risk of the list drifting from the actual struct

## Dependencies

- **Depends On:** None
- **Blocks:** None

## Risks

- [ ] The known-field-name list could drift from the `Item` struct if fields are added/removed without updating the validation. Mitigation: derive the list from code or add a compile-time/test-time check.

## Assumptions

- **Light mode chosen**: This is a small, well-understood defensive hardening item. The triage already analyzed the problem thoroughly, so deep discovery was unnecessary.
- **Post-deserialization validation preferred**: A custom serde `Deserialize` impl would be more complex with minimal benefit. A validation function called after deserialization is simpler and easier to maintain.
- **Reject rather than strip**: Colliding keys indicate a data integrity issue that should be surfaced as an error, not silently fixed, so the caller can investigate.
- **No existing bad data**: Assumed no production JSONL files currently have non-`x-` extension keys, since all writes go through the CLI's `--set` path which enforces the prefix.

## Open Questions

- [ ] Should non-`x-`-prefixed extension keys be hard errors or warnings? (Proposed: hard error for known-field collisions, warning for missing `x-` prefix)
- [ ] Should the validation function live on `Item` (e.g., `Item::validate()`) or be a standalone function in the `extensions` module?

## References

- `src/model/item.rs:48` — `#[serde(flatten)]` on extensions field
- `src/model/extensions.rs:14` — `x-` prefix enforcement in `parse_dot_path`
- `src/cli/commands/add.rs:64` — Extension parsing in `add` command
- `src/cli/commands/edit.rs:103` — Extension parsing in `edit` command
