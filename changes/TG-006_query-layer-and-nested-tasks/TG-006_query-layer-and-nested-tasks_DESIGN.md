# Design: Query Layer and Nested Tasks

**ID:** TG-006
**Status:** Complete
**Created:** 2026-04-14
**PRD:** ./TG-006_query-layer-and-nested-tasks_PRD.md
**Tech Research:** ./TG-006_query-layer-and-nested-tasks_TECH_RESEARCH.md
**Mode:** Medium

## TL;DR

- **Approach:** Add `parent: Option<String>` to the existing `Item` struct (mirror `serialize_option_nullable` pattern). Introduce a new `src/cache/` module that owns a SQLite sidecar at `.taskgolem/cache.db`, rebuilt **lazily on `tg query` entry** when its composite `(mtime, size, xxh3_64)` stamp disagrees with the JSONL. Rebuild acquires the existing advisory lock while reading JSONL, releases it before opening SQLite. SELECT-only enforced by a **three-layer sandbox** (OPEN_READONLY + query_only/trusted_schema/defensive pragmas + allowlist authorizer). Existing Rust command implementations (`list`, `ready`, `next`, …) are untouched.
- **Key decisions:** **Lazy rebuild** (not eager-after-write) — JSONL remains the only cross-file invariant, cache is pure derived state. **DELETE journal mode** (not WAL) — no sidecar files to gitignore. **Normalized tables + fully-materialized `task_view`** — `is_ready`, `depth_from_root`, `unmet_dep_count` computed once at rebuild so agents rarely need to write recursive CTEs. **Active-only cache for v1** — archived tasks deferred (PRD Out-of-Scope updated). **Allowlist authorizer** (default-deny) for future-proofing against upstream SQLite changes.
- **Tradeoffs:** Query-after-write pays rebuild cost (~10-100ms typical, <500ms at 5k tasks) in exchange for simple crash-safety and zero cross-file transactions. One extra compile dep (`rusqlite` bundled SQLite adds ~1MB + 5-10s compile).
- **Needs attention:** None — all four directional items resolved during review (PRD amended to lazy rebuild + archive out-of-scope; sandbox flipped to allowlist; `task_view` stays fully materialized).

## Overview

TG-006 adds two orthogonal capabilities — a `parent` field enabling arbitrary task nesting, and a SELECT-only SQL query interface — without disrupting the existing Rust CLI. The query interface is powered by a disposable SQLite cache derived from the authoritative JSONL. The cache rebuilds lazily when its composite stamp (mtime, size, content hash) disagrees with the current JSONL, which makes the whole system crash-safe by construction: there's no cross-file transaction to get wrong, and a corrupted or missing cache is always rebuildable from the JSONL. Existing commands continue to work in pure Rust with no SQL touching their path; the cache is read-only infrastructure bolted on the side.

---

## System Design

### High-Level Architecture

```
                        ┌──────────────────────────────────┐
                        │         CLI dispatch             │
                        │       src/cli/mod.rs             │
                        └──┬────────────────────────────┬──┘
                           │                            │
              write/read commands               tg query (new)
              (existing)                               │
                           │                            ▼
                           ▼                   ┌───────────────┐
                  ┌────────────────┐           │ cache::query  │
                  │  store::Store  │           │  (sandbox +   │
                  │  (JSONL +      │           │   stamp check │
                  │   lock)        │           │   + rebuild   │
                  └───────┬────────┘           │   if stale)   │
                          │                    └───────┬───────┘
                          ▼                            │
                  ┌─────────────────┐                  │
                  │  tasks.jsonl    │◀─────────────────┤ reads JSONL
                  │   archive.jsonl │                  │ when rebuilding
                  │   (source of    │                  │
                  │    truth)       │                  │
                  └─────────────────┘                  │
                                                       ▼
                                              ┌─────────────────┐
                                              │  cache.db       │
                                              │  (disposable    │
                                              │   derived       │
                                              │   state)        │
                                              └─────────────────┘
```

**Rule:** JSONL is the single cross-file invariant. Cache is pure derived state — always rebuildable, never load-bearing.

### Component Breakdown

#### `src/cache/` (new module)

**Purpose:** Own everything SQLite-related. Keep it sealed off so the rest of the codebase stays JSONL-pure.

**Submodules (kept minimal — 3 files):**
- `cache/mod.rs` — public API + stamp logic + DDL constants. Functions:
  - `open_or_rebuild(jsonl_path: &Path, cache_path: &Path, store: &Store) -> Result<Cache, TgError>` — stamp check, rebuild if stale, returns read-only connection handle.
  - `rebuild_to(jsonl_path: &Path, cache_path: &Path, store: &Store) -> Result<(), TgError>` — force rebuild; used by `tg doctor`.
  - `Stamp { mtime_nanos, size, xxh3_64 }` — internal value type.
- `cache/rebuild.rs` — rebuild-from-JSONL logic. Acquires `store.with_lock()` for the JSONL read, runs `detect_all_parent_cycles` before insert, opens temp `.db.tmp-<pid>`, `BEGIN IMMEDIATE`, bulk-insert with prepared statements, populate `task_view`, `COMMIT`, release lock, `fsync`, atomic rename over `cache.db`, sweep any stale `.db.tmp-*` from previous runs.
- `cache/query.rs` — SELECT-only sandbox. Opens connection with `SQLITE_OPEN_READ_ONLY`, sets `query_only=ON, trusted_schema=OFF, defensive=ON`, registers authorizer with explicit pragma allowlist + function denylist, registers per-query `progress_handler` with deadline, executes statement, returns typed `QueryResult`.

