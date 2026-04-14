# SPEC: Query Layer and Nested Tasks

**ID:** TG-006
**Status:** Draft
**Created:** 2026-04-14
**PRD:** ./TG-006_query-layer-and-nested-tasks_PRD.md
**Design:** ./TG-006_query-layer-and-nested-tasks_DESIGN.md
**Tech Research:** ./TG-006_query-layer-and-nested-tasks_TECH_RESEARCH.md
**Execution Mode:** human-in-the-loop
**New Agent Per Phase:** yes
**Max Review Attempts:** 3

## TL;DR

- **Phases:** 6 phases (1 Low, 3 Med, 2 High).
- **Approach:** Land `parent` field first (data model → CLI), then build the SQLite cache as isolated infrastructure, then expose `tg query` on top, then add doctor checks and skill guidance. Each phase leaves the repo shippable.
- **Key risks:** (1) SQLite sandbox correctness — allowlist authorizer must deny every non-read action code; (2) Cache rebuild correctness under concurrent writes — rebuild must hold `store.with_lock()` during JSONL read.
- **Needs attention:** None remain. All directional items resolved: `tg archive` skip-during-sweep; `sqlite_master` reads allowed; Cargo version bumps `0.1.0 → 0.2.0` in Phase 6. Three minor spec-level defaults also locked in (tabular format, `--schema` output shape, verbose rebuild notice).

## Context

TG-006 lands two capabilities: a `parent: Option<String>` field enabling arbitrary task nesting, and a SELECT-only `tg query` command backed by a disposable SQLite cache at `.taskgolem/cache.db`. The JSONL remains authoritative; the cache is pure derived state, rebuilt lazily when its composite stamp `(mtime, size, xxh3_64)` disagrees with the JSONL.

The design is exhaustive (see linked doc). This SPEC translates that design into phases respecting dependency order: the `parent` field must exist before anything can query or validate it, and the cache must exist before `tg query` can read it. Existing Rust command implementations are intentionally not reimplemented on top of SQL — the cache is additive infrastructure.

Relevant existing code:
- `src/model/item.rs:41-70` — `Item` struct receiving the new `parent` field.
- `src/model/deps.rs:9-35, 67-166` — existing DFS + Kahn's cycle detection to mirror.
- `src/cli/commands/edit.rs` — canonical `with_lock`-based command shape.
- `src/store/jsonl.rs:129-159` — atomic temp+fsync+rename pattern to mirror for cache rebuild.
- `tests/common/mod.rs` — `TestProject` harness for integration tests.

## Approach

Implement in dependency order across six atomic phases. Phases 1–2 deliver nesting via pure-Rust paths with zero SQLite dependency. Phase 3 introduces the cache module as inert infrastructure (no user-visible command yet). Phase 4 exposes `tg query` on top. Phase 5 teaches `tg doctor` about the new surface. Phase 6 updates the task-golem skill with canonical queries and guidance.

A new `src/cache/` module (3 files: `mod.rs`, `rebuild.rs`, `query.rs`) owns everything SQLite; the rest of the codebase stays JSONL-pure. A new `src/model/parent.rs` is the single orchestration point for reparent mutations (used by `add`, `edit`, and future batch-edit) so the validate + cycle-check + mutate invariant can't be bypassed. `Store` gains two helpers: `tasks_jsonl_path()` (accessor for the cache module) and `ensure_gitignore()` (idempotent creator of `.taskgolem/.gitignore`).

**Patterns to follow:**

- `src/model/item.rs:30-39,48-66` — serde `Option` field pattern; add `parent` with `#[serde(serialize_with = "serialize_option_nullable")]` and append to `KNOWN_FIELD_NAMES`.
- `src/model/deps.rs:9-35` — DFS cycle-detection shape for `would_create_parent_cycle`.
- `src/model/deps.rs:67-166` — Kahn's-algorithm shape for `detect_all_parent_cycles`.
- `src/model/deps.rs:41-63` — `validate_dep` shape for `validate_parent`.
- `src/cli/commands/edit.rs:29-107` — command shape: resolve store → `with_lock` → load → mutate → validate → save → print.
- `src/cli/commands/list.rs` — filter-flag pattern for `--parent <id>`.
- `src/cli/commands/show.rs` — section-appending pattern for the "Children:" block.
- `src/cli/commands/doctor.rs::run` — `Issue` struct aggregation pattern; mirror `check_jsonl_syntax` and the later inlined checks (appending `Issue` structs to a shared `issues: Vec<Issue>` vector).
- `src/store/jsonl.rs:129-159` — temp + `sync_all` + `persist` atomic-rename for the cache `.db.tmp-<pid>` → `cache.db` path.
- `src/store/mod.rs:34-41` — `with_lock<F, R>(&self, F) -> Result<R, TgError>` callback contract; cache rebuild acquires this for the JSONL read phase only.
- `tests/common/mod.rs:10-66` — `TestProject` harness; all new integration tests use this.
- `src/errors.rs:7-72` — thiserror variants with `exit_code()` mapping; add new variants here.
- `src/cli/args.rs:71-106` — clap subcommand variant shape for `Query { sql, schema, json, timeout }`.

**Implementation boundaries:**

- **Do not reimplement** `list`, `ready`, `next`, `show` on top of SQL. They stay pure Rust in v1.
- **Do not modify** `src/store/jsonl.rs` write path. Cache writes live in `src/cache/rebuild.rs`, not in `store::jsonl`.
- **Do not add** archive tables to the cache in v1. `tasks.jsonl` only.
- **Do not expand** `tg query` beyond SELECT. Authorizer is default-deny.
- **Do not refactor** existing cycle-detection functions in `deps.rs`. Add parallel `parent_*` variants alongside them; share only the internal helpers that already exist.

