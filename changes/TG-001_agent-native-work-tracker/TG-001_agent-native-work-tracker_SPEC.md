# SPEC: Agent-Native Work Tracker

**ID:** TG-001
**Status:** Draft
**Created:** 2026-02-24
**PRD:** ./TG-001_agent-native-work-tracker_PRD.md
**Execution Mode:** autonomous
**New Agent Per Phase:** yes
**Max Review Attempts:** 3

## Context

Task Golem is a greenfield Rust CLI project — there is no existing code to modify. The tool (`tg`) provides project-scoped task management with hash-based IDs, a 4-state machine, dependency graphs, and JSONL backing store, designed for AI agent interoperability. The PRD identifies a gap: no lightweight, zero-config, agent-native work tracker exists. The design specifies a three-layer architecture (CLI → Domain → Persistence) with atomic writes, flock-based concurrency, and `x-*` extension metadata for orchestrator interop.

### PRD Deviations

The following intentional deviations from the PRD are documented here for traceability:

1. **Stale lock detection (flock replaces PID+timestamp).** The PRD specifies "Stale lock detection: lock file contains PID + timestamp, auto-clear if PID is dead after 30-second timeout" as a Must-Have. This SPEC uses flock-based advisory locking (via `fd-lock`), which provides kernel-enforced auto-release on process death — strictly superior to userspace PID polling. The underlying intent (stale locks are eventually cleared) is fully met. PID/timestamp may be written to the lock file as optional diagnostic metadata for `tg doctor`, but is not relied upon for correctness. See Design doc "Decision: Separate Lock File with Backoff" for full rationale.

2. **ID generation (pure random, no timestamp component).** The PRD specifies "random bytes + timestamp" as hash input. Since the input is already random, the timestamp adds no entropy. The SPEC generates random hex directly (3 random bytes → hex encode → truncate to 5 chars). The Design and Tech Research both recommend this simplification. No hash function (blake3/sha2) is needed.

3. **`--clear-deps` flag on `tg rm`.** The PRD specifies `tg rm <ID>` with `--force` but does not mention `--clear-deps`. The Design adds `--clear-deps` as a necessary implementation detail to avoid silent corruption (dangling deps that permanently block items from appearing in `tg ready`). Without it, `--force` removes the item but leaves deps that can never be resolved, silently hiding dependents from the ready queue. This is a safety-oriented addition, not scope drift.

### Resolved Design Deferrals

1. **Nested `Value::Object` key ordering.** The Design defers to the SPEC whether to enable `serde_json`'s `preserve_order` feature for nested extension values. **Resolution: Do NOT enable `preserve_order`.** With the default `serde_json` configuration, `Value::Object` uses `BTreeMap` internally, which produces alphabetical key ordering at all nesting levels. This ensures byte-identical round-trips regardless of insertion order. Combined with the top-level `BTreeMap<String, Value>` for extensions, all extension data is deterministically ordered. This is the correct choice for git-diff stability.

## Approach

The implementation follows the three-layer architecture from the design document:

1. **Persistence Layer** — JSONL read/write with schema versioning, atomic file writes (tempfile → fsync → rename), flock-based file locking with exponential backoff, project root resolution via parent directory walking, and archive management (append-only for done items).

2. **Domain Layer** — `Item` struct with `#[serde(flatten)]` for `x-*` extension fields using `BTreeMap<String, serde_json::Value>` for deterministic ordering. `Status` enum with `can_transition_to()` enforcing the 4-state machine. Random hex ID generation with collision detection. DFS-based cycle detection for dependencies. Dot-path extension mutation with JSON-first value parsing.

3. **CLI Layer** — clap derive-based argument parsing, command dispatch, and output formatting (JSON to stdout, diagnostics to stderr, colored tables for human mode).

Every write follows the lock-load-mutate-atomic-save-unlock cycle. Every read skips locking, relying on atomic rename for consistency. The `tg done` command uses archive-first write ordering (append to archive, then rewrite active store) so failure modes are always "benign duplicate" rather than "data loss."

**Patterns to follow:**

- This is a greenfield project — no existing patterns to reference. Standard Rust project conventions apply:
  - `src/main.rs` as entry point, `src/{module}/mod.rs` for module roots
  - `#[cfg(test)] mod tests` for inline unit tests
  - `tests/` directory for integration tests
  - clap derive macros for CLI argument definitions
  - `thiserror` for typed error enums, `anyhow` for internal propagation

**Implementation boundaries:**

- Do not implement: daemon mode, MCP server, TUI dashboard, cross-project sync, import from external trackers
- Do not add: tokio async runtime, SQLite/database dependencies, network calls
- Do not over-abstract: no Store trait (design explicitly defers this), no generic persistence interface

## Phase Summary

| Phase | Name | Complexity | Description |
|-------|------|------------|-------------|
| 1 | Foundation | High | Project scaffold, error types, Item model with serde PoC, persistence layer (JSONL, atomic writes, file locking), project root resolver, ID generator, `tg init` command |
| 2 | Core CRUD | High | `tg add`, `tg list`, `tg show`, `tg edit`, `tg rm` with dot-path extensions, cycle detection, ID resolution |
| 3 | State Machine & Ready Queue | High | `tg do`/`done`/`block`/`unblock`/`todo` transitions, claim semantics, archival, `tg ready` with dep resolution |
| 4 | Polish & Should-Have | Med-High | `tg next`, `tg dep add/rm`, colored table output, `--verbose`, `tg doctor`, configurable ID prefix |
| 5 | Hardening & Nice-to-Have | Med | Tab completion, `tg archive`, idempotent transitions, performance benchmarks, binary size verification, concurrency stress tests |

**Ordering rationale:** Each phase builds on the previous. Phase 1 establishes the data model and persistence that all commands depend on. Phase 2 adds CRUD operations that Phase 3's state transitions operate on. Phase 4 adds Should-Have features that enhance but don't change core behavior. Phase 5 hardens and polishes. Phases 1-3 cover all Must-Have PRD requirements. The tool is fully functional after Phase 3.

---

## Phases

### Phase 1: Foundation

> Project scaffold, core types, persistence layer, and `tg init`

**Phase Status:** complete

**Complexity:** High

**Goal:** Establish the Rust project, prove the `Item` serde behavior via a focused PoC, define the full data model, implement the persistence layer (JSONL read/write, atomic writes, file locking with backoff), and deliver `tg init` as the first working command.

**Files:**

- `Cargo.toml` — create — project manifest with dependencies and release profile
- `.gitignore` — create — ignore `/target`, editor files
- `src/main.rs` — create — binary entry point, delegates to CLI
- `src/errors.rs` — create — typed error enum via thiserror with exit code mapping
- `src/model/mod.rs` — create — module root re-exporting Item, Status, id, deps, extensions
- `src/model/status.rs` — create — Status enum with can_transition_to() and serde
- `src/model/item.rs` — create — Item struct with all fields, serde derives, #[serde(flatten)] for extensions
- `src/model/id.rs` — create — ID generation (random hex) and resolution (exact/prefix-prepend/prefix-match)
- `src/store/mod.rs` — create — Store struct coordinating load/save/lock/archive operations
- `src/store/jsonl.rs` — create — JSONL parsing (schema header + items) and atomic writing
- `src/store/lock.rs` — create — flock wrapper with exponential backoff (10ms-500ms, 5s timeout, jitter)
- `src/store/root.rs` — create — project root resolver (parent directory walking)
- `src/cli/mod.rs` — create — CLI module root with run() function
- `src/cli/args.rs` — create — clap derive structs for Init subcommand and global flags (--json, --verbose)
- `src/cli/commands/mod.rs` — create — command handler module root
- `src/cli/commands/init.rs` — create — tg init handler (create directory, files, lock file)
- `src/cli/output.rs` — create — output formatting (JSON to stdout, basic human-readable)
- `tests/common/mod.rs` — create — shared test helpers (temp project creation, command runner)
- `tests/init_test.rs` — create — integration tests for tg init