**Interfaces:**
- **Input:** explicit `jsonl_path: &Path` and `cache_path: &Path` (CLI layer resolves from `Store`). `store: &Store` passed only for `with_lock`. Raw SQL strings for query.
- **Output:** `QueryResult { columns: Vec<String>, column_types: Vec<SqlType>, rows: Vec<Vec<SqlValue>> }` where `SqlValue` is an enum over `Null | Integer(i64) | Real(f64) | Text(String) | Blob`. CLI layer owns JSON/tabular formatting.
- **Errors:** `TgError` (see Error Handling section below).

**Dependencies:** `rusqlite` (bundled, hooks, limits features), `xxhash-rust` (xxh3 feature), existing `Store` for lock acquisition only.

#### `src/model/item.rs` (extended)

**Purpose:** Add the `parent` field.

**Changes:**
- Add `parent: Option<String>` with `#[serde(serialize_with = "serialize_option_nullable")]`.
- Append `"parent"` to `KNOWN_FIELD_NAMES`.
- No explicit migration: existing records deserialize `parent` as `None` via serde default.

#### `src/model/deps.rs` (extended) + `src/model/parent.rs` (new)

**Purpose:** Cycle detection for the parent DAG + single orchestration point for reparent mutations.

**Changes to `deps.rs`:**
- `would_create_parent_cycle(items, source_id, new_parent_id) -> bool` — DFS following `parent` edges. Parallel in shape to the existing dependency-DAG function.
- `detect_all_parent_cycles(items) -> Vec<Vec<String>>` — Kahn's sort over parent edges. **Called during cache rebuild** before INSERTing into SQL (defense against manually-edited JSONL introducing cycles that would loop the materialization CTE).
- `validate_parent(source_id, proposed_parent_id, active_items) -> Result<()>` — rejects self-parent, dangling reference, archived-task reference.
- `parent` and `dependencies` are treated as **independent DAGs** (PRD-decided). No cross-graph cycle checks.

**New `parent.rs`:**
- `reparent(items: &mut [Item], id: &str, new_parent: Option<String>) -> Result<(), TgError>` — single entry point that runs `validate_parent` + `would_create_parent_cycle` + applies the mutation on the in-memory slice. Callers (`add`, `edit`, future batch-edit) use this one function — invariant can't be bypassed.

#### `src/cli/commands/query.rs` (new)

**Purpose:** Handle the new `tg query` subcommand.

**Responsibilities:**
- Parse args: `<sql>` positional | `--schema` (discovery) | `--json` (output mode) | `--timeout N` (seconds, default 5, no upper cap).
- Resolve store, call `cache::open_or_rebuild(jsonl_path, cache_path, &store)` (triggers stamp check + lazy rebuild if stale).
- Execute SQL through `cache::query` → `QueryResult`.
- Format output: JSON envelope for `--json`, aligned tabular otherwise. Serialize `SqlValue` variants: Integer→JSON number, Real→JSON number (with NaN/Inf rejected at query time), Text→JSON string, Null→JSON null, Blob→error (not expected in this schema).
- Map errors to exit codes per `TgError::exit_code()`.

**Not included in v1:** `--no-rebuild` (dropped — correct behavior is always to rebuild on stale), `tg index` (PRD Nice-to-Have, deferred).

#### Existing commands (minor extensions)

- `init` — also calls `store::ensure_gitignore()` (new helper) that creates `.taskgolem/.gitignore` if missing and appends `cache.db` + `cache.db-journal` + `cache.db.tmp-*` if the lines aren't already present. Idempotent.
- `edit` — accepts `--parent <id>` / `--parent-clear` flags; delegates to `model::parent::reparent` (single orchestration point).
- `add` — accepts `--parent <id>`; same `reparent` path after initial insert.
- `rm` — rejects deletion when any active task has `parent == this.id`. **Archived children are not checked** — they may end up with dangling parent refs, which `tg doctor` detects and can repair (consistent with the "active-only cache" scope).
- `archive` — rejects archiving a task with active children (parallel to `rm`). Consistent invariant: a parent can only leave the active set if its children have already left too.
- `list` — new `--parent <id>` filter (uses existing Rust path; no cache dependency).
- `show` — appends a "Children:" section listing up to 10 direct children when non-empty; "(N more)" suffix when truncated.
- `doctor` — adds checks:
  - Parent cycles (via `deps::detect_all_parent_cycles`).
  - Dangling parent refs in active tasks.
  - Dangling parent refs in archive (repair mode: clear the parent field).
  - Cache consistency: delete existing cache, rebuild, compare `schema_version` and per-table row counts. (Byte-count compare removed — SQLite page layout isn't deterministic across rebuilds.)
  - Gitignore check: warn if `.taskgolem/.gitignore` missing or missing `cache.db` line. Repair via `ensure_gitignore()`.

### Data Flow

**Write path (unchanged structurally):**
1. User runs `tg add/edit/rm/…`.
2. Command acquires `store.with_lock()`.
3. Loads `tasks.jsonl`, applies mutation, validates (incl. new parent validation), writes via `jsonl::write_atomic` (temp + fsync + rename).
4. Lock released. **Cache is not touched.** Its stamp is now stale.

**Query path (new):**
1. User runs `tg query "SELECT …"`.
2. `cache::open_or_rebuild(jsonl_path, cache_path, &store)`:
   - If `cache.db` doesn't exist → rebuild.
   - Else open with `SQLITE_OPEN_READONLY`, read stored stamp from `_cache_meta`.
   - If `schema_version` mismatch → rebuild.
   - Else `stat(2)` current JSONL; if `(mtime, size)` differ → rebuild.
   - Else stream-hash the JSONL; if `xxh3_64` differs → rebuild. *(No per-invocation caching of the freshness result — every `tg` invocation is fresh; `stat` is microseconds, hash is sub-ms at our file sizes.)*
   - `PRAGMA quick_check` is **not** run on the happy path — it's reserved for `tg doctor` (running it every query is wasted work since corruption is rare and the stamp check doesn't catch it anyway; if the DB is corrupt, the query itself fails and we report it).