## Phase Summary

| Phase | Name | Complexity | Description |
|-------|------|------------|-------------|
| 1 | Parent field — data model + validation | Med | Add `parent: Option<String>` to `Item`; cycle + validation functions; `reparent()` orchestrator; unit tests. |
| 2 | Parent field — CLI integration | Med | Wire `--parent` into `add`/`edit`; block deletion/archive with children; `list --parent` filter; `show` Children section. |
| 3 | Cache foundation — module + rebuild | High | Add `rusqlite`/`xxhash-rust` deps; `src/cache/{mod,rebuild}.rs`; stamp logic; atomic rebuild; `Store::ensure_gitignore` + init wiring. |
| 4 | Query sandbox + `tg query` CLI | High | `src/cache/query.rs` with allowlist authorizer + progress handler; `tg query` command with `--schema`/`--json`/`--timeout`; new error variants; sandbox tests. |
| 5 | Doctor extensions | Med | Parent cycles, dangling parent refs, cache consistency, gitignore checks added to `tg doctor`. |
| 6 | Skill update | Low | 5 canonical queries + transition-point guidance in the task-golem skill. |

**Ordering rationale:** P1 must precede P2 (CLI uses the validator). P1–P2 must precede P3 (cache schema includes `parent`). P3 must precede P4 (`tg query` opens the cache). P5 verifies the full surface, so it comes after P4. P6 ships the user-facing documentation of the new capability last.

---

## Phases

Each phase leaves the codebase in a functional, testable state. `just check` (fmt + clippy + tests) must pass at each phase boundary per the project's verification rule.

---

### Phase 1: Parent field — data model + validation

> Add `parent: Option<String>` to `Item`; add parent cycle + reference validation; expose single `reparent()` orchestrator.

**Phase Status:** completed

**Complexity:** Med

**Goal:** Every task record can carry an optional `parent` ID that survives JSONL round-trip, is validated for cycles and dangling references, and flows through a single orchestration point so future callers cannot bypass the invariants.

**Files:**

- `src/model/item.rs` — modify — add `parent: Option<String>` field with `#[serde(serialize_with = "serialize_option_nullable")]`; append `"parent"` to `KNOWN_FIELD_NAMES`.
- `src/model/deps.rs` — modify — add `would_create_parent_cycle(items, source_id, new_parent_id) -> bool`, `detect_all_parent_cycles(items) -> Vec<Vec<String>>`, `validate_parent(source_id, proposed_parent_id, active_items, archive_items) -> Result<(), TgError>`.
- `src/model/parent.rs` — create — `reparent(items: &mut [Item], id: &str, new_parent: Option<String>, archive: &[Item]) -> Result<(), TgError>` — the single orchestration point combining validate + cycle-check + mutation on an in-memory slice.
- `src/model/mod.rs` — modify — add `pub mod parent;`.
- `src/errors.rs` — modify — add `ParentSelfReference { id: String }`, `ParentCycle { ids: Vec<String> }`, `ParentDangling { id: String, parent: String }`, `ParentHasChildren { id: String, children: Vec<String> }` variants. Exit code 1. Update `exit_code()` mapping.
- `tests/parent_test.rs` — create — round-trip serde, reparent orchestration, cycle rejection (direct and transitive), self-parent rejection, dangling rejection, archived-target rejection.

**Patterns:**

