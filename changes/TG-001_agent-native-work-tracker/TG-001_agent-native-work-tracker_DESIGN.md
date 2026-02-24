# Design: Agent-Native Work Tracker

**ID:** TG-001
**Status:** Complete
**Created:** 2026-02-24
**PRD:** ./TG-001_agent-native-work-tracker_PRD.md
**Tech Research:** ./TG-001_tech-research.md
**Mode:** Medium

## Overview

Task Golem is a three-layer CLI application: a clap-based CLI layer for argument parsing and output formatting, a domain layer for the state machine, dependency graph, and ID generation, and a JSONL persistence layer with atomic writes and flock-based concurrency. Every write operation follows a lock-load-mutate-atomic-save-unlock cycle with exponential backoff on lock contention. Every read operation loads the store without locking, relying on atomic rename for consistency. The architecture prioritizes simplicity — no async runtime, no database, no background processes — while leaving clear extension points for the Nice-to-Have daemon mode.

---

## System Design

### High-Level Architecture

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

Data flows top-down: CLI parses args, calls into domain logic, which uses the persistence layer to load/save state. Errors flow bottom-up as a typed error enum (via thiserror) that maps to exit codes at the CLI layer, with anyhow for internal error propagation.

### Component Breakdown

#### CLI

**Purpose:** Parse user input, dispatch to command handlers, format output.

**Responsibilities:**
- Define all subcommands and flags via clap derive macros
- Parse `--set x-foo.bar=value` as raw key-value pairs, passing them to the domain layer for dot-path interpretation
- Dispatch to the appropriate command handler
- Format command results as JSON (to stdout) or human-readable tables
- Map error types to exit codes (0 success, 1 user error, 2 system error)
- Route diagnostics and warnings to stderr only

**Interfaces:**
- Input: argv (command-line arguments)
- Output: stdout (results), stderr (diagnostics/errors), exit code

**Dependencies:** clap, thiserror, anyhow, Domain Layer, Output Formatter

#### Domain: Item Model

**Responsibilities:**
- Define the Item struct with all fields (id, title, status, priority, description, tags, dependencies, timestamps, claim fields, blocked fields, extensions)
- Serialize/deserialize items via serde with `#[serde(flatten)]` for `x-*` extension fields
- Validate state transitions via `Status::can_transition_to()`
- Handle claim semantics (set/clear `claimed_by`/`claimed_at` on transitions)
- Store/restore `blocked_from_status` on block/unblock
- Parse dot-path extension mutations into nested `serde_json::Value` structures

**Interfaces:**
- Input: Raw field values from CLI or deserialized JSON
- Output: Validated Item instances, transition results

**Dependencies:** serde, serde_json, chrono, thiserror

#### Domain: ID Generator

**Purpose:** Generate collision-resistant IDs.

**Responsibilities:**
- Generate 5-hex-char random IDs with fixed `tg-` prefix (configurable later via config file)
- Check collisions against active + archived item IDs
- Retry on collision (up to 10 attempts, then exit code 2)

**Interfaces:**
- Input: Set of existing IDs (active + archived)
- Output: Unique ID string (e.g., `tg-a3f82`)

**Dependencies:** rand, hex

#### Domain: Dependency Graph and Readiness

**Purpose:** Dependency validation and ready-queue computation.

**Responsibilities:**
- Detect cycles at dependency insertion time via DFS
- Reject self-referential dependencies
- Compute ready queue: todo items whose deps are all resolved (present in done set built from active `done` items + archived IDs)
- Identify deps on IDs absent from both active store and archive as unmet (with warning)

**Interfaces:**
- Input: Item list, archived ID set, proposed dependency addition
- Output: Cycle detection result, sorted ready queue

**Dependencies:** None (no graph library needed for ≤500 items)

#### Persistence: Store

**Purpose:** Load and save JSONL data with concurrency safety.

**Responsibilities:**
- Load JSONL files: parse `{"schema_version": N}` header, validate version, then one Item per line. Exit 2 if schema version is newer than supported
- Save JSONL files atomically: write to `NamedTempFile` in same directory, `sync_all()`, `persist()` (rename). Exit 2 on rename failure
- Manage file locking via flock on `.task-golem/tasks.lock` with exponential backoff (10ms–500ms, 5s total timeout). Exit 2 on timeout
- Move done items to archive on `tg done` (archive-first write ordering)
- Load archive IDs for collision detection on `tg add` and ready-queue dep resolution
- Load archived items for `tg show` fallback
- Expose `all_known_ids()` (active + archive) for ID collision checks

**Interfaces:**
- Input: Project root path, items to save
- Output: Loaded items, schema version, save confirmation

**Dependencies:** fd-lock, tempfile, serde_json, thiserror

#### Project Root Resolver

**Purpose:** Locate the `.task-golem/` project directory.

**Responsibilities:**
- Walk parent directories from CWD looking for `.task-golem/` (nearest ancestor wins, like git)
- Return resolved root path or a typed error
- Used by CLI dispatch before any store operation (except `tg init`)

**Interfaces:**
- Input: Current working directory
- Output: Resolved `.task-golem/` path, or `UserError` (exit 1) with message: "No task-golem project found. Run `tg init` to create one."

**Dependencies:** None (stdlib only)

