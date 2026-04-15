# Design: Task Events Log

**ID:** TG-008
**Status:** Initial
**Created:** 2026-04-15
**PRD:** ./TG-008_blocked-status-and-task-events_PRD.md
**Tech Research:** (none — implementation patterns are well-established in this codebase)
**Mode:** Medium

## TL;DR

- **Approach:** Introduce `events.jsonl` + `events.archive.jsonl` as a second JSONL store alongside `tasks.jsonl`; add a compile-enforced chokepoint (`StatusChange` witness) that every status-mutating code path must redeem through `Store::commit_*` helpers, which write the event first (single `write(2)` under `O_APPEND`, fsynced) and then mutate tasks.
- **Key decisions:** (1) Header-less events file so every line stands alone — enables lock-free concurrent appends. (2) `StatusChange` is a `#[must_use]` witness returned by each `Item::apply_*` method and consumed by `Store::commit_status_change`/`commit_done` — the type system enforces "no status mutation without an event." (3) `tg note` is lock-free; `tg do/done/block/unblock/todo` take the existing store lock and piggyback on it for events.
- **Tradeoffs:** Event-first ordering buys single-fsync durability at the cost of a crash window where the event exists but the task mutation didn't — reconciled by `tg doctor`, not by a cross-file transaction.
- **Needs attention:** None.

## Overview

Add an append-only, per-task event log that captures status transitions (emitted automatically by every status-mutating verb) and free-text notes (explicit via `tg note`). Events live in `events.jsonl` alongside `tasks.jsonl`, move to `events.archive.jsonl` when their task archives, and are viewed via `tg events <id>`. Concurrency relies on POSIX `O_APPEND` + `PIPE_BUF` atomicity for sub-2048-byte lines written as a single `write(2)` syscall. A compile-time `StatusChange` witness makes it impossible to mutate `Item::status` without emitting the matching event.

---

## System Design

### High-Level Architecture

```
┌────────────────────────────────────────────────────────────────┐
│ CLI (src/cli/commands/)                                        │
│                                                                │
│  do / done / todo / block / unblock   note         events      │
│          │                             │             │         │
│          ▼                             │             ▼         │
│  Item::apply_*()  ── returns ──▶ StatusChange        │         │
│          │                    (#[must_use] witness)  │         │
│          ▼                             │             │         │
│  Store::commit_status_change ◀─ consumes ─┐          │         │
│  Store::commit_done        ◀─ consumes ───┤          │         │
│     (chokepoint)                          │          │         │
│          │                                │          │         │
└──────────┼────────────────────────────────┼──────────┼─────────┘
           ▼                                ▼          ▼
    ┌──────────────────────────────────────────────────────────┐
    │ events::append::write (single write(2), fsync)           │
    │   ↓                                                       │
    │ events.jsonl ◀───── read ─── events::read::for_task()    │
    │   ↓ (on tg done)                                         │
    │ events::archive::move_for_task                           │
    │   ↓                                                       │
    │ events.archive.jsonl                                      │
    └──────────────────────────────────────────────────────────┘
           │
           ▼
    ┌──────────────────────────────────────────────────────────┐
    │ events::author::resolve                                   │
    │   TG_AUTHOR env ─▶ `git config user.email` ─▶ "unknown"  │
    └──────────────────────────────────────────────────────────┘
```

Data flow for a status-mutating verb (`tg block`):

```
tg block tg-abc12 --reason "needs review"
    │
    ▼
store.with_lock(|store| {
    let mut items = store.load_active()?;
    let change: StatusChange = items[idx].apply_block(reason);  // mutates item, returns witness
    store.commit_status_change(&items, change)?;                // redeems witness
    //   ├── author::resolve()             → "hamy@example.com"
    //   ├── Event::status_transition(...) → serialize JSON line
    //   ├── events::append::write(...)    → single write(2) + fsync on events.jsonl
    //   └── jsonl::write_atomic(tasks.jsonl, &items)
    Ok(())
})
```

### Component Breakdown

#### `events::record` (new module — `src/events/record.rs`)

**Purpose:** Event struct + serde.

**Shape:**
```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub v: u32,                          // schema version (always 1 on write)
    pub task_id: String,
    pub ts: DateTime<Utc>,               // RFC3339 UTC, microsecond precision
    pub author: String,
    #[serde(rename = "type")]
    pub event_type: EventType,
    pub text: String,                    // may be empty for notes with no text? No — CLI rejects empty notes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<Status>,          // only present when event_type == StatusTransition
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum EventType { StatusTransition, Note }
```