- Mirror `src/model/deps.rs:9-35` (DFS) for `would_create_parent_cycle` — identical shape, following `.parent` edges instead of `.dependencies`.
- Mirror `src/model/deps.rs:65-166` (Kahn's) for `detect_all_parent_cycles`.
- Mirror `src/model/deps.rs:41-63` for `validate_parent`.
- Mirror the existing `Option<String>` serde declarations in `src/model/item.rs:48-66`.

**Tasks:**

- [x] Add `parent: Option<String>` field to `Item` with `serialize_option_nullable`.
- [x] Append `"parent"` to `KNOWN_FIELD_NAMES` in collision-safe alphabetical position.
- [x] Implement `would_create_parent_cycle` in `deps.rs` (DFS).
- [x] Implement `detect_all_parent_cycles` in `deps.rs` (Kahn's).
- [x] Implement `validate_parent` in `deps.rs` (self-parent, dangling, archived-target checks).
- [x] Create `src/model/parent.rs` with `reparent()` orchestrator.
- [x] Register `pub mod parent;` in `src/model/mod.rs`.
- [x] Add the four new `TgError` variants; wire into `exit_code()`.
- [x] Write tests covering: serde round-trip with and without `parent` field in JSONL; existing records (without `parent`) deserialize as `None`; self-parent rejected; dangling parent rejected; archived-target rejected; direct cycle rejected; transitive cycle rejected; valid reparent accepted.

**Verification:**

- [x] `just check` passes.
- [x] `cargo test parent_test` passes; all cycle/validation cases exercised.
- [x] Existing test suite passes unchanged (no regressions in item serialization, deps, etc.).
- [x] Code review passes (`/code-review` → fix → repeat until pass).

**Commit:** `[TG-006][P1] Feature: Add parent field and validation to Item model`

**Notes:**

- `parent` and `dependencies` are independent DAGs per design decision. No cross-graph cycle check in `validate_parent`.
- `validate_parent` rejects archived-task targets (archived tasks are not in the active set). `reparent` takes `archive` param to support this without loading it twice.

**Followups:**

<!-- Items discovered during this phase that should be addressed but aren't blocking -->

---

### Phase 2: Parent field — CLI integration

> Wire `--parent` into `add`/`edit`, block destructive ops on parents with children, add `list --parent` filter and `show` Children section.

**Phase Status:** completed

**Complexity:** Med

**Goal:** Users can set, change, and inspect parent relationships via the existing CLI surface, and cannot accidentally orphan child tasks through `rm` or `archive`.

**Files:**

- `src/cli/args.rs` — modify — add `--parent <id>` to `Add`, `--parent <id>` and `--parent-clear` (mutually exclusive) to `Edit`, add `--parent <id>` to `List`.
- `src/cli/commands/add.rs` — modify — accept `--parent`; call `model::parent::reparent` after insert; if validation fails, the added item must not persist (i.e., validate before the save, inside the `with_lock` closure).
- `src/cli/commands/edit.rs` — modify — accept `--parent` / `--parent-clear`; delegate to `reparent`; cycle detection runs on the proposed post-edit graph.
- `src/cli/commands/rm.rs` — modify — before deletion, scan active tasks for `parent == this.id`; if any, return `ParentHasChildren { id, children }`.
- `src/cli/commands/archive.rs` — modify — `tg archive` is a bulk sweep today (scans active for `done`, moves them to archive), not per-ID. Apply the rule during the sweep: for each `done` candidate, skip it if any active (non-`done`) item has `parent == candidate.id`, and emit a warning-style message listing the offending child IDs. The sweep continues for other candidates. Exit nonzero only if every candidate was skipped because of blocks; otherwise exit zero with warnings so the successful archives are not lost.
- `src/cli/commands/list.rs` — modify — add `--parent <id>` filter (filters after existing status/tag filters).
- `src/cli/commands/show.rs` — modify — after existing sections, append "Children:" listing direct children (sorted by priority desc, then ID); limit 10 with "(N more)" suffix when truncated; omit section entirely when no children.
- `tests/parent_test.rs` — modify — extend with CLI-level tests for add/edit/rm/archive/list/show.
- `tests/list_test.rs` — modify — add tests for `--parent` filter (including empty-result and combined with `--status`).
- `tests/show_test.rs` — modify — add test for "Children:" section rendering.
- `tests/rm_test.rs` — modify — add test for rejection when children exist.
- `tests/archive_test.rs` — modify — add tests: a `done` parent with an active child is SKIPPED during sweep (archived children list does not include it, warning is emitted); a `done` parent whose children are all `done` (and thus also archiving this sweep) archives normally.

**Patterns:**

- Mirror `src/cli/commands/edit.rs:29-107` for the `Add` and `Edit` parent handling inside the `with_lock` closure.
- Mirror the filter chain in `src/cli/commands/list.rs` for `--parent`.
- Mirror existing section-append pattern in `src/cli/commands/show.rs` for "Children:".

**Tasks:**

- [x] Add clap flags for `--parent`/`--parent-clear` on Add/Edit/List in `src/cli/args.rs`.
- [x] Wire `add` to validate `--parent` via `reparent` before `save_active`. Load the archive (`Store::load_all_archive()`) inside the `with_lock` closure to pass to `reparent`.
- [x] Wire `edit` to handle `--parent` and `--parent-clear` via `reparent`. Load the archive inside the `with_lock` closure to pass to `reparent`.
- [x] Note in Phase Notes that the archive is re-read on every parent-mutating edit; if this becomes hot (measurable), introduce a `Store::archive_has_id(&str)` helper in a follow-up.
- [x] Block `rm` when active children exist; return typed error with IDs.
- [x] Update the `archive` sweep to skip `done` candidates that have active children; emit a per-candidate warning naming the child IDs; continue processing other candidates. Return nonzero only if zero candidates succeeded because of blocks (otherwise succeed with warnings).
- [x] Implement `--parent` filter in `list` (combines with existing filters).
- [x] Render "Children:" section in `show` (sorted, capped at 10, `(N more)` suffix).
- [x] Write tests: `add --parent <id>`, `add --parent <bogus>` (errors), `edit --parent <id>`, `edit --parent-clear`, `rm <parent-with-children>` rejected, `archive <parent-with-children>` rejected, `list --parent <id>`, `show <parent>` includes Children, `show <leaf>` omits Children.

**Verification:**

- [x] `just check` passes.
- [x] `tg add "child" --parent <id>` round-trips and `tg show <id>` displays the child.
- [x] `tg rm <parent>` is rejected with a clear error pointing at child IDs.
- [x] `tg list --parent <id>` returns only direct children.
- [x] All existing CLI tests pass unchanged.
- [x] Code review passes.

**Commit:** `[TG-006][P2] Feature: Wire parent field into CLI commands`

**Notes:**

- Archived tasks may retain dangling parent refs (if parent was later removed). Phase 5 doctor repairs these. Do not repair during normal commands.
- `--parent` on `list` uses direct children only — not recursive descendants. Recursive traversal is a Phase 4 `tg query` use case.
- The archive is re-read on every parent-mutating `add`/`edit` (via `Store::load_all_archive`). At solo-dev scale this is fine. If it becomes measurably hot, introduce a `Store::archive_has_id(&str) -> bool` fast-path that streams the archive for a single ID check without full deserialization. Tracked as a followup.
- `tg archive` sweep now returns nonzero exit with `TgError::InvalidInput` when every done candidate is blocked by active children. When at least one candidate succeeds, the sweep exits zero and writes warnings to stderr for skipped candidates, preserving forward progress. The JSON envelope gains `skipped` and `skipped_ids` keys.
- `--force` on `tg rm` intentionally does NOT bypass the children check. Orphaning children silently is a harder-to-reverse invariant violation than dangling deps (which `--force --clear-deps` already handles). Users must explicitly reparent or delete children first.

**Followups:**

<!-- Items discovered during this phase that should be addressed but aren't blocking -->

---

### Phase 3: Cache foundation — module + rebuild

> Introduce `src/cache/` module, rusqlite/xxhash-rust deps, stamp logic, and the lazy rebuild path. Gitignore wiring in init.

**Phase Status:** not_started

**Complexity:** High

**Goal:** A working cache module that can rebuild `.taskgolem/cache.db` from JSONL on demand, with composite-stamp freshness detection and atomic temp-rename writes. No user-facing command yet — inert infrastructure exercised via unit + integration tests.

**Files:**

- `Cargo.toml` — modify — add `rusqlite = { version = "=0.32", features = ["bundled", "hooks", "limits"] }` and `xxhash-rust = { version = "0.8", features = ["xxh3"] }`.
- `src/cache/mod.rs` — create — public API (`open_or_rebuild`, `rebuild_to`), `Stamp { mtime_nanos, size, xxh3_64 }`, DDL constants (`pub const SCHEMA_VERSION: u32 = 1`, `pub const DDL: &str` containing the full DDL string from Design §Cache Schema — exported so Phase 4's `--schema` can render it), meta-table read/write helpers.
- `src/cache/rebuild.rs` — create — rebuild orchestration: acquire `store.with_lock` (read side) → load active items → `detect_all_cycles` + `detect_all_parent_cycles` → compute stamp → release lock → sweep stale `.db.tmp-*` → open `cache.db.tmp-<pid>` with `SQLITE_OPEN_CREATE|READWRITE` + `journal_mode=MEMORY` + `synchronous=OFF` → apply DDL → `BEGIN IMMEDIATE` → prepared-statement inserts into `tasks`, `task_tags`, `task_deps` → materialize `task_view` (with `WHERE depth < 64` on the recursive CTE) → insert stamp rows into `_cache_meta` → `COMMIT` → close → `fsync` → atomic rename over `cache.db` → best-effort `ensure_gitignore` on first rebuild in a project missing it.
- `src/lib.rs` — modify — `pub mod cache;`.
- `src/store/mod.rs` — modify — add `pub fn tasks_jsonl_path(&self) -> &Path` accessor; add `pub fn ensure_gitignore(&self) -> Result<(), TgError>` that creates `.taskgolem/.gitignore` if missing and appends `cache.db`, `cache.db-journal`, `cache.db.tmp-*` if not already present (idempotent; per-line dedupe).
- `src/cli/commands/init.rs` — modify — after project creation, call `store.ensure_gitignore()`.
- `src/errors.rs` — modify — add `CacheCorrupt { detail: String }`, `CacheRebuildFailed { source: String }`, `CacheSchemaVersionMismatch { stored: u32, expected: u32 }` variants (exit 2).
- `tests/cache_test.rs` — create — fresh rebuild from empty JSONL; rebuild populates `tasks`/`task_tags`/`task_deps`/`task_view`; stamp round-trips; second rebuild is idempotent (row counts match); stamp mismatch triggers rebuild; `task_view.depth_from_root` correct over nested parents; `task_view.is_ready` correct for todo-with-no-deps and todo-with-unmet-dep; cycle in JSONL aborts rebuild with typed error.
- `tests/init_test.rs` — modify — add test that `tg init` creates `.taskgolem/.gitignore` containing the cache lines.

**Patterns:**

- Mirror `src/store/jsonl.rs:129-159` for the temp-file + `sync_all` + atomic rename pattern (use `tempfile::NamedTempFile` the same way).
- Mirror `src/store/mod.rs:34-41` for the `with_lock` callback acquired during JSONL read.
- Mirror the existing `#[derive(thiserror::Error)]` structure in `src/errors.rs` for new variants.

**Tasks:**

- [ ] Add `rusqlite` and `xxhash-rust` dependencies with the exact versions/features specified.
- [ ] Run `cargo build` to confirm `bundled` SQLite compiles on this machine (flag any toolchain issues early).
- [ ] Define `Stamp` struct and `compute_stamp(path: &Path) -> Result<Stamp, TgError>` using `xxh3_64` on the full file bytes + `metadata()` for mtime/size.
- [ ] Write the DDL constant (tables + indices from design §Cache Schema) with `SCHEMA_VERSION = 1`.
- [ ] Implement `rebuild_to(jsonl_path, cache_path, store)` per flow in design §"Cache rebuild (internal)".
- [ ] Implement `open_or_rebuild(jsonl_path, cache_path, store)` — open read-only, read stamp from `_cache_meta`, compare to current JSONL stamp, trigger rebuild if mismatch or schema version mismatch; fall back to `:memory:` SQLite if cache path is unwritable and print `--verbose` notice.
- [ ] Implement `Store::tasks_jsonl_path` and `Store::ensure_gitignore`.
- [ ] Wire `ensure_gitignore` into `init`.
- [ ] Add the three new `TgError` variants and their exit codes.
- [ ] Write cache tests including: rebuild on fresh checkout, rebuild idempotence (two rebuilds produce identical row counts), rebuild invalidation on stamp mismatch (mutate JSONL, next `open_or_rebuild` rebuilds), `task_view` correctness for nested parents (depth=0/1/2), `is_ready`/`unmet_dep_count` correctness, parent-cycle in JSONL aborts rebuild with `ParentCycle`, cyclic-JSONL error message names the offending IDs and suggests `tg doctor`, duplicate-ID in JSONL aborts rebuild pointing at the duplicated IDs.
- [ ] Write an unwritable-filesystem test: simulate `.taskgolem/` read-only (e.g., `chmod 0555` on a tempdir or redirect cache path to an unwritable directory) and verify `open_or_rebuild` succeeds via in-memory SQLite and that `--verbose` emits a fallback notice to stderr.
- [ ] Measure rebuild time with a 500-task and 5000-task fixture; record numbers in Phase Notes. Flag if 5k takes >500ms. Fixture must include representative descriptions (~200-500 bytes each) so file-size approximates real usage — the stamp hash is O(file bytes), not O(task count).

**Verification:**

- [ ] `just check` passes.
- [ ] `cargo test cache_test` passes.
- [ ] `tg init` in a fresh directory creates `.taskgolem/.gitignore` with the cache lines.
- [ ] Rebuild of a 5k-task fixture completes in <500ms on local SSD (PRD non-functional requirement).
- [ ] No existing test regresses (cache module is inert from the CLI side).
- [ ] Code review passes.

**Commit:** `[TG-006][P3] Feature: Add SQLite cache foundation with lazy rebuild`

**Notes:**

- `PRAGMA quick_check` is NOT run on the happy path — reserved for doctor (Phase 5).
- Rebuild acquires `with_lock` only during JSONL read to keep write-side contention minimal; the lock is released before the SQLite transaction starts.
- `cache.db.tmp-<pid>` uses the current PID to avoid collision between concurrent rebuilds; stale temp files from killed processes are swept best-effort at the start of each rebuild.
- A pre-existing cyclic JSONL (from a bad merge that bypassed write-time cycle checks) will block every `tg query` until repaired — by design, since the cache would otherwise loop during `task_view` materialization. The error message must name the offending IDs and recommend `tg doctor --fix` (delivered in Phase 5). Phase 3 must include a test for this failure path to lock in the message.

**Followups:**

<!-- Items discovered during this phase that should be addressed but aren't blocking -->

---

### Phase 4: Query sandbox + `tg query` CLI

> Add SELECT-only query execution with three-layer sandbox and expose `tg query <sql>` with `--schema`, `--json`, `--timeout`.

**Phase Status:** not_started

**Complexity:** High

**Goal:** Users and agents can run arbitrary SELECT queries against the cache through a safe sandbox. Non-SELECT statements, ATTACH, file-access functions, and non-readonly pragmas are all denied at prepare time.

**Files:**

- `src/cache/query.rs` — create — sandbox + execution. Opens a read-only connection, applies `query_only=ON`/`trusted_schema=OFF`/`defensive=ON`, registers an allowlist authorizer (default deny: allow `SQLITE_SELECT`/`SQLITE_READ`/`SQLITE_RECURSIVE`; allow `SQLITE_FUNCTION` except `load_extension`/`readfile`/`writefile`/`edit`/`fts3_tokenizer`; allow `SQLITE_PRAGMA` only for `table_info`/`index_list`/`index_info`; deny all else), registers a per-query `progress_handler(1000, |deadline|)` with user-specified timeout, prepares and executes, returns typed `QueryResult`.
- `src/cache/mod.rs` — modify — expose `QueryResult { columns: Vec<String>, column_types: Vec<SqlType>, rows: Vec<Vec<SqlValue>> }`, `SqlType { Integer, Real, Text, Null, Blob }`, `SqlValue { Null, Integer(i64), Real(f64), Text(String), Blob(Vec<u8>) }`.
- `src/cli/commands/query.rs` — create — clap dispatch: parse `<sql>` positional, `--schema`, `--json`, `--timeout N` (default 5, no upper cap); resolve store; call `cache::open_or_rebuild`; for `--schema` print the Markdown schema document (tables, indices, `task_view` column descriptions, active-only note, `depth < 64` reminder) and exit; otherwise call `cache::query::execute(&sql, timeout)` and format output (aligned tabular default, JSON envelope with `--json`); map errors to exit codes.
- `src/cli/mod.rs` — modify — wire `Commands::Query { ... }` to the new handler.
- `src/cli/args.rs` — modify — add `Query { sql: Option<String>, #[arg(long)] schema: bool, #[arg(long)] json: bool, #[arg(long, default_value = "5")] timeout: u64 }` variant. Validate: `sql` required unless `--schema`.
- `src/errors.rs` — modify — add `QueryTimeout { limit_secs: u64 }`, `QueryDenied { action: String, hint: String }`, `QuerySyntax { source: String }` variants (exit 1). Update `exit_code()` mapping.
- `tests/query_test.rs` — create — `SELECT count(*) FROM tasks`, `WITH RECURSIVE ...` descendants/ancestors, `SELECT * FROM task_view WHERE is_ready = 1`, `--json` output shape, `--schema` output shape, `--timeout 0` triggers immediate `QueryTimeout`, oversized integer as JSON string with stderr warning.
- `tests/sandbox_test.rs` — create — one test per denied action (confirmed via `QueryDenied`): `ATTACH`, `DETACH`, `PRAGMA writable_schema=ON`, `PRAGMA query_only=OFF` (escape attempt — must be denied), `INSERT`, `UPDATE`, `DELETE`, `CREATE TABLE`, `CREATE VIRTUAL TABLE ... USING ...` (virtual-table escape — must be denied), `DROP TABLE`, `ALTER TABLE`, `SELECT load_extension(...)`, `SELECT readfile('/etc/passwd')`, `BEGIN/COMMIT`, `REINDEX`, `ANALYZE`. Also test that an obfuscated `WITH x AS (INSERT ...)` is caught. Plus one positive test: `SELECT name, type, sql FROM sqlite_master` succeeds — introspection reads are allowed.

**Patterns:**

- Mirror `src/cli/commands/edit.rs` for command dispatch shape.
- Mirror `tests/common/mod.rs` harness for `run_tg_json` in query tests.

**Tasks:**

- [ ] Define `QueryResult`/`SqlType`/`SqlValue` in `src/cache/mod.rs` and re-export.
- [ ] Implement `cache::query::execute(cache_path: &Path, sql: &str, timeout: Duration) -> Result<QueryResult, TgError>` (the query module owns connection lifecycle):
  - Internally call `cache::open_or_rebuild` to get a freshly-stamped cache, then re-open read-only for the query connection.
  - Apply `PRAGMA query_only=ON; PRAGMA trusted_schema=OFF; PRAGMA defensive=ON;`.
  - Register allowlist authorizer (default deny).
  - Register per-query `progress_handler` bounded by `Instant::now() + timeout`.
  - Prepare statement (authorizer fires here).
  - Execute, collect rows into typed result.
  - Map `rusqlite::Error::SqliteFailure` → `QueryTimeout` on `SQLITE_INTERRUPT`; `QueryDenied` on `SQLITE_AUTH`; `QuerySyntax` otherwise.
- [ ] Implement `tg query` CLI handler with `--schema`, `--json`, `--timeout`.
- [ ] Format tabular output: aligned columns (mirror `tg list` width calculation); header row with column names; emit "(0 rows)" for empty result.
- [ ] Format JSON output: `{"columns":[...],"rows":[[...],...]}` with serialization per design §Error Handling (Integer→number, oversized Integer→string + stderr warn, Real→number with NaN→null+warn, Text→string, Null→null, Blob→error).
- [ ] Implement `--schema` output: Markdown document listing all tables + indices from the DDL constant, plus a `task_view` columns block with one-line descriptions, an "Active tasks only in v1" callout, and a "Bound recursive CTEs with `depth < 64`" reminder.
- [ ] Write sandbox tests covering each denied action code individually.
- [ ] Write query tests for tabular and JSON output shapes, timeout, schema command, descendants CTE, ancestors CTE.
- [ ] Add `--verbose` rebuild notice in `tg query`: when `open_or_rebuild` triggers a rebuild, print `rebuilding cache (N tasks, X ms)` to stderr.

**Verification:**

- [ ] `just check` passes.
- [ ] `tg query "SELECT count(*) FROM tasks"` returns correct count.
- [ ] `tg query --schema` prints a well-formed Markdown schema document.
- [ ] All 14+ sandbox tests pass — every attempted write/attach/file-access returns `QueryDenied` with a useful hint.
- [ ] `tg query "SELECT 1" --timeout 0` returns `QueryTimeout` immediately.
- [ ] Recursive descendants and ancestors queries return correct results over a nested fixture.
- [ ] No regression in existing CLI.
- [ ] Code review passes.

**Commit:** `[TG-006][P4] Feature: Add tg query command with SELECT-only sandbox`

**Notes:**

- Authorizer default-deny: any new SQLite action code introduced by an upstream version bump remains blocked automatically.
- Progress handler replaces the previous handler for each query — no leakage between invocations.
- `rusqlite = "=0.32"` is exact-pinned to pin the bundled SQLite version and thus the authorizer action-code surface.
- `sqlite_master` / `sqlite_schema` reads are **allowed** — they're read-only and the authorizer blocks every mutation path regardless. Agents can introspect via standard SQL alongside `tg query --schema`.

**Followups:**

<!-- Items discovered during this phase that should be addressed but aren't blocking -->

---

### Phase 5: Doctor extensions

> Teach `tg doctor` about parent cycles, dangling parent refs, cache consistency, and gitignore hygiene.

**Phase Status:** not_started

**Complexity:** Med

**Goal:** `tg doctor` is the user-facing reconciliation tool for every invariant introduced in TG-006. `tg doctor --fix` can repair dangling archive parent refs and a missing gitignore.

**Files:**

- `src/cli/commands/doctor.rs` — modify — add these new checks after the existing dep-cycle check:
  - **Parent cycles** — run `deps::detect_all_parent_cycles(active_items)`; emit `parent_cycle` issues (severity `error`). Not auto-repaired (user decides where to cut the cycle).
  - **Dangling parent (active)** — for each active item with `Some(parent)`, check the ID exists in active items; emit `parent_dangling_active` issues (severity `error`). Not auto-repaired.
  - **Dangling parent (archive)** — for each archived item with `Some(parent)`, check the ID exists in active or archive items; emit `parent_dangling_archive` issues (severity `warning`). Repair: clear the parent field via archive rewrite (mirror existing archive rewrite path).
  - **Duplicate IDs (active + archive)** — scan both stores for any ID appearing more than once; emit `duplicate_id` issues (severity `error`). Not auto-repaired (user must manually decide which record to keep). PRD Must-Have explicitly asks for this.
  - **Cache consistency** — full rebuild into a temp path, reopen, compare `schema_version` + per-table row counts between existing `cache.db` and the fresh rebuild; emit `cache_drift` issues (severity `warning`) when they disagree. Repair: atomic-rename the fresh rebuild over `cache.db`.
  - **Gitignore** — verify `.taskgolem/.gitignore` exists and contains `cache.db`, `cache.db-journal`, `cache.db.tmp-*`; emit `gitignore_missing` issues (severity `warning`). Repair: call `Store::ensure_gitignore`.
- `tests/doctor_test.rs` — modify — add tests for each new check:
  - Parent cycle detected on manually-crafted cyclic JSONL.
  - Dangling active parent detected.
  - Dangling archive parent detected; `--fix` repairs.
  - Duplicate ID in active detected; duplicate ID between active and archive detected.
  - Cache drift (simulate by mutating JSONL without rebuilding): first `--fix` run repairs by rebuild; second run reports clean.
  - Missing gitignore: detected; `--fix` creates it.

**Patterns:**

- Mirror existing check shape in `src/cli/commands/doctor.rs:170-370` — each check appends `Issue` structs to a shared vector, aggregated into the final `DoctorReport`.
- For archive repair, mirror the existing `archive` rewrite path (atomic rewrite via `jsonl::write_atomic`).

**Tasks:**

- [ ] Add `parent_cycle` check using `deps::detect_all_parent_cycles`.
- [ ] Add `parent_dangling_active` check.
- [ ] Add `parent_dangling_archive` check with `--fix` repair (clear parent field).
- [ ] Add `duplicate_id` check scanning active + archive; no auto-repair.
- [ ] Add `cache_drift` check: rebuild to temp, compare schema_version + per-table row counts; repair via rename.
- [ ] Add `gitignore_missing` check with `--fix` repair.
- [ ] Write doctor tests for each new check, both detection and repair paths where applicable. Include a duplicate-ID fixture.
- [ ] Update `tg doctor`'s summary output to include counts of the new issue types.

**Verification:**

- [ ] `just check` passes.
- [ ] Cyclic parent JSONL is detected by `tg doctor`.
- [ ] Missing gitignore is detected; `tg doctor --fix` creates it; second run is clean.
- [ ] Stale cache is detected; `tg doctor --fix` rebuilds; second run is clean.
- [ ] Existing doctor tests pass unchanged.
- [ ] Code review passes.

**Commit:** `[TG-006][P5] Feature: Extend tg doctor with parent, cache, and gitignore checks`

**Notes:**

- Dangling active parent is NOT auto-repaired (data loss risk — the user must decide whether to reparent or clear). Archive dangling refs ARE auto-repaired since archive is read-only once written and clearing a dangling ref is safe.
- Cache drift should be rare (benign stamp mismatch triggers a rebuild on next query). A persistent drift after repair indicates a bug.

**Followups:**

<!-- Items discovered during this phase that should be addressed but aren't blocking -->

---

### Phase 6: Skill update

> Add the 5 canonical SQL examples and transition-point guidance to the task-golem skill.

**Phase Status:** not_started

**Complexity:** Low

**Goal:** Agents using the task-golem skill have concrete, copy-pasteable queries for the most common tree and readiness questions, and guidance on when to query (starting a task, finishing a task, explicit "what's next") rather than every turn.

**Files:**

- `~/.claude/skills/task-golem/SKILL.md` (external — lives outside this repo; locate it first — may also be shadowed by a repo-local `.claude/skills/task-golem/SKILL.md`) — modify — add a new section "Query recipes" with the 5 canonical queries from PRD §Desired Outcome #5:
  1. All descendants of a task (recursive CTE bounded `depth < 64`).
  2. All ancestors of a task (recursive CTE bounded `depth < 64`).
  3. Ready tasks under a given parent (join `task_view` on `is_ready = 1` with recursive descendants CTE).
  4. Unblocked todos by priority (`SELECT * FROM task_view WHERE is_ready = 1 ORDER BY priority DESC`).
  5. Orphan tasks with no parent (`SELECT * FROM task_view WHERE parent IS NULL`).
- Same file — add a "When to query" section with three transition points: starting a task (to find candidates), finishing a task (to find the next one), explicit "what's next" requests. Emphasize NOT every turn.
- Same file — add a short note pointing agents at `tg query --schema` for schema discovery.
- `Cargo.toml` — modify — bump `version = "0.1.0"` → `version = "0.2.0"`. This ships alongside the feature-complete surface and signals a pre-1.0 minor feature release.

**Patterns:**

- Mirror the existing section structure in the task-golem skill (locate by reading the skill file first).

**Tasks:**

- [ ] Locate the canonical task-golem skill source file.
- [ ] Draft the 5 canonical queries against the actual `task_view` schema shipped in Phase 3. Verify each runs (copy-paste into a test fixture).
- [ ] Draft the "When to query" guidance (~3 bullets).
- [ ] Add a pointer to `tg query --schema`.
- [ ] Verify the skill renders correctly (no broken markdown).
- [ ] Hand-test: run each of the 5 queries against a real `tg` project and confirm output matches the example.
- [ ] Bump `Cargo.toml` version from `0.1.0` to `0.2.0`; run `cargo build` to refresh `Cargo.lock`; commit lockfile alongside the version bump.

**Verification:**

- [ ] All 5 canonical queries were hand-executed against a real `tg` project; outputs captured in Phase Notes.
- [ ] Skill file renders correctly (no broken markdown).
- [ ] `tg query --schema` output matches the schema the canonical queries assume.
- [ ] Code review passes (manual review of the skill diff; no repo tests to run).

**Commit:** The skill lives outside this repo. If the skill is version-controlled in its own repository, commit there with `[TG-006][P6] Docs: Add task-golem canonical queries and guidance`. If not, document the update in the change's review log only — nothing is committed to task-golem itself for P6.

**Notes:**

- The skill update is tied to this change per PRD decision (not split to a follow-up). Shipping with working defaults; iterate separately if prompting empirically needs tuning.
- Because the skill is external, Phase 6 does not flow through `just check` and is not covered by the repo's commit convention. Confirm the skill path with the user at phase start if ambiguous.

**Followups:**

<!-- Items discovered during this phase that should be addressed but aren't blocking -->

---

## Final Verification

- [ ] All 6 phases complete.
- [ ] All PRD must-have success criteria met (see checklist below).
- [ ] All PRD should-have success criteria met or explicitly deferred with rationale.
- [ ] `just check` passes.
- [ ] No regressions in any existing test.
- [ ] `tg doctor` reports clean on a fresh post-upgrade project.
- [ ] Rebuild performance meets PRD targets: <100ms at 500 tasks, <500ms at 5000 tasks.
- [ ] Sandbox verified against every denied action code with an explicit test.

### PRD Must-Have Traceability

| PRD Must-Have | Covered By |
|---|---|
| `parent: Option<String>` field, serde round-trip, validation | P1 |
| Deleting a task with children is rejected | P2 (`rm`) |
| `tg edit` supports `--parent` with cycle check | P2 |
| Persistent SQLite cache at `.taskgolem/cache.db` | P3 |
| Writes update JSONL atomically; cache rebuilt lazily | P3 |
| Composite stamp `(mtime, size, xxh3_64)` in `_cache_meta` | P3 |
| Rebuild on stale/missing/corrupt | P3 (stale/missing); P5 (corrupt via doctor) |
| Parent cycle detection independent of deps | P1 (writes); P5 (doctor) |
| Duplicate JSONL IDs detected on load | P3 (rebuild abort) |
| `tg query "SELECT ..."` SELECT-only | P4 |
| `tg query --schema` | P4 |
| Recursive CTE over `parent` works | P4 (tests) |
| Existing CLI unchanged | P1–P5 (no reimplementation) |
| JSONL remains human-readable and git-friendly | P1 (stable field ordering, omit-when-null via serde default) |
| `tg doctor` verifies JSONL ↔ cache consistency | P5 |

### PRD Should-Have Traceability

| PRD Should-Have | Covered By |
|---|---|
| `tg list --parent <id>` filter | P2 |
| `tg show <id>` Children section | P2 |
| `task_view` with id/title/status/priority/parent/depth_from_root/is_ready/unmet_dep_count | P3 |
| Skill update with 5 canonical queries | P6 |
| Statement timeout on `tg query` (default 5s, `--timeout` override) | P4 |

### PRD Nice-to-Have (Deferred)

- `tg tree <id>` — not in scope; can be a small follow-up.
- Batch-write `tg add` — not in scope; revisit if agent burst write amplification becomes a measured problem.
- `tg index` — not in scope; `tg doctor --fix` covers manual-rebuild use cases.

## Execution Log

<!-- Updated automatically during execution via /implement-spec -->

| Phase | Status | Commit | Notes |
|-------|--------|--------|-------|

## Followups Summary

<!-- Aggregated from all phases by change-review. Items for post-implementation triage. -->

### Critical

### High

### Medium

### Low

## Design Details

### Key Types

Summarized here for implementer convenience; authoritative definitions live in the Design doc §Interface Contracts.

```rust
// src/model/item.rs (extended)
pub struct Item {
    pub id: String,
    pub title: String,
    pub status: Status,
    pub priority: i64,
    #[serde(serialize_with = "serialize_option_nullable")]
    pub description: Option<String>,
    pub tags: Vec<String>,
    pub dependencies: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(serialize_with = "serialize_option_nullable")]
    pub blocked_reason: Option<String>,
    #[serde(serialize_with = "serialize_option_nullable")]
    pub blocked_from_status: Option<Status>,
    #[serde(serialize_with = "serialize_option_nullable")]
    pub claimed_by: Option<String>,
    #[serde(serialize_with = "serialize_option_nullable")]
    pub claimed_at: Option<DateTime<Utc>>,
    #[serde(serialize_with = "serialize_option_nullable")]
    pub parent: Option<String>,          // <-- new
    #[serde(flatten)]
    pub extensions: BTreeMap<String, serde_json::Value>,
}

// src/cache/mod.rs (new)
pub struct Stamp { pub mtime_nanos: i128, pub size: u64, pub xxh3_64: u64 }
pub struct QueryResult {
    pub columns: Vec<String>,
    pub column_types: Vec<SqlType>,
    pub rows: Vec<Vec<SqlValue>>,
}
pub enum SqlType { Integer, Real, Text, Null, Blob }
pub enum SqlValue { Null, Integer(i64), Real(f64), Text(String), Blob(Vec<u8>) }

// src/errors.rs (extended variants)
// ParentSelfReference { id }, ParentCycle { ids }, ParentDangling { id, parent },
// ParentHasChildren { id, children } — exit 1
// CacheCorrupt { detail }, CacheRebuildFailed { source },
// CacheSchemaVersionMismatch { stored, expected } — exit 2
// QueryTimeout { limit_secs }, QueryDenied { action, hint }, QuerySyntax { source } — exit 1
```

### Architecture Details

See Design doc §System Design. Key invariants:

1. **JSONL is the single cross-file invariant.** The cache is pure derived state, never load-bearing for correctness.
2. **Lazy rebuild.** Write commands never touch the cache. `tg query` entry is the only rebuild trigger.
3. **Rebuild atomicity.** `cache.db.tmp-<pid>` → `fsync` → atomic rename over `cache.db`. Concurrent rebuilds are safe; last rename wins; POSIX unlink-while-open keeps readers of the old inode alive.
4. **Sandbox defense-in-depth.** Three layers: OS-level `SQLITE_OPEN_READ_ONLY`, engine pragmas (`query_only`/`trusted_schema`/`defensive`), allowlist authorizer (default-deny).

### Design Rationale

Locked-in spec-level defaults (resolved during SPEC draft):

- **Default tabular output format:** aligned columns (mirror `tg list` width calc). Rationale: consistent UX with existing CLI.
- **`tg query --schema` output shape:** Markdown — tables section lists each CREATE TABLE + index; `task_view` columns block has one-line description per column; explicit "Active tasks only in v1" + "bound recursive CTEs with `depth < 64`" notes. Rationale: matches how agents consume docs.
- **Rebuild progress indicator:** silent by default; under `--verbose`, print `rebuilding cache (N tasks, X ms)` to stderr after rebuild completes. Rationale: interactive users won't see noise; debugging users can verify rebuild happened.

All other rationale lives in the Design doc §Technical Decisions and §Alternatives Considered.

---

## Retrospective

[Fill in after completion]

### What worked well?

### What was harder than expected?

### What would we do differently next time?