#### Output Formatter

**Purpose:** Format command results for human or machine consumption.

**Responsibilities:**
- JSON mode: serialize result structs to JSON on stdout
- Human mode: colored table output with status indicators
- Error JSON: `{"error": "<message>", "exit_code": N}` on stderr when `--json` is set
- Respect `NO_COLOR` / `FORCE_COLOR` env vars

**Interfaces:**
- Input: Command result struct, `--json` flag
- Output: Formatted text to stdout/stderr

**Dependencies:** owo-colors, tabled

### JSON Output Schemas

The `--json` flag is the primary machine-readable API. All success output goes to stdout; all error output goes to stderr.

#### Item Schema

The core data structure returned by most commands:

```json
{
  "id": "tg-a3f82",
  "title": "Fix login timeout on /auth endpoint",
  "status": "todo",
  "priority": 0,
  "description": "The endpoint times out after 30s under load",
  "tags": ["auth", "bug"],
  "dependencies": ["tg-b1c23"],
  "created_at": "2026-02-24T12:00:00Z",
  "updated_at": "2026-02-24T12:00:00Z",
  "blocked_reason": null,
  "blocked_from_status": null,
  "claimed_by": null,
  "claimed_at": null,
  "x-phasegolem": {
    "phase": "build"
  }
}
```

**Field types:**

| Field | Type | Notes |
|-------|------|-------|
| `id` | string | Format `{prefix}-{5 hex chars}`, e.g., `"tg-a3f82"` |
| `title` | string | Single-line, non-empty |
| `status` | string | One of `"todo"`, `"doing"`, `"done"`, `"blocked"` |
| `priority` | integer | Higher = more important, default `0`, unbounded range |
| `description` | string \| null | Multi-line allowed |
| `tags` | string[] | May be empty `[]` |
| `dependencies` | string[] | Item IDs this item depends on, may be empty `[]` |
| `created_at` | string | ISO 8601 UTC, e.g., `"2026-02-24T12:00:00Z"` |
| `updated_at` | string | ISO 8601 UTC |
| `blocked_reason` | string \| null | Set when `tg block --reason` used |
| `blocked_from_status` | string \| null | Original status before block, used by unblock |
| `claimed_by` | string \| null | Agent ID, set via `tg do --claim` |
| `claimed_at` | string \| null | ISO 8601 UTC, auto-set with `claimed_by` |
| `x-*` | any JSON value | Extension fields, preserved through read/write cycles |

Null fields are included in JSON output (not omitted) for schema consistency. Extension fields (`x-*`) appear as top-level keys alongside core fields.

#### Per-Command Output

| Command | Success stdout | Notes |
|---------|---------------|-------|
| `tg init` | `{"initialized": true, "path": ".task-golem/"}` | |
| `tg add` | Item | The newly created item |
| `tg show` | Item | Includes archived items |
| `tg list` | Item[] | Array, may be empty `[]` |
| `tg do` | Item | After transition |
| `tg todo` | Item | After transition |
| `tg done` | Item | The archived item (status=done) |
| `tg block` | Item | After transition |
| `tg unblock` | Item | After transition |
| `tg edit` | Item | After modifications |
| `tg rm` | `{"removed": "tg-a3f82", "cleared_deps_from": ["tg-b1c23"]}` | `cleared_deps_from` only present with `--clear-deps` |
| `tg ready` | Item[] | Sorted by priority desc, created_at asc |
| `tg next` | Item \| null | Single item or `null` if queue empty |

#### Error Output (stderr)

When `--json` is set and an error occurs:

```json
{"error": "Item not found: tg-a3f82", "exit_code": 1}
```

- `error`: string, human-readable error message
- `exit_code`: integer, `1` (user error) or `2` (system error)

Non-fatal warnings (e.g., dep on non-existent ID) go to stderr as plain text in both human and `--json` modes. Agents should check exit code for success/failure, not stderr content.

### Data Flow

**Write operation (add, edit, do, todo, done, block, unblock, rm):**

```
CLI parses args
    → Resolve project root (exit 1 if not found)
    → Validate schema version on load (exit 2 if unsupported)
    → Acquire exclusive flock on tasks.lock (backoff 10ms–500ms, 5s timeout, exit 2 on timeout)
    → Read tasks.jsonl into Vec<Item>
    → Execute domain operation (validate, mutate)
    → Write updated items to temp file (sorted by ID)
    → sync_all() on temp file
    → Atomic rename temp file → tasks.jsonl (exit 2 on rename failure)
    → (For done: append to archive.jsonl first, fsync, then rewrite active store)
    → Drop lock handle (releases flock)
    → Format and output result
```

**Read operation (list, show, ready, next):**

```
CLI parses args
    → Resolve project root (exit 1 if not found)
    → Validate schema version on load (exit 2 if unsupported)
    → Read tasks.jsonl into Vec<Item> (no lock needed)
    → (For show: also read archive.jsonl if not found in active)
    → (For ready: also load archived IDs for dep resolution)
    → Filter, sort, compute
    → Format and output result
```

Read operations skip locking because atomic rename guarantees readers see either the complete old file or the complete new file, never a partial write. The consequence is that two agents may both read `tg ready`, see the same item as available, and both attempt `tg do --claim` — the second writer will fail with exit 1 ("already claimed"). This is expected and handled.

