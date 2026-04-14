# Tech Research: Query Layer and Nested Tasks

**ID:** TG-006
**Status:** Complete
**Created:** 2026-04-14
**PRD:** ./TG-006_query-layer-and-nested-tasks_PRD.md
**Mode:** Medium

## TL;DR

- **Researched:** SELECT-only SQLite sandboxing in rusqlite, JSONL→SQLite cache rebuild patterns, fast non-cryptographic 64-bit hashing, recursive CTE safety, cache-invalidation stamp design, statement timeouts, and existing task-golem internals (item/store/deps/cli/doctor/tests).
- **Key finding:** The PRD's broad strokes are well-aligned with the Rust/SQLite ecosystem's canonical patterns. The open technical decisions resolve cleanly — **rollback journal (not WAL)**, **xxh3 via `xxhash-rust`**, **composite stamp with short-circuit**, and **layered sandbox (OPEN_READONLY + authorizer + pragmas)**.
- **Recommended approach:** Add `parent: Option<String>` using existing `serialize_option_nullable` pattern; build a `cache` module (stamp + rebuild + query); rebuild-from-JSONL on stale stamp inside `tg query`; write commands keep their current atomic-rename JSONL flow and trigger rebuild afterward; SELECT-only enforced via `Authorization::Deny` on `SQLITE_ATTACH/DETACH/PRAGMA/FUNCTION(load_extension)` plus `query_only=ON` and `SQLITE_OPEN_READ_ONLY`.
- **Concerns:** `.taskgolem/` is NOT currently gitignored (it's committed); adding the cache requires an `.taskgolem/.gitignore` on `tg init` and a documented migration for existing users. Strict-vs-lenient JSONL parsing (archive is lenient, active is strict) needs a consistent policy for cache rebuild. Write amplification per write is real but measurable.
- **Needs attention:** Cache rebuild timing (eager-after-write vs lazy-on-query); whether archived tasks are in the cache; whether rebuild failure rolls back the JSONL write or just logs.

## Overview

This research answers the concrete implementation questions left open by the PRD: what Rust crates to use, what SQLite pragmas/flags enforce a safe read-only surface, how to make cache rebuild fast and crash-safe, what the stamp should actually compare, and where in the existing task-golem code each piece plugs in. It also surfaces three latent decisions that the PRD implies but doesn't explicitly resolve (cache rebuild timing, archive inclusion, rebuild-failure policy) — flagged under "Open Questions" for the design phase.

## Research Questions

- [x] What's the minimum viable safe SELECT-only SQLite sandbox in `rusqlite`?
- [x] Which fast non-cryptographic 64-bit hash crate fits a static-linked Rust binary?
- [x] WAL vs rollback journal for single-writer CLI + full-rebuild workload?
- [x] How do we enforce a statement timeout in `rusqlite`?
- [x] What are safe patterns for recursive CTEs over a `parent` column?
- [x] What's the right composite stamp, and what known mtime pitfalls should it avoid?
- [x] Where in the existing code does each new piece fit?

---

## External Research

### Landscape Overview

JSONL-as-source-of-truth with a derived SQLite cache is a well-trodden pattern in Rust CLIs. The closest prior art:

- **`ripgrep-all`** caches extracted document text in a SQLite sidecar for fast re-query; rebuilds from canonical files when stale.
- **`fossil-scm`** uses SQLite as primary store with bundled-export artifacts; demonstrates atomic-rename rebuild patterns.
- **`notmuch`** derives an index (Xapian, but structurally identical) from a canonical mail directory.

Shared lessons: treat the cache as fully disposable, invalidate aggressively, keep rebuild cheap. All three tolerate rebuild-from-scratch at our target scale (thousands of records).

For read-only SQL sandboxing, SQLite ships four overlapping defenses: `SQLITE_OPEN_READONLY`, `PRAGMA query_only`, `sqlite3_set_authorizer`, and hardening pragmas (`trusted_schema=OFF`, `defensive=ON`, `SQLITE_DBCONFIG_ENABLE_LOAD_EXTENSION=0`). Defense in depth is cheap — combine them all. `rusqlite` exposes all four via the `hooks` feature.

The Rust ecosystem has converged: `xxhash-rust` with feature `xxh3` is the default fast 64-bit hash (pure Rust, zero C deps, SIMD-accelerated, GB/s). `progress_handler` is the only in-process statement-timeout mechanism SQLite offers.

### Common Patterns & Approaches

#### Pattern: Atomic-Rename Cache Rebuild

**How it works:** Build cache in `.taskgolem/cache.db.tmp-<pid>`; `fsync`; atomic `rename` to `cache.db`. Inside the rebuild, wrap inserts in a single `BEGIN IMMEDIATE; ... COMMIT;` transaction with `PRAGMA synchronous=OFF` and `PRAGMA journal_mode=MEMORY` (safe because we atomic-rename). After rename, set runtime pragmas and run `PRAGMA optimize`.

**When to use:** Bulk-load or full-rebuild workloads; makes rebuild idempotent and crash-safe.

**Tradeoffs:**
- Pro: Crash during rebuild never corrupts the live cache.
- Pro: ~100× speedup over per-row fsync.
- Con: Brief double-disk-usage during rename.

**References:**
- https://github.com/phiresky/ripgrep-all
- https://phiresky.github.io/blog/2020/sqlite-performance-tuning/
- https://cj.rs/blog/sqlite-pragma-cheatsheet-for-performance-and-consistency/

#### Pattern: Layered SELECT-Only Sandbox

**How it works:** Open connection with `OpenFlags::SQLITE_OPEN_READ_ONLY`. Register `Connection::authorizer` that returns `Authorization::Deny` for `SQLITE_ATTACH`, `SQLITE_DETACH`, `SQLITE_PRAGMA` (except a read-only whitelist), and `SQLITE_FUNCTION` when the function name is `load_extension`. Set `PRAGMA query_only=ON`, `PRAGMA trusted_schema=OFF`, `PRAGMA defensive=ON` after open. Sanity-check the SQL starts with `SELECT` (cheap defense against parser quirks) or verify `sqlite3_stmt_readonly()` after prepare.

**When to use:** Exposing arbitrary SQL to untrusted callers (agents, users pasting queries). Authorizer fires at prepare time, so denied statements never execute.

**Tradeoffs:**
- Pro: Defense in depth; each layer closes a known escape.
- Pro: Cheap — no runtime cost on happy-path queries.
- Con: Some initial wiring; must test every denied code path.

**References:**
- https://sqlite.org/c3ref/set_authorizer.html
- https://sqlite.org/security.html
- https://sqlite.work/restricting-sqlite-plugins-to-select-statements-safely-and-effectively/

#### Pattern: Composite Stamp with Short-Circuit

**How it works:** Cache-meta stores `(jsonl_mtime_nanos, jsonl_size, xxh3_64)`. On query entry, `stat(2)` the JSONL — if mtime or size differ, invalidate immediately (no hash). Only hash if both match but we want to confirm (belt-and-suspenders). In practice: `stat` always runs (microseconds); hash runs only when mtime+size match but content_hash is also stored as a regression check, recomputed on rebuild.

**When to use:** Any file-as-source-of-truth with a derived artifact. Mtime alone fails on `git checkout`, `cp -p`, coarse-resolution filesystems, clock skew. Hash alone is slow on hot paths.

**Tradeoffs:**
- Pro: Hot-path is a single `stat(2)` + three-int compare.
- Pro: Cold-path hashes the file once; xxh3 is fast enough to be negligible at our scale.
- Con: Three fields to keep in sync. Small.

**References:**
- https://github.com/moby/moby/issues/9391
- https://github.com/python/mypy/issues/3403
- https://philipwalton.com/articles/cascading-cache-invalidation/

#### Pattern: Bounded Recursive CTE

**How it works:** Every recursive CTE carries a `depth` counter with `WHERE depth < 64`. Even though we cycle-check at write time, belt-and-suspenders prevents runaway queries if the write-time check ever fails.

**Why it matters:** SQLite does not implement SQL:2023 `CYCLE` clause. Without a bound, a cycle in the data (bug or manual JSONL edit) spins until the 5s progress_handler fires.

**References:**
- https://sqlite.org/lang_with.html
- https://sqlfordevs.com/cycle-detection-recursive-query
- https://modern-sql.com/caniuse/cycle_(recursion)

### Technologies & Tools

#### Crates

| Crate | Purpose | Pros | Cons | Verdict |
|---|---|---|---|---|
| `rusqlite` `["bundled","hooks","limits"]` | SQLite binding | Static-link, authorizer/progress_handler available, mature | Adds compile time; bundles ~1MB C | **Use** |
| `xxhash-rust` `["xxh3"]` | Content hash | Pure Rust, SIMD, GB/s, zero deps | None relevant | **Use** |
| `twox-hash` | Content hash | Zero deps | Older API; `xxhash-rust` benches faster | Skip |
| `ahash`, `foldhash` | General hashing | Fast | **Randomized seed** → different hash per process → cache always stale | **Do not use for stamps** |
| `fnv` | General hashing | Zero deps | 10-30× slower than xxh3 on >1KB | Skip |

**Recommendation:** `rusqlite = { version = "0.32", features = ["bundled", "hooks", "limits"] }` and `xxhash-rust = { version = "0.8", features = ["xxh3"] }`.

### Standards & Best Practices

- **Atomic rename for cache swaps.** Always write-temp + fsync + rename.
- **Single transaction for bulk inserts.** ~100× faster; one fsync instead of N.
- **`PRAGMA optimize` after major writes.** Updates statistics for the query planner.
- **Authorizer is the only SQL-level sandbox control.** `query_only` blocks DML but not `ATTACH`.
- **Never re-enable `load_extension`.** Off by default; authorizer belt-and-suspenders.
- **Bounded depth on all recursive CTEs** even with write-time cycle checks.

### Common Pitfalls

| Pitfall | Why It's a Problem | How to Avoid |
|---|---|---|
| WAL sidecar files (`-wal`, `-shm`) committed to git | Breaks reproducible checkouts; confusing diffs | Use rollback journal (DELETE mode). If WAL needed later, extend gitignore. |
| Seeded hashers (`ahash`/`foldhash`) used for stamps | Different hash every process run → cache always rebuilds | Use xxh3 (stable seed). |
| mtime-only cache check | Fails on `git checkout`, `cp -p`, 1s/2s resolution FSes, clock skew | Composite `(mtime, size, content_hash)`. |
| `PRAGMA query_only` without authorizer | Blocks DML but not `ATTACH DATABASE '/evil.db'` | Authorizer denies `SQLITE_ATTACH`. |
| Authorizer not covering `SQLITE_FUNCTION(load_extension)` | OPEN_READONLY doesn't block `SELECT load_extension(...)` if enabled elsewhere | Deny by function name. |
| Recursive CTE without depth cap | Any cycle in data (manual edit, merge artifact) → runs until timeout | `WHERE depth < 64`. |
| Per-row INSERT transactions on rebuild | 5000 fsyncs = seconds | One `BEGIN`/`COMMIT` wrapping the full rebuild. |
| Progress handler leaking between queries | Stale deadline affects next query | Register fresh per query; overwrite closes previous. |

### Key Learnings

- **The PRD's composite stamp is correct; just add short-circuit.** `stat` first, hash only if needed on the cold path.
- **Rollback journal is the right call for a single-user CLI** with full-rebuild semantics. WAL's advantages (concurrent read+write) don't apply; its sidecar files are a user-facing liability.
- **Sandboxing costs are one-time wiring**, not runtime overhead.
- **Bounded CTE depth is non-negotiable**, independent of write-time cycle checks.

---

## Internal Research

### Existing Codebase State

**Task model** (`src/model/item.rs:41-70`):
- `Item` struct with `id`, `title`, `status`, `priority`, `description`, `tags`, `dependencies`, timestamps, blocked/claimed metadata, flattened `extensions: BTreeMap<String, serde_json::Value>`.
- Custom `serialize_option_nullable` (line 30-39) emits explicit `null` for Option fields — critical for round-trip fidelity and git-friendly stable JSONL.
- `KNOWN_FIELD_NAMES` (line 13-27) enforces `x-*` extension namespace; must add `"parent"` when field is added.
- Schema header on line 1: `{"schema_version":1}`.

**Storage** (`src/store/`):
- `mod.rs:35-41` — `store.with_lock(|s| ...)` is the canonical pattern for writes.
- `jsonl.rs:127-159` — atomic write: temp → fsync → rename. All full-file rewrites go through this.
- `jsonl.rs:161-213` — archive append with crash-recovery truncated-line detection.
- `lock.rs` — advisory `fd_lock::RwLock` on `.tasks.lock` with 5s timeout + exponential backoff.
- Active reads are **strict** (fail on bad JSON); archive reads are **lenient** (skip + warn). Cache-rebuild policy needs to pick one.

**Dependency logic** (`src/model/deps.rs`):
- `would_create_cycle()` — DFS; directly parallelizable for `would_create_parent_cycle()`.
- `detect_all_cycles()` — Kahn's topological sort; same pattern for parent DAG.
- `validate_dep()` — rejects self-deps, warns on dangling refs.
- `compute_ready_queue()` — filter + dep-status check.

**CLI** (`src/cli/`):
- `args.rs:19-206` — all subcommands as `Commands` enum variants.
- `mod.rs:9-69` — dispatch.
- Commands split into read-only (list/show/ready/next/dump/doctor sans-fix/completions) and write (add/edit/rm/do/done/todo/block/unblock/dep/archive/doctor --fix/init).
- Every write wraps in `store.with_lock()`.

**Errors** (`src/errors.rs:7-72`):
- `TgError` enum; `exit_code() -> i32` maps user errors to 1, system errors to 2.
- `to_json()` for `--json` mode.

**Init** (`src/cli/commands/init.rs:52-65`):
- Creates `.taskgolem/`, `tasks.jsonl`, `archive.jsonl` with schema headers, empty `tasks.lock`.
- **Does not currently create `.taskgolem/.gitignore`** — `.taskgolem/` is committed to repo today.

**Doctor** (`src/cli/commands/doctor.rs:37-168`):
- Checks: JSONL syntax, duplicate IDs, items-in-both-files, invalid status, dep cycles, dangling deps.
- Repair mode creates timestamped backups.

**Tests** (`tests/`):
- `common/mod.rs` — `TestProject` helper: tempdir, `assert_cmd::cargo_bin!()`, JSON output parser.
- One integration test file per command.

**Build** (`justfile`):
- `just check` = `cargo fmt --check` + `cargo clippy -- -D warnings` + `cargo test`.

### Relevant Files

| File | Role | Integration Touch |
|---|---|---|
| `src/model/item.rs` | Task struct + serialization | Add `parent` field with `serialize_option_nullable`; add to `KNOWN_FIELD_NAMES` |
| `src/model/deps.rs` | Cycle detection | Add `would_create_parent_cycle`, `detect_all_parent_cycles` |
| `src/store/mod.rs` | Store entry points | Hook cache rebuild after `save_active()` completes |
| `src/store/jsonl.rs` | Atomic JSONL I/O | No change; cache layer reads JSONL separately |
| `src/store/lock.rs` | File lock | No change; query command does not take lock |
| `src/cli/args.rs` | Subcommand enum | Add `Commands::Query { sql, schema, json, timeout }` variant |
| `src/cli/mod.rs` | Dispatch | Wire new handler |
| `src/cli/commands/init.rs` | Init flow | Write `.taskgolem/.gitignore` |
| `src/cli/commands/doctor.rs` | Consistency checks | Add parent cycle + dangling parent + cache consistency checks |
| `src/cli/commands/edit.rs` | Task edit | Accept `--parent`; call parent cycle check |
| `src/cli/commands/list.rs` | Task list | Add `--parent <id>` filter |
| `src/cli/commands/show.rs` | Task show | Render "Children:" section |
| `src/cli/commands/rm.rs` | Task delete | Reject delete when children exist |
| `src/errors.rs` | Error enum | Add variants for parent-validation and cache errors (or reuse existing) |
| `Cargo.toml` | Deps | Add `rusqlite`, `xxhash-rust` |
| `tests/` | Integration tests | New `query_test.rs`, `parent_test.rs`, `cache_test.rs` |

### Existing Patterns to Follow

- **Option serialization:** `#[serde(serialize_with = "serialize_option_nullable")]` — mirror for `parent`.
- **Lock wrapping:** every write inside `store.with_lock(|s| ...)`.
- **Error propagation:** `TgError` with `From<anyhow::Error>` bridges; `exit_code` method.
- **Test harness:** `TestProject::new()` + `run_tg_json(&["..."])`.
- **JSON output mode:** subcommands take `--json` flag; wrap result in a deterministic envelope.

### Reusable Components

- `deps::would_create_cycle` DFS template → adapt for parent DAG.
- `deps::detect_all_cycles` Kahn's sort template → adapt for parent DAG.
- `jsonl::write_atomic` temp+fsync+rename pattern → mirror for cache rebuild.
- `TestProject` harness → works as-is for cache + query tests.

### Constraints from Existing Code

- **`.taskgolem/` is committed today.** Adding `cache.db` to it without action leaks binary churn into git. Must add `.gitignore` on `init` AND document the manual fix for existing projects.
- **Active JSONL is strict; archive is lenient.** Cache rebuild must decide whether to (a) adopt strict for both, (b) adopt lenient for both, or (c) skip archive from cache entirely.
- **Deterministic JSONL serialization** via BTreeMap + sorted writes must continue to round-trip. Adding `parent` with explicit-null serialization preserves this.
- **Clippy `-D warnings`** in `just check`. New modules need to be clean.
- **Single-binary distribution.** Any new dep must be static-linkable; `rusqlite["bundled"]` handles it for SQLite.

---

## PRD Concerns

| PRD Assumption | Research Finding | Implication |
|---|---|---|
| "WAL mode if used — its sidecars are gitignored" (non-functional #4) | WAL's concurrent read+write advantage doesn't apply to single-user CLI; sidecar files are a user-facing liability | **Use rollback journal (DELETE) instead.** Simpler gitignore, simpler mental model. Revisit if incremental cache updates become the norm. |
| Content hash is "cheap (e.g. xxhash3 over full file)" | True at our scale (≤5MB); short-circuit optimization lets us skip hashing in the hot path | Composite stamp with `stat` first, hash only when mtime+size match → confirmed PRD approach, just add short-circuit |
| "`tg init` auto-adds cache.db to `.taskgolem/.gitignore`" | **`.taskgolem/` is not currently gitignored at all — it's committed today.** New `.gitignore` file inside `.taskgolem/` is the right approach, but existing projects need a one-line manual update | Document migration in change-review step; doctor could detect+warn |
| "Existing implementations stay in Rust for v1" | Research confirms this is prudent — retrofitting list/ready on SQL would widen scope and risk correctness regressions | No conflict |
| "SQL view `task_view` with `is_ready`, `depth_from_root`, etc." | `is_ready` and `unmet_dep_count` need joins; `depth_from_root` needs recursive CTE (not cheap to materialize on every query) | Suggest: `task_view` materialized at rebuild time, not a SQL VIEW. Refresh when JSONL changes — trivial since we rebuild from scratch. |
| Cache staleness check runs "on every query-invoking command" | `stat(2)` is microseconds; full rebuild on staleness is O(n). Measurable at 5k tasks | Add a `--no-rebuild` / `--trust-cache` escape-hatch for power users? Defer to Should-Have. |
| "Corruption detection via `PRAGMA quick_check` + cache_schema_version sentinel" | Correct; also add authorizer probe on startup to verify sandbox intact | No conflict |

---

## Critical Areas

### Cache Rebuild Timing Policy

**Why it's critical:** Determines whether writes pay for cache updates synchronously (latency + risk) or lazily (simplicity + query-path cost).

**Why it's easy to miss:** PRD says both "writes update JSONL first, then the cache, then the stamp" AND "every query-invoking command checks the stamp and rebuilds if stale." These are two different triggers; only one is needed for correctness.

**What to watch for:** Design must pick one as primary. Recommendation: **lazy rebuild on query entry** (simpler, crash-safe by construction — no cross-file transaction needed). Eager rebuild becomes a nice-to-have optimization later.

### JSONL Parse Strictness During Rebuild

**Why it's critical:** Active JSONL is parsed strictly; archive is parsed leniently. Cache needs a consistent policy.

**Why it's easy to miss:** PRD treats "the cache" monolithically but doesn't say whether archived tasks appear in it, or what happens when the archive has a corrupt line.

**What to watch for:** Decide in design: (a) cache only active, (b) cache both with strict parse + abort on bad archive line, (c) cache both with lenient archive parse + warn. Recommendation: **cache only active for v1**; `tg query` explicitly targets live work. Add archived tasks in a follow-up if needed.

### Gitignore Migration for Existing Users

**Why it's critical:** `.taskgolem/` is committed today. Without action, `cache.db` starts churning the repo.

**Why it's easy to miss:** PRD assumes `.taskgolem/.gitignore` is the pattern; it's not, today.

**What to watch for:** Two tasks — (1) `tg init` writes `.taskgolem/.gitignore`, (2) `tg doctor` detects missing gitignore in an existing project and offers to add it.

### Sandbox Testing

**Why it's critical:** SELECT-only is a security boundary; regressions are silent until exploited.

**Why it's easy to miss:** Easy to test the happy path; easy to forget `ATTACH`, `load_extension`, `PRAGMA writable_schema`.

**What to watch for:** Integration test suite for the sandbox must explicitly cover each denied action code. Add golden-output tests that every denial surfaces a consistent error.

---

## Deep Dives

### WAL vs Rollback for this workload

**Question:** PRD mentions WAL as a possibility. Is it worth the complexity?

**Summary:** WAL's benefits (concurrent read during write, better write throughput) are nullified because (a) task-golem is single-user, (b) writes are full-file JSONL rewrites, not incremental, (c) cache rebuild uses atomic-rename, which works identically under either journal mode. WAL's costs (sidecar `-wal` / `-shm` files, potential git-diff pollution, extra gitignore entries) are real and user-facing.

**Implications:** Use DELETE journal mode. Set `PRAGMA synchronous=NORMAL` for runtime, `OFF` during rebuild (safe via atomic rename). Keep the option to switch later if we add incremental cache updates.

### Transaction strategy during rebuild

**Question:** How fast is a 5,000-task rebuild, realistically?

**Summary:** With per-row INSERTs outside a transaction: SQLite fsyncs per row → seconds. With one wrapping transaction + `synchronous=OFF` in a temp file: tens of milliseconds at our scale. Prior art (ripgrep-all) bulk-loads hundreds of thousands of rows in under a second with this pattern.

**Implications:** Always wrap rebuild in `BEGIN IMMEDIATE; ... COMMIT;`. Safe under atomic-rename because a partial temp-file is simply discarded; no half-state reaches `cache.db`.

---

## Synthesis

### Open Questions

| Question | Why It Matters | Possible Answers |
|---|---|---|
| Lazy vs eager cache rebuild on writes | Affects write latency, crash-safety model, and the "writes update JSONL then cache then stamp" PRD clause | **Lazy** (rebuild on query entry, simpler, crash-safe by construction); **Eager** (rebuild after each write, lower query latency, cross-file invariant to maintain); **Both** (eager best-effort, lazy as safety net) |
| Archive in cache? | Determines rebuild cost and query surface for historical work | **Active-only for v1** (recommended); **both** (richer queries, higher rebuild cost, lenient-parse risk) |
| Rebuild failure on write | If eager path chosen, what happens when cache update fails? | **Log + continue** (JSONL still consistent; next query rebuilds); **Rollback JSONL** (cross-file txn — complex and unnecessary) |
| `task_view` as SQL VIEW or materialized table | `is_ready` + `depth_from_root` cost non-trivial per query if computed in a VIEW | **Materialized** at rebuild (recommended — we rebuild from scratch, basically free); **VIEW** (always consistent with underlying tables but repeats work) |
| Journal mode | WAL vs DELETE | **DELETE** (recommended, no sidecar files); **WAL** (future incremental-write story) |

### Recommended Approaches

#### Cache Rebuild Timing

| Approach | Pros | Cons | Best When |
|---|---|---|---|
| **Lazy (rebuild on query)** | Simple; crash-safe by construction; JSONL stays single source of truth | Query after a write pays rebuild cost | Recommended for v1 — matches "cache is fully disposable" semantic |
| Eager (rebuild after write) | Queries are fast; stamp matches after every write | Cross-file invariant; what if rebuild fails? | Optimization if users complain about first-query latency |
| Hybrid (eager best-effort, lazy safety net) | Best-of-both | Most code; risk of divergent behavior | Defer |

**Initial recommendation:** Lazy. Eager is a pure optimization we can add later.

#### Journal Mode

| Approach | Pros | Cons | Best When |
|---|---|---|---|
| **DELETE (rollback)** | Single file; simple gitignore; no sidecars | Slightly slower for incremental writes | Recommended — matches our rebuild-from-scratch model |
| WAL | Concurrent read+write; faster incremental writes | Sidecar files (`-wal`, `-shm`); extra gitignore | If we add incremental cache updates |

**Initial recommendation:** DELETE.

#### Content Hash Crate

| Approach | Pros | Cons | Best When |
|---|---|---|---|
| **`xxhash-rust` `["xxh3"]`** | Pure Rust; SIMD; GB/s; zero deps | None relevant | Recommended |
| `twox-hash` | Zero deps | Older API; slower | Skip |
| `ahash`/`foldhash` | Fast | Seeded — unusable for stable stamps | Do not use |

**Initial recommendation:** `xxhash-rust`.

#### Cache Table Shape

| Approach | Pros | Cons | Best When |
|---|---|---|---|
| **Normalized + materialized `task_view`** | Standard schema; `task_view` is fast | Two write paths (tables + view materialization) | Recommended — rebuild is atomic, both happen together |
| Single flat table | Simplest schema | Tags as comma-separated strings or JSON; harder to query | For a throwaway cache, but we want good DX |
| SQL VIEW (not materialized) | Always consistent | Each query recomputes derived columns | Nope — `depth_from_root` is pricey |

**Initial recommendation:** Normalized tables (`tasks`, `task_tags`, `task_deps`) + materialized `task_view` table rebuilt on every cache rebuild.

### Key References

| Reference | Type | Why It's Useful |
|---|---|---|
| [rusqlite docs](https://docs.rs/rusqlite/latest/rusqlite/struct.Connection.html) | Docs | Authorizer + progress_handler API |
| [SQLite authorizer](https://sqlite.org/c3ref/set_authorizer.html) | Docs | Action codes to deny |
| [SQLite progress_handler](https://sqlite.org/c3ref/progress_handler.html) | Docs | Statement timeout pattern |
| [SQLite security](https://sqlite.org/security.html) | Docs | Hardening pragmas, known escape vectors |
| [SQLite recursive CTE](https://sqlite.org/lang_with.html) | Docs | Tree queries |
| [phiresky SQLite perf tuning](https://phiresky.github.io/blog/2020/sqlite-performance-tuning/) | Blog | Bulk-insert, journal mode, pragma choices |
| [ripgrep-all](https://github.com/phiresky/ripgrep-all) | Code | Prior art for JSONL-like-source + SQLite sidecar |
| [xxhash-rust](https://crates.io/crates/xxhash-rust) | Crate | Content-hash implementation |
| [SELECT-only sandbox](https://sqlite.work/restricting-sqlite-plugins-to-select-statements-safely-and-effectively/) | Blog | Layered sandbox pattern |
| [cycle detection in CTE](https://sqlfordevs.com/cycle-detection-recursive-query) | Blog | Bounded-depth pattern |

---

## Research Log

| Date | Activity | Outcome |
|---|---|---|
| 2026-04-14 | External landscape research (rusqlite sandboxing, hashing, CTE, stamps, timeouts) via subagent | Consolidated above; recommendations lock in |
| 2026-04-14 | Internal codebase research (item/store/deps/cli/doctor/tests/build) via subagent | Full integration-point map above |
| 2026-04-14 | Synthesis + PRD concern triage | Three latent decisions surfaced: rebuild timing, archive-in-cache, gitignore migration |