**Tasks:**

*Project Setup:*

- [x] Initialize Cargo project: `cargo init --name task-golem` with binary target named `tg` (via `[[bin]]` in Cargo.toml)
- [x] Add core dependencies to Cargo.toml: clap (derive), serde (derive), serde_json (without `preserve_order` — use default BTreeMap-backed Value::Object for deterministic nested ordering), fd-lock, rand, hex, chrono (serde), tempfile, thiserror, anyhow, humantime. Dev-dependencies: assert_cmd, predicates. Note: owo-colors and tabled deferred to Phase 4 (not needed until colored output)
- [x] Configure release profile: opt-level "z", lto true, codegen-units 1, strip true, panic "abort"
- [x] Create `.gitignore` with `/target` and editor artifacts
- [x] Commit `Cargo.lock` (standard practice for binary crates — pins dependency versions)

*Serde PoC (GATE — must pass before proceeding to full Item implementation):*

- [x] Write a standalone `#[test]` in `src/model/item.rs` with a minimal struct that has `#[serde(flatten)] extensions: BTreeMap<String, serde_json::Value>` alongside `Option<T>` fields. Verify: (1) known fields serialize in declaration order, (2) extension fields serialize in alphabetical order after known fields, (3) `Option<T>` with value `None` serializes as `null` not omitted (requires `#[serde(serialize_with)]` or explicit `Option` handling), (4) nested `Value::Object` keys are alphabetically ordered (BTreeMap default), (5) round-trip (serialize then deserialize) produces byte-identical JSON output. This PoC gates all subsequent Item-dependent work. If `None` fields are omitted by default, implement a custom serializer or use `#[serde(default)]` on deserialize with explicit null serialization

*Error Types:*

- [x] Implement `TgError` enum in `src/errors.rs` with variants: `ItemNotFound`, `InvalidTransition`, `AmbiguousId`, `CycleDetected`, `AlreadyClaimed`, `InvalidInput`, `NotInitialized`, `DependentExists`, `StorageCorruption`, `LockTimeout`, `IoError`, `IdCollisionExhausted`, `SchemaVersionUnsupported`. Map each to exit code 1 (user) or 2 (system). Include JSON error serialization for `--json` mode

*Domain Model:*

- [x] Implement `Status` enum in `src/model/status.rs`: Todo, Doing, Done, Blocked. Add `can_transition_to()` with the full transition table (todo→doing, todo→done, todo→blocked, doing→done, doing→blocked, doing→todo; blocked→restored is handled separately via unblock; done→anything is invalid; blocked→blocked is invalid). Serde as lowercase strings. Display trait
- [x] Write unit tests for Status: test all valid transitions return true, all invalid transitions return false (done→anything is false, blocked→blocked is false, blocked→doing is false, blocked→done is false), serde serializes as lowercase strings
- [x] Implement `Item` struct in `src/model/item.rs` with all fields per the design's JSON schema. Use `BTreeMap<String, serde_json::Value>` with `#[serde(flatten)]` for extensions. Null fields must be serialized (not skipped). Add validation methods (validate_title rejects newlines — check for `\n`, `\r\n`, `\r`)
- [x] Write unit tests for Item: serde round-trip produces byte-identical output, extension fields preserved through flatten, null fields included in JSON (not omitted), title validation rejects embedded newlines, chrono DateTime serialization as ISO 8601 UTC (`"2026-02-24T12:00:00Z"` format), nested extension `Value::Object` keys sorted alphabetically
- [x] Implement ID generator in `src/model/id.rs`: generate 3 random bytes, hex-encode to 6 chars, truncate to 5, prefix with `tg-`. Accept `HashSet<String>` of existing IDs, retry up to 10 times on collision, return `TgError::IdCollisionExhausted` after 10 failures
- [x] Implement ID resolver in `src/model/id.rs`: three-step resolution (exact match → prepend `tg-` → prefix match). Accept a scope parameter to control which stores to search (active-only for write commands, active+archive for read commands). Return `TgError::AmbiguousId` with matching IDs list on ambiguous prefix
- [x] Write unit tests for ID generation: format is `tg-{5 hex chars}`, collision retry works, 10 consecutive collisions returns error. Tests for resolution: exact match, bare hex, prefix match, ambiguous prefix, no match

*Persistence Layer:*

- [x] Implement project root resolver in `src/store/root.rs`: walk parent directories from CWD looking for `.task-golem/`. Return path or `TgError::NotInitialized` with CWD path in error message for debuggability
- [x] Write unit tests for root resolver: finds `.task-golem/` in CWD, in parent, in grandparent; returns error when not found
- [x] Implement file lock in `src/store/lock.rs`: open `.task-golem/tasks.lock`, non-blocking flock attempt, exponential backoff (10ms initial, doubling, 500ms cap, 5s total timeout, 0-50% random jitter). Return lock guard that releases on drop (RAII). Return `TgError::LockTimeout` on timeout
- [x] Write unit tests for lock: (a) acquisition succeeds on uncontended lock, (b) RAII drop releases lock, (c) backoff calculation function produces correct delays (10ms, 20ms, 40ms, ..., 500ms cap), (d) total backoff stays under 5s before returning LockTimeout
- [x] Write a cross-process lock PoC test: spawn a child process that holds the lock, verify the parent's attempt returns LockTimeout. This validates fd-lock's cross-process mutual exclusion before Phase 3 builds concurrency on top of it
- [x] Implement JSONL reader in `src/store/jsonl.rs`: parse first line as `{"schema_version": N}`, validate version is exactly 1 (exit 2 with `TgError::SchemaVersionUnsupported` if different — v1 only implements version 1 with no migration transform; the version check infrastructure is in place for future use). Parse subsequent lines as Item. Active store: fail-fast on malformed lines (`TgError::StorageCorruption` with line number). Archive: skip-and-warn on malformed lines (warning to stderr), including truncated last lines (detect incomplete JSON on the last line, log warning, skip — this handles crash-mid-append recovery)
- [x] Implement JSONL writer in `src/store/jsonl.rs`: write schema header, then items sorted by ID, one per line. Atomic write via a single encapsulated function that enforces fsync-before-rename by construction: NamedTempFile in same directory → write all data → sync_all() → persist(). Never expose a code path where `persist()` can be called without a preceding `sync_all()`
- [x] Write unit tests for JSONL: round-trip (write then read produces identical items), schema version validation (reject version 0, reject version 2), malformed line handling (active store fails with line number, archive skips and warns), items sorted by ID in output, truncated last line in archive skipped gracefully
- [x] Implement Store in `src/store/mod.rs` — broken into sub-operations:
  - [x] `with_lock(callback)` — acquire flock, execute callback, release on drop. Provides the lock-load-mutate-save-unlock cycle
  - [x] `load_active() -> Vec<Item>` — read and parse tasks.jsonl (no lock needed for reads)
  - [x] `save_active(items: &[Item])` — atomic write of sorted items to tasks.jsonl (must be called under lock)
  - [x] `load_archive_ids() -> HashSet<String>` — scan archive.jsonl line-by-line extracting only IDs (fast path for collision checks and dep resolution)
  - [x] `load_archive_item(id: &str) -> Option<Item>` — scan archive for a specific item (for `tg show` fallback)
  - [x] `load_all_archive() -> Vec<Item>` — full archive deserialization (for `tg list --status done`)
  - [x] `all_known_ids() -> HashSet<String>` — union of active IDs + archive IDs
  - [x] `append_to_archive(item: &Item)` — append single item to archive.jsonl + fsync (for `tg done`)