3. Open read-only connection with sandbox.
4. Run SQL via progress-handler-bounded execution.
5. Return typed `QueryResult`.

**Cache rebuild (internal):**
1. Acquire `store.with_lock()` (shared — read side of the advisory lock). This prevents `tg add` / `tg edit` from atomic-renaming the JSONL mid-read.
2. Load active tasks from JSONL (strict parse; abort on malformed with error pointing at offending line; detect duplicate IDs).
3. Run `deps::detect_all_cycles(items)` AND `deps::detect_all_parent_cycles(items)`. If any cycles found, abort rebuild with `TgError::CycleDetected` pointing at offending IDs. Prevents downstream CTE materialization from looping.
4. Compute stamp `(mtime_nanos, size, xxh3_64)` from the just-read file handle (before releasing the lock to avoid TOCTOU drift between read and stamp).
5. Release the lock.
6. Sweep any stale `cache.db.tmp-*` files from previous crashed rebuilds (best-effort unlink; ignore errors).
7. Open `.taskgolem/cache.db.tmp-<pid>` with `SQLITE_OPEN_CREATE | SQLITE_OPEN_READWRITE`.
8. `PRAGMA journal_mode=MEMORY; PRAGMA synchronous=OFF;` (safe — temp file, crash discards it).
9. Apply DDL, `BEGIN IMMEDIATE`.
10. Prepared-statement bulk insert into `tasks`, `task_tags`, `task_deps`.
11. Compute and materialize `task_view` in-transaction. `depth_from_root` via recursive CTE with `WHERE depth < 64` as defense-in-depth (cycles were caught in step 3, but the bound prevents regressions).
12. Insert stamp rows into `_cache_meta`: `schema_version`, `jsonl_mtime_nanos`, `jsonl_size`, `jsonl_xxh3` (one row each, key/value shape for extensibility).
13. `COMMIT`, close, `fsync` file, atomic `rename` over `cache.db`. On Linux, readers holding the old `cache.db` fd continue working against the unlinked inode — safe.
14. If `.taskgolem/.gitignore` is missing, call `ensure_gitignore()` as a best-effort side effect and print a one-line stderr notice.
15. Reopen in read-only mode for the query.

**Concurrent access model:**
- Two `tg query` processes both seeing stale stamp → both rebuild, both rename. Last rename wins; the other's work is wasted but harmless. `cache.db.tmp-<pid>` uniqueness prevents collision.
- `tg query` rebuild racing against `tg add` write → the lock in step 1 serializes the read; the write can proceed after rebuild completes. No torn reads possible.
- Windows: cross-platform atomic rename with an open read handle on the target is not guaranteed. Task-golem is Linux/macOS primary; document Windows as best-effort for now.

### Key Flows

#### Flow: `tg query` on a fresh checkout

> User clones a task-golem project, runs `tg query "SELECT count(*) FROM tasks"`.

1. **Resolve store.** Find `.taskgolem/` by walking parent directories.
2. **Open cache for query.** `cache.db` doesn't exist → trigger rebuild.
3. **Rebuild from JSONL.** Read active tasks, build temp, rename.
4. **Run query.** Open readonly + sandbox, prepare, execute, emit.

**Edge cases:**
- **`.taskgolem/` read-only filesystem** → fall back to `:memory:` SQLite for this invocation; print notice under `--verbose`; don't persist.
- **JSONL malformed (corrupt merge)** → strict parse fails with `StorageCorruption`; cache not updated; error points at offending line.
- **Duplicate IDs** → rebuild aborts with `InvalidInput`; points at duplicate IDs.

#### Flow: Query immediately after write

> User runs `tg add "foo"`, then `tg query "SELECT * FROM tasks"`.

1. `tg add` wrote JSONL; cache stamp is now stale (mtime+size differ).
2. `tg query` → stamp mismatch → rebuild → query.

**Tradeoff:** Query after write has rebuild latency. Measured target: <100ms for 500 tasks, <500ms for 5k tasks. If this becomes objectionable, add eager rebuild as an optimization later (see Alternatives).

#### Flow: Recursive descendants query

> `tg query "WITH RECURSIVE sub(id, depth) AS (SELECT id, 0 FROM tasks WHERE id = 'ABC' UNION ALL SELECT t.id, sub.depth+1 FROM tasks t JOIN sub ON t.parent = sub.id WHERE sub.depth < 64) SELECT * FROM sub"`.

1. Stamp check → fresh (user just queried).
2. Open readonly + sandbox; progress_handler set for 5s.
3. Prepare statement. **Sanity check:** SQL starts with `SELECT` or `WITH` — accepted.
4. Authorizer at prepare time approves `SELECT`, `READ`, `RECURSIVE`; denies nothing applicable.
5. Execute with bounded `depth < 64`; safe even if a cycle sneaks past write-time check.

**Edge case:** Runaway query hits 5s budget → `SQLITE_INTERRUPT` → return user error "Query timeout (5s). Use --timeout N to extend."

#### Flow: Sandbox attempts to ATTACH

> Buggy or confused query: `tg query "ATTACH DATABASE '/tmp/evil.db' AS x"`.

1. `prepare_v2` runs the authorizer, which returns `Authorization::Deny` for action code `SQLITE_ATTACH`.
2. Statement fails at prepare with `TgError::QueryDenied { action: "ATTACH", hint: "tg query is SELECT-only" }`.