### Key Flows

#### Flow: Init

> Create a new Task Golem project in the current directory.

1. **Check existing** — Look for `.task-golem/` in CWD. If exists and no `--force`, exit 1 with error
2. **Create directory** — `mkdir .task-golem/`
3. **Create active store** — Write `tasks.jsonl` with `{"schema_version":1}` header line
4. **Create archive** — Write `archive.jsonl` with `{"schema_version":1}` header line
5. **Create lock file** — Create empty `tasks.lock` file (flock will create on demand, but explicit creation ensures clean state)
6. **Output** — Confirmation message or JSON `{"initialized": true, "path": ".task-golem/"}`

**Edge cases:**
- `--force` on existing project: recreates empty store files, warns about data loss on stderr
- Parent directory is read-only: exit 2 with system error

#### Flow: Add Item

> Create a new tracked work item with a collision-resistant ID.

1. **Parse args** — Title (required), optional description, priority, deps, tags, extension fields
2. **Validate title** — Reject if contains newlines (single-line enforced)
3. **Find root + lock** — Resolve project root (exit 1 if not found). Acquire exclusive lock with backoff (exit 2 on timeout). Load active store (exit 2 if schema unsupported)
4. **Load archive IDs** — Read `archive.jsonl` line-by-line, extracting only ID fields into a set for collision detection
5. **Generate ID** — Random 5-hex-char with `tg-` prefix, check collisions against active + archive ID set, retry up to 10x. Exit 2 if 10 collisions
6. **Validate deps** — Check each dep ID exists in active store or archive. Warn (stderr) if not found in either. Reject self-dep
7. **Check cycles** — If deps provided, DFS from each dep target to verify no path back to new item
8. **Create item** — Status=todo, priority=0 (or provided), timestamps=now, extensions parsed from `--set` flags
9. **Save** — Atomic write with new item inserted in ID-sorted order
10. **Output** — New item ID (or full item in JSON mode)

**Edge cases:**
- 10 consecutive ID collisions: exit 2 with system error (probability ~10^-10 at 500 items)
- Dep on non-existent ID (not in active or archive): warning on stderr, dep is stored but considered unmet for ready-queue
- Empty title: rejected by clap (required positional arg)

#### Flow: Show Item

> Display a single item with all fields including extension metadata.

1. **Find root** — Resolve project root (exit 1 if not found)
2. **Load active store** — Read `tasks.jsonl` (no lock)
3. **Resolve ID** — Exact match first, then prefix match in active store
4. **Archive fallback** — If not found in active store, search `archive.jsonl` for the ID
5. **Not found** — If absent from both active and archive, exit 1: "Item not found"
6. **Output** — Full item with all fields (JSON or human-readable)

**Edge cases:**
- Ambiguous prefix match: exit 1 with list of matching IDs
- Item in archive: displayed with status=done, all fields preserved

#### Flow: List Items

> List items filtered by status and/or tag.

1. **Find root** — Resolve project root (exit 1 if not found)
2. **Load active store** — Read `tasks.jsonl` (no lock)
3. **Filter** — Apply `--status` filter if provided. Apply `--tag` filter if provided. Default (no filters): all items in active store (which excludes done/archived items)
4. **Sort** — Priority descending, then `created_at` ascending
5. **Output** — Item list (JSON array or human-readable table)

**Edge cases:**
- `--status done`: requires loading `archive.jsonl` to include archived items
- No matching items: `[]` in JSON, "No items found" message in human mode

#### Flow: State Transition (todo / do / done / block / unblock)

> Transition an item through the 4-state machine with claim and archive semantics.

The five transition commands:
- `tg do <ID> [--claim <agent-id>]` — `todo`→`doing`
- `tg done <ID>` — `todo`→`done` or `doing`→`done`
- `tg block <ID> [--reason "..."]` — `todo`→`blocked` or `doing`→`blocked`
- `tg unblock <ID>` — `blocked`→(restored via `blocked_from_status`)
- `tg todo <ID>` — `doing`→`todo` (unclaim and return to queue)

1. **Find root + lock** — Resolve project root (exit 1 if not found). Acquire exclusive lock with backoff (exit 2 on timeout). Load active store (exit 2 if schema unsupported)
2. **Resolve ID** — Exact match first, then prefix match. Exit 1 if not found or ambiguous
3. **Validate transition** — `current_status.can_transition_to(target)`. Exit 1 if invalid (e.g., `blocked`→`blocked`, `done`→anything)
4. **Handle claims (do)** — If `--claim` provided: fail if already claimed by a different agent (exit 1). Same agent re-claiming: update `claimed_at`. No `--claim`: transition without claim
5. **Clear claims (todo/done/block)** — On any transition out of `doing`, clear `claimed_by` and `claimed_at`
6. **Handle blocked** — On `block`: store current status in `blocked_from_status`, optionally store `--reason`. On `unblock`: restore from `blocked_from_status` (default to `todo` if missing), clear reason
7. **Update timestamps** — Set `updated_at` to now
8. **Archive (done only)** — Append item (with status=done) to `archive.jsonl` + fsync. If archive append fails: exit 2, item remains in active store unchanged. Then rewrite active store without the item. If active rewrite fails after archive append: exit 2, item exists in both stores (benign — `tg doctor` detects and resolves)
9. **Save** — Atomic write active store (exit 2 on rename failure)
10. **Output** — Updated item or confirmation