*CLI Layer:*

- [x] Implement clap args in `src/cli/args.rs`: top-level `Cli` struct with global `--json` and `--verbose` flags, `Commands` enum with `Init` variant (with `--force` flag)
- [x] Implement output formatter in `src/cli/output.rs`: JSON mode serializes to stdout via serde_json (route all output through a single function that checks `--json` flag — never use `println!` directly in command handlers). Human mode prints basic text. Error JSON `{"error": "...", "exit_code": N}` to stderr when `--json` set
- [x] Implement `tg init` handler in `src/cli/commands/init.rs`: create `.task-golem/` dir, write empty `tasks.jsonl` and `archive.jsonl` with `{"schema_version":1}` header, create empty `tasks.lock`. Check for existing dir (exit 1 unless `--force`). When `--force` on existing project: warn about data loss on stderr before overwriting. Output JSON `{"initialized": true, "path": ".task-golem/"}` or human message
- [x] Wire up CLI dispatch in `src/cli/mod.rs` and `src/main.rs`: parse args, dispatch to init handler, format output, set process exit code

*Test Infrastructure:*

- [x] Create test helpers in `tests/common/mod.rs`: `TestProject` struct that creates a temp dir (auto-cleaned on drop, including on test failure for CI disk hygiene), runs `tg init`, provides methods to run `tg` commands and parse JSON output. `TestProject::new()` should return `Result` not panic. Concurrent tests using `TestProject` get isolated temp dirs (no path collisions)
- [x] Write integration tests for `tg init`: creates directory and files, schema headers correct, `--force` reinitializes with data-loss warning on stderr (check via `predicates::str::contains`), error on existing without `--force` (exit code 1), `--json` output matches expected schema

**Verification:**

- [x] `cargo build --release` succeeds with no warnings
- [x] `cargo test` passes all unit and integration tests
- [x] `cargo clippy -- -D warnings` passes (run incrementally during development, not only at phase end)
- [x] Serde PoC passes: byte-identical round-trip with null fields and nested extension ordering
- [x] `tg init` creates a valid `.task-golem/` directory with `tasks.jsonl`, `archive.jsonl`, and `tasks.lock`
- [x] `tg init --json` produces `{"initialized": true, "path": ".task-golem/"}`
- [x] `tg init` on existing project exits with code 1; `tg init --force` succeeds with warning on stderr
- [x] Cross-process lock PoC passes: child holding lock prevents parent from acquiring

**Commit:** `[TG-001][P1] Feature: Project foundation — data model, persistence layer, tg init`

**Notes:**

The serde PoC is the single most important task in Phase 1. If `#[serde(flatten)]` with `BTreeMap` + null `Option` fields does not produce the expected deterministic JSON, the entire JSONL storage model must be reconsidered (e.g., custom Serialize impl). Do not proceed past the PoC until it passes.

The schema version check in v1 only validates version == 1 and rejects anything else. No auto-migration transform is implemented — the scaffolding (version check, reject-if-newer) is in place for future use. Actual migration logic will be added when schema version 2 is defined.

**Followups:**

- `#[serde(flatten)]` field-name collision: extension keys that collide with known Item field names (e.g., "id", "title") could cause unpredictable serde behavior. Phase 2's `x-` prefix validation in `src/model/extensions.rs` will enforce the convention, but no runtime validation exists at deserialization time. Consider adding a post-deserialization check in a future phase.
- `append_to_archive` now defensively writes the schema header if the file is missing, but the primary guarantee is that `tg init` creates the file. If further resilience is needed, `tg doctor` (Phase 4) can detect and repair headerless archives.

---

### Phase 2: Core CRUD

> add, list, show, edit, rm commands with dot-path extensions and cycle detection

**Phase Status:** complete

**Complexity:** High

**Goal:** Implement the five CRUD commands so that items can be created, queried, modified, and deleted. After this phase, `tg` is a functioning (if stateless) work tracker.

**Files:**

- `src/cli/args.rs` — modify — add Add, List, Show, Edit, Rm subcommand variants with all flags
- `src/cli/commands/add.rs` — create — tg add handler
- `src/cli/commands/list.rs` — create — tg list handler
- `src/cli/commands/show.rs` — create — tg show handler
- `src/cli/commands/edit.rs` — create — tg edit handler
- `src/cli/commands/rm.rs` — create — tg rm handler
- `src/cli/commands/mod.rs` — modify — re-export new command modules
- `src/cli/mod.rs` — modify — dispatch to new commands
- `src/model/deps.rs` — create — cycle detection (DFS), dependency validation, and `detect_all_cycles()` for full-graph validation
- `src/model/extensions.rs` — create — dot-path parsing, nested object creation, deletion, value parsing
- `src/model/mod.rs` — modify — re-export deps and extensions modules
- `src/model/id.rs` — modify — add archive-scope parameter to resolver for `tg show` vs write commands
- `src/store/mod.rs` — modify — ensure `load_all_archive()` is wired to `tg list --status done` path
- `tests/add_test.rs` — create — integration tests for tg add
- `tests/list_test.rs` — create — integration tests for tg list
- `tests/show_test.rs` — create — integration tests for tg show
- `tests/edit_test.rs` — create — integration tests for tg edit
- `tests/rm_test.rs` — create — integration tests for tg rm

**Tasks:**

*Dot-Path Extension Mutation (implement and test in isolation before wiring into commands):*

- [x] Implement dot-path parsing in `src/model/extensions.rs`: parse `x-foo.bar.baz` into path segments, validate first segment starts with `x-` (reject non-x- keys with exit 1)
- [x] Implement value parsing: empty string = delete, else try JSON parse (numbers, booleans, objects, arrays, null), else treat as string literal. Note: `--set x-foo=true` produces boolean `true`, `--set x-foo=42` produces number `42`, `--set x-foo=hello` produces string `"hello"`
- [x] Implement set/overwrite/create-intermediate logic: create intermediate objects for nested paths. Overwrite non-object values when creating nested paths (e.g., `x-foo` is string `"hello"`, then `--set x-foo.bar=1` overwrites with `{"bar": 1}`). Apply multiple `--set` flags left-to-right sequentially
- [x] Implement delete with recursive parent cleanup: `--set x-foo.bar=` (empty value) deletes key `bar`. If parent object `x-foo` is now empty, delete it too. Recurse up the chain
- [x] Write unit tests for extensions: nested object creation (`x-foo.bar=1` → `{"x-foo": {"bar": 1}}`), JSON value parsing (numbers, booleans, objects, arrays), string fallback, deletion via empty value, recursive parent cleanup, `x-` prefix validation (reject non-x- keys), overwrite conflict (existing string overwritten by nested set), multiple sets interaction (`--set x-a=1 --set x-a.b=2` → second overwrites first)