**Edge cases:**
- Nested/obfuscated attacks (`WITH x AS (ATTACH …)`) are caught because the authorizer fires at prepare time on every AST node.
- Function-call denials (`SELECT load_extension(...)`) handled by a second authorizer rule: deny `SQLITE_FUNCTION` when name matches any of `{load_extension, sqlite_version_compile_options, readfile, writefile, edit, fts3_tokenizer}`.
- Pragma allowlist (see Sandbox section): only `table_info`, `index_list`, `index_info` allowed. All other pragmas denied. Prevents `PRAGMA writable_schema=ON` and similar escape attempts.

#### Flow: `tg doctor --fix`

> User notices weird state; runs doctor.

1. Existing checks run (JSONL syntax, dup IDs, dep cycles, dangling deps).
2. **New parent checks:** parent cycles, dangling parent refs.
3. **New cache consistency check:** full rebuild into a `.tmp` file, byte-count compare against existing `cache.db`, report drift (usually stamp-mismatch indicates cache stale — benign — but different schema_version or row count is a bug).
4. **New gitignore check:** if `.taskgolem/.gitignore` missing or doesn't include `cache.db`, warn (and add if `--fix`).
5. Offer repairs with timestamped backups (existing pattern).

---

## Technical Decisions

### Key Decisions

#### Decision: Lazy cache rebuild (on query entry), not eager (after each write)

**Context:** PRD describes both a write-time ordering ("JSONL, then cache, then stamp") and a query-time stamp check. Both aren't necessary for correctness.

**Decision:** Rebuild is **triggered only at `tg query` entry**, when the stamp disagrees. Write commands do not touch the cache.

**Rationale:**
- JSONL stays the **only** cross-file invariant. No two-file transaction to reason about or fail.
- Crash during a write → cache stays stale → next query rebuilds. No data loss, no cleanup needed.
- Writes don't pay any extra cost. This is especially valuable for agent bursts (5-10 adds in a row today cost 5-10 cache rebuilds under eager; zero under lazy).
- Query latency trades off against this: first query after a write pays the rebuild cost. At target scale (<5k tasks), <500ms. Measured at initial release; optimize if users complain.

**Consequences:**
- `Cache::rebuild_from_jsonl` is the only rebuild path — simpler mental model.
- `cache.db` can lag JSONL arbitrarily until the next query. Fine — it's pure derived state.
- Doctor's cache consistency check is the catch-all if rebuild ever misbehaves.

#### Decision: DELETE journal mode (rollback), not WAL

**Context:** PRD mentions WAL as a possibility.

**Decision:** Use `PRAGMA journal_mode=DELETE` (default rollback) for runtime queries. During rebuild, use `PRAGMA journal_mode=MEMORY; PRAGMA synchronous=OFF;` on the temp file (safe under atomic rename).

**Rationale:**
- WAL's benefits (concurrent read+write, faster incremental writes) don't apply to a single-user CLI with full-rebuild semantics.
- WAL's costs are real: `-wal` and `-shm` sidecar files that may be accidentally committed, clutter `.gitignore`, confuse users.
- Atomic-rename rebuild works identically under either mode; we gain nothing from WAL.

**Consequences:**
- One gitignore line (`cache.db` — journal file is ephemeral during connections and auto-cleaned).
- Cleaner mental model: single-file cache.
- Can switch to WAL later if we add incremental cache updates.

#### Decision: Normalized tables + materialized `task_view`

**Context:** `task_view` is the agent-friendly denormalized view with `is_ready`, `depth_from_root`, `unmet_dep_count`. It could be a SQL VIEW or a materialized table.

**Decision:** Normalized base tables (`tasks`, `task_tags`, `task_deps`) plus a materialized `task_view` **table**, populated in the same transaction as the base tables during rebuild.

**Rationale:**
- Rebuild is from scratch every time, so materializing the view adds negligible cost.
- Query time is where it pays off: `depth_from_root` and `is_ready` are non-trivial to compute per query.
- Agents can write single-statement queries against `task_view` without manual joins.
- Base tables still exist for advanced cases (join `task_tags`, recursive CTE over `tasks.parent`).

**Consequences:**
- Schema has one extra "table" to populate on rebuild.
- Slightly larger `cache.db` file.
- Debug-friendly: `SELECT * FROM task_view LIMIT 5` shows the agent-facing contract at a glance.

#### Decision: `xxhash-rust` with `xxh3` feature for content hashing

**Context:** PRD says "any fast non-cryptographic 64-bit hash."

**Decision:** `xxhash-rust = { version = "0.8", features = ["xxh3"] }`; compute `xxh3_64(&jsonl_bytes)` one-shot.

**Rationale:**
- Pure Rust; no C deps; SIMD-accelerated; GB/s throughput.
- Stable across runs (unseeded — unlike `ahash` / `foldhash`, which are unusable for stamps).
- Zero dep footprint; doesn't threaten the single-binary story.

**Consequences:** 100KB-5MB JSONL hashes in sub-ms.

#### Decision: Three-layer SELECT-only sandbox with allowlist authorizer

**Context:** `tg query` accepts arbitrary SQL from agents and humans. Needs to be safe.