**Edge cases:**
- `done` on item not in active store: exit 1 (done items are archived, cannot re-done)
- `unblock` without stored `blocked_from_status`: default to `todo`
- `do` without `--claim`: transitions to `doing` with no claim (solo use, backwards compatible)
- `todo`→`done` (skip doing): valid per state machine, archives directly

#### Flow: Ready Queue

> Return unblocked todo items in priority order for agent consumption.

1. **Find root** — Resolve project root (exit 1 if not found)
2. **Load store** — Read active items (no lock). Load archived IDs from `archive.jsonl` (line-by-line ID extraction)
3. **Build done set** — Collect IDs of all `done` items in active store + all archived IDs
4. **Filter ready** — Items where: status == `todo` AND every dependency ID is either in the done set or in the archive. Deps on IDs absent from both active store and archive are unmet (item not ready)
5. **Sort** — Priority descending, then `created_at` ascending (FIFO within same priority)
6. **Handle `--include-stale`** — If provided, also include `doing` items whose `updated_at` is older than the specified duration, surfacing potentially dead agent work
7. **Output** — List of ready items (JSON array or human table)

**Edge cases:**
- Dep on non-existent ID (never created, not in archive): dep is unmet, item not in ready queue
- No ready items: `[]` in JSON, "No items ready" message in human mode

#### Flow: Edit Item

> Modify fields on an existing active item.

1. **Find root + lock** — Resolve project root (exit 1 if not found). Acquire exclusive lock with backoff (exit 2 on timeout). Load active store
2. **Resolve ID** — Exact match, then prefix match. Exit 1 if not found or ambiguous
3. **Apply field changes** — Title (validate single-line), priority, description
4. **Apply dep changes** — `--add-dep`: validate target exists, check cycles via DFS, add. `--rm-dep`: remove
5. **Apply tag changes** — `--add-tag`: add if not present. `--rm-tag`: remove if present
6. **Apply extension changes** — `--set x-foo.bar=value`: parse dot-path, create nested objects, set value. `--set x-foo.bar=` (empty): delete key
7. **Update timestamp** — Set `updated_at` to now
8. **Save** — Atomic write (exit 2 on rename failure)
9. **Output** — Updated item

**Edge cases:**
- Adding dep that creates a cycle: reject with exit 1 and error message showing the cycle
- Editing a done/archived item: not possible (not in active store), exit 1

#### Flow: Remove Item

> Hard-delete an item from the active store.

1. **Find root + lock** — Resolve project root (exit 1 if not found). Acquire exclusive lock with backoff (exit 2 on timeout). Load active store
2. **Resolve ID** — Exact match, then prefix match. Exit 1 if not found or ambiguous
3. **Check dependents** — Scan all items for any that list this ID as a dependency
4. **Warn if dependents** — If dependents exist and no `--force`, exit 1 with message: `"Item tg-a3f82 is depended on by: tg-b1c23, tg-c4d56. Use --force to remove anyway (leaves dangling deps) or --force --clear-deps to also remove this dependency from all dependents."`
5. **Clear deps (if `--clear-deps`)** — Remove the deleted item's ID from all dependents' dependency lists
6. **Remove** — Remove item from active store
7. **Save** — Atomic write (exit 2 on rename failure)
8. **Output** — Confirmation. JSON includes `cleared_deps_from` array if `--clear-deps` was used

