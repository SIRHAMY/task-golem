# Change: Task Events Log (and `blocked` Convention Hardening)

**Status:** Proposed
**Created:** 2026-04-15
**Author:** HAMY

## TL;DR

- **Problem:** Agents hit verification ceilings (e.g. UI/e2e checks) or external blockers, and there's no first-class way to surface "stopped here, here's why" or to leave durable context for the next session. The `blocked` status already exists but lacks the scratchpad/history needed to make it useful across sessions.
- **Solution:** Add an append-only `events.jsonl` log that captures status transitions and free-text notes, with narrow CLI verbs (`tg note`, `tg events`) and a single chokepoint so every status mutation emits an event automatically. Archive events alongside tasks so the file doesn't grow unbounded.
- **Key criteria:** `tg events <id>` shows what was tried and why it stalled; events are append-safe under concurrent agent writes; `tg archive` moves a task's events into archival storage; defaults stay quiet (events hidden from `tg show` unless asked).
- **Pre-existing context:** `Status::Blocked`, `blocked_reason`, `blocked_from_status`, `tg block`, `tg unblock`, `tg list --status blocked`, and prior-status restoration on unblock are already shipped. This change assumes and builds on them; it does not re-deliver them.

## Problem Statement

Task Golem positions itself as "agent-native." In practice, agents now drive most task execution, but two recurring friction points have surfaced:

1. **Partial completion with verification gaps.** Agents can usually verify their own work via tests, type-checking, or lint. But some changes (UI/frontend tweaks, integration behaviors that need a real environment, judgment calls on copy/UX) cannot be fully verified by the agent. The `blocked` status exists, but there is no durable place to say *what was attempted* and *what still needs eyes* beyond the single `blocked_reason` line — so agents either over-claim success, or leave vague reasons like "needs review" with no context.