**Decision:** Three layers:
1. Open with `SQLITE_OPEN_READ_ONLY` (OS-level — blocks writes at the file level).
2. `PRAGMA query_only=ON; PRAGMA trusted_schema=OFF; PRAGMA defensive=ON;` (engine-level).
3. **Allowlist authorizer** (default-deny — safer against future SQLite action codes):
   - Allow `SQLITE_SELECT`, `SQLITE_READ`, `SQLITE_RECURSIVE`.
   - Allow `SQLITE_FUNCTION` **except** when function name ∈ `{load_extension, readfile, writefile, edit, fts3_tokenizer}` (name denylist applied within the function category).
   - Allow `SQLITE_PRAGMA` **only** for pragma names in `{table_info, index_list, index_info}`.
   - **Deny everything else by default.** This includes `ATTACH`, `DETACH`, `CREATE_*`, `DROP_*`, `ALTER_*`, `INSERT`, `UPDATE`, `DELETE`, `TRANSACTION`, `SAVEPOINT`, `REINDEX`, `ANALYZE`, and any future action codes added by upstream SQLite.

**Rationale:** SELECT is a narrow operation — ~5 allow rules cover it. Default-deny means upstream additions to SQLite's action code set stay blocked automatically, with no audit required on `rusqlite` version bumps. Same code complexity as denylist, strictly safer.

**Consequences:** Each allow category gets dedicated integration tests. If agents discover they need another read-only pragma or function, expand the allowlist with a conscious change. `rusqlite` version pin (`"=0.32"`) is retained for build reproducibility but is no longer load-bearing for security — the allowlist handles that.

**Rusqlite features required:** `["bundled", "hooks", "limits"]`. `hooks` provides the authorizer API; `progress_handler` is in default features.

#### Decision: Active-only cache for v1

**Context:** Archive has lenient parsing; cache rebuild needs a consistent policy. Including archived tasks adds query surface but expands cost and risk.

**Decision:** Only active tasks (`.taskgolem/tasks.jsonl`) are in the cache for v1. Archived tasks (`.taskgolem/archive.jsonl`) are not queryable via `tg query` yet.

**Rationale:**
- Archive parse is lenient today (skips bad lines). Strict-parse in cache would change that behavior; lenient-parse in cache would drop rows silently.
- Primary use cases (agents navigating "what's next", humans browsing live work) are active-only.
- Reduces rebuild cost and complexity.
- Easy to add archive in a follow-up if demand appears.

**Consequences:** `tg query "SELECT * FROM tasks"` returns active only. Document clearly; schema comment reinforces. Archive-query is a candidate for v2.

#### Decision: Statement timeout via `progress_handler`, default 5s

**Context:** Need to prevent runaway queries (especially recursive CTEs without depth bounds).

**Decision:** Register `progress_handler(1000, |now > deadline|)` per query, deadline = `now + Duration::from_secs(timeout_secs)`. Default 5s. `--timeout N` accepts any positive integer; no upper cap (user-explicit override is trusted).

**Rationale:** Only in-process mechanism SQLite offers. 1000 VM-ops granularity is sub-ms wall-clock — imperceptible overhead. No cap on `--timeout` because the user is deliberately overriding; capping would force them to edit code for legitimate long queries.

**Consequences:** Timed-out query returns nonzero exit with clear error ("Query exceeded timeout of Ns. Use --timeout N to extend."). Per-query handler replaces previous; no leakage.

### Tradeoffs Accepted