Constructors: `Event::status_transition(task_id, status, text)`, `Event::note(task_id, text)`.

**Why BTreeMap-style deterministic field order isn't needed:** events are append-only and never re-serialized; each line is written once and read back as-is. serde's struct-field order is stable enough.

#### `events::append` (new — `src/events/append.rs`)

**Purpose:** Single-`write(2)` append with fsync. No `BufWriter`.

**API:** `pub fn write(path: &Path, event: &Event) -> Result<(), TgError>`

**Implementation contract:**
1. Serialize event with `serde_json::to_string(event)`; push `'\n'` onto the owned `String`.
2. Check total byte length ≤ 2048. If over, return `TgError::InvalidInput("event line exceeds 2048-byte limit — shorten the text")`.
3. Open file with `OpenOptions::new().create(true).append(true).open(path)` — same pattern as `jsonl::append_to_archive`. No `O_NOFOLLOW`: consistent with the existing task store (an attacker with write access to `.task-golem/` already owns `tasks.jsonl` and source files; symlink-hardening only events is cargo-cult).
4. **Single `write(2)`:** call `file.write(bytes)` directly — **not** `write_all` (which loops) and **not** `writeln!` (which may split formatting from the newline in some impls). If the returned `n < bytes.len()`, treat as `TgError::StorageCorruption(...)` — we refuse to leave a torn line in the file. On local POSIX FS for sub-PIPE_BUF payloads this short-write never happens; the error branch exists only to fail loudly if it somehow does.
5. `file.sync_data()` — fsync data but not metadata (cheaper than `sync_all`; metadata sync not required for durability of the content).

**Rationale:** PRD's concurrency guarantee requires *exactly one* `write(2)` syscall. Pre-formatting the full line in memory and calling `file.write` once (rather than `write_all` or `writeln!`) makes the single-syscall contract explicit at the code-level; the size cap keeps us comfortably under `PIPE_BUF` (4096 on Linux).

**No header line.** `events.jsonl` is header-less so concurrent processes can append without coordinating on a "does the header exist yet" race. Each event carries `"v":1` per line.

#### `events::read` (new — `src/events/read.rs`)

**Purpose:** Scan events for a task_id.

**APIs:**
- `pub fn for_task(path: &Path, task_id: &str) -> Result<Vec<Event>, TgError>` — line-scan, skip-and-warn on malformed lines (like `read_archive`), skip events with unknown `v`, sort by `ts` ascending.
- `pub fn all(path: &Path) -> Result<Vec<Event>, TgError>` — used by doctor.

Readers also tolerate a trailing malformed line (treat as crash tail, warn to stderr once per file).

#### `events::archive` (new — `src/events/archive.rs`)

**Purpose:** Move events for a task_id from `events.jsonl` → `events.archive.jsonl` atomically, as part of the `tg done` flow.

**API:** `pub fn move_for_task(active_path: &Path, archive_path: &Path, task_id: &str) -> Result<usize, TgError>`

