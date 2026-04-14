# Change: Query Layer and Nested Tasks

**Status:** Proposed
**Created:** 2026-04-14
**Author:** HAMY

## TL;DR

- **Problem:** Task-golem's CLI surface is thin on queries — humans can't efficiently navigate task trees or find "what's next under epic X," and agents under-query because the query surface is limited to flat filters on status and tags. There is also no way to express parent/child structure (epics, nested work).
- **Solution:** Add a first-class `parent` field to every task (enabling arbitrary nesting), and introduce a SELECT-only SQL query interface backed by a persistent SQLite cache that derives from the authoritative JSONL. Cache staleness is detected via a composite stamp (mtime + size + content hash) so the cache self-heals across merges, manual edits, and crashes.
- **Key criteria:** Agents can express arbitrary SELECT queries in SQL; recursive tree navigation works; JSONL remains source of truth and git-diff friendly; no regressions in existing CLI commands.
- **Needs attention:** See Needs Attention section for directional decisions requiring human input.

## Glossary

Reader context for those unfamiliar with the codebase:
- **Task record**: current schema is defined in `src/model/item.rs:41-70` — `id`, `title`, `status` (todo/doing/done/blocked), `priority`, `description`, `tags`, `dependencies` (IDs), `created_at`/`updated_at`, `extensions` (map of `x-*` fields), plus blocked/claimed metadata. Stored in JSONL with a schema header `{"schema_version":1}`.
- **mtime**: file modification timestamp as reported by the filesystem.
- **Recursive CTE**: recursive Common Table Expression — SQL construct for traversing hierarchical data (trees, graphs).
- **"Ready" task**: a task in `todo` status whose dependencies are all `done` (or archived). Computed today by `src/model/deps.rs:173-219` and surfaced via `tg ready` / `tg next`.
- **Epic**: not a distinct type — just a task that has children via the new `parent` relationship.
- **JSONL path today**: active tasks at `.taskgolem/tasks.jsonl` with a `.tasks.lock` advisory file lock; archived tasks in a separate `archive.jsonl`. `.taskgolem/` is currently committed to git (not gitignored).

## Problem Statement

Task-golem is working well for HAMY as a daily driver, but four pain points have emerged that share a common root:

1. **Hard to query what's next.** `tg list` takes only `--status` and `--tag`; no compound conditions, no joins across dependencies, no tree-aware queries.
2. **Hard to browse/modify as a human.** No good way to walk through tasks, read descriptions, and edit. (TUI deferred — see out-of-scope.)
3. **Agents don't query enough.** When working autonomously, agents rarely consult the tracker beyond minimal reads. Partly a capability gap (thin query surface) and partly a prompting gap (skill doesn't encourage it).
4. **No sense of larger work structure.** Dependencies are flat. There's no way to express "these 8 tasks are part of this epic" and then navigate that tree, see what's unblocked under it, or scope focus to it.

Root cause analysis narrowed these four symptoms to two underlying gaps: **read power** (thin query surface — affects issues 1, 3, 4) and **missing structural relationship** (no parent concept — affects issue 4, enables richer queries for 1). Issue 2 (human editing) is largely solved by existing CLI (`tg edit`, `tg show`) once querying is strong enough to find the task you want.

## User Stories / Personas

- **HAMY (solo developer using task-golem daily)** — Wants to ask ad-hoc questions of the backlog ("what's unblocked under this epic?", "what are my highest-priority todos that aren't blocked?"), see work structure at multiple levels, and keep using git diffs to audit state changes.
- **Claude agents working on behalf of HAMY** — Need a machine-friendly query interface. Agents are fluent in SQL and will self-serve queries if the surface supports it. Currently they under-query because compound questions require custom CLI flag combinations that don't exist.

## Desired Outcome

When this change is complete:

1. **Nested tasks.** Every task can have a `parent: Option<String>` field pointing to another task ID. Nesting is arbitrary (a child can itself have children). `parent` cycles are rejected on write. Self-parent (`parent == id`) is rejected. Parent references must point to existing, non-archived tasks.

2. **SQL query interface.** A CLI surface exposes SELECT-only SQL against the task data. Agents and humans can run arbitrary SELECT queries. Recursive CTEs work for tree navigation.