*Dependency Validation and Cycle Detection:*

- [x] Implement `would_create_cycle(items, source_id, new_dep_id) -> bool` in `src/model/deps.rs` via DFS from new_dep_id following dependency edges in active items only (archived items are terminal, cannot create cycles)
- [x] Implement `validate_dep(dep_id, active_items, archive_ids) -> Result<Vec<Warning>>` — checks existence in active or archive, warns on missing from both stores. Self-dep rejection (`source_id == dep_id`)
- [x] Implement `detect_all_cycles(items) -> Vec<Vec<String>>` — full-graph cycle detection via topological sort for use by `tg doctor` in Phase 4. More efficient than per-edge DFS for whole-store validation
- [x] Write unit tests for deps: self-dep rejected, direct cycle (A↔B), transitive cycle (A→B→C→A), diamond (non-cyclic), dep on archived item (no warning), dep on non-existent item (warning), multiple deps added together checked correctly

*Commands:*

- [x] Add clap args for `tg add`: positional title (required), `--description`, `--priority` (i64), `--dep` (repeated), `--tag` (repeated), `--set` (repeated key=value pairs)
- [x] Implement `tg add` handler: validate title (single-line), find root, acquire lock, load active store, load archive IDs for collision check, generate ID, validate deps (existence check with warnings, self-dep rejection, cycle detection for each dep), parse `--set` extensions via dot-path, create Item (status=todo, priority=default 0, timestamps=now), save, output new item
- [x] Write integration tests for add: basic add, add with all optional fields, JSON output schema validation (verify: id matches `^tg-[0-9a-f]{5}$`, status is `"todo"`, all null fields present not omitted, timestamps parse as ISO 8601), title newline rejection (exit 1), dep on non-existent ID produces warning on stderr but succeeds, multiple adds create distinct IDs
- [x] Add clap args for `tg list`: `--status` (optional filter), `--tag` (optional filter), `--json`
- [x] Implement `tg list` handler: find root, load active store (no lock). If `--status done`, load full archive via `load_all_archive()`. Apply status filter, tag filter. Default (no filters): all non-done active items (explicitly filter out status=done, matching PRD's "Default: all non-done items" — do not rely on the incidental behavior that done items are archived). Sort by priority desc then created_at asc. Output list
- [x] Write integration tests for list: default shows all active non-done items, filter by status, filter by tag, combined filters, sort order verification, `--status done` loads archive and returns done items, empty result is `[]` in JSON
- [x] Add clap args for `tg show`: positional ID (required), `--json`
- [x] Implement `tg show` handler: find root, load active store (no lock), resolve ID with active+archive scope (exact/prefix-prepend/prefix-match across active then archive). If not found in either, exit 1. Output full item
- [x] Write integration tests for show: full ID, bare hex (`a3f82`), prefix match, ambiguous prefix error with matching IDs listed, archive fallback (create item → manually write to archive.jsonl for test), not found exits 1, JSON schema validated for archived items (all fields present including null claim fields)
- [x] Add clap args for `tg edit`: positional ID, `--title`, `--priority`, `--description`, `--add-dep` (repeated), `--rm-dep` (repeated), `--add-tag` (repeated), `--rm-tag` (repeated), `--set` (repeated)
- [x] Implement `tg edit` handler: find root, acquire lock, load active store, resolve ID (active-only scope — cannot edit archived items). Apply field changes: title (validate single-line), priority, description. Apply dep changes: `--add-dep` validates existence, self-dep rejection, cycle detection; `--rm-dep` removes. Apply tag changes. Apply extension changes via dot-path. Update `updated_at`. Save. Output updated item
- [x] Write integration tests for edit: change each field type, add dep with cycle rejection, remove dep, add/remove tags, set/delete extension fields (including overwrite-conflict integration test: `tg add "T" --set x-foo=hello && tg edit <id> --set x-foo.bar=1` → verify `x-foo` is `{"bar": 1}`), updated_at changes, edit non-existent ID exits 1
- [x] Add clap args for `tg rm`: positional ID, `--force`, `--clear-deps`
- [x] Implement `tg rm` handler: find root, acquire lock, load active store, resolve ID. Check for dependents (items that list this ID in their dependencies). If dependents exist and no `--force`: exit 1 with message listing dependents and explaining both `--force` and `--force --clear-deps` options. With `--force`: remove item (leave dangling deps). With `--force --clear-deps`: remove item AND remove its ID from all dependents' dep lists. `--clear-deps` without `--force`: ignored (no-op if no dependents). Save. Output confirmation JSON `{"removed": "tg-xxxxx"}` (with `"cleared_deps_from": [...]` if `--clear-deps` was used)
- [x] Write integration tests for rm: basic remove, remove with dependents (error with helpful message), `--force` (dangling deps remain), `--force --clear-deps` (cascading cleanup — verify dependent's dep list no longer contains removed ID via `tg show`), `--clear-deps` without `--force` on item with no dependents (succeeds normally), remove non-existent exits 1, JSON output schema

**Verification:**

- [x] `cargo test` passes all unit and integration tests (including Phase 1 tests — no regressions)
- [x] `cargo clippy -- -D warnings` passes
- [x] Full lifecycle (as automated integration test): `tg init && tg add "Task A" && tg add "Task B" --dep <A-id> && tg list --json && tg show <B-id> --json && tg edit <A-id> --priority 5 && tg rm <A-id> --force --clear-deps && tg show <B-id> --json` (verify B's deps no longer contain A)
- [x] Extension fields: `tg add "Test" --set x-meta.key=42 && tg show <id> --json` shows `"x-meta": {"key": 42}`
- [x] Cycle detection: `tg add "A" && tg add "B" --dep <A-id> && tg edit <A-id> --add-dep <B-id>` exits 1 with cycle error
- [x] Per-command JSON schema validation: each command's `--json` output checked for required fields (id format, status enum, timestamps as ISO 8601, null fields present, extension fields preserved)

**Commit:** `[TG-001][P2] Feature: Core CRUD commands — add, list, show, edit, rm`

**Notes:**

The dot-path extension mutation is one of the highest-complexity areas. Implement and unit-test each sub-operation (parsing, set/overwrite, delete/cleanup) independently before wiring into add/edit commands.

For `tg list --status done`, full archive deserialization is needed (not just IDs). This uses `Store::load_all_archive()` defined in Phase 1.

**Followups:**

- `load_archive_ids()` fully deserializes all archive items to extract IDs. For large archives, consider a lightweight `IdOnly` struct or line-by-line ID extraction. Not a concern for v1 but noted for optimization if archive grows beyond ~10K items.
- `tg edit` with no mutation flags still bumps `updated_at` and writes to disk. Consider requiring at least one mutation flag or detecting no-op edits to skip the write.
- `description` field can only be set via `tg edit --description`, never cleared back to `null`. Consider supporting `--description ""` → `None` or adding `--clear-description`.

---

### Phase 3: State Machine & Ready Queue

> State transitions, claim semantics, archival, and ready-queue computation

**Phase Status:** complete

**Complexity:** High

**Goal:** Implement the 4-state machine with all transitions, claim semantics for multi-agent coordination, archival on `tg done`, and the ready-queue computation. After this phase, `tg` supports the full agent workflow.

**Files:**

- `src/cli/args.rs` — modify — add Do, Done, Block, Unblock, Todo, Ready subcommand variants
- `src/cli/commands/transition.rs` — create — handlers for do, done, todo, block, unblock (shared logic)
- `src/cli/commands/ready.rs` — create — handler for tg ready
- `src/cli/commands/mod.rs` — modify — re-export transition and ready modules
- `src/cli/mod.rs` — modify — dispatch to new commands
- `src/model/item.rs` — modify — add methods for transition side effects (set/clear claims, set/clear blocked fields)
- `src/model/deps.rs` — modify — add `compute_ready_queue()` function
- `tests/transition_test.rs` — create — integration tests for all state transitions
- `tests/ready_test.rs` — create — integration tests for ready queue
- `tests/concurrency_test.rs` — create — basic multi-process claim race tests (stress tests deferred to Phase 5)

**Tasks:**

*Transition Logic:*

- [x] Add transition methods to Item in `src/model/item.rs`: `apply_do(claim: Option<String>)` — set status to Doing, optionally set claimed_by/claimed_at, update updated_at. `apply_done()` — set status to Done, clear claimed_by/claimed_at, update updated_at. `apply_block(reason: Option<String>)` — store current status in blocked_from_status, set status to Blocked, store reason, clear claims if from Doing, update updated_at. `apply_unblock()` — restore status from blocked_from_status (default to Todo if missing/None — defensive fallback), clear blocked_reason/blocked_from_status, update updated_at. `apply_todo()` — set status to Todo, clear claimed_by/claimed_at, update updated_at
- [x] Add clap args for `tg do`: positional ID, `--claim <agent-id>` (optional string)
- [x] Add clap args for `tg done`, `tg todo`: each takes positional ID
- [x] Add clap args for `tg block`: positional ID, `--reason` (optional string)
- [x] Add clap args for `tg unblock`: positional ID
- [x] Implement shared transition logic in `src/cli/commands/transition.rs`: find root, acquire lock, load active store, resolve ID (active-only scope), validate transition via `Status::can_transition_to()` (exit 1 if invalid — this includes blocked→blocked, which is rejected to prevent blocked_from_status corruption). For `tg do --claim`: check if already claimed by different agent (exit 1 with `TgError::AlreadyClaimed`), same agent re-claim updates claimed_at only, no `--claim` transitions without setting claim fields. Apply transition method. Save. Output updated item

*Archival:*

- [x] Implement `tg done` archival: after applying the done transition, append the item (with status=done) to archive.jsonl via `Store::append_to_archive()` + fsync. If archive append fails: exit 2, item unchanged in active store. Then rewrite active store without the item. If active rewrite fails after archive append: exit 2 (item in both stores — benign duplicate, cross-phase dependency: detectable by `tg doctor` in Phase 4; no self-healing in Phase 3)
- [x] Ensure archive JSONL reader handles truncated last lines (implemented in Phase 1's `src/store/jsonl.rs`) — write a test that simulates crash mid-append: truncate archive.jsonl mid-line, then call `tg done` on another item and verify it succeeds (archive append adds a new valid line after the truncated one is skipped on read)

*Transition Tests:*

- [x] Write integration tests for transitions: every valid path (todo→doing, todo→done, todo→blocked, doing→done, doing→blocked, doing→todo), every invalid path (done→anything exits 1, blocked→doing exits 1, blocked→done exits 1, blocked→blocked exits 1), claim set on `tg do --claim`, claim cleared on done/block/todo, blocked_from_status stored and restored correctly, `done` archives item (verify item is in archive.jsonl directly, not just via `tg show`), terminal done (cannot transition further)
- [x] Write dedicated `tg todo` test: create item, `tg do --claim agent-1`, `tg todo` — verify status is todo, claimed_by is null, claimed_at is null, updated_at changed
- [x] Write dedicated `todo→done` test (skip doing): create item, immediately `tg done` — verify item archived, no claim fields were set, appears in archive with status=done
- [x] Write `apply_unblock()` fallback test: inject item with blocked status but `blocked_from_status: null` (manually edit JSONL), call `tg unblock` — verify status defaults to todo
- [x] Write integration tests for claim semantics: agent-A claims, agent-B claim fails with exit 1, agent-A re-claims (updates claimed_at), do without claim works (no claim fields set), claimed_by/claimed_at visible in `tg list --json`

*Ready Queue:*

- [x] Implement `compute_ready_queue()` in `src/model/deps.rs`: load active items and archived IDs. Build done set (active items with status=done + all archived IDs). Filter active items where: status==todo AND every dependency ID is in the done set. Deps on IDs absent from both active and archive are unmet (item not ready — emit warning to stderr listing the item ID and its unmet dep IDs so the issue is visible). Sort by priority desc, created_at asc
- [x] Add clap args for `tg ready`: `--json`, `--include-stale=<duration>` (humantime), `--limit N` (optional)
- [x] Implement `--include-stale` duration logic: parse humantime string (e.g., `4h`, `30m`) into a `Duration`, compute threshold as `Utc::now() - duration`, include Doing items whose `updated_at` < threshold. Use `<` (strictly older) — items exactly at threshold are not stale
- [x] Implement `tg ready` handler: find root, load active store and archive IDs (no lock), compute ready queue. If `--include-stale` provided: also include stale Doing items per above logic. Apply `--limit` if provided. Output list

*Ready Queue Tests:*

- [x] Write integration tests for ready: items with no deps are ready, items with met deps (dep is done) are ready, items with unmet deps (dep is todo/doing) are not ready, items with dep on archived item are ready, items with dep on non-existent ID are not ready (and warning emitted), sort order (priority desc then created_at asc), stale inclusion (`--include-stale` — manually set old updated_at in JSONL to test), empty queue returns `[]`, completing a dep makes downstream items ready, `--include-stale` boundary: item exactly at threshold is NOT stale

*Concurrency (basic functional tests — stress tests in Phase 5):*

- [x] Write basic concurrency tests: spawn two processes both running `tg do <same-id> --claim <different-agent>` — assert exactly one succeeds (exit 0) and one fails (exit 1). Spawn 5 processes each adding a different item — assert all 5 items present after, no corruption

**Verification:**

- [x] `cargo test` passes all tests (Phases 1-3, no regressions)
- [x] `cargo clippy -- -D warnings` passes
- [x] Full agent workflow (as integration test): create item → `tg do <id> --claim agent-1` → `tg done <id>` → `tg show <id> --json` — item is in archive with status=done, claim fields cleared
- [x] `tg todo` unclaim: `tg do <id> --claim agent-1` → `tg todo <id>` → verify status=todo, claimed_by=null
- [x] Block/unblock cycle: `tg do <id>` → `tg block <id> --reason "waiting"` → `tg unblock <id>` — restored to doing state
- [x] Ready queue with deps: add A (no deps), add B (depends on A) → `tg ready` returns only A → `tg done A` → `tg ready` returns B
- [x] Claim conflict: `tg do <id> --claim agent-1` succeeds, `tg do <id> --claim agent-2` exits 1
- [x] Basic concurrent claim race: exactly one winner, no corruption
- [x] Per-command JSON schema validation for all new commands

**Commit:** `[TG-001][P3] Feature: State machine, claim semantics, archival, ready queue`

**Notes:**

The `tg done` two-phase write (archive-first) is the highest-risk operation. The failure mode where the item ends up in both stores is benign but not self-healing in Phase 3 — it requires `tg doctor` (Phase 4) to detect and resolve. This is an accepted cross-phase dependency.

Full concurrency stress tests (10 threads × 100 operations) are deferred to Phase 5 (Hardening) to avoid timing-sensitive flaky tests blocking this phase. Phase 3 includes basic functional concurrency tests (claim race, concurrent adds) that validate correctness.

The `compute_ready_queue` function returns warnings as data (not eprintln!) so the CLI layer controls output — follows the same pattern as `deps::validate_dep()`. The `append_to_archive` function was enhanced to handle crash recovery: it seeks to the last byte to check for a trailing newline, prepending one if needed so appended items always start on their own line.

Same-agent re-claim (`tg do <id> --claim agent-1` when already doing+claimed by agent-1) succeeds and updates `claimed_at`. This is a special case handled before the state machine validation since `doing→doing` is not a valid transition in the general case.

**Followups:**

- [ ] `tg do <id> --claim agent-1` on an already-Doing unclaimed item returns `InvalidTransition` rather than setting the claim. Consider whether retroactive claiming should be supported as a separate feature.
- [ ] Stale doing items in `tg ready --include-stale` are appended after ready (todo) items rather than merged and globally sorted. Consider interleaving for a more intuitive priority-ordered output.

---

### Phase 4: Polish & Should-Have

> Sugar commands, colored output, diagnostics, and integrity validation

**Phase Status:** not_started

**Complexity:** Med-High

**Goal:** Add Should-Have features from the PRD: `tg next`, `tg dep` subcommands, colored human-readable output, `--verbose` diagnostics, `tg doctor` for integrity validation, and configurable ID prefix. `tg doctor` is the most substantial item and should be prioritized first.

**Files:**

- `Cargo.toml` — modify — add owo-colors (supports-colors), tabled
- `src/cli/args.rs` — modify — add Next, Dep (with Add/Rm subcommands), Doctor subcommand variants
- `src/cli/commands/next.rs` — create — tg next handler (delegates to ready with limit 1)
- `src/cli/commands/dep.rs` — create — tg dep add/rm handlers (delegate to edit)
- `src/cli/commands/doctor.rs` — create — tg doctor handler
- `src/cli/commands/mod.rs` — modify — re-export new modules
- `src/cli/mod.rs` — modify — dispatch to new commands, wire --verbose
- `src/cli/output.rs` — modify — add colored table output via owo-colors + tabled, NO_COLOR/FORCE_COLOR support
- `src/store/config.rs` — create — config.yaml parsing for ID prefix
- `src/store/mod.rs` — modify — integrate config loading
- `src/model/id.rs` — modify — accept configurable prefix from config
- `tests/next_test.rs` — create — integration tests for tg next
- `tests/dep_test.rs` — create — integration tests for tg dep add/rm
- `tests/doctor_test.rs` — create — integration tests for tg doctor

**Tasks:**

*Doctor (highest priority in this phase):*

- [ ] Implement `tg doctor` in `src/cli/commands/doctor.rs` with these specific checks:
  1. **JSONL syntax** — attempt to parse every line in both files; report line numbers of failures
  2. **Duplicate IDs** — check for duplicate IDs across active + archive
  3. **Items in both files** — detect items present in both tasks.jsonl and archive.jsonl (partial `tg done` failure recovery)
  4. **Invalid status** — validate all status strings are valid enum variants
  5. **Dependency cycles** — use `detect_all_cycles()` from Phase 2's deps module for full-graph validation
  6. **Dangling deps** — deps on IDs not present in active or archive
- [ ] Implement `tg doctor` JSON output: `{"issues": [...], "summary": {"total": N, "by_type": {...}}}` in JSON mode, human-readable report in default mode
- [ ] Implement `--fix` flag with specific repair actions:
  - Duplicate in both files: remove from active store (archive is authoritative for done items)
  - Invalid status string: report but do not auto-fix (requires human judgment)
  - Dependency cycles: report but do not auto-fix (breaking a cycle requires choosing which edge to remove)
  - Dangling deps: remove the dangling dep ID from the item's dependency list
  - Before any repair: create timestamped backup files (`.task-golem/tasks.jsonl.bak.{ISO8601}`, `.task-golem/archive.jsonl.bak.{ISO8601}`) using atomic write. Warn if backup files already exist
- [ ] Write integration tests for doctor: clean store reports zero issues, inject duplicate ID → detected, inject invalid status string → detected, inject item in both files → detected, inject cycle → detected, inject dangling dep → detected, `--fix` removes duplicates and dangling deps and creates timestamped backup files

*Sugar Commands:*

- [ ] Implement `tg next` handler: delegate to ready-queue computation with limit=1. Return single Item or null (JSON) / "No items ready" (human)
- [ ] Write integration tests for next: returns highest-priority ready item (identical to first element of `tg ready --json` array), returns null when queue empty, `--json` output is Item or null
- [ ] Implement `tg dep add <ID> <depends-on-ID>` and `tg dep rm <ID> <dep-ID>`: delegate to edit logic (add-dep / rm-dep). These are sugar commands
- [ ] Write integration tests for dep: `tg dep add` with cycle detection, non-existent ID warning, self-dep rejection — verify equivalence with `tg edit --add-dep`. `tg dep rm` removes dep — verify equivalence with `tg edit --rm-dep`

*Colored Output:*

- [ ] Add owo-colors and tabled dependencies to Cargo.toml
- [ ] Add colored table output in `src/cli/output.rs`. Table columns by command:
  - `tg list` / `tg ready`: ID, Status (colored), Priority, Title (truncated)
  - `tg show`: all fields, formatted as labeled rows
  - Status colors: todo=default/white, doing=yellow, done=green, blocked=red
  - Respect `NO_COLOR` and `FORCE_COLOR` environment variables via owo-colors `supports-colors` feature
- [ ] Write integration tests for color: `NO_COLOR=1 tg list` produces output with no ANSI escape sequences (regex check), `FORCE_COLOR=1` forces colors even when stdout is not a TTY

*Diagnostics:*

- [ ] Implement `--verbose` global flag: when set, output diagnostics to stderr: lock acquisition timing, file paths loaded, schema version found, item count loaded, archive size. Guard with `if verbose { eprintln!(...) }` — no logging framework
- [ ] Write integration test for `--verbose`: run `tg list --verbose`, verify diagnostics appear on stderr, verify stdout is unaffected (still valid JSON when `--json` also set)

*Configuration:*

- [ ] Implement config file parsing in `src/store/config.rs`: look for `.task-golem/config.yaml`, parse `id_prefix` field (default: `tg`). Keep config minimal — only the prefix for now. Document that `id_prefix` should be set at init time; warn in `tg doctor` if active items have mixed prefixes
- [ ] Update ID generator to use configurable prefix from config (fallback to `tg` if no config)
- [ ] Write integration test for config: create config.yaml with `id_prefix: proj`, `tg add "Test"` produces ID matching `^proj-[0-9a-f]{5}$`. Missing config produces `tg-xxxxx`

**Verification:**

- [ ] `cargo test` passes all tests (Phases 1-4, no regressions)
- [ ] `cargo clippy -- -D warnings` passes
- [ ] `tg next --json` returns the same item as first element of `tg ready --json` array (tested with items at different priorities)
- [ ] `NO_COLOR=1 tg list` produces uncolored output (no ANSI escapes)
- [ ] Custom prefix: create config.yaml with `id_prefix: proj`, `tg add "Test"` produces `proj-xxxxx` ID
- [ ] `tg doctor` on clean store: reports zero issues in JSON and human output
- [ ] `tg doctor --fix` on corrupted store: creates timestamped backup, repairs duplicates and dangling deps, reports results
- [ ] `--verbose` shows diagnostics on stderr without affecting stdout JSON
- [ ] Per-command JSON schema validation for all new commands

**Commit:** `[TG-001][P4] Feature: next, dep subcommands, colored output, doctor, verbose, config`

**Notes:**

`tg doctor` is the most substantial item in this phase — it touches every part of the data model and store. Build it first so it can validate the complete system, then add the lighter items (next, dep, colors, verbose, config).

**Followups:**

---

### Phase 5: Hardening & Nice-to-Have

> Tab completion, bulk archive, idempotent transitions, benchmarks, stress tests

**Phase Status:** not_started

**Complexity:** Med

**Goal:** Add selected Nice-to-Have features (PRD Nice-to-Have), verify performance targets from the PRD, and harden the tool with stress tests and cross-platform verification.

**Files:**

- `src/cli/args.rs` — modify — add Archive, Dump subcommand variants, optional clap_complete feature
- `src/cli/commands/archive.rs` — create — tg archive handler
- `src/cli/commands/dump.rs` — create — tg dump handler
- `src/cli/commands/transition.rs` — modify — add idempotent transition handling
- `src/cli/mod.rs` — modify — dispatch to new commands, add completions generation
- `Cargo.toml` — modify — add clap_complete as optional dependency, add criterion as dev-dependency
- `.github/workflows/ci.yml` — create — CI pipeline with Linux + macOS matrix
- `tests/archive_test.rs` — create — integration tests for tg archive
- `tests/stress_test.rs` — create — concurrent stress tests and large-store tests
- `benches/` — create — criterion benchmarks for ready-queue and store operations

**Tasks:**

*Nice-to-Have Commands (PRD Nice-to-Have):*

- [ ] Implement `tg archive [--before DATE]`: scan the active store for items with status=done that were NOT yet archived (edge case recovery), plus optionally prune the archive by removing items whose `updated_at` is before DATE (ISO 8601 format, parsed via chrono). This is primarily a maintenance/cleanup command. Writes pruned items to a separate `.task-golem/archive-pruned.jsonl` rather than deleting them
- [ ] Implement idempotent state transitions: add a pre-check before `Status::can_transition_to()` in the transition handler — if the item is already in the target state, return success with `{"idempotent": true, "previous_state": "<status>"}` instead of calling `can_transition_to()`. This avoids modifying the state machine contract from Phase 3. Add a test that `tg done` on an already-done item returns success (requires archive fallback check)
- [ ] Implement `tg dump --json` / `tg dump --yaml`: full export of all items (active + archive) in human-readable format. YAML output uses serde_yaml or manual formatting
- [ ] Add tab completion generation via clap_complete (optional feature): `tg completions bash/zsh/fish` outputs shell completion script. Integration test: verify output contains all subcommand names (`add`, `list`, `show`, etc.)

*Performance & Hardening:*

- [ ] Write performance benchmarks using criterion: `tg ready` on 500-item store must complete in <50ms, all commands in <200ms (cold start, release build). Generate test data via domain-layer bulk insertion (bypass CLI for speed), not 500 subprocess calls
- [ ] Verify binary size: stripped release build under 15MB (PRD target: 8-10MB with tokio, which is excluded; expect 4-7MB without tokio). Add a CI step that checks `stat` output against 15MB limit
- [ ] Verify memory usage: measure peak RSS via `/usr/bin/time -v tg ready --json` on a 500-item store. Must be under 10MB RSS per PRD NFR
- [ ] Write concurrent stress tests: 10 processes × 100 operations (mix of add, edit, do, done). Assertions: (a) total item count matches expected, (b) all IDs unique, (c) `tg doctor` reports zero issues, (d) no exit-code-2 errors
- [ ] Write large-store tests: generate 2000 active items + 5000 archived items via domain-layer bulk insertion into JSONL files. Verify operations stay within performance budget
- [ ] Test UTF-8 edge cases in `tests/add_test.rs` or `tests/encoding_test.rs`: titles with emoji, CJK characters, RTL text, zero-width joiners. Newline check must handle `\n`, `\r\n`, `\r` but accept other Unicode characters. Multi-byte single-line titles must be accepted
- [ ] Cross-platform CI: create `.github/workflows/ci.yml` with matrix (`ubuntu-latest`, `macos-latest`). Run `cargo test`, `cargo clippy`, binary size check on both platforms

**Verification:**

- [ ] `cargo test` passes all tests (all phases, no regressions)
- [ ] `cargo clippy -- -D warnings` passes
- [ ] `tg archive --before 2026-01-01` operates correctly
- [ ] `tg dump --json` produces valid JSON with all items from both stores
- [ ] `tg completions bash` outputs script containing all subcommand names
- [ ] Performance: `hyperfine 'tg ready --json'` on 500 items < 50ms
- [ ] Binary size: `stat target/release/tg` < 15MB (target 4-7MB)
- [ ] Memory: peak RSS < 10MB on 500-item store
- [ ] Stress test: no data loss under concurrent load, `tg doctor` clean after
- [ ] CI passes on both Linux and macOS

**Commit:** `[TG-001][P5] Feature: Archive, completions, idempotent transitions, benchmarks, hardening`

**Notes:**

This phase is optional — the tool is fully functional after Phase 4. Every item here can be independently skipped or deferred without impacting core value. Prioritize performance benchmarks and stress tests as they validate the design's assumptions about scale. `tg archive` introduces the first case of rewriting the archive file (previously append-only) — test carefully.

**Followups:**

---

## Final Verification

- [ ] All phases complete
- [ ] All PRD Must-Have success criteria met (see PRD deviations section for documented divergences)
- [ ] All PRD Should-Have criteria addressed (Phase 4)
- [ ] Tests pass (`cargo test`)
- [ ] Clippy clean (`cargo clippy -- -D warnings`)
- [ ] Binary size under 15MB
- [ ] Performance within budget (200ms all commands, 50ms ready queue at 500 items)
- [ ] Memory under 10MB RSS for 500-item store
- [ ] No regressions introduced
- [ ] CI green on Linux and macOS

## Execution Log

| Phase | Status | Commit | Notes |
|-------|--------|--------|-------|
| 1 | complete | `[TG-001][P1]` | 37 unit tests + 6 integration tests + 2 lock PoC tests. Serde PoC passed. All verification items green. |
| 2 | complete | `[TG-001][P2]` | 68 unit tests + 54 integration tests (122 total). All CRUD commands implemented with dot-path extensions, cycle detection, dependency validation. Code review passed — fixed status parsing, dep/tag deduplication. Clippy clean. |
| 3 | complete | `[TG-001][P3]` | 76 unit tests + 99 integration tests (175 total). All state transitions, claim semantics, archival, ready queue implemented. Code review passed — fixed apply_block fragility, unblock error message, moved eprintln from domain layer, added unit tests for apply_* methods, optimized archive append. Clippy clean. |

## Followups Summary

### Critical

### High

- [ ] Update project `.claude/CLAUDE.md` to say "JSONL backing store" instead of "YAML backing store" — current text conflicts with the PRD and SPEC

### Medium

- [ ] Archive ID index file (`.task-golem/archive-ids.idx`) — one ID per line, appended on `tg done` — would eliminate the archive scan bottleneck beyond ~50K archived items. Not needed for v1 but a well-defined future optimization (see Design doc Scaling Characteristics)
- [ ] `--project-dir` / `TG_PROJECT_DIR` environment variable to override root resolver — useful for CI/Docker/agent sandboxes where CWD may not be under the project root
- [ ] `tg merge-resolve` command for JSONL git merge conflict resolution (PRD Nice-to-Have)
- [ ] `tg watch --json` event stream (PRD Nice-to-Have, requires inotify or daemon)

### Low

- [ ] `tg do --force-claim <agent-id>` to override an existing claim without needing `tg todo` + `tg do` as two operations
- [ ] Library crate API for tighter orchestrator integration (PRD defers to v1.1)

## Design Details

### Key Types

```rust
// src/model/status.rs
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Status {
    Todo,
    Doing,
    Done,
    Blocked,
}

// src/model/item.rs
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Item {
    id: String,
    title: String,
    status: Status,
    priority: i64,
    description: Option<String>,
    tags: Vec<String>,
    dependencies: Vec<String>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    blocked_reason: Option<String>,
    blocked_from_status: Option<Status>,
    claimed_by: Option<String>,
    claimed_at: Option<DateTime<Utc>>,
    #[serde(flatten)]
    extensions: BTreeMap<String, serde_json::Value>,
}

// src/errors.rs
#[derive(Debug, thiserror::Error)]
enum TgError {
    // User errors (exit code 1)
    #[error("Item not found: {0}")]
    ItemNotFound(String),
    #[error("Invalid transition: {from} cannot transition to {to}")]
    InvalidTransition { from: Status, to: Status },
    #[error("Ambiguous ID prefix '{prefix}': matches {matches:?}")]
    AmbiguousId { prefix: String, matches: Vec<String> },
    #[error("Dependency cycle detected: {0}")]
    CycleDetected(String),
    #[error("Already claimed by {0}")]
    AlreadyClaimed(String),
    #[error("{0}")]
    InvalidInput(String),
    #[error("No task-golem project found. Run `tg init` to create one.")]
    NotInitialized,
    #[error("Item {0} is depended on by: {1}")]
    DependentExists(String, String),
    // System errors (exit code 2)
    #[error("Storage corruption: {0}")]
    StorageCorruption(String),
    #[error("Lock timeout after {0:?}")]
    LockTimeout(std::time::Duration),
    #[error(transparent)]
    IoError(#[from] std::io::Error),
    #[error("ID collision exhausted after {0} attempts")]
    IdCollisionExhausted(u32),
    #[error("Unsupported schema version {found} (max supported: {supported})")]
    SchemaVersionUnsupported { found: u32, supported: u32 },
}
```

### Architecture Details

```
┌─────────────────────────────────────────────────────┐
│                     CLI Layer                        │
│  clap arg parsing → command dispatch → output fmt    │
├─────────────────────────────────────────────────────┤
│                   Domain Layer                       │
│  Item model │ State machine │ ID gen │ Dep graph     │
├─────────────────────────────────────────────────────┤
│                Persistence Layer                     │
│  JSONL store │ Atomic writes │ File lock │ Archive   │
├─────────────────────────────────────────────────────┤
│                   Filesystem                         │
│  .task-golem/tasks.jsonl │ archive.jsonl │ tasks.lock│
└─────────────────────────────────────────────────────┘
```

Module structure:
```
src/
├── main.rs                    # Entry point
├── errors.rs                  # TgError enum
├── model/
│   ├── mod.rs                 # Re-exports
│   ├── status.rs              # Status enum + state machine
│   ├── item.rs                # Item struct + validation
│   ├── id.rs                  # ID generation + resolution
│   ├── deps.rs                # Cycle detection + ready queue
│   └── extensions.rs          # Dot-path mutation
├── store/
│   ├── mod.rs                 # Store struct + coordination
│   ├── jsonl.rs               # JSONL read/write + atomic save
│   ├── lock.rs                # flock + backoff
│   ├── root.rs                # Project root resolver
│   └── config.rs              # Config file parsing (Phase 4)
└── cli/
    ├── mod.rs                 # run() + dispatch
    ├── args.rs                # clap derive structs
    ├── output.rs              # JSON + human formatting
    └── commands/
        ├── mod.rs             # Re-exports
        ├── init.rs            # tg init
        ├── add.rs             # tg add
        ├── list.rs            # tg list
        ├── show.rs            # tg show
        ├── edit.rs            # tg edit
        ├── rm.rs              # tg rm
        ├── transition.rs      # tg do/done/block/unblock/todo
        ├── ready.rs           # tg ready
        ├── next.rs            # tg next (Phase 4)
        ├── dep.rs             # tg dep add/rm (Phase 4)
        ├── doctor.rs          # tg doctor (Phase 4)
        ├── archive.rs         # tg archive (Phase 5)
        └── dump.rs            # tg dump (Phase 5)
```

### Design Rationale

See the Design document for full rationale on all technical decisions. Key points relevant to implementation:

- **BTreeMap over HashMap** for extension fields: deterministic alphabetical ordering ensures byte-identical round-trips and clean git diffs. `serde_json` used without `preserve_order` so nested `Value::Object` also uses BTreeMap (alphabetical at all nesting levels)
- **flock over PID-based locks**: kernel-enforced auto-release on process death, no stale lock detection needed (see PRD Deviations section)
- **No async runtime**: every operation is synchronous, avoiding tokio's binary size and complexity overhead
- **Archive-first write ordering on done**: ensures failure modes are "benign duplicate" (item in both files) not "data loss" (item in neither)
- **Items sorted by ID in JSONL**: predictable positioning reduces git merge conflicts when two branches both add items
- **Pure random ID generation**: no timestamp component, no hash function — simplest approach that satisfies collision resistance requirements (see PRD Deviations section)

---

## Retrospective

[Fill in after completion]

### What worked well?

### What was harder than expected?

### What would we do differently next time?