**Implementation:**
1. Read all events from `events.jsonl` (lenient).
2. Partition into `(keep, move)`.
3. If `move` is empty, return 0 (no-op fast path).
4. For each event in `move`: `append(archive_path, &event)` (single-syscall append, fsynced).
5. Rewrite `events.jsonl` atomically with `keep` (temp file + rename, same pattern as `jsonl::write_atomic` but without a header — we'll add `events_jsonl::write_atomic` helper).

**Crash windows:**
- After step 4, before step 5: events exist in both files. Doctor's "event-dup across active/archive" check catches this (new check); repair = drop duplicates from active.

Called from `tg done`'s runner under the existing store lock.

#### `events::author` (new — `src/events/author.rs`)

**Purpose:** Resolve the author string for a new event.

**API:** `pub fn resolve() -> String`

**Logic:**
1. `std::env::var("TG_AUTHOR")`: if set and `trim().is_empty() == false`, return the trimmed value.
2. Run `git config user.email` via `std::process::Command`. Suppress stderr; bound wall-clock with a ~2s guard (spawn + `wait_timeout` via an explicit `kill` if exceeded — avoids the pathological case where a `credential.helper` or git hook waits on a TTY). If exit 0 within the guard and stdout trimmed non-empty, return it.
3. Return `"unknown"`.

Never errors. Never panics. Git absence, non-zero exit, or guard-kill all silently fall through.

#### `Store` extensions (modify — `src/store/mod.rs`)

**New paths:**
```rust
pub fn events_path(&self) -> PathBuf { self.project_dir.join("events.jsonl") }
pub fn events_archive_path(&self) -> PathBuf { self.project_dir.join("events.archive.jsonl") }
```

**New chokepoint methods:**

```rust
impl Store {
    /// Redeem a StatusChange witness: emit the status_transition event
    /// (fsynced), then rewrite tasks.jsonl. Call under with_lock.
    pub fn commit_status_change(
        &self,
        items: &[Item],
        change: StatusChange,  // consumed — cannot be reused
    ) -> Result<(), TgError>;

    /// Redeem a StatusChange witness for a `done` transition: emit event,
    /// append to archive.jsonl, move events, rewrite tasks.jsonl without
    /// the done item. Call under with_lock.
    pub fn commit_done(
        &self,
        items: &[Item],            // WITHOUT the done item (already removed by caller)
        done_item: &Item,
        change: StatusChange,
    ) -> Result<(), TgError>;

    /// Append a plain note event. Lock-free — relies on O_APPEND atomicity.
    /// Validates task_id exists in active or archive.
    pub fn append_note(&self, task_id: &str, text: &str) -> Result<Event, TgError>;
}
```

#### `StatusChange` witness (new — `src/model/item.rs` or `src/events/witness.rs`)

```rust
#[must_use = "StatusChange must be redeemed via Store::commit_status_change or commit_done"]
pub struct StatusChange {
    pub(crate) task_id: String,
    pub(crate) new_status: Status,
    pub(crate) text: String,   // blocked_reason for block; empty otherwise
}
```

Private constructor in `crate::events`; `Item::apply_*` methods are modified to return `StatusChange`. The only public way to obtain a `StatusChange` is via `Item::apply_*`. The only way to consume it is via `Store::commit_*`. This is the chokepoint.

Consumption is by value (not `&StatusChange`), so a single witness can't be replayed.

#### CLI commands

- `tg note <id> <text>` (`src/cli/commands/note.rs`) — resolve ID, validate, `store.append_note(...)`. No lock.
- `tg events <id> [--json]` (`src/cli/commands/events.rs`) — resolve ID in active ∪ archive, scan events.jsonl + events.archive.jsonl, print.
- `tg show --events <id>` (`src/cli/commands/show.rs`) — thin alias that calls `tg events` after the existing show output.
- `tg list --blocked` (sugar for `--status blocked`) — trivial flag in `args.rs`.
- `tg doctor` additions — see below.
- `tg archive` (existing) — no CLI change, but internally needs to also move events if the recovery sweep archives any done items.

#### Existing transition runners (modify — `src/cli/commands/transition.rs`)

Each runner's internals change minimally:

```rust
// Before (run_block):
item.apply_block(reason);
let result = item.clone();
store.save_active(&items)?;

// After:
let change = item.apply_block(reason);  // now returns StatusChange
store.commit_status_change(&items, change)?;  // chokepoint
```

For `run_done`:
```rust
// Before:
items[idx].apply_done();
let done_item = items[idx].clone();
store.append_to_archive(&done_item)?;
items.remove(idx);
store.save_active(&items)?;

// After:
let change = items[idx].apply_done();
let done_item = items[idx].clone();
items.remove(idx);
store.commit_done(&items, &done_item, change)?;
// commit_done handles: emit event, append archive, move events, save active.
```

### Data Flow

#### Flow: `tg block tg-abc12 --reason "needs review"`

> Block a task with a reason, leaving a durable record.

1. **Resolve + load** — `with_lock` → `load_active` → resolve `tg-abc12`.
2. **Validate** — `can_transition_to(Blocked)` (existing).
3. **Apply** — `item.apply_block(Some("needs review"))` mutates item in-place and returns `StatusChange { task_id, new_status: Blocked, text: "needs review" }`.
4. **Commit** — `store.commit_status_change(&items, change)`:
   - `author::resolve()` → e.g. `"hamy@example.com"`.
   - Build `Event::status_transition(task_id, Blocked, "needs review")` with current timestamp.
   - Serialize → check length ≤ 2048 → `events::append::write(events_path, &event)` → single write + fsync.
   - `jsonl::write_atomic(tasks_path, &items)` (existing atomic rename).
5. **Return** — the runner prints success.

**Edge cases:**
- Serialized event > 2048 bytes (long reason) → commit fails with `InvalidInput`, task state unchanged, no event emitted. User retries with shorter reason.
- `write(2)` returns less than full length → assertion fails; stderr warning. Task/event state is consistent only if either both complete or neither did. A short write would leak a partial line, caught by doctor's malformed-line check. Practically never happens on local POSIX FS for sub-PIPE_BUF writes.

#### Flow: `tg note tg-abc12 "tried CSS Grid, breaks on iOS Safari"`

> Append a free-text note without changing status.

1. **Resolve** — `store.load_active()` + `store.load_archive_ids()` (lock-free reads, atomic rename guarantees consistent snapshots).
2. **Resolve ID** — in active ∪ archive scope. If ID is ambiguous or missing, error. If ID resolves to archive-only, proceed (notes on archived tasks are allowed — they append to `events.archive.jsonl`? No — actually, we reject notes on archived tasks for now; see Decision below).
3. **Append** — `events::append::write(events_path, &event)` — single write, fsync. No store lock.

**Edge cases:**
- Two `tg note` processes concurrently → both `O_APPEND` writes succeed, both lines present, both ordered by kernel-assigned file position. Timestamps may be out of order by microseconds — `tg events` sorts by `ts` on read.
- Task archives between resolve and append → note lands in `events.jsonl` for a task no longer active. Doctor's "active-events-for-archived-task" check catches and suggests cleanup.

#### Flow: `tg done tg-abc12`

> Mark a task done, archive it, and move its events.

1. **With lock** — `load_active`, resolve, validate.
2. **Apply** — `items[idx].apply_done()` returns `StatusChange`.
3. **Remove from active slice** — `items.remove(idx)`.
4. **Commit done** — `store.commit_done(&items, &done_item, change)`:
   - Emit `status_transition` event to `events.jsonl` (fsynced).
   - `append_to_archive(&done_item)` (existing pattern, fsynced).
   - Rewrite `tasks.jsonl` atomically without the done item.
   - `events::archive::move_for_task(events_path, events_archive_path, &done_item.id)` — moves all events for this task.

**Crash windows:**
- Between event emit and archive append → doctor's "status event says done but task still active" catches; repair = move to archive.
- Between archive append and active rewrite → `items_in_both` check (existing) catches.
- Between active rewrite and events move → doctor's new "events-in-active-for-archived-task" catches; repair = run the move.

#### Flow: `tg events tg-abc12`

> Show chronological event log.

1. **Resolve ID** — in active ∪ archive.
2. **Scan both files** — `events::read::for_task(events_path, id)` + `events::read::for_task(events_archive_path, id)`.
3. **Merge + sort** by `ts` ascending.
4. **Render:**
   - Human mode (default):
     ```
     TIMESTAMP             AUTHOR              TYPE                STATUS   TEXT
     2026-04-15T14:30:22Z  hamy@example.com    status_transition   blocked  needs review
     2026-04-15T14:31:05Z  hamy@example.com    note                —        tried CSS Grid, breaks on iOS Safari
     ```
     Text is sanitized: strip C0 (0x00–0x1F except `\t`) and C1 (0x80–0x9F) bytes before printing to a TTY. When stdout is piped, emit raw text (caller owns escaping).
   - `--json` mode: NDJSON, one event per line, same shape as on-disk (with `v`, `ts`, `author`, `type`, `status?`, `text`).
5. **Empty log** → exit 0, no output.

### Key Flows summary

| Flow | Takes store lock? | Writes tasks.jsonl? | Writes events.jsonl? | Writes archive.jsonl? |
|------|-------------------|---------------------|----------------------|------------------------|
| `tg do/todo/block/unblock` | yes | yes (rewrite) | yes (append) | no |
| `tg done` | yes | yes (rewrite, remove) | yes (append, then rewrite) | yes (append) + events.archive (append) |
| `tg note` | **no** | no | yes (append) | no |
| `tg events` | no (read-only) | no | no | no |
| `tg archive` (recovery) | yes | yes | yes (rewrite) | yes + events.archive (append) |
| `tg doctor` | yes (if `--fix`) | yes (if `--fix`) | possibly | possibly |

---

## Technical Decisions

### Decision: Header-less `events.jsonl`

**Context:** `tasks.jsonl` uses a `{"schema_version":1}` header line. Should `events.jsonl` follow suit?

**Decision:** No. Each event carries its own `"v":1` field.

**Rationale:**
1. Concurrent appends from N processes would otherwise need to coordinate on "does the header exist yet" — either via a lock (defeats O_APPEND) or racy `create_new`-style logic.
2. Per-line `v` gives per-event schema evolution for free; unknown `v` lines are silently skipped (forward-compat per PRD).
3. First write to a nonexistent `events.jsonl` is a simple `O_APPEND | O_CREAT` open — no special-case.

**Consequences:** Slightly more bytes per line (`"v":1,`). Readers must handle the empty-file case cleanly (return empty vec, don't error).

### Decision: The chokepoint is `StatusChange` consumption, embodied by two methods

**Context:** PRD mandates "a single chokepoint — a mutation function all status-changing code paths must route through." We have two chokepoint methods (`commit_status_change` and `commit_done`), not one. Is that still "single"?

**Decision:** The chokepoint is the *act of consuming a `StatusChange` witness*. `commit_status_change` and `commit_done` are the two shapes of that consumption (active-only vs. active+archive). No other method accepts a `StatusChange`. The regression test asserts every public status-mutating verb emits exactly one `status_transition` event — satisfying PRD's intent.

**Rationale:** Forcing `tg done` through `commit_status_change` would require an awkward "also archive this, actually" second call; splitting into two redemption methods keeps each one simple. The witness type is the real chokepoint, and it's single.

**Consequences:** A third shape (e.g., `commit_reparent_and_status` — not currently needed) would be a new consumption point; we'd add it, the test suite enforces event emission for all verbs. `tg edit --status <X>` does not currently exist; if added later, it uses `Item::apply_*` → `commit_status_change` (same as every other verb).

### Decision: `StatusChange` compile-time witness

**Context:** PRD mandates a "single internal chokepoint" with regression test. How strictly do we enforce it?

**Decision:** Use a `#[must_use]` witness type (`StatusChange`) returned by `Item::apply_*` and consumed by `Store::commit_*`. Private constructor in `crate::events`. The only way to save tasks after a status change is to redeem the witness.

**Rationale:** The PRD identifies "chokepoint discipline" as a key risk ("a new direct-write path added later could silently break event coverage"). A runtime test catches regressions at CI time; a type-level witness catches them at `cargo check` time — earlier, cheaper, more obvious. Cost: one new type, minor signature changes on `Item::apply_*`.

**Consequences:**
- Adding a new status-mutating code path cannot compile without going through `commit_*`.
- `Item::apply_*` signatures change to return `StatusChange` — callers in `transition.rs` and anywhere else (ripgrep: only `transition.rs` and tests) must update.
- Tests that exercise `Item::apply_*` in isolation (in `item.rs` unit tests) now need to do `let _ = item.apply_do(None)` or drop the witness — trivial.

### Decision: `tg note` is lock-free

**Context:** Should `tg note` acquire the store lock?

**Decision:** No. `tg note` validates the task_id with lock-free reads (atomic rename guarantees consistent snapshots), then appends to `events.jsonl` via single-syscall O_APPEND.

**Rationale:** Notes are the highest-frequency event type (agents scribbling progress). Requiring the store lock would serialize them through the ~5s lock timeout window and make concurrent agents wait on each other. PRD explicitly promises concurrent-append safety, and `O_APPEND` + `PIPE_BUF` gives it without a lock.

**Consequences:**
- A race where task archives between validate and append leaves a note in `events.jsonl` for a non-active task. Doctor catches it. Acceptable.
- `tg note` on an archived task is **rejected** (see Decision below) to avoid this race being exploited as a feature.

### Decision: `tg note` rejects archived tasks

**Context:** Can users append notes to archived tasks?

**Decision:** No. `tg note <id>` requires `id` to resolve to an active task. Archived tasks are read-only for event purposes.

**Rationale:**
- Writing to archived tasks means appending to `events.archive.jsonl` lock-free, racing with `tg archive`'s own writes to that file — the concurrency contract is harder to keep.
- Notes on closed work rarely have durable value; if a user needs to annotate why a task was done, they should do it before archiving.
- Simpler mental model: active events live in `events.jsonl`; archived events live in `events.archive.jsonl`; neither is bidirectional.

**Consequences:** Users who want to annotate an archived task must reopen it (currently not supported — `Done` is terminal). Acceptable given PRD's "no re-opening" posture.

### Decision: `Item::apply_unblock` still emits an event

**Context:** Unblocking restores a prior status — what event type fires?

**Decision:** Emit a `status_transition` event whose `status` is the *restored* status (from `blocked_from_status`, default `Todo`). `text` is empty.

**Rationale:** Unblocking is a status transition from the user's perspective. The event's `status` field reflects the new state, consistent with every other transition. Empty text is fine.

**Consequences:** Nice Have #1 (`tg unblock <id> [text]` optional post-note) becomes a straightforward follow-up: runner emits the status_transition, then emits a separate `note` event. Deferred per PRD.

### Decision: Event-first, then task mutation (reaffirming PRD)

**Context:** Write ordering for durability.

**Decision:** Always fsync the event before mutating the task. A committed event with no task mutation is discoverable (doctor drift check); a committed task mutation with no event is invisible forever.

**Rationale:** Events are the higher-value durability target — they're the history. Task state can be reconstructed/reconciled from the event trail; the reverse is not true.

**Consequences:** On crash between event and task write, the system is in a documented "drifted" state that doctor reports. One fsync per transition (event only) — task writes use atomic rename (already fsynced via `jsonl::write_atomic`).

### Decision: No `O_NOFOLLOW` on event files (diverges from PRD NFR)

**Context:** PRD's Security NFR says "Event files are opened with `O_NOFOLLOW` (or equivalent path-canonicalization check) to reject symlink attacks on shared repos."

**Decision:** Skip `O_NOFOLLOW`. Event files are opened with plain `OpenOptions::new().create(true).append(true).open(path)`, identical to the existing `jsonl::append_to_archive`.

**Rationale:** The existing `tasks.jsonl` and `archive.jsonl` — equally or more sensitive — are not protected with `O_NOFOLLOW` today. An attacker with write access to `.task-golem/` can already redirect those files (or modify source). Hardening only the events file is inconsistent security theater. If shared-repo hardening becomes a real requirement, it should be applied uniformly across all four files in one change, not one-off here.

**Consequences:** No `libc` dependency added. Symlink-redirect attacks on `.task-golem/` remain possible, as they already are on the rest of the store. Revisit as a separate hardening change if the threat model shifts.

### Decision: `tg archive --before` does not prune events

**Context:** `tg archive --before DATE` moves old archived tasks from `archive.jsonl` → `archive-pruned.jsonl`. Should their events also be pruned out of `events.archive.jsonl`?

**Decision:** No. `events.archive.jsonl` is cold storage and grows unbounded. Pruned tasks leave their events behind; events retain `task_id` fields that still resolve correctly (doctor's orphan check is tolerant: an event whose task_id is in `archive-pruned.jsonl` is not an orphan).

**Rationale:** PRD explicitly scopes out "event pruning independent of task archival." Implementing it would add a third file (`events.archive-pruned.jsonl`) and another migration path for marginal value — users wanting to reclaim disk space can `rm` the cold archive at their discretion.

**Consequences:** Doctor's orphan check loads `archive-pruned.jsonl` IDs into its "known" set so pruned-task events don't trigger false positives. Trivial addition. Documented.

### Decision: Events do not trigger cache rebuild

**Context:** The SQLite cache (TG-006) lazy-rebuilds when `tasks.jsonl` changes (mtime+size stamp).

**Decision:** `events.jsonl` and `events.archive.jsonl` are not watched by the cache and do not trigger rebuilds. `tg query` cannot see events (PRD Decision 3).

**Rationale:** Events aren't in the cache schema. Watching them would cause spurious rebuilds on every note.

**Consequences:** If a future change adds an `events` table to the cache, the rebuild trigger list grows — clean additive change.

### Decision: Terminal escape sanitization on display only

**Context:** Events may contain arbitrary text including control sequences.

**Decision:** Store raw. Sanitize at display time: when rendering to a TTY in human mode, strip C0 (0x00–0x1F except `\t` and `\n` — and `\n` is already forbidden by JSONL serialization) and C1 (0x80–0x9F) bytes. When `--json` or piped, emit raw.

**Rationale:** Storing sanitized text would lose information; sanitizing at display gives the safety the PRD wants (terminal-escape injection protection) without modifying the durable record. JSON consumers handle their own escaping.

**Consequences:** One small helper `sanitize_for_tty(&str) -> Cow<str>`. Check `atty::is(Stream::Stdout)` once at the top of the human renderer.

### Tradeoffs Accepted

| Tradeoff | We're Accepting | In Exchange For | Why This Makes Sense |
|----------|-----------------|-----------------|----------------------|
| Event-first ordering | A crash can leave a committed event with no task mutation | Durability of the "something was attempted" signal; single-fsync cost | Doctor reconciles drift; history is higher-value than forcing atomic cross-file state |
| Separate events file | Two-file mental model vs. unified | Lock-free concurrent appends; archive shrinks `events.jsonl` | `tg events <id>` hides the join |
| `tg note` lock-free | Theoretical race with archive → note on archived task | No serialization of agent scribbles; matches PRD's "concurrent agents don't wait" | Rare race, doctor catches, easy fix |
| `StatusChange` witness | Small API surface change in `Item::apply_*` signatures | Compile-time enforcement that a new status path can't silently skip events | Matches PRD-identified "chokepoint discipline" risk; cost is trivial |
| `O_APPEND` + 2048-byte cap | Notes truncated past 2048 bytes | No write lock on events; simplest concurrency story on Linux | Agents can retry with shorter text; well under `PIPE_BUF` (4096) for headroom |
| Header-less events file | ~8 bytes/line overhead from per-line `v` | No header-exists race on first write; append stays a single syscall | Events files skew large over time; 8 bytes/line is negligible |
| No cache integration | No `tg query` access to events (v1) | No schema surface area growth in SQLite cache; JSONL scan is under PRD's 200ms budget for ≤500 events | PRD defers explicitly; revisit if `tg query` over events becomes a need |

---

## Alternatives Considered

### Alternative: Inline `events: Vec<Event>` field on `Item`

**Summary:** Store events as an array directly on each `Item` in `tasks.jsonl`.

**How it would work:**
- `Item` grows an `events: Vec<Event>` field.
- Every status change / note does a full `tasks.jsonl` rewrite.
- Archive carries events naturally since they're part of the item.

**Pros:**
- Single source of truth per task; no cross-file joins.
- No new files, no new doctor checks for events-side integrity.
- `tg show` can surface events trivially.

**Cons:**
- Every note/event rewrites the entire `tasks.jsonl` (current size times event frequency = write amplification).
- Concurrent `tg note` calls serialize on the store lock — PRD explicitly wants lock-free concurrent notes.
- Unbounded Item size as events accumulate; at some point `Item` lines exceed reasonable JSONL line sizes and violate the "one line per item" contract.
- Breaks schema compatibility with v1 consumers (cache, `tg query`).

**Why not chosen:** Fails the PRD's "concurrent appends from multiple tg processes do not lose events" requirement without adding a heavy locking story, and the write-amplification cost scales poorly.

### Alternative: SQLite-backed events in the existing cache

**Summary:** Add an `events` table to `cache.db`; writes go through SQLite; `tg query` can join across tasks and events.

**How it would work:**
- New SQLite migration in the existing cache module.
- `tg note` / transitions INSERT rows; `tg events` SELECTs.
- `tg query` can join events with tasks.

**Pros:**
- Rich query surface for free via `tg query`.
- Transactional consistency between event insert and task update (single SQLite write).
- Indexed retrieval for large event histories.

**Cons:**
- Violates TG's existing invariant that SQLite is a *cache* — rebuildable from JSONL at any time. Events would have nowhere to rebuild *from* unless we also keep JSONL, which just moves the two-store problem inside the cache module.
- Adds a migration path; `cache.db` deletion currently loses nothing, but events would be authoritative and couldn't be recovered.
- Cache is gitignored; events need to be durable across developer machines (the PRD implies events are part of the repo's durable state, alongside `tasks.jsonl`). Putting them in the cache breaks that.
- Concurrent writers on SQLite require file-locking (BEGIN IMMEDIATE / busy timeout) — higher latency than O_APPEND.

**Why not chosen:** Events must be durable and portable like tasks are. SQLite-as-authoritative undermines the cache-is-derivable invariant that makes TG-006 clean. PRD Decision 3 already explicitly rules this out.

---

## Risks

| Risk | Impact | Likelihood | Mitigation |
|------|--------|------------|------------|
| `write(2)` short-write on a sub-PIPE_BUF payload (unexpected on local POSIX FS) | Partial line in events.jsonl; reader sees malformed trailing line | Very Low | `file.write()` called once (not `write_all`); treat `n < bytes.len()` as `StorageCorruption`; doctor catches malformed lines |
| `git config user.email` hangs on a credential helper or TTY-blocking hook | `tg note` / transition verbs appear to freeze | Low | Author resolution wraps git with a ~2s wall-clock guard; kill and fall through to `"unknown"` on timeout |
| `StatusChange` witness is workaround-able (e.g., constructing via `unsafe` or reflection) | Chokepoint bypassed | Very Low | Private constructor; integration test still runs every verb and asserts event emission (belt + suspenders) |
| `events.archive.jsonl` concurrent writer during `tg archive` recovery sweep vs. `tg done` on same task | Double-archive, duplicate events | Low | Archive and done both hold the store lock; events archive move happens inside that lock |
| Agents forget to set `TG_AUTHOR` and everything is `unknown` | Reduced provenance value | Medium | Doc prominently; future doctor check deferred per PRD |
| Windows / NFS break concurrency | Torn lines possible | Low (not our platform) | Docs: "Linux-local-FS only for concurrency contract" per PRD |
| Long existing `events.jsonl` slows `tg events <id>` for tasks with few events | `tg events` p95 creep | Low | `tg archive` shrinks the active file as tasks complete; doctor reports line counts; revisit with cache plan if >500 events common |
| New `--events` / `--blocked` flags collide with future semantics | Minor UX breakage if reused | Low | Names are specific and idiomatic; document |

---

## Integration Points

### Existing Code Touchpoints

- `src/model/item.rs` — `apply_do`, `apply_done`, `apply_todo`, `apply_block`, `apply_unblock` change signatures to return `StatusChange`. Internal unit tests updated to drop/consume the witness.
- `src/cli/commands/transition.rs` — each runner swaps `save_active(&items)` for `commit_status_change(&items, change)`; `run_done` swaps for `commit_done`.
- `src/cli/commands/archive.rs` — the recovery sweep that promotes stale done items also calls `events::archive::move_for_task` for each.
- `src/cli/commands/doctor.rs` — adds checks: `events_malformed`, `events_drift_status_mismatch`, `events_orphan`, `events_in_active_for_archived_task`. Repair story for each.
- `src/cli/commands/show.rs` — adds `--events` flag that appends a "Events" section via the shared `tg events` render path.
- `src/cli/args.rs` — adds `Note { id, text }` command, `Events { id, json }` command, `--events` on `Show`, `--blocked` on `List`.
- `src/store/mod.rs` — new `events_path`, `events_archive_path`, `commit_status_change`, `commit_done`, `append_note` methods.
- `src/lib.rs` — `pub mod events;`.
- `README.md`, `CLAUDE.md` — documentation additions (`TG_AUTHOR`, `tg note`, `tg events`, archived-task rule).

### Cargo.toml

No new dependencies. `chrono`, `serde`, `serde_json` already present. `std::io::IsTerminal` (stable since 1.70) covers TTY detection. The 2s guard on `git config` uses a small in-tree helper (spawn thread, join with timeout, kill on miss) rather than pulling in `wait-timeout`.

### External Dependencies

- `git` binary for `author::resolve` — optional; silent fall-through on absence (same pattern as doctor already uses).

---

## Open Questions

None — all PRD-flagged items resolved by Decisions above.

---

## Design Review Checklist

- [x] Design addresses all PRD must-have requirements
  - [x] `events.jsonl` append-only, co-located with `tasks.jsonl`
  - [x] Event record schema v1 matches PRD
  - [x] Single chokepoint via `StatusChange` witness + `Store::commit_*`
  - [x] Event-first ordering with fsync
  - [x] `tg note <id> <text>` with 2048-byte cap
  - [x] `tg events <id>` with `--json` and chronological ordering
  - [x] Author resolution: TG_AUTHOR → git → "unknown"
  - [x] Single `write(2)` via raw `File` (no `BufWriter`), asserted
  - [x] Archive integration moves events alongside tasks
  - [x] Default UX unchanged (no events in `tg show` / `tg list` unless opted in)
- [x] Key flows documented (block, note, done, events, archive)
- [x] Tradeoffs explicitly documented and acceptable
- [x] Integration points with existing code identified (transition, archive, doctor, show, args, store, item)
- [x] No major open questions

---

## Design Log

| Date | Activity | Outcome |
|------|----------|---------|
| 2026-04-15 | Initial design draft | Medium-mode design with `StatusChange` witness chokepoint; lock-free `tg note`; separate `events.jsonl` + `events.archive.jsonl` |
| 2026-04-15 | Self-critique + auto-fix | Tightened `write(2)` single-syscall assertion; clarified archive flow crash windows; added archived-task note rejection decision; added C0/C1 sanitization decision |
| 2026-04-15 | Self-critique round 2 (post-save) | Replaced `write_all` with single `file.write` + short-write error; clarified "one chokepoint = two redemption shapes"; documented archive-pruned + events interaction; noted events don't trigger cache rebuild; added git-hang risk with 2s guard |
| 2026-04-15 | Review pass (HAMY) | Dropped `O_NOFOLLOW` / `libc` dep — symlink hardening would be inconsistent with existing unprotected `tasks.jsonl` / `archive.jsonl`; revisit uniformly if shared-repo hardening becomes a real threat model |