**Edge cases:**
- `--force` without `--clear-deps`: remove item, leave dangling deps (dependents become not-ready until deps are manually removed via `tg edit --rm-dep`)
- `--force --clear-deps`: remove item AND clean up all dependents' dep lists (cascading but explicit)
- `--clear-deps` without `--force`: ignored (no dependents to clear if `--force` isn't needed)
- Item not found: exit 1

---

## Technical Decisions

### Decision: Read Operations Skip Locking

**Context:** Should read-only commands (list, show, ready) acquire a shared lock?

**Decision:** No lock for reads. Rely on atomic rename for consistency.

**Rationale:** Atomic rename is an atomic filesystem operation. A reader sees either the complete old file or the complete new file, never a partial write. This eliminates the need for shared/exclusive lock coordination on reads. The worst case is reading slightly stale data (an item was just modified by another process), which is acceptable for a CLI tool where the caller will simply re-run the command.

**Consequences:** Simpler code, no reader-writer lock complexity, reads are fast. In multi-agent scenarios, two agents may both read `tg ready`, see the same item, and race to `tg do --claim` it — the second writer fails with exit 1 ("already claimed by a different agent"). This is expected behavior, not a bug.

### Decision: Separate Lock File with Backoff

**Context:** What file should the flock be placed on, and what happens under contention?

**Decision:** Use a dedicated `.task-golem/tasks.lock` file. Acquire with non-blocking flock + exponential backoff (10ms initial, doubling, cap 500ms, 5s total timeout, 0-50% jitter). Exit 2 on timeout.

**Rationale:** The atomic write pattern replaces the data file via rename, creating a new inode. Locks on the old inode would be invalidated. A separate lock file persists across data file replacements. Flock auto-releases on process death, eliminating the need for PID-based stale lock detection for correctness. Backoff with jitter prevents thundering herd under multi-agent contention.

**Consequences:** One extra file in `.task-golem/`. Flock auto-releases on process death — no stale lock detection logic needed for correctness. PID/timestamp can optionally be written to the lock file as diagnostic metadata for `tg doctor`, but this is informational only, not relied upon for locking semantics. This deviates from the PRD's Must-Have "stale lock detection: lock file contains PID + timestamp, auto-clear if PID is dead after 30-second timeout" — the flock approach is strictly superior (kernel-enforced auto-release vs. userspace PID polling) and the PRD requirement should be updated to reflect this.

### Decision: BTreeMap for Extension Fields

**Context:** Should extension fields use HashMap or BTreeMap?

**Decision:** `BTreeMap<String, serde_json::Value>` for deterministic key ordering.

**Rationale:** BTreeMap produces alphabetically-sorted keys on serialization. Combined with serde's deterministic struct field ordering, the same in-memory data always produces byte-identical JSON output. This is critical for minimal git diffs — rewriting the JSONL file without changing data should produce zero diff. The tech research recommends `HashMap` with `serde_json`'s `preserve_order` feature (insertion-order via IndexMap). We prefer BTreeMap because alphabetical normalization is more predictable than insertion-order preservation — two processes writing the same extension keys in different orders will produce identical output with BTreeMap but different output with insertion-order. Whether to also enable `preserve_order` for nested `Value::Object` types within extension values is deferred to spec.

**Consequences:** O(log n) insertion vs O(1), but n is tiny (a handful of extension keys per item). Deviates from tech research recommendation — documented here as an intentional choice.

### Decision: Items Sorted by ID in JSONL

**Context:** What order should items appear in the JSONL file?

**Decision:** Sort items by ID when writing the JSONL file.

**Rationale:** Predictable item positioning reduces git merge conflicts. When two branches both add items, the new items are inserted at different positions (random hex IDs distribute across the sorted order), making git auto-merge more likely to succeed. Without sorting, items appended to the end always conflict.

**Consequences:** Slightly more work on write (sort before save). Better git merge behavior. Consistent `git diff` output. Note: the PRD states "modifying a single item changes exactly one line." This holds for edits (the item's line changes, all other lines are identical). For adds, the new item's line is inserted and surrounding lines shift — git diff shows this as one added line, which is correct. For removes, one line disappears.

### Decision: Archive as Separate File with Append Semantics

**Context:** Should done items stay in the active store, and how should the archive be written?

**Decision:** Move done items to `.task-golem/archive.jsonl` on `tg done`. The archive is append-only (not rewritten on every operation).

**Rationale:** Keeps the active store small and fast to load. Most operations only need active items. The archive is read for: ID collision checks (on `tg add`, line-by-line ID extraction), `tg show` fallback, and dep resolution in `tg ready` (checking if a dep ID exists in archive = met). Appending is O(1) vs O(n) for a full rewrite.

**Consequences:** Two files to manage. `tg done` performs two writes under a single lock:
1. Append item to `archive.jsonl` + fsync. If this fails: exit 2, item remains in active store — no data loss.
2. Rewrite active store without the item. If this fails after step 1: exit 2, item exists in both stores — benign duplicate detectable by `tg doctor`.

Archive-first ordering ensures the failure mode is always "duplicate" (benign) rather than "missing" (data loss).

### Decision: Validate Schema Version on Every Load

**Context:** Should the schema version header be checked on every JSONL load or lazily?

**Decision:** Validate on every load. Fail gracefully if the file's schema version is newer than the binary supports.

**Rationale:** It's a single-line comparison, trivially fast. Early detection of version mismatches prevents confusing errors downstream. A clear error message ("This store uses schema version 2, but this tg binary only supports version 1. Please upgrade tg.") is far better than silently misinterpreting fields.

**Consequences:** Every load pays the cost of one string comparison. Older binaries fail fast on newer stores with an actionable error. Note: `#[serde(flatten)]` will silently capture unknown fields into the extensions map. This means an older binary reading a newer schema's fields will preserve them through read/write cycles rather than dropping them — a form of forward compatibility, but also a risk if the field semantics differ. Schema version gating (refuse to load newer schemas) is the primary defense.

### Tradeoffs Accepted

| Tradeoff | We're Accepting | In Exchange For | Why This Makes Sense |
|----------|-----------------|-----------------|---------------------|
| No read locking | Reads may see microseconds-stale data; multi-agent claim races are possible | Simpler code, no lock contention on reads | Claim races resolve correctly via write-lock serialization. Staleness window is negligible |
| Full active store rewrite on every write | O(n) write cost for n items | Simple implementation, guaranteed atomic writes | n ≤ 500 items ≈ 250KB. Rewrite takes <1ms. Simplicity gain is massive |
| No async runtime | Cannot do daemon mode in v1 | ~3MB smaller binary, no tokio complexity | Daemon is Nice-to-Have. File-lock concurrency handles v1 requirements |
| 5-char hex IDs | ~1M namespace, birthday collision probability rises around ~1.2k items | Short, human-typeable IDs | Collision detection + rehash handles it. Can migrate to 6+ chars later |
| Append-only archive | Archive grows unbounded | Simple archive writes, fast `tg done` | Archive is rarely fully loaded. `tg archive --before` (Nice-to-Have) addresses pruning |
| Fixed `tg-` ID prefix for v1 | No multi-project disambiguation | Simpler code, no config file parsing | Configurable prefix is a Should-Have, easy to add via `.task-golem/config.yaml` |
| `tg rm --force` requires explicit `--clear-deps` for cascading | Two flags needed for the "clean remove" path | No accidental cascading side effects from `--force` alone | Error message clearly explains both options. Agents can parse the error and add `--clear-deps` |

---

## Alternatives Considered

### Alternative 1: Append-Only Operation Log with Compaction

**Summary:** Instead of read-modify-write on a state file, store every operation as a log entry. Periodically compact the log into a snapshot.

**How it would work:**
- Every write appends a JSON operation record: `{"op":"add","item":{...}}`, `{"op":"transition","id":"tg-a3f82","to":"doing"}`, etc.
- Reads replay the log from the last snapshot to reconstruct current state
- Periodic compaction (or on-demand via `tg compact`) rewrites the log as a clean snapshot

**Pros:**
- Faster writes (append vs full rewrite)
- Natural audit trail (who changed what, when)
- Easier git merge (appending to a log rarely conflicts since new entries go to different lines)

**Cons:**
- Reads become slower — must replay log entries since last snapshot
- State reconstruction is complex (apply operations in order, handle partial failures)
- Compaction adds a second code path and a potential data loss window
- Git diffs show operations not current state (harder to review "what does the backlog look like now?")
- Significantly more complex for the same end result at this scale

**Why not chosen:** The active store is ≤500 items, ≤250KB. Full rewrite takes <1ms. The simplicity of "load all, modify, save all" far outweighs the performance benefit of append-only, which matters at much larger scale. The audit trail is appealing but not a PRD requirement, and git history provides a similar function. The git merge advantage is partially captured by sorting items by ID in the state file.

### Alternative 2: Hybrid SQLite + JSONL (like beads_rust)

**Summary:** Use SQLite as the primary data store for fast queries and ACID transactions, with JSONL export for git-friendliness.

**How it would work:**
- SQLite database at `.task-golem/tasks.db` for all read/write operations
- JSONL files generated on-demand via `tg sync` or `tg dump` for git commits
- Reads and writes go through SQLite, avoiding file-lock complexity
- Git hooks or manual commands keep JSONL in sync with SQLite

**Pros:**
- ACID transactions (no manual file locking needed)
- Fast queries even at larger scale
- SQL-based filtering could power `tg ready --where x-foo=bar`
- Proven pattern (beads_rust uses this approach)

**Cons:**
- Two sources of truth (SQLite and JSONL) that can drift
- Sync between them is a new failure mode
- SQLite binary diffs are not git-reviewable — the JSONL is a secondary export, not the primary store
- Adds a heavy dependency (SQLite) for a use case that doesn't need it at ≤500 items
- PRD explicitly rejects databases as a constraint

**Why not chosen:** The PRD constraint "No database" rules this out. Beyond the constraint, the added complexity of maintaining two representations (SQLite for speed, JSONL for git) is not justified when the JSONL-only approach handles the target scale with sub-millisecond performance. SQLite would be the right choice at 10k+ items, but Task Golem's design point is hundreds, not thousands.

---

## Risks

| Risk | Impact | Likelihood | Mitigation |
|------|--------|------------|------------|
| JSONL merge conflicts on active store | Lost items or corrupted state when merging branches | Medium | Sort items by ID for predictable positioning. Archive aggressively to keep active store small. `tg doctor` validates integrity. `tg merge-resolve` (Nice-to-Have). Note: concurrent edits to the same item on different branches will always conflict regardless of sort order |
| Archive file grows very large | Slow `tg add` (collision check), `tg show` (fallback), `tg ready` (dep resolution) | Low | Only load archive when needed. For collision/dep checks, scan line-by-line extracting only IDs without full deserialization. At 5,000 archived items (~2.5MB), scan takes ~5ms — well within budget. `tg archive --before` (Nice-to-Have) for pruning |
| flock not supported on network filesystems | Lock failures on NFS/SMB mounts | Low | Document limitation. All local filesystems (ext4, btrfs, APFS, NTFS) support flock. Network filesystem use is out of scope. NFS produces `ENOLCK` — surfaces as exit 2 with clear error |
| Partial failure during `tg done` two-phase write | Item in both active and archive (if archive append succeeds but active rewrite fails) | Very Low | Archive-first write ordering. Failure mode is always "benign duplicate" not "data loss." `tg doctor` detects items present in both files and removes from active store |
| Extension field key conflicts between orchestrators | Different tools overwrite each other's `x-*` data | Low | Document namespace convention: `x-{toolname}-*` (e.g., `x-phasegolem-*`). Convention-based, no runtime enforcement |
| `#[serde(flatten)]` captures unknown fields silently | Future schema fields land in extensions map on older binaries | Low | Schema version validation refuses to load newer schemas. If version matches but a field is new (minor addition), it is preserved through read/write — acceptable forward compatibility. Major changes require version bump |

---

## Scaling Characteristics

The design targets ~500 active items (PRD 95th-percentile estimate) but has significant headroom beyond that. Here's where things stand at various scales, assuming SSD, release build, ~500 bytes/item average.

### Active Store (full load + full rewrite on every write)

| Active Items | File Size | Read+Write Latency | Lock Hold Time | Verdict |
|---|---|---|---|---|
| 500 | ~250KB | <2ms | ~3ms | PRD target. Plenty of room |
| 2,000 | ~1MB | ~5ms | ~8ms | Comfortable |
| 10,000 | ~5MB | ~15ms | ~25ms | Functional. Git diffs get noisy |
| 50,000 | ~25MB | ~80ms | ~100ms | Pushing 200ms CLI budget under contention |

### Archive (line-by-line ID scan on `tg add` and `tg ready`)

| Archived Items | File Size | ID Scan Latency | Verdict |
|---|---|---|---|
| 5,000 | ~2.5MB | ~5ms | Fine |
| 20,000 | ~10MB | ~20ms | Fine |
| 50,000 | ~25MB | ~50ms | Hits the 50ms `tg ready` budget |
| 100,000 | ~50MB | ~100ms | Exceeds budget — needs index file |

### ID Collisions (5 hex chars = ~1M namespace)

| Total Items (active + archived) | P(collision per add) | Practical Impact |
|---|---|---|
| 500 | ~0.05% | Invisible |
| 1,200 | ~50% per attempt | Fine — 10 retries makes failure probability ~10^-10 |
| 5,000 | ~2.4% per attempt | Occasional retry, still fast |
| 50,000 | ~95% per attempt | Multiple retries every add. Should extend to 6+ hex chars |

### What Breaks First

The **archive ID scan** is the first bottleneck — `tg ready` exceeds its 50ms budget around 50K archived items. Mitigation if needed: an archive ID index file (`.task-golem/archive-ids.idx`, one ID per line, appended on `tg done`) would push the limit to 100K+ trivially. Not needed for v1.

### Comfortable Operating Range

- **Active:** ~2,000 items comfortably, ~10,000 functional
- **Archive:** ~20,000 comfortably, ~50,000 functional
- **IDs:** Extend to 6+ hex chars if total items (active + archived) approaches 5,000+

Well beyond what any single project should need. If a project has 2,000+ active tasks, it likely needs a database-backed tracker, not a JSONL file.

---

## Integration Points

### Existing Code Touchpoints

Greenfield project — no existing code to modify. The `.task-golem/` directory is self-contained within the user's project.

### External Dependencies

- **Filesystem** — All operations require a writable local filesystem supporting atomic rename (POSIX `rename(2)`) and advisory file locks (`flock(2)`). This covers ext4, btrfs, APFS, NTFS
- **Phase-golem (future consumer)** — Will use Task Golem as its work queue substrate. v1 integration via CLI (`tg` commands with `--json`). Phase-golem stores domain data in `x-phasegolem-*` extension fields. Note: PRD defers library API to v1.1; v1 architecture need not optimize for crate-public API surface
- **Changes workflow (future consumer)** — The `/changes` skill could create/close Task Golem items as it works through the PRD → Research → Design → Spec → Build → Review pipeline

### Decision: Dot-Path Extension Mutation Semantics

**Context:** How does `--set x-foo.bar=value` work for setting, overwriting, and deleting nested extension fields?

**Decision:**
- **Value parsing:** Try JSON parse first, fall back to string. `--set x-foo.count=42` → number `42`. `--set x-foo.name=hello` → string `"hello"`. `--set x-foo.obj={"a":1}` → parsed JSON object
- **Conflict with existing type:** If `--set x-foo.bar=value` and `x-foo` is currently a non-object (e.g., string), overwrite `x-foo` with `{"bar": value}`. The dot-path creates the structure
- **Deletion:** `--set x-foo.bar=` (empty value) deletes the key. If deleting the last key under a parent object, remove the empty parent recursively
- **First segment validation:** First segment of the key must start with `x-`. Reject otherwise (exit 1)
- **Max depth:** No artificial limit

**Consequences:** The JSON-first parsing means agents can pass structured values without quoting gymnastics. The overwrite-on-conflict behavior is aggressive but predictable.

### Decision: Duration Format for `--include-stale`

**Context:** What format does `--include-stale=<duration>` accept?

**Decision:** Use the `humantime` crate. Accepts human-readable durations: `30s`, `5m`, `4h`, `1d`, `1h30m`, `2h30m15s`.

**Consequences:** Adds one small dependency (~50KB). Well-tested, widely used in the Rust ecosystem. Agents can use simple formats (`4h`), humans can use compound formats (`1h30m`).

### Decision: ID Resolution Algorithm

**Context:** How are IDs resolved when a user provides a full or partial ID?

**Decision:** Three-step resolution:
1. **Exact match** on the full ID string (e.g., `tg-a3f82` matches `tg-a3f82`)
2. **Prefix prepend** — if no exact match, try prepending `tg-` (e.g., `a3f82` → `tg-a3f82`)
3. **Prefix match** — if still no match, find all items whose ID starts with the input (e.g., `a3f` matches `tg-a3f82`)

If prefix match returns multiple items: exit 1 with `"Ambiguous ID prefix 'a3f': matches tg-a3f82, tg-a3f91. Provide more characters to disambiguate."`

No minimum prefix length — even a single character is accepted, but likely ambiguous at scale.

**Consequences:** Bare hex IDs (`a3f82`) work naturally. Short prefixes work for small stores. Agents should always use full IDs for reliability.

### Decision: `--verbose` Flag

**Context:** What debug-level output should be available?

**Decision:** Global `--verbose` flag on all commands. Outputs to stderr only (never affects stdout/JSON). Surfaces: lock acquisition timing, file paths loaded, schema version, item count loaded, archive size.

**Consequences:** No impact on normal usage. Useful for debugging agent issues. Does not require a logging framework — plain `eprintln!` guarded by the flag.

### Decision: Malformed JSONL Line Handling

**Context:** What happens when a JSONL line cannot be parsed?

**Decision:**
- **Active store (`tasks.jsonl`):** Fail-fast. Exit 2 with error: `"Malformed item at line N in tasks.jsonl: <parse error>. Run tg doctor to repair."` Active store integrity is critical for correctness
- **Archive (`archive.jsonl`):** Skip-and-warn. Log warning to stderr: `"Skipping malformed line N in archive.jsonl"`. Continue processing remaining lines. Archive is read-only and less critical; a single corrupt line should not block operations

**Consequences:** Active store corruption is surfaced immediately. Archive corruption degrades gracefully. `tg doctor` (Should-Have, potential followup) can repair both.

### Decision: Schema Migration Mechanics

**Context:** How does auto-migration work when a newer binary encounters an older schema?

**Decision:**
- On load: check `schema_version`. If older than current supported version, auto-migrate in place
- Migration steps: load all items with old schema, transform to new schema, atomic write with new version header
- Before migration: backup original file to `.task-golem/tasks.jsonl.v{N}.bak` (and same for archive)
- Migration runs automatically on first access after binary upgrade
- For v1: only schema version 1 exists, so no migration code is needed. The mechanism is designed for future use — the version check infrastructure is in place

**Consequences:** Zero manual intervention for schema upgrades. Backup file provides rollback safety. The first actual migration (v1→v2) will require implementing the transform logic, but the scaffolding (version check, backup, atomic rewrite) is already part of the design.

### Decision: Daemon Compatibility (Deferred)

**Context:** Should the persistence layer be abstracted behind a trait to enable future daemon mode?

**Decision:** Defer. Do not add a Store trait in v1. The current design has clean component boundaries (Store component with defined interfaces) that make introducing a trait later a straightforward refactor.

**Consequences:** Adding daemon mode later will require refactoring command handlers to go through an abstracted store interface instead of calling file operations directly. This is a bounded refactor (touch each command handler once) and is acceptable given that daemon mode is a Nice-to-Have. Over-abstracting now would add complexity for a feature that may never ship.

---

## Potential Followups

Items identified during design that are not blockers but should be tracked:

- **`tg doctor` command** (PRD Should-Have) — Validates JSONL syntax, checks for duplicate IDs across active+archive, detects items in both files (partial `tg done` failure), validates state machine integrity, detects dependency cycles, detects dangling deps, offers automated repair with backup. Load-bearing for several recovery scenarios but the failure modes it addresses are rare. Pick up as a separate work item
- **`tg dep add` / `tg dep rm` subcommands** (PRD Should-Have) — Sugar over `tg edit --add-dep` / `--rm-dep`. Nice ergonomics but functionally equivalent
- **`tg next` command** (PRD Should-Have) — Sugar over `tg ready --limit 1`. Returns single Item or null
- **Tab completion** (PRD Nice-to-Have) — Via `clap_complete` for bash, zsh, fish

---

## Design Review Checklist

Before moving to SPEC:

- [x] Design addresses all PRD must-have requirements
- [x] Key flows are documented (init, add, show, list, transition, ready, edit, rm)
- [x] Tradeoffs are explicitly documented and acceptable
- [x] Integration points with existing code are identified (greenfield)
- [x] Dep resolution for non-existent IDs resolved (check archive; absent from both = unmet)
- [x] `tg rm --force --clear-deps` behavior specified
- [x] JSON output schemas defined for all commands
- [x] Quality items resolved (dot-path semantics, duration format, ID resolution, verbose, JSONL error handling, migration, daemon deferral)

---

## Design Log

| Date | Activity | Outcome |
|------|----------|---------|
| 2026-02-24 | Initial design draft | Three-layer architecture, 6 key flows, 6 decisions, 2 alternatives evaluated |
| 2026-02-24 | Self-critique (7 agents) | Added `tg todo` command, `tg show`/`tg list` flows, project root resolver, lock backoff, schema validation in flows, archive-first failure paths, resolved dep resolution open question, documented PRD deviation on stale locks, clarified BTreeMap vs tech research HashMap recommendation |
| 2026-02-24 | Design iteration | Added JSON output schemas, `--force --clear-deps` on rm, resolved quality items (dot-path semantics, humantime duration, ID resolution algorithm, --verbose, JSONL error handling, migration mechanics, daemon deferral). Noted tg doctor as potential followup |
| 2026-02-24 | Design finalized | Added scaling characteristics section. All checklist items resolved. Status → Complete |