2. **No durable scratchpad on a task.** When work spans sessions, or multiple agents touch the same task, there is no place to leave context: what was attempted, what was learned, why a decision was made. Agents either bury this in commit messages (where it's hard to find later) or rewrite the task description (which mixes intent with history and creates merge-fragile prose).

Both gaps push critical context out of `tg` and into commit messages and task descriptions where it isn't queryable. Humans don't know which tasks need them; agents resuming a task cannot reconstruct prior reasoning.

## User Stories / Personas

- **Agent (autonomous executor)** — Wants to leave structured breadcrumbs when blocking a task (what was tried, why it stalled) so a future session (agent or human) can pick up cleanly.
- **Human collaborator** — Wants to read a task's history without running `git log`, and to see structured context on blocked tasks at a glance.
- **Future-self / second agent** — Wants to land on a task and immediately understand prior context: what was tried, why it stalled, what decisions were made.

## Desired Outcome

When complete, a Task Golem user can:

- Append a free-text note to a task at any time (`tg note <id> "tried approach X, hit error Y"`).
- View a task's full event history on demand (`tg events <id>`), including all status transitions and notes, with author and timestamp on each entry.
- Trust that existing status transitions (`tg do`, `tg done`, `tg block`, `tg unblock`, `tg edit --status …`) automatically emit a status-transition event — no opt-in required.
- Trust that two agents appending events to the same task at nearly the same instant cannot lose data.
- Archive a task and have its events move into archival storage alongside it (via existing `tg done` → archive flow), keeping the active `events.jsonl` from growing unbounded.

Existing UX is unchanged for users who don't opt in: `tg show` and `tg list` do not display events by default.

## Success Criteria

### Must Have

- [ ] New `events.jsonl` file co-located with `tasks.jsonl`, append-only during normal operation.
- [ ] Event record schema (committed; each event is one JSON line):
  ```
  {"v":1,"task_id":"tg-…","ts":"2026-04-15T14:30:22.123456Z","author":"hamy@example.com","type":"status_transition","text":"needs manual browser verification","status":"blocked"}
  {"v":1,"task_id":"tg-…","ts":"2026-04-15T14:31:05.987654Z","author":"hamy@example.com","type":"note","text":"tried CSS Grid, layout breaks on iOS Safari"}
  ```
  - `v`: schema version (integer, starts at `1`). Readers MUST ignore events with `v` they don't understand (forward-compat).
  - `ts`: RFC3339 UTC with microsecond precision.
  - `type`: closed enum `status_transition | note` (adding variants is a schema-breaking change — bumps `v`).
  - `status`: only present when `type == status_transition`; names the new status.
  - `text`: free-form. Empty string permitted for notes (CLI rejects at input; see NFR).
- [ ] Single internal chokepoint — a mutation function all status-changing code paths must route through — that writes the task change and emits the corresponding `status_transition` event. Enforced by a regression test that drives every public status-mutating verb and asserts an event appears.
- [ ] Event-first ordering on every status mutation: append event to `events.jsonl` (fsynced), then mutate `tasks.jsonl`. If the task mutation fails, the event remains as evidence of the attempt; `tg doctor` reconciles drift between most-recent status event and current task state.
- [ ] `tg note <id> <text>` appends a `note` event without changing status. Rejects if `<id>` is not found in active tasks. Text limit: 2048 bytes on the serialized JSON line (post-serialization check); overflow returns a clear error with a suggestion to shorten.
- [ ] `tg events <id>` prints the chronological event log (oldest first) for a task — active or archived. Human-readable default. `--json` emits newline-delimited JSON matching the on-disk shape. Empty event log → silent exit 0. Walks both `events.jsonl` and `events.archive.jsonl` as needed.
- [ ] Author resolution per event: `TG_AUTHOR` env var (non-empty after trim) → `git config user.email` (if `git` present and exits 0) → `"unknown"`. Never errors on missing/failed git. Documented as self-reported / advisory, not cryptographically attributed.
- [ ] Concurrent appends from multiple `tg` processes do not lose events. Guarantee rests on POSIX `O_APPEND` atomicity for single-syscall writes under PIPE_BUF; implementation constraint: events MUST be written as a single `write(2)` (no `BufWriter`). Enforced by a concurrency regression test that spawns N writers and asserts no torn or lost lines.
- [ ] `tg archive` / `tg done` (the existing archive flow): when a task moves from `tasks.jsonl` to `archive.jsonl`, its events move from `events.jsonl` to `events.archive.jsonl` in the same operation. The active `events.jsonl` shrinks as tasks archive. `events.archive.jsonl` is append-only from the perspective of archive-moves; neither file is edited in place.
- [ ] Default UX unchanged: `tg show` does not include events unless `--events` is passed; `tg list` output is unchanged.

### Should Have

- [ ] `tg show --events <id>` as an alternative surface (calls into the same code path as `tg events <id>`; one is the canonical implementation, the other is a thin alias — `tg events <id>` is canonical).
- [ ] `tg doctor` checks:
  - `events.jsonl` and `events.archive.jsonl` for malformed JSON lines (report file + line number; readers skip-and-warn at runtime, doctor escalates to error).
  - Drift between most-recent `status_transition` event and current `task.status` (indicates a crashed mid-mutation; report for manual reconciliation).
  - Orphan events: events whose `task_id` is absent from both active tasks and archive (should be rare post-archive-integration; catches bugs).
- [ ] `--blocked` flag on `tg list` as a sugar alias for `--status blocked`.
- [ ] Documentation updates: README quickstart shows `tg note` / `tg events`; CLAUDE.md / agent-facing docs explain the `TG_AUTHOR` convention.

### Nice to Have

- [ ] `tg unblock <id> [text]` accepts an optional note that lands as a separate `note` event after the `status_transition`.
- [ ] `tg events <id> --since <duration>` filter. Defer — `tg query` already exists for arbitrary time filtering via SQL, so this is pure sugar.

## Scope

### In Scope

- New `events.jsonl` and `events.archive.jsonl` files.
- Event record schema (v1) and serialization.
- Chokepoint function through which all status mutations flow.
- Event-first write ordering and associated durability contract.
- New CLI verbs: `tg note`, `tg events`.
- Author resolution (env → git → unknown).
- Archive integration: events move with their task on archive.
- `tg doctor` checks for events integrity and task/event drift.
- Documentation updates.
- `--blocked` alias on `tg list` (Should Have).

### Out of Scope

- Re-delivering the `blocked` status, `blocked_reason` field, or `tg block`/`tg unblock` verbs — these are already shipped.
- A full comments subsystem (threading, replies, mentions, reactions).
- Event pruning independent of task archival. Active events shrink via archive; historical archives grow but are considered cold storage.
- Distinguishing typed event categories beyond `status_transition` and `note`. No `comment`, `system`, `error` taxonomy.
- Event editing or deletion. Events are immutable once written; corrections happen by appending a new event.
- Multi-author conflict resolution beyond what `O_APPEND` atomicity gives us. No locking, no merge logic.
- A `review` workflow status. Modeled as a special case of `blocked` (`tg block <id> "needs review"`); revisit if patterns emerge that justify a dedicated status.
- Webhooks, notifications, or any push-based "task became blocked" alerting.
- A UI / TUI for browsing events. CLI only.
- Windows support for the concurrency guarantee. Linux-like POSIX filesystems only (see Constraints).
- SQLite cache integration for events. `tg events <id>` scans JSONL on demand; no event table in the cache. Revisit if `tg query` over events becomes a hard requirement — deferred as a separate change.

## Non-Functional Requirements

- **Performance:**
  - Appending an event completes in well under 50ms on a local SSD for task stores up to ~10,000 active tasks. Fsync is performed on the event append only (not on every buffered write); crash-loss window is one event at most.
  - `tg events <id>` returns in under 200ms for tasks with up to 500 events on local SSD. JSONL-scan performance is the bound; if it degrades in practice, revisit with a cache plan.
- **Concurrency / Durability:**
  - Two `tg` processes writing events for the same or different tasks at the same instant must not corrupt the file or lose events under single-`write(2)` appends within PIPE_BUF (4096 bytes on Linux).
  - Serialized JSON lines are capped at 2048 bytes total (well under PIPE_BUF to leave headroom); the CLI rejects with a clear error on overflow after serialization (agents retry with a shorter note rather than silently lose information).
  - Implementation constraint: events are written with a raw `std::fs::File` using `writeln!`, producing a single `write()` syscall. No `BufWriter` on the append path.
- **Backward compatibility:**
  - Existing repos without `events.jsonl` / `events.archive.jsonl` continue to work; files are created on first event write.
  - Readers tolerate a trailing malformed line (treat as a crash tail, warn to stderr); non-trailing malformed lines are a hard error surfaced by `tg doctor`.
  - Schema evolution: readers ignore unknown `v` values (forward-compat); writers always use current `v`.
- **Security:**
  - Event files are opened with `O_NOFOLLOW` (or equivalent path-canonicalization check) to reject symlink attacks on shared repos.
  - Event text is escaped when rendered to a TTY (strip or escape C0/C1 control sequences) to block terminal-escape injection.
  - `events.jsonl` and `events.archive.jsonl` are part of the repo's durable state; users should not paste secrets into notes. Documented.
- **Observability:** No new metrics or logs required. `tg doctor` covers integrity surface area.

## Constraints

- Must integrate with the existing JSONL task store (no migration to a new format).
- Must remain CLI-first; no daemon, no server.
- Must not require any external dependencies beyond what's already in `Cargo.toml` unless strongly justified.
- **Platform:** Linux and other Unix-like POSIX systems with atomic `O_APPEND` semantics on local filesystems. Windows and networked filesystems (NFS, SMB) are out of the concurrency contract; `tg` may still run on them but concurrent writes are undefined.
- **Git as an optional runtime dep:** `git config user.email` is consulted for author resolution; `git` absence or failure falls through to `"unknown"` silently. Not a hard dependency.

## Dependencies

- **Depends On:** None. This change does not depend on the SQLite cache from TG-006 — events are scanned from JSONL on demand. (If a future change wants events in the cache, that's a separate design.)
- **Blocks:** None currently identified.

## Risks

- [ ] **Event/task drift on crash between the two writes.** Event-first ordering means a committed event may have no corresponding task mutation. *Mitigation:* `tg doctor` compares most-recent status event to current task state and reports drift; users reconcile manually. This is the acknowledged trade for not building a cross-file transaction.
- [ ] **Chokepoint discipline.** Every status-mutating code path must route through the chokepoint to emit events. A new direct-write path added later could silently break event coverage. *Mitigation:* regression test that exercises every public status-mutating CLI verb and asserts event emission; internal API surface structured to make the non-chokepoint path hard to reach.
- [ ] **`O_APPEND` / PIPE_BUF assumption.** POSIX `O_APPEND` atomicity for sub-PIPE_BUF writes holds on common Linux filesystems (ext4, xfs) but is weaker or absent on some network filesystems and on Windows. *Mitigation:* document as Linux-local-FS-only; cap payloads at 2048 bytes; `tg doctor` can detect malformed lines if the assumption breaks. A future `BufWriter` refactor would silently break this — called out as an implementation constraint and test.
- [ ] **Author convention drift.** If agents fail to set `TG_AUTHOR`, every event lands as `git config user.email` (or `"unknown"`), reducing provenance value. *Mitigation:* document prominently; `tg doctor` could grow a "high fraction of recent events are unknown" warning later if drift becomes real — deferred.
- [ ] **Two-file mental model.** Tasks live in `tasks.jsonl`/`archive.jsonl`, events live in `events.jsonl`/`events.archive.jsonl`. *Mitigation:* `tg events <id>` hides the join; archive integration keeps the files aligned; doctor surfaces inconsistencies.
- [ ] **Archive move is no longer pure-append on `events.jsonl`.** Moving events on archive means reading from `events.jsonl`, writing to `events.archive.jsonl`, and rewriting `events.jsonl` without the moved lines. This is a heavier operation than a pure append and needs a lock (same lock as the archive operation itself). *Mitigation:* archive already holds a write lock on the store; piggy-back on it. Document that archive is not concurrent-safe across writers (it wasn't before this change either).

## Open Questions

None at PRD time. All directional items resolved (see "Decisions" below).

## Decisions

Decisions committed during critique-triage (2026-04-15):

1. **Scope:** events log only. `blocked` status and its field/verb apparatus are already shipped (TG-001 P3, commit `ef78e27`).
2. **Write ordering:** event-first, then task mutation. Drift reconciled by `tg doctor`.
3. **Cache integration:** none. `tg events <id>` scans JSONL. Reconsider if `tg query` over events becomes required.
4. **Reason storage:** `blocked_reason` on the task remains authoritative for "why is this currently blocked"; the status_transition event captures reason-at-transition for history. Both coexist by design.
5. **Platform:** Linux-like POSIX only for concurrency guarantees. Documented.
6. **Payload cap:** 2048 bytes of serialized JSON line; error on overflow.
7. **Archive:** events move to `events.archive.jsonl` when their task archives. Keeps active `events.jsonl` bounded by active-task count. Archive operation must hold the store lock (it already does).
8. **`tg events` output:** human-readable default, chronological ascending; `--json` for NDJSON.

## References

- TG-006 (SQLite cache & query layer) — `changes/TG-006_query-layer-and-nested-tasks/`. Introduced a SQLite cache that mirrors `tasks.jsonl` with lazy rebuild triggered by file stamp (mtime+size) changes. This PRD deliberately does *not* extend it; events are scanned from JSONL on demand.
- TG-002 (extension key collision via serde flatten) — `changes/TG-002_validate-extension-key-collision-with-serde-flatten/`. Relevant context on schema-extension patterns; events are *not* an extension field, but the serde patterns inform implementation.
- TG-001 P3 (state machine + blocked status) — commit `ef78e27`. Delivered the `Blocked` status variant, `blocked_reason`, `blocked_from_status`, `tg block`, `tg unblock`, and the transition matrix that this change builds on.
- Pre-PRD discussion (2026-04-15) — design axes considered, alternatives ruled out (orthogonal `needs_human` flag, tags, child-task workaround, embedded-events-in-task), and rationale for separate `events.jsonl` over inline.
- Critique synthesis (2026-04-15) — this file's "Needs Attention" section was resolved and the resulting decisions are captured in "Decisions" above.