| Tradeoff | We're Accepting | In Exchange For | Why This Makes Sense |
|---|---|---|---|
| Lazy rebuild latency | First query after write pays rebuild cost (target <500ms at 5k tasks) | Zero cross-file invariants, simple crash model, no write amplification | At solo-dev scale, 500ms is imperceptible; simpler system is worth more than query microperf |
| Active-only cache | `tg query` doesn't see archived tasks in v1 | Consistent strict-parse rebuild; smaller cache | Most queries target live work; archive queries are a clear v2 feature |
| Bundled SQLite | ~1MB binary size + 5-10s compile time | No runtime deps; true single-binary install | Matches project value of "single binary" |
| DELETE journal (not WAL) | Slightly slower incremental writes (which we don't do) | Single-file cache; no `-wal`/`-shm` sidecar file gitignore concerns | Future WAL migration is possible if workload changes |
| Gitignore migration step for existing users | One-time manual update | Avoids committing binary churn | Clearly documented; `tg doctor` detects |

---

## Alternatives Considered

### Alternative: Eager rebuild after each write

**Summary:** Writes update JSONL → update cache → update stamp within the same `store.with_lock()` callback. Query path trusts the cache (stamp check is just a safety net).

**How it would work:**
- Each write-command's lock callback ends with `cache::apply_delta(&tasks)` or `cache::rebuild_from_jsonl()`.
- Stamp-check on query still runs (safety net); mismatch triggers rebuild.
- Query latency becomes constant — no rebuild on first query.

**Pros:**
- First query after write is fast.
- Stamp mismatch is "should never happen" — when it does, it's a real bug signal.

**Cons:**
- Cross-file invariant (JSONL + cache must both succeed or both roll back). Rollback is ugly because JSONL has already been atomically renamed; reversing means another full write.
- Write amplification: each of 10 rapid `tg add` calls triggers a full cache rebuild.
- Partial-failure handling is complex: what if JSONL write succeeds and cache update fails? Log? Retry? Exit non-zero after a successful write?

**Why not chosen:** Complexity isn't justified at our scale. Lazy rebuild is correct-by-construction, trivial to reason about, and matches the PRD's explicit "JSONL is source of truth, cache is disposable" value. **Can be added later as a pure optimization** — flag `TG_CACHE_EAGER=1` or similar — if first-query latency becomes a concrete pain point.

### Alternative: Single flat `tasks` table with tags/deps as JSON columns

**Summary:** One big table with `tags TEXT` (comma-separated or JSON array) and `deps TEXT` (JSON array) columns. No separate join tables.

**How it would work:**
- Schema: `CREATE TABLE tasks(id, title, …, tags_json, deps_json, parent)`.
- Queries using `json_each(tags_json)` for tag filters, CTE for dep traversal.

**Pros:**
- Simpler schema; one insert per task during rebuild.
- Smaller cache file.
- Fewer moving parts.

**Cons:**
- Harder to write agent-friendly queries: `WHERE 'urgent' IN (SELECT value FROM json_each(tags_json))` vs `JOIN task_tags ON …`.
- Can't index tags for filter performance.
- `json_each` is SQLite-specific, makes SQL look unusual.

**Why not chosen:** The whole point of the query layer is agent DX. Normalized schema produces queries that look and behave like standard SQL, matching what agents are already fluent in.

### Alternative: Multi-parent / DAG parent relationship

**Summary:** Let `parent` be a list (`Vec<String>` instead of `Option<String>`), allowing a task to belong to multiple epics.

**Pros:**
- Natural for cross-cutting concerns (e.g., "OAuth refactor" epic + "Q2 infrastructure" epic both include the same task).

**Cons:**
- Two representations for the same concept (parent-as-list vs parent-as-optional-single) split the validation surface.
- Tree visualizations have to pick a "primary" parent anyway.
- No user demand signalled in the PRD.

**Why not chosen:** PRD specifies single parent; multi-parent can be added via a tag convention (`x-epic: foo`) or a future change without breaking the schema we ship.

### Alternative: DuckDB instead of SQLite

**Summary:** Use DuckDB as the query engine — also single-binary, optimized for analytic/recursive workloads.

**Pros:**
- Columnar execution is faster for aggregations.
- Better recursive CTE support in modern versions.

**Cons:**
- Younger ecosystem; fewer Rust-binding options (`duckdb` crate exists but less mature than `rusqlite`).
- Larger bundled binary size.
- Overkill for <5k-row datasets; advantage appears at millions of rows.

**Why not chosen:** `rusqlite` is the mature, ubiquitous choice at our scale. DuckDB would be right for analytics workloads in task-golem-for-teams-with-10k-projects, not solo-dev.

---

## Risks

| Risk | Impact | Likelihood | Mitigation |
|---|---|---|---|
| Rebuild latency objectionable on hot query path at larger scales | Query UX degrades | Low at v1 scale; Med at 10k+ | Measure; add eager-mode flag if needed. Archival pruning deferred to future change. |
| Concurrent `tg query` + `tg add` / two concurrent queries racing on rebuild | Wasted work; stamp drift; torn JSONL read | Low | Rebuild acquires `store.with_lock()` during JSONL read phase; pid-suffixed temp files avoid rename collision; atomic rename + POSIX-unlink-while-open semantics keep readers safe |
| Partial-write JSONL recovery (crash between truncate and rename, fsck repair) | Strict-parse aborts every rebuild until fixed | Low | `jsonl::write_atomic` is already fsync+rename so partial states shouldn't persist; `tg doctor --fix` can truncate trailing garbage; error messages point user at doctor |
| Sandbox bypass via SQLite feature we overlooked | Data exfiltration or filesystem access via `tg query` | Low (three-layer defense, pinned SQLite version) | Integration tests per denied action code; `rusqlite` version pin is audited on bump; see Needs Attention for allowlist alternative |
| `.taskgolem/.gitignore` not created for existing projects → `cache.db` churn in git | Noisy diffs, large repo growth | Medium | `tg init` creates it; `cache::rebuild` best-effort creates it + stderr notice; `tg doctor` detects and repairs — three independent guards |
| Duplicate-ID detection too strict, breaks workflows with historical duplicates | Users blocked | Low | Error message points at offending IDs and suggests `tg doctor --fix`; error surfaces consistently on both write and rebuild paths |
| Progress handler interferes with long legitimate queries | Agent queries time out | Low | Default 5s is generous for <5k tasks; `--timeout N` has no upper cap |
| `rusqlite` bundled SQLite adds compile friction for contributors | Slower dev loop | Medium | Accepted cost; cached in `target/`; `just check` sequence unchanged |
| Manual JSONL edit introduces parent cycle → rebuild CTE loops | Cache rebuild hangs until progress_handler fires (but handler isn't active during rebuild) | Low | Rust-side `detect_all_parent_cycles` runs before SQL INSERT; CTE also carries `WHERE depth < 64` as belt-and-suspenders |
| Bundled SQLite version bump introduces new action code that defaults to allowed | Silent sandbox gap | Low | Pin `rusqlite = "=0.32"`; audit action code set on any version change |

---

## Integration Points

### Existing Code Touchpoints

- `src/model/item.rs:41-70` — Add `parent: Option<String>` field; update `KNOWN_FIELD_NAMES` (line 13-27); reuse `serialize_option_nullable` (line 30-39).
- `src/model/deps.rs` — Add `would_create_parent_cycle`, `detect_all_parent_cycles`, `validate_parent` (parallel to existing dep functions at line 7-63).
- `src/model/parent.rs` (new) — `reparent(items, id, new_parent)` single orchestration point combining validate + cycle-check + mutation.
- `src/store/mod.rs` — Add `tasks_jsonl_path()` accessor and `ensure_gitignore()` helper. No structural change to `with_lock`.
- `src/store/mod.rs` — No structural change; cache module borrows `&Store` for path resolution only.
- `src/store/jsonl.rs` — No change.
- `src/cli/args.rs` — Add `Commands::Query { sql, schema, json, timeout }` variant (no `--no-rebuild`); add `--parent` to `Add`, `Edit`, `List` variants.
- `src/cli/mod.rs` — Wire new dispatch handler.
- `src/cli/commands/init.rs:52-65` — Append write of `.taskgolem/.gitignore` (creates if missing).
- `src/cli/commands/edit.rs` — Accept `--parent`; call `validate_parent` + cycle check.
- `src/cli/commands/add.rs` — Accept `--parent`; same validation.
- `src/cli/commands/rm.rs` — Check for active children; reject if present. Archived children are not checked (doctor covers dangling archive refs).
- `src/cli/commands/archive.rs` — Mirror rm: block archiving a task with active children.
- `src/cli/commands/list.rs` — Add `--parent <id>` filter.
- `src/cli/commands/show.rs` — Render "Children:" section.
- `src/cli/commands/doctor.rs:37-168` — Add parent cycle / dangling / gitignore / cache-consistency checks.
- `src/errors.rs:7-52` — Add variants (or reuse `InvalidInput`, `StorageCorruption`) for parent-validation and cache errors.
- `Cargo.toml` — Add `rusqlite = { version = "0.32", features = ["bundled", "hooks", "limits"] }` and `xxhash-rust = { version = "0.8", features = ["xxh3"] }`.

### New Files

- `src/cache/mod.rs` — public API, stamp logic, DDL constants
- `src/cache/rebuild.rs` — rebuild orchestration
- `src/cache/query.rs` — sandbox + execution
- `src/cli/commands/query.rs` — CLI handler
- `src/model/parent.rs` — reparent orchestration
- `tests/query_test.rs` — SELECT queries, JSON output, timeouts
- `tests/parent_test.rs` — reparent validation, cycle rejection
- `tests/cache_test.rs` — stamp staleness, rebuild correctness, concurrent access
- `tests/sandbox_test.rs` — per-denied-action-code integration tests

### External Dependencies

- `rusqlite = "=0.32"` (with `bundled`, `hooks`, `limits`) — pinned exact to pin bundled SQLite version, which pins the authorizer action code surface. Upgrades require explicit review of action codes.
- `xxhash-rust = "0.8"` (with `xxh3`) — hard dep; alternative `twox-hash` is slower and not worth a feature flag.

### Error Handling

New `TgError` variants (in `src/errors.rs`):

| Variant | Exit | Meaning |
|---|---|---|
| `ParentSelfReference { id }` | 1 | `parent == id` rejected on write |
| `ParentCycle { ids }` | 1 | Parent-DAG cycle detected on write or rebuild |
| `ParentDangling { id, parent }` | 1 | Parent ID does not exist in active tasks |
| `ParentHasChildren { id, children }` | 1 | `rm`/`archive` blocked because children exist |
| `CacheCorrupt { detail }` | 2 | `PRAGMA quick_check` failed or DB unopenable |
| `CacheRebuildFailed { source }` | 2 | JSONL read or SQL insert during rebuild failed |
| `CacheSchemaVersionMismatch { stored, expected }` | 2 | Cache DB built by a different binary version — triggers rebuild, not a user error, surfaced only under `--verbose` |
| `QueryTimeout { limit_secs }` | 1 | `progress_handler` interrupted statement |
| `QueryDenied { action, hint }` | 1 | Authorizer denied the statement (`SELECT load_extension`, `ATTACH`, etc.) |
| `QuerySyntax { source }` | 1 | `rusqlite::Error::SqliteFailure` on prepare |

`StorageCorruption` (existing) is reused for malformed JSONL during rebuild (with duplicate-ID detection pointing at offending IDs). `InvalidInput` (existing) covers malformed `--timeout` / `--parent` arguments.

### Interface Contracts

**`Store::tasks_jsonl_path(&self) -> &Path`** — new accessor; needed because `cache/` resolves paths directly rather than coupling to `Store` internals. Added to `src/store/mod.rs`.

**`Store::ensure_gitignore() -> Result<(), TgError>`** — new helper. Creates `.taskgolem/.gitignore` if missing; appends `cache.db`, `cache.db-journal`, `cache.db.tmp-*` lines if not already present. Idempotent. Called from `init`, `doctor --fix`, and (best-effort) from `cache::rebuild` on a project that's missing it.

**`QueryResult`** — returned by `cache::query`:

```rust
pub struct QueryResult {
    pub columns: Vec<String>,
    pub column_types: Vec<SqlType>,
    pub rows: Vec<Vec<SqlValue>>,
}

pub enum SqlType { Integer, Real, Text, Null, Blob }
pub enum SqlValue { Null, Integer(i64), Real(f64), Text(String), Blob(Vec<u8>) }
```

**JSON serialization rules for `--json`:**
- `Null` → JSON `null`
- `Integer(n)` → JSON number (safe up to 2^53; larger values emit as JSON string with a one-line stderr warning)
- `Real(f)` → JSON number; `NaN` / `±Inf` → JSON `null` with stderr warning (not expected in our schema)
- `Text(s)` → JSON string
- `Blob(b)` → `TgError::InvalidInput` (our schema has no BLOB columns; if one appears, it's a bug)

---

## Cache Schema (Reference)

```sql
-- Meta
CREATE TABLE _cache_meta (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL
);
-- Rows: schema_version=1, jsonl_mtime_nanos=..., jsonl_size=..., jsonl_xxh3=...

-- Base: one row per active task
CREATE TABLE tasks (
  id TEXT PRIMARY KEY,
  title TEXT NOT NULL,
  status TEXT NOT NULL,            -- 'todo'|'doing'|'done'|'blocked'
  priority INTEGER NOT NULL,
  description TEXT,
  parent TEXT,                     -- FK to tasks.id (not enforced; we validate on write)
  created_at TEXT NOT NULL,        -- ISO-8601
  updated_at TEXT NOT NULL,
  blocked_reason TEXT,
  blocked_from_status TEXT,
  claimed_by TEXT,
  claimed_at TEXT
);
CREATE INDEX idx_tasks_status ON tasks(status);
CREATE INDEX idx_tasks_parent ON tasks(parent);
CREATE INDEX idx_tasks_priority ON tasks(priority);

-- One row per (task, tag)
CREATE TABLE task_tags (
  task_id TEXT NOT NULL,
  tag TEXT NOT NULL,
  PRIMARY KEY (task_id, tag)
);
CREATE INDEX idx_tags_tag ON task_tags(tag);

-- One row per (task, dependency)
CREATE TABLE task_deps (
  task_id TEXT NOT NULL,
  dep_id TEXT NOT NULL,
  PRIMARY KEY (task_id, dep_id)
);
CREATE INDEX idx_deps_dep_id ON task_deps(dep_id);

-- Materialized agent-facing view (populated during rebuild).
-- Contains ACTIVE tasks only in v1; archived tasks are not in the cache.
-- Column set matches the PRD Should-Have list exactly (no child_count — agents
-- can compute via `SELECT parent, COUNT(*) FROM tasks GROUP BY parent` when needed).
CREATE TABLE task_view (
  id TEXT PRIMARY KEY,
  title TEXT NOT NULL,
  status TEXT NOT NULL,
  priority INTEGER NOT NULL,
  parent TEXT,
  depth_from_root INTEGER NOT NULL,    -- 0 = root. Computed with WHERE depth < 64.
  is_ready INTEGER NOT NULL,           -- 1 iff status='todo' AND all deps done
  unmet_dep_count INTEGER NOT NULL
);
CREATE INDEX idx_view_status ON task_view(status);
CREATE INDEX idx_view_parent ON task_view(parent);
CREATE INDEX idx_view_ready ON task_view(is_ready);
```

The 5 canonical queries (skill update) use `task_view` almost exclusively. `depth_from_root` being materialized means agents rarely need recursive CTEs themselves — a major DX win. Tree-descendants-of-X queries still use a recursive CTE over `tasks.parent`, always bounded `WHERE depth < 64`.

**`tg query --schema` output** includes: each table's DDL, a "task_view columns" summary with types + one-line column descriptions, a note that v1 cache contains **active tasks only**, and a reminder to bound recursive CTEs with `depth < 64`.

---

## Open Questions

Resolved during self-critique (applied above):
- ~~Exact `rusqlite` version~~ → pinned at `=0.32`.
- ~~`--no-rebuild` flag~~ → dropped (correct behavior is always rebuild on stale).
- ~~`task_view.child_count`~~ → dropped (matches PRD Should-Have exactly; agents can GROUP BY).
- ~~Prefix check in sandbox~~ → dropped (redundant with authorizer).
- ~~Doctor byte-count cache compare~~ → replaced with schema_version + row-count compare.
- ~~`TgError` variant plan~~ → enumerated under Error Handling.
- ~~`QueryResult` contract~~ → typed; JSON rules specified.

Remaining minor questions for spec phase:
- [ ] **Default tabular output format.** Aligned columns (current `tg list` style) vs. one-record-per-line. Leaning: aligned.
- [ ] **Schema-discovery format.** `tg query --schema` output shape — Markdown table with types + column descriptions (recommended, matches what agents read). Finalize during spec.
- [ ] **Rebuild progress indicator.** At 5k tasks rebuild is ~500ms. Silent by default; `--verbose` prints "rebuilding cache…" + timing. Confirm in spec.

---

## Needs Attention

None — all four directional items surfaced during self-critique were resolved in discussion with the change author on 2026-04-14:

1. **PRD write-order ambiguity** → Resolved: lazy rebuild is intended. PRD Must-Have amended to reflect lazy model.
2. **Sandbox stance** → Resolved: allowlist authorizer (default-deny) replaces denylist. Same code complexity, safer against upstream changes.
3. **Materialized `task_view` vs SQL VIEW** → Resolved: stay fully materialized (current design). Uniformity wins; easy to flip to hybrid later if drift becomes a problem.
4. **Archive queryability scope** → Resolved: active-only v1. PRD Out-of-Scope amended.

---

## Design Review Checklist

- [x] Design addresses all PRD must-have requirements
- [x] Key flows are documented and make sense
- [x] Tradeoffs are explicitly documented and acceptable
- [x] Integration points with existing code are identified
- [x] No major open questions remain (open questions are minor/deferred)

---

## Design Log

| Date | Activity | Outcome |
|---|---|---|
| 2026-04-14 | Initial design draft (medium mode) | Lazy rebuild, DELETE journal, normalized+materialized schema, layered sandbox |
| 2026-04-14 | Evaluated eager-rebuild alternative | Deferred to future optimization flag if latency becomes a concern |
| 2026-04-14 | Evaluated single-flat-table schema | Rejected — normalized schema is better agent DX |
| 2026-04-14 | Self-critique (7 parallel critic agents) | 0 Critical, ~8 High, ~15 Medium; auto-fixes applied |
| 2026-04-14 | Auto-fixes applied | Cache module collapsed to 3 files; `reparent()` orchestration added; rebuild acquires advisory lock during JSONL read; `QueryResult` + `TgError` variants enumerated; `--no-rebuild` dropped; `child_count` dropped; sandbox prefix check dropped; doctor byte-count compare replaced with schema_version + row-count; `rusqlite` pinned; stamp short-circuit clarified; archive interaction documented; multi-parent and DuckDB alternatives noted |
| 2026-04-14 | Directional items surfaced | PRD write-order amendment; denylist vs allowlist sandbox; materialized vs VIEW task_view; archive scope |
| 2026-04-14 | Directional items resolved with author | PRD amended (lazy rebuild + archive out-of-scope); sandbox flipped to allowlist; task_view stays fully materialized |