3. **Persistent SQLite cache with composite stamp.** Source of truth remains JSONL. A SQLite cache lives alongside it at `.taskgolem/cache.db`, gitignored by default (added to `.taskgolem/.gitignore` on init). On every query-invoking `tg` command:
   - The cache's stored JSONL stamp `(mtime, file_size, content_hash)` is compared to the current JSONL file.
   - If any component differs (or the cache file is missing/corrupt), the cache is rebuilt from JSONL before queries run. Corruption detection uses `PRAGMA quick_check` plus a `cache_schema_version` sentinel row.
   - Writes update JSONL first (atomic rename, fsync'd — the existing pattern), then update the cache, then write the new stamp. If a crash occurs between JSONL and cache, the next command detects the stamp mismatch and rebuilds — no data loss.
   - If the cache path is unwritable (read-only filesystem), fall back to in-memory SQLite for that invocation with a `--verbose` notice.

4. **Existing CLI commands continue to work unchanged.** `tg list`, `tg ready`, `tg next`, `tg show`, `tg add`, `tg edit`, `tg block`, `tg unblock`, `tg do`, `tg done`, `tg todo`, `tg rm`, `tg dep add`, `tg dep rm`, `tg archive`, `tg dump`, `tg init`, `tg doctor`, `tg completions` — the full set enumerated in `src/cli/args.rs:19-206`. Existing implementations stay in Rust for v1; reimplementation on top of SQL is explicitly out of scope.

5. **Skill update.** The task-golem skill is updated with exactly 5 canonical SQL examples (one each for: all descendants of a task, all ancestors of a task, ready tasks under a given parent, unblocked todos by priority, orphan tasks with no parent). Guidance is added on when to query: starting a task, finishing a task, and when explicitly asked "what's next." Not every turn.

## Success Criteria

### Must Have

- [ ] `parent: Option<String>` field exists on every task record, serialized into JSONL, validated on write. Validation rules: referenced ID must exist in active tasks; no cycles via `parent` alone; no self-parenting. Existing JSONL records without a `parent` field deserialize as `None` (serde default handles this; no explicit migration required).
- [ ] Deleting a task with children is rejected. User must reparent or delete children first. (Rationale: safest default; cascade/orphan semantics can be added later if needed.)
- [ ] `tg edit` supports changing `parent`; cycle detection runs on the proposed post-edit graph.
- [ ] Persistent SQLite cache at `.taskgolem/cache.db`, auto-added to `.taskgolem/.gitignore` on `tg init` (existing users: documented one-line manual update).
- [ ] Writes update JSONL atomically (existing temp-file + fsync + rename pattern). The cache is rebuilt lazily on the next `tg query` invocation when its composite stamp disagrees with the JSONL — writes do not touch the cache. `rusqlite` is used with the `bundled` feature (static-link SQLite) to preserve single-binary install UX.
- [ ] Cache staleness stamp is a composite of `(jsonl_mtime, jsonl_size, jsonl_content_hash)`, stored in a `_cache_meta` table inside SQLite. All three must match for the cache to be considered fresh. Content hash is cheap (e.g., xxhash3 over the full file — at solo-dev scale, milliseconds).
- [ ] Cache rebuilds automatically when stale, missing, or corrupt. Corruption is detected via `PRAGMA quick_check` plus a `cache_schema_version` sentinel. Rebuild is idempotent: build to a temp file, atomic rename on success.
- [ ] Cycle detection covers `parent` relationships. `parent` and `dependencies` are treated as two independent DAGs — they do not share a cycle space. (Rationale: simpler mental model; can be tightened later if cross-graph cycles cause confusion.)
- [ ] Duplicate task IDs in JSONL (from e.g. a bad git merge) are detected on load; the command aborts with a clear error pointing at the offending IDs. Cache is not updated on parse failure.
- [ ] A SQL query interface is exposed via `tg query "SELECT ..."`. The connection is opened with `SQLITE_OPEN_READONLY` and rejects `ATTACH`, `PRAGMA`-writes, and any DDL/DML. JSON output via `--json` for agent consumption. Invalid SQL returns nonzero exit and a parseable error.
- [ ] A schema-discovery command exists: `tg query --schema` prints the cache's schema so agents can write queries without reading source. The canonical schema is also documented in the skill.
- [ ] Recursive CTE over `parent` works correctly (validated by tests for "all descendants of X", "all ancestors of X", "depth from root").
- [ ] Existing CLI commands continue to work without regression. The existing integration test suite passes unchanged.
- [ ] JSONL remains human-readable and git-friendly: each task remains on one line; field ordering is stable; adding `parent` adds at most one key per record, present only when non-null.
- [ ] `tg doctor` is extended to verify JSONL ↔ cache consistency by full rebuild comparison, and to report duplicate IDs and dangling `parent` references. Becomes the user-facing reconciliation tool.

### Should Have

- [ ] `tg list` gains a `--parent <ID>` filter so the common "show me children of X" case doesn't require writing SQL.
- [ ] `tg show <id>` output includes a "Children:" section listing direct children (if any). Empty case: no section printed. Leaf tasks show no change from today.
- [ ] SQL view `task_view` exposes at minimum these columns: `id`, `title`, `status`, `priority`, `parent`, `depth_from_root`, `is_ready` (bool), `unmet_dep_count` — so agents can write single-statement queries without manual joins.
- [ ] Task-golem skill is updated with the 5 canonical queries listed under Desired Outcome #5 and checked during review.
- [ ] Statement timeout on `tg query` (default 5s, `--timeout` flag to override) to prevent agent-driven runaway recursive CTEs.

### Nice to Have

- [ ] `tg tree <id>` command that renders the subtree under a task visually in the terminal.
- [ ] Batch-write API: `tg add` accepts multiple tasks in one invocation (single JSONL rewrite, single cache update) to avoid write amplification when agents add 5-10 tasks in a burst.
- [ ] `tg index` command to force a cache rebuild explicitly (useful for scripting or debugging).

## Scope

### In Scope

- Data model: add `parent: Option<String>` to task records; validation (cycle, self-parent, dangling, delete-with-children rejection).
- Storage: persistent SQLite cache file alongside JSONL with composite-stamp invalidation, auto-rebuild on stale/missing/corrupt.
- Query: `tg query` SELECT-only SQL interface, `tg query --schema` for discoverability, JSON output.
- CLI continuity: list/ready/next/show/edit/add/etc keep working with their current Rust implementations.
- Skill update: 5 canonical queries + transition-point guidance.
- Extensions to `tg doctor` for consistency verification.

### Out of Scope

- **TUI.** Explicitly deferred. May revisit once CLI + SQL are in place; may not be needed.
- **First-class "epic" entity.** Epics are just tasks that have children. No separate type, status, or schema concept.
- **"Next up" / sprint semantics.** Priority + dependencies + parent are sufficient. No mutable ordering field, no sprint entity.
- **Reimplementing existing commands on top of SQL.** Keep existing Rust implementations. Revisit in a future change if duplication becomes painful.
- **Cross-project parent references.** `parent` scope is same-project only.
- **Cross-graph cycle detection** (e.g., disallowing a task from depending on its own descendant). `parent` and `dependencies` are independent DAGs.
- **Archival/pruning story for unbounded JSONL growth.** Noted as future concern — not addressed here.
- **Archive queryability via `tg query`.** Deferred to v2. The SQLite cache contains active tasks only in v1; archived tasks remain accessible via `tg dump` and future work. Rationale: archive JSONL parsing is lenient today (skips bad lines), which conflicts with the strict-parse rebuild needed for cache correctness. Picking a policy for both without a concrete use case expands scope.
- **Multi-user coordination or concurrent-write improvements** beyond the existing file lock. Two concurrent `tg` processes that both see "stale cache" may both rebuild; SQLite's own locking and the atomic-rename pattern must make this safe but we do not optimize for it.
- **Network filesystem support.** Cache assumes local filesystem; behavior on NFS/SMB is undefined.
- **Remote/sync backends.** Local filesystem only.

## Non-Functional Requirements

- **Performance:** No regression over 20% on `tg list` / `tg ready` / `tg show` for current-size backlogs measured today. Query performance on the `task_view` targets sub-100ms for backlogs up to 5,000 tasks on local SSD. Cache rebuild targets sub-100ms for 500 tasks, sub-500ms for 5,000 tasks. Cache check overhead (stat + stamp read) for `--help` and parse-error paths: zero — cache is only opened by commands that actually query.
- **Durability:** JSONL writes remain fsync'd and atomic via temp-file + rename for full-file rewrites and append for new records. JSONL is authoritative; cache corruption must never cause JSONL data loss — the cache is always rebuildable from JSONL.
- **Observability:** Cache rebuilds, fallbacks to in-memory mode, and `tg doctor` warnings are surfaced under `--verbose`. Silent otherwise.
- **Git-friendliness:** JSONL diffs remain single-line-per-task and stable-ordered. The `.taskgolem/cache.db` and its SQLite sidecars (`-wal`, `-shm` if WAL mode is used) are gitignored.

## Constraints

- Task-golem is a Rust binary. Query layer ships as a library dep (`rusqlite` with `bundled` feature), not a separate service.
- JSONL-as-source-of-truth and git-diff readability are non-negotiable. The existing dataset is small and owned by a single user, so wire-format migration cost is negligible — breaking existing records is acceptable where needed (though `parent` addition does not require it).
- Solo-developer scale. We're not building for teams or concurrent writers beyond today's protections (advisory file lock via `.taskgolem/tasks.lock`).

## Dependencies

- **Depends On:** `rusqlite` crate with `bundled` feature. A fast non-cryptographic hash (e.g., `xxhash-rust` or `twox-hash`) for the JSONL content hash.
- **Blocks:** Any future work on richer agent workflows that benefit from query expressiveness (e.g., automated backlog grooming, better dependency visualizations). Also enables dogfooding — see Follow-up Work.

## Risks

- [ ] **Write amplification under batch operations.** Every write updates two files. Mitigation: Should-Have batch-write API. Measure first.
- [ ] **Parent semantics are wrong for future needs** (e.g., we later want "parent is blocked until all children done" cascading). Mitigation: current design treats parent as purely structural. Reversible — can add cascade semantics later without schema change.
- [ ] **Agents adopt SQL inconsistently or over-eagerly.** Mitigation: skill guidance is "at transition points," not every turn; measure adoption via HAMY's subjective feel after a few weeks.
- [ ] **Cache rebuild becomes slow as JSONL grows unbounded.** Mitigation: archival story deferred to a future change; content-hash check is O(n) but cheap; worst case, `tg index` can be run manually.
- [ ] **SQLite `bundled` feature adds compile time and toolchain needs.** Mitigation: accepted cost for single-binary UX; documented in build instructions.

## Needs Attention

Directional decisions that the self-critique surfaced but that are best resolved explicitly with the human during design phase:

- **JSONL growth / archival strategy.** Multiple critics flagged that rebuild-from-scratch scales linearly with file size. Today this is fine; at 10,000+ tasks it may not be. Should we define an archival trigger (e.g., auto-move `done` tasks older than N days into `archive.jsonl`) as part of this change, or defer to a separate TG-NNN? **Recommendation:** Defer. Solo-dev scale makes this a non-issue for months; addressing it now expands scope.
- **Skill update bundling vs splitting.** One critic suggested splitting the skill update into a follow-up change (TG-NNN) since prompt iteration is empirical and should happen after the CLI surface is used in anger. Keeping it here gives agents something to use immediately but ties this change's "done" to a judgment call on prompting. **Recommendation:** Keep in this change — ship with working defaults, iterate separately if needed.
- **Parent-via-dependency soft cycles.** A task can depend on its parent's sibling, its parent itself, or its own descendant without tripping either cycle check. Probably fine — these don't represent bugs, just unusual structures. Confirm that's intentional before design. **Recommendation:** Confirm, then document.

## Follow-up Work

Work that is adjacent to this change but not part of it:

- **Migrate task-golem's own backlog to use `tg` for tracking.** Today the project tracks work through the `changes/` workflow folders (TG-001..TG-006). Once TG-006 ships, task-golem should dogfood itself: adopt `tg` as the primary work tracker for its own development, with the `changes/` folders remaining for PRD/SPEC artifacts. This should be captured as its own follow-up change after TG-006 lands. Likely a small scope: `tg init` the project, seed existing backlog items via `tg add`, document the convention in CLAUDE.md.

## Open Questions

- [ ] **SQL surface shape beyond `tg query`.** Must-have is `tg query "SELECT ..."`. Should we also add structured convenience subcommands (`tg children <id>`, `tg ancestors <id>`) now, or wait and see which queries become common? Leaning: add `--parent` filter to `tg list` (captured as Should-Have); defer other convenience commands.
- [ ] **Cache schema details.** Normalized tables for `tasks`, `dependencies`, `tags` plus a denormalized `task_view`? Or a single flat table? Decide during design.
- [ ] **Hash algorithm for JSONL stamp.** xxhash3 vs twox-hash vs fnv — pick during design based on dep weight. Any fast non-cryptographic 64-bit hash is fine.
- [ ] **SQLite journal mode.** WAL vs rollback. WAL is generally better for read-heavy workloads but adds sidecar files. Decide during design.

## References

- TG-001 through TG-005 for current data model and CLI surface context.
- `src/model/item.rs:41-70` — current Task struct.
- `src/model/status.rs:6-11` — current Status enum.
- `src/cli/args.rs:19-206` — current CLI subcommands.
- `src/cli/commands/list.rs:7-76` — current list filters.
- `src/model/deps.rs:173-219` — current ready-queue computation.
- Conversation history on 2026-04-14 establishing design direction (JSONL authoritative + SQLite cache; first-class `parent`; drop epic-as-type; skill updates at transition points).
