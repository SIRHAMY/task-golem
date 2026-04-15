# SPEC: Task Events Log

**ID:** TG-008
**Status:** Draft
**Created:** 2026-04-15
**PRD:** ./TG-008_blocked-status-and-task-events_PRD.md
**Design:** ./TG-008_blocked-status-and-task-events_DESIGN.md
**Execution Mode:** human-in-the-loop
**New Agent Per Phase:** yes
**Max Review Attempts:** 3

## TL;DR

- **Phases:** 5 phases (1 Low-Med, 3 Med, 1 Low-Med)
- **Approach:** Build events as a new `src/events/` module (record/append/read/author) first, then introduce a compile-enforced `StatusChange` witness + two `Store::commit_*` chokepoint methods *and* wire all existing transition runners through it (signature change forces this); Phase 3 adds the archive events-move, chokepoint regression test, and concurrency test; Phase 4 ships `tg note` / `tg events` CLI; Phase 5 adds doctor integrity checks + sugar flags + docs.
- **Key risks:** Chokepoint discipline (regression-tested + type-enforced via `#[must_use]` witness + `#![deny(unused_must_use)]`); `write(2)` single-syscall contract (no `BufWriter`, size cap, injectable-`Write` test seam for short-write path); concurrent-append correctness under `O_APPEND` + `PIPE_BUF` (FS-detected skip on non-local filesystems, never silently `#[ignore]`).
- **Needs attention:** None (all directional items resolved — see Self-Critique Triage Notes below).

## Self-Critique Triage Notes

After drafting, this SPEC went through a 6-agent self-critique (phase completeness, ordering, risk, PRD alignment, testability, scoping). Findings were triaged and either auto-fixed in place or auto-decided with rationale below. Key decisions:

- **Phase 2 scope clarified:** Phase 2 DOES fully wire runners through `commit_*` and DOES emit events on transitions. The prior "Phase 3 does that" wording was a contradictory artifact. Phase 3 adds archive event-move + regression/concurrency tests only.
- **Phase 4 ordering corrected:** Phase 4 requires Phase 3 (some integration tests need events-in-archive fixtures). The earlier "could run in parallel with Phase 3" claim was wrong.
- **`#[must_use]` hardening:** add `#![deny(unused_must_use)]` at crate root; tests consume witnesses via explicit helper rather than `let _ = ...`.
- **Testability seams adopted:** `events::append::write` gains an internal `write_with_writer<W: Write>` for short-write fault injection; `events::author::resolve` takes an injected env reader + command builder so timeout and env paths are deterministically testable.
- **Render helper extracted in Phase 4:** `print_events_human` is designed as a shared helper from the start so Phase 5's `tg show --events` is a pure call, not a refactor.
- **`write_atomic_headerless` location decided:** inline in `events::archive` (no caller outside the events module).
- **`tg edit --status` status:** the PRD mentions it; it does not currently exist. Added as an explicit Context note — the witness enforces correctness by construction if it's ever added.
- **5th doctor check added:** `events_dup_across_active_and_archive` (archive-move crash window); deferred out explicitly would be dishonest given DESIGN identifies the crash window.
- **Concurrency test posture:** never silently `#[ignore]`. Skip-with-visible-reason on non-local FS (detect and assert); N=16 per-PR smoke; stress (N=1000 x 10 runs) deferred to an `xtask` nightly followup.
- **Cargo.toml bump:** committed to 0.2.0 → 0.3.0 in Phase 5 (no longer gated on user confirmation).
- **Events file size:** followup logged for a `tg doctor` size warning once real-world usage shows it mattering.

## Context

TG-008 adds a durable, append-only event log for tasks so agents can leave structured breadcrumbs when work stalls (verification ceilings, blocked-on-external) and so humans can see a task's history without `git log`. The `Blocked` status and `tg block`/`tg unblock` apparatus already shipped in TG-001 P3 (commit `ef78e27`); this change adds only the events log and associated CLI.

The design hinges on three contracts that must survive refactors:

1. **Single chokepoint** — all status mutations must emit an event. Enforced by a `#[must_use]` `StatusChange` witness returned from `Item::apply_*` and consumed (by value) by `Store::commit_status_change` / `commit_done`. Belt-and-suspenders: a regression test drives every public status-mutating verb and asserts exactly one `status_transition` event.
2. **Single `write(2)` on event append** — no `BufWriter`, no `write_all` loops, no `writeln!`. Concurrent-append safety depends on POSIX `O_APPEND` + sub-`PIPE_BUF` payload atomicity. A 2048-byte serialized-line cap (well under Linux's 4096 `PIPE_BUF`) preserves headroom.
3. **Event-first ordering** — fsync the event, then rewrite `tasks.jsonl`. A crash between the two leaves a committed event with no corresponding task mutation, surfaced by a new `tg doctor` drift check. The reverse (task mutation without event) would be invisible forever and is therefore forbidden by construction.

Existing code to study:

- `src/store/jsonl.rs` — `write_atomic`, `append_to_archive` patterns (fsync + atomic rename; append-with-fsync for single items; lenient reader with stderr warnings on malformed lines).
- `src/store/mod.rs` — `with_lock`, `load_active`, `save_active`, `append_to_archive`; store-scoped path accessors.
- `src/cli/commands/transition.rs` — `run_do`, `run_done`, `run_todo`, `run_block`, `run_unblock` all share the same shape: lock → load → resolve → validate → `apply_*` → `save_active`.
- `src/model/item.rs` — `apply_*` methods currently return `()`. Signatures will change to return `StatusChange`.
- `src/cli/commands/doctor.rs` — checks-with-optional-fix pattern; existing backup-on-fix story.
- `src/cli/commands/archive.rs` — recovery sweep that promotes stale done items into `archive.jsonl`.
- `tests/common/mod.rs` — `TestProject` helper (tempfile-isolated, `run_tg_json`).

**PRD deviations documented (for traceability):**

- **`O_NOFOLLOW`:** PRD NFR requires it; DESIGN explicitly drops it (inconsistent with existing unprotected `tasks.jsonl` / `archive.jsonl`). The PRD NFR should be treated as superseded by DESIGN's decision. Revisit as a uniform hardening change across all four files if the threat model shifts.
- **"`events.archive.jsonl` append-only; neither file is edited in place"** (PRD Must-Have #8): precisely, `events.archive.jsonl` is append-only; `events.jsonl` is append-only under concurrent notes/transitions, and atomically *rewritten* (temp-file + rename) during the archive event-move under the store lock. Rename-based rewrite is not in-place mutation but IS a content change; interpret the PRD clause as "neither file is torn by concurrent writers."
- **`tg edit --status`:** PRD Desired Outcome lists this among transitions that auto-emit events; DESIGN notes the verb does not currently exist. SPEC does not add it. If added later, the `StatusChange` witness compile-enforces event emission — no additional work beyond updating the Phase 3 regression test's verb array.
- **`sync_data` vs `sync_all`:** PRD says "fsync"; SPEC uses `sync_data()` which durably persists file content (metadata sync is not required for correctness). Treat as equivalent to the PRD's intent.

## Approach

Five phases, each leaving the codebase green (`just check`):

1. **Events module foundation** — `src/events/{mod,record,append,read,author}.rs` + unit tests. Pure library code, no integration yet.
2. **StatusChange witness + Store chokepoint** — introduce `StatusChange` type; change `Item::apply_*` signatures; add `Store::{commit_status_change, commit_done, append_note, events_path, events_archive_path}`. Update existing unit tests.
3. **Wire transition runners + archive flow** — swap `save_active` for `commit_*` in all 5 transition runners; add `events::archive::move_for_task`; integrate into `tg done` and `tg archive` recovery. Add regression test (every verb emits one event) + concurrency test (N writers, no loss/tears).
4. **`tg note` + `tg events` CLI** — new commands, args, dispatch, human + NDJSON rendering, TTY C0/C1 sanitization. Integration tests.
5. **Doctor checks, sugar flags, docs** — doctor: malformed / drift / orphan / active-events-for-archived-task; `--blocked` alias on `tg list`; `--events` flag on `tg show`; README + CLAUDE.md updates.

**Patterns to follow:**

- `src/store/jsonl.rs:167-213` (`append_to_archive`) — existing single-item append-with-fsync pattern. **Deviation:** `events::append::write` must use a single `file.write()` call (not the loop inside `writeln!`/`write_all`) and `sync_data()` instead of `sync_all()`.
- `src/store/jsonl.rs:66-125` (`read_archive`) — lenient reader with stderr warning on malformed lines; `events::read::for_task` mirrors this.
- `src/store/jsonl.rs:129-159` (`write_atomic`) — temp-file + rename pattern for `events::archive::move_for_task`'s rewrite step.
- `src/cli/commands/transition.rs:97-155` (`run_done`) — archive-first durability ordering; we preserve this.
- `src/cli/commands/doctor.rs` — `Issue { issue_type, severity, message, details }` + fix-under-lock pattern; new checks slot in naturally.
- `tests/common/mod.rs` + `tests/transition_test.rs` — `TestProject` harness. New integration tests use the same pattern.

**Implementation boundaries:**

- Do not modify: `src/cache/` (events deliberately do not participate in the SQLite cache — PRD Decision 3).
- Do not refactor: existing `jsonl.rs` helpers. We add a small new helper (`write_atomic_headerless` or inline in `events::archive`) rather than generalizing `write_atomic`.
- Do not add: `libc` crate, `O_NOFOLLOW`, any cross-platform locking. Linux-local-POSIX is the concurrency contract (DESIGN's "No `O_NOFOLLOW`" decision).
- Do not change: public CLI output for existing commands (`tg show` / `tg list` default output is unchanged — events are strictly opt-in).

## Phase Summary

| Phase | Name | Complexity | Description |
|-------|------|------------|-------------|
| 1 | Events module foundation | Low-Med | `src/events/{record,append,read,author}.rs` with unit tests; no integration yet. |
| 2 | StatusChange witness + chokepoint + runner rewiring | Med | Introduce witness, change `apply_*` signatures, add `Store::commit_*`, rewire all 5 transition runners to emit events. |
| 3 | Archive event-move + regression & concurrency tests | Med | Events move with their task on archive; chokepoint regression test; `O_APPEND` concurrency test. |
| 4 | `tg note` + `tg events` CLI | Med | New commands, args, dispatch, shared render helper, TTY sanitization, integration tests. |
| 5 | Doctor checks, sugar flags, docs | Low-Med | Five new doctor checks, `--blocked` / `--events` flags, README + CLAUDE.md, version bump. |

**Ordering rationale:**

- Phase 1 is pure library code with no callers — safest starting point, unblocks everything else.
- Phase 2 introduces the witness AND rewires all runners. These cannot be separated because changing `Item::apply_*` signatures forces every caller to update simultaneously (the code won't compile otherwise). Events are emitted on transitions at the end of Phase 2; archive event-move is the only remaining piece, which lands in Phase 3.
- Phase 3 closes the archive integration loop and adds the two highest-value tests (chokepoint regression, `O_APPEND` concurrency). It must ship in the same release as Phase 2 to avoid leaving archived tasks with stranded events in `events.jsonl`.
- Phase 4 requires Phases 1-3 (integration tests need events-in-archive fixtures produced by Phase 3's `move_for_task`). The shared `print_events_human` helper is introduced in Phase 4 from the start, so Phase 5's `tg show --events` is a pure call, not a refactor.
- Phase 5 is polish + integrity checks; runs last so all the data paths it inspects exist, and reuses `print_events_human` from Phase 4 + `events::archive::move_for_task` from Phase 3 for its `--fix` repair of the `events_in_active_for_archived_task` check.

**Release boundary:** Phases 2 and 3 should be committed separately but released together (a single crate version bump lands in Phase 5). Do not tag or publish between P2 and P3.

---

## Phases

### Phase 1: Events module foundation

> Ship the library primitives: `Event` record, single-syscall append, lenient reader, author resolution.

**Phase Status:** complete

**Complexity:** Low-Med

**Goal:** Land `src/events/` as a self-contained module with full unit-test coverage, no integration with `Store` or CLI yet. At end of phase, `cargo test` passes and the module compiles but is unused.

**Files:**

- `src/events/mod.rs` — create — module declarations + re-exports (`Event`, `EventType`, `append`, `read`, `author`).
- `src/events/record.rs` — create — `Event` struct, `EventType` enum, constructors (`Event::status_transition`, `Event::note`), serde derives.
- `src/events/append.rs` — create — public `pub fn write(path: &Path, event: &Event) -> Result<(), TgError>` that opens the file and delegates to an internal `fn write_with_writer<W: Write>(w: &mut W, bytes: &[u8]) -> Result<(), TgError>` (the test seam). The public fn: serialize, enforce 2048-byte cap (post-serialization, including trailing `\n`), open with `OpenOptions::new().create(true).append(true).open(path)`, call `write_with_writer` (which calls `w.write(bytes)` ONCE and handles short-write as `StorageCorruption`), then `file.sync_data()`.
- `src/events/read.rs` — create — `pub fn for_task(path: &Path, task_id: &str) -> Result<Vec<Event>, TgError>` + `pub fn all(path: &Path) -> Result<Vec<Event>, TgError>`. Parse each line in two steps: first deserialize a minimal prelude `{ v: u32 }` to check schema version, skip-and-continue on unknown `v`; then deserialize the full `Event`. Line-scan, skip malformed with stderr warn (mirror `read_archive`), sort by `ts` ascending.
- `src/events/author.rs` — create — `pub fn resolve() -> String` (convenience wrapper) + `pub(crate) fn resolve_with(env: &dyn EnvReader, git: &dyn GitProbe) -> String` (testable core). `GitProbe::probe(timeout: Duration) -> Option<String>` runs `git config user.email` and returns `None` on timeout, non-zero, or missing binary. Default production impls: `RealEnv` reads `std::env::var`; `RealGit` spawns `Command`, waits from a scratch thread via mpsc `recv_timeout`, calls `child.kill()` + `child.wait()` (reap) on timeout, returns `None`. Never errors, never panics.
- `src/lib.rs` — modify — add `pub mod events;` AND `#![deny(unused_must_use)]` at crate root (hardens `#[must_use]` on `StatusChange` beyond a warning).
- `src/errors.rs` — modify — confirm `StorageCorruption` and `InvalidInput` variants cover the new error paths (they do — no new variants expected).
- `src/events/record_test.rs` (or inline `#[cfg(test)]`) — serialization roundtrip, `type` field rename, optional `status` serde (absent on `Note`, present on `StatusTransition`), deterministic field order, microsecond-precision regex assertion (on-disk `ts` matches `\.\d{6}Z"` exactly).
- `src/events/append_test.rs` (or inline) — append creates file, second append preserves first, 2048-byte cap boundary tests (2047-byte line accepted, 2048-byte line accepted, 2049-byte line rejected with `InvalidInput`; newline IS counted toward the cap), short-write path via `write_with_writer` + a `TrackingWriter` that returns `Ok(n-1)` and asserts `StorageCorruption`.
- `src/events/read_test.rs` (or inline) — empty file → `Ok(vec![])`, single-event roundtrip, malformed-trailing-line warn-and-ignore, malformed-middle-line warn-and-continue, filters by `task_id`, sorts by `ts`, skips unknown `v` silently (including a `v:99` line with an unknown `type` value — prelude-parse must precede full deserialization).
- `src/events/author_test.rs` (or inline) — uses `resolve_with` with injected fakes: `TG_AUTHOR` set → returned; `TG_AUTHOR` whitespace-only → falls through; git returns email → returned; git times out → falls through to `"unknown"` (test asserts wall-clock ≤ ~timeout+200ms); git absent → `"unknown"`. No global env mutation; no `serial_test` dep needed.

**Patterns:**

- Follow `src/store/jsonl.rs:167-213` for the open-and-append shape, **but swap `write_all` for a single `file.write(bytes)` call** and `sync_all` for `sync_data`.
- Follow `src/store/jsonl.rs:66-125` for lenient-reader pattern (stderr warn once per file, continue past malformed lines).

**Tasks:**

- [x] Scaffold `src/events/mod.rs` + register in `src/lib.rs`. Add `#![deny(unused_must_use)]` to `src/lib.rs`.
- [x] Implement `Event` + `EventType` with serde derives; `#[serde(rename = "type")]` on the `event_type` field; `#[serde(skip_serializing_if = "Option::is_none")]` on `status`. Use a custom serializer for `ts` to pin 6-digit microsecond precision.
- [x] Implement `Event::status_transition(task_id, author, status, text)` + `Event::note(task_id, author, text)` constructors (author injected so tests stay deterministic). Stamp `ts: Utc::now()` and `v: 1`.
- [x] Implement `events::append::write` + internal `write_with_writer<W: Write>` seam. Add a doc-comment explaining the `PIPE_BUF` assumption and why `BufWriter`/`write_all`/`writeln!` are forbidden on the append path.
- [x] Implement `events::read::for_task` + `events::read::all` with lenient parsing, prelude-first version check, stderr warn-once per file.
- [x] Implement `events::author::resolve` + `resolve_with(env, git)` + `EnvReader` / `GitProbe` traits + real impls. Use a documented 2s timeout constant (`DEFAULT_GIT_TIMEOUT: Duration = Duration::from_secs(2)`).
- [x] Write unit tests for each module (see Files). Include boundary tests on the 2048-byte cap and microsecond-precision regex roundtrip.
- [x] Run `just check` → green.

**Verification:**

- [x] `cargo build` succeeds; new module compiles.
- [x] `cargo test` passes including all new unit tests.
- [x] `just check` green (format, lints, tests).
- [x] Grep confirms no `BufWriter`, `write_all`, or `writeln!` in `events::append`.
- [x] Grep confirms `events` module is not yet called from `src/cli/` or `src/store/`.
- [x] Code review passes (`/code-review` → fix issues → repeat until pass).

**Commit:** `[TG-008][P1] Feature: Add events module (record, append, read, author)`

**Notes:**

- 2s git guard implementation: `Command::spawn` the child, start a scratch thread that calls `child.wait_with_output()` and sends the result over `mpsc::channel`; the main thread calls `recv_timeout(DEFAULT_GIT_TIMEOUT)`. On timeout: call `child.kill()` on the retained handle (the `Child` is not moved into the thread — wrap in `Arc<Mutex<Option<Child>>>` or split `ChildStdin`-style, pick what's simplest) AND call `child.wait()` to reap the zombie. Detach the reader thread if kill succeeds. Injected via `GitProbe` trait for testability.
- Microsecond precision on `ts`: use a custom serializer (`ts.format("%Y-%m-%dT%H:%M:%S%.6fZ").to_string()`) rather than chrono's default nanos output. Add a regex assertion in the roundtrip test.
- 2048-byte cap: applies to the full serialized line INCLUDING the trailing newline. Documented explicitly so the CLI error message can be precise ("shorten by N bytes").
- Empty-text notes: library layer accepts empty; CLI rejects in Phase 4.
- Short-write test: use a `TrackingWriter { short_by: usize }` that returns `Ok(len - short_by)`; assert `StorageCorruption` propagates.

**Followups:**

- [ ] [Low] Real `git config user.email` integration test — current coverage uses a `GitProbe` fake; we do not exercise the real `probe_git_with_timeout` path end-to-end. Deferred because it would require shelling out to `git` (environment-dependent) or a more elaborate harness; low value vs. the unit tests over `resolve_with`.
- [ ] [Low] `ts` microsecond-precision assertion uses a hand-rolled structural check rather than a true regex, because `regex` is not a direct dependency of `task-golem`. Revisit if `regex` is added for another reason; otherwise the hand-rolled check covers the same property.

---

### Phase 2: StatusChange witness + chokepoint + runner rewiring

> Introduce the `#[must_use]` `StatusChange` witness returned by `Item::apply_*` and consumed by `Store::commit_*` methods; rewire all 5 transition runners; events are emitted on every status transition by end of phase.

**Phase Status:** complete

**Complexity:** Med

**Goal:** Every status-mutating code path compiles only if it redeems a `StatusChange` via `Store::commit_*`. Public `Item::apply_*` signatures change to return `StatusChange` — this forces all 5 transition runners (`run_do`, `run_done`, `run_todo`, `run_block`, `run_unblock`) to be rewired in this same phase (the code doesn't compile otherwise). After Phase 2, running `tg do/done/todo/block/unblock` appends a `status_transition` event to `events.jsonl`.

**Deferred to Phase 3:** archive event-move (`events::archive::move_for_task`), chokepoint regression test, concurrency test. After Phase 2, archiving a task leaves its events stranded in `events.jsonl` — this intermediate state is closed by Phase 3 and the two phases must ship in the same release.

**Files:**

- `src/events/witness.rs` — create — `StatusChange` with fully private fields + `pub(crate) fn new(...)` constructor and `pub(crate) fn fields(&self) -> (&str, Status, &str)` accessor for `Store::commit_*` to read. `#[must_use = "StatusChange must be redeemed via Store::commit_status_change or commit_done"]`. Also provide `#[cfg(test)] pub fn consume_for_test(self)` so test code consumes witnesses explicitly rather than via `let _ = ...` (which would silence the lint).
- `src/events/mod.rs` — modify — `pub use witness::StatusChange;` and module declaration.
- `src/model/item.rs` — modify — `apply_do`, `apply_done`, `apply_todo`, `apply_block`, `apply_unblock` now return `StatusChange`. Each calls `StatusChange::new(&self.id, new_status, text)` just before returning.
- `src/store/mod.rs` — modify — add `events_path()`, `events_archive_path()`, `commit_status_change(&self, items: &[Item], change: StatusChange)`, `commit_done(&self, items: &[Item], done_item: &Item, change: StatusChange)`, `append_note(&self, task_id: &str, text: &str) -> Result<Event, TgError>`.
- `src/cli/commands/transition.rs` — modify — each runner captures the returned `StatusChange` and passes it to `commit_status_change` / `commit_done` instead of calling `save_active` / `append_to_archive` directly.
- `src/cli/commands/archive.rs` — modify — recovery sweep does not yet call events-move (Phase 3 adds that); the existing `save_active` after the recovery loop is unchanged. No witness flow here because the items being promoted are already in `Done` state (no new transition) — archive recovery does NOT emit events and does NOT back-fill status_transition events for pre-TG-008 tasks (doctor's drift check is the surfacing mechanism for pre-TG-008 task/event mismatches). Document this in the module doc-comment.
- Unit tests in `src/model/item.rs` that call `apply_*` — update to consume the returned `StatusChange` via `change.consume_for_test()` (not `let _ = ...`, which would defeat the `#[must_use]` lint even under `#![deny(unused_must_use)]`).

**Patterns:**

- Follow the `commit_done` example in `DESIGN.md:188-200`. `commit_*` methods take `&self` on `Store`; caller must hold `with_lock`. Document that contract in doc-comments (cannot enforce at type level without redesigning the lock API, which is out of scope).

**Tasks:**

- [x] Add `src/events/witness.rs` with `StatusChange` type and `#[must_use]` attr.
- [x] Update `Item::apply_do/done/todo/block/unblock` signatures to return `StatusChange`. Construct the witness at the end of each method with the correct `new_status` and `text` (`text = blocked_reason` for block, empty string for the rest).
- [x] Add `Store::events_path` and `Store::events_archive_path` accessors.
- [x] Implement `Store::commit_status_change`: resolve author, build `Event::status_transition`, call `events::append::write` (fsynced), then `jsonl::write_atomic(tasks.jsonl, items)`.
- [x] Implement `Store::commit_done`: resolve author, build `Event::status_transition(done)`, call `events::append::write`, call existing `append_to_archive(done_item)`, call `jsonl::write_atomic(tasks.jsonl, items)` (WITHOUT the done item — caller removed it). The events-move to `events.archive.jsonl` lands in Phase 3 (since `events::archive::move_for_task` doesn't exist yet — add a TODO + followup, see Notes below).
- [x] Implement `Store::append_note`: validate `task_id` exists in ACTIVE ONLY via lock-free `load_active` (archived tasks rejected — see Phase 4 CLI enforcement), build `Event::note`, call `events::append::write`. Return the constructed `Event` for CLI output.
- [x] Update all 5 transition runners (`run_do/done/todo/block/unblock`) to consume the witness and call `commit_status_change` / `commit_done`.
- [x] Update `item.rs` unit tests to use `change.consume_for_test()` on returned witnesses.
- [x] Explicitly do NOT write archive-path event tests in Phase 2 (the archive event-move contract lands in Phase 3; tests for it belong there to avoid rewriting assertions).
- [x] Run `just check` → green.

**Verification:**

- [x] `cargo build` succeeds; no `#[must_use]` warnings (which would indicate a dropped witness).
- [x] `cargo test` passes; existing `transition_test.rs` tests still pass (events are now emitted but those tests don't check events yet — Phase 3).
- [x] `just check` green.
- [x] Manual check: `tg do <id>` followed by `cat .task-golem/events.jsonl` shows a status_transition event.
- [x] Grep confirms `apply_*` methods are only called from `transition.rs` and `item.rs` tests.
- [x] Code review passes.

**Commit:** `[TG-008][P2] Feature: Introduce StatusChange witness and Store commit chokepoint`

**Notes:**

- **Intermediate state:** Phase 2 emits events on transitions but does NOT move events on archive (Phase 3). Archiving a task after Phase 2 leaves its events in `events.jsonl`. Phases 2 and 3 must ship in the same release (see release boundary in Phase Summary).
- Doc-comment `commit_status_change` / `commit_done` / `append_note` with "must be called under `with_lock`" (for commit_*) and "lock-free; relies on O_APPEND atomicity" (for append_note).
- `append_note`'s validation reads `load_active` without the lock; a race where the task archives between validate and append is acceptable — the late note becomes an active-events-for-archived-task condition caught by Phase 5's `events_in_active_for_archived_task` doctor check.
- `commit_done` in Phase 2 includes a `TODO: call events::archive::move_for_task` comment to make the Phase 3 handoff unmistakable. Phase 3 removes it.

**Followups:**

- None — the archive integration is tracked as a required Phase 3 task, not a followup.

---

### Phase 3: Wire archive event-move + regression & concurrency tests

> Close the archive integration loop (events move with their task), add the regression test that proves chokepoint discipline, and add the concurrency test that proves `O_APPEND` safety.

**Phase Status:** not_started

**Complexity:** Med

**Goal:** Events follow their task when it archives. A dedicated regression test asserts every public status-mutating verb emits exactly one `status_transition` event. A concurrency test spawns N writer processes and asserts no lost/torn lines.

**Files:**

- `src/events/archive.rs` — create — `pub fn move_for_task(active_path: &Path, archive_path: &Path, task_id: &str) -> Result<usize, TgError>`. Read all events from `active_path` (lenient), partition keep/move, no-op if move is empty, append each moved event to `archive_path` via `events::append::write`, rewrite `active_path` atomically with `keep` (temp file + rename — add a small inline helper or put it in `events/archive.rs` alongside the move logic).
- `src/events/mod.rs` — modify — declare `pub mod archive;`.
- `src/store/mod.rs` — modify — `commit_done` now also calls `events::archive::move_for_task(events_path, events_archive_path, &done_item.id)` as its last step (under the caller's `with_lock`).
- `src/cli/commands/archive.rs` — modify — recovery sweep: for each item promoted from active to archive, call `events::archive::move_for_task` to follow the task's events. (Still no events are *emitted* here — we're just moving existing ones.)
- `tests/events_chokepoint_test.rs` — create — parameterized over the 5 verbs (`do`, `done`, `todo`, `block`, `unblock`). For each: create a task in the appropriate precondition state, run the verb, read `events.jsonl`, assert exactly one new `status_transition` event with the expected `status` and `task_id`. Include a meta-assertion: count `fn apply_` occurrences in `src/model/item.rs` (string-scan the file at test start) and assert it matches the verb array length — if a sixth `apply_*` is added without updating this test, it fails.
- `tests/events_concurrency_test.rs` — create — detect filesystem type at test start (`statfs` on the temp dir); if NOT local POSIX (ext4, tmpfs, xfs, btrfs, apfs), skip the test WITH a visible `eprintln!` explaining why (never silent `#[ignore]`). Otherwise: spawn N=16 `tg note` processes in parallel against the same task, each with a distinct text. After all join, read `events.jsonl`, assert (a) line count == N + setup events, (b) every serialized text appears exactly once, (c) every line parses as valid JSON, (d) no line exceeds 2048 bytes. Loop the assertion block 3x in one test invocation to strengthen signal without becoming a stress test.
- `tests/events_archive_integration_test.rs` — create — three scenarios: (1) happy path — task with 3 events → `tg done` → events moved to archive, active file clean; (2) crash-window dedup — pre-seed `events.archive.jsonl` with 2 of the 3 events (simulating crash between append and rewrite), run `move_for_task` again, assert archive does not contain duplicates (OR assert the `events_dup_across_active_and_archive` doctor check would flag them for repair); (3) recovery sweep — seed a `Done` task still in active with 2 events, run `tg archive`, assert events moved.

**Patterns:**

- `events::archive::move_for_task`'s rewrite step follows `jsonl::write_atomic` (temp file + fsync + rename) but headerless. Factor into `events::archive` as a helper.
- `tests/transition_test.rs` for the `TestProject` harness and ID-resolution pattern.

**Tasks:**

- [ ] Implement `events::archive::move_for_task` with atomic rewrite of the active file (inline helper for the headerless rewrite — do NOT generalize `jsonl::write_atomic`).
- [ ] Wire `commit_done` to call `events::archive::move_for_task` as its last step. Remove the `TODO` comment introduced in Phase 2.
- [ ] Wire `archive.rs` recovery sweep to call `events::archive::move_for_task` for each promoted item.
- [ ] Write the chokepoint regression test with verb-count meta-assertion.
- [ ] Write the concurrency test with FS-type detection + skip-with-reason.
- [ ] Write the archive-integration test (happy path + crash-window dedup + recovery sweep).
- [ ] Run `just check` → green.

**Verification:**

- [ ] `cargo test` passes — especially the three new integration tests.
- [ ] `just check` green.
- [ ] Manual smoke test: create a task, add a note, mark done, run `tg events <id>` (will not work yet — Phase 4), but `cat .task-golem/events.archive.jsonl` shows 2 events for the task.
- [ ] Grep `save_active` usage in `transition.rs` — should be 0 occurrences (all replaced by `commit_*`).
- [ ] Code review passes.

**Commit:** `[TG-008][P3] Feature: Events archive integration + chokepoint/concurrency regression tests`

**Notes:**

- The concurrency test is the single most important verification in this SPEC. If it flakes on a supported FS, the `O_APPEND` assumption is broken — investigate, do not paper over with retry logic or silent `#[ignore]`. On unsupported FS it skips with visible reason.
- Archive recovery does NOT emit new events (the task's status didn't change — it was already `Done`); it only moves existing events. Document this in the archive module.
- Archive-move crash windows are acknowledged: if we append to archive but fail before rewriting active, events exist in both files. Phase 5's `events_dup_across_active_and_archive` doctor check (new — added during triage) catches this; `--fix` drops duplicates from active.
- Concurrency test stress runs (N=1000 × 10 iterations) are out of scope — logged as a Low followup for an `xtask` nightly.

**Followups:**

- [ ] [Low] Add an `xtask` nightly stress run of the concurrency test at N=1000 × 10 iterations.

---

### Phase 4: `tg note` and `tg events` CLI commands

> User-facing surfaces. Add `tg note <id> <text>` and `tg events <id> [--json]`; wire args, dispatch, and rendering with TTY C0/C1 sanitization.

**Phase Status:** not_started

**Complexity:** Med

**Goal:** Agents can append notes and view event history via `tg note` / `tg events`. Human rendering sanitizes terminal-escape sequences; `--json` emits raw NDJSON.

**Files:**

- `src/cli/commands/note.rs` — create — `pub fn run(json_mode: bool, id_input: String, text: String) -> Result<(), TgError>`. Validate non-empty text (reject empty with `InvalidInput`); resolve ID via `id::resolve_id` in active scope (reject archive-only per DESIGN decision); call `store.append_note(task_id, &text)`; print success in either human or JSON mode.
- `src/cli/commands/events.rs` — create — `pub fn run(json_mode: bool, id_input: String) -> Result<(), TgError>`. Resolve ID in active ∪ archive; scan both `events.jsonl` and `events.archive.jsonl` via `events::read::for_task`; merge and sort by `ts`; render.
- `src/cli/commands/mod.rs` — modify — register `pub mod note; pub mod events;`.
- `src/cli/args.rs` — modify — add `Note { id, text }` and `Events { id, json }` variants to `Commands` enum with clap derives.
- `src/cli/mod.rs` — modify — add dispatch arms for the two new variants.
- `src/cli/output.rs` — modify — add `sanitize_for_tty(&str) -> Cow<str>` helper that strips C0 (0x00–0x1F except `\t`; `\n` already impossible in JSONL values) and C1 (0x80–0x9F) bytes. Take an `is_tty: bool` parameter (don't call `IsTerminal` inside the helper) so tests inject the branch.
- `src/cli/output.rs` — modify — add `pub fn print_events_human(events: &[Event], is_tty: bool)` (shared helper; callable from both `tg events` and Phase 5's `tg show --events`). Emits the fixed-column table from DESIGN's Flow section, applying `sanitize_for_tty` when `is_tty`.
- `tests/events_cli_test.rs` — create — `tg note <id> "text"` appends; `tg note` with empty text errors; `tg note` on non-existent ID errors; `tg note` on archived ID errors; `tg events <id>` prints in chronological order; `tg events <id> --json` is valid NDJSON; `tg events <id>` on a task with zero events exits 0 with no output; `tg events` on a task with events in both files (straddling active + archive, with archive events having OLDER timestamps) produces a single chronologically-ascending merged output — use `tg done` to produce realistic fixtures (relies on Phase 3); piped (non-TTY) `tg events` output preserves injected C0 bytes; TTY-mode output strips them (test via the injectable `is_tty` parameter on `print_events_human`).

**Patterns:**

- Follow `src/cli/commands/show.rs` for ID resolution in active ∪ archive and dual-mode rendering.
- Follow `src/cli/commands/transition.rs` for the `run(json_mode, …)` function signature and dispatch.
- Follow `src/cli/args.rs` existing `List { … }` / `Show { … }` variants for clap derive style.
- Follow `src/cli/output.rs`'s existing `print_item_detail` for TTY-aware color rendering (similar pattern for sanitization guard).

**Tasks:**

- [ ] Implement `sanitize_for_tty` + unit tests (C0 stripped, C1 stripped, `\t` preserved, plain ASCII unchanged, Unicode preserved).
- [ ] Implement `cli/commands/note.rs`.
- [ ] Implement `cli/commands/events.rs` (including active ∪ archive scan and merge-sort).
- [ ] Add args variants with clap derives; wire `--json` on `Events`.
- [ ] Add dispatch arms in `src/cli/mod.rs`.
- [ ] Implement `print_events_human` with TTY detection.
- [ ] Write integration tests in `events_cli_test.rs`.
- [ ] Run `just check` → green.

**Verification:**

- [ ] `cargo test` passes including new integration tests.
- [ ] `just check` green.
- [ ] Manual smoke: `tg add "x"` → `tg note <id> "tried approach"` → `tg do <id>` → `tg events <id>` shows both events in order, with the `do` transition second.
- [ ] Manual smoke: `tg events <id> --json | jq .` produces valid JSON.
- [ ] Manual smoke: inject a C0 byte (`tg note <id> $'\x1b[31mred'`) and confirm `tg events <id>` does NOT produce colored output; `tg events <id> --json` preserves the raw byte.
- [ ] Code review passes.

**Commit:** `[TG-008][P4] Feature: Add tg note and tg events CLI commands`

**Notes:**

- Empty text handling: `tg note <id> ""` errors with `InvalidInput("note text cannot be empty")`. This is a CLI-level check; the library (`events::append::write`) is permissive.
- TTY detection: `std::io::stdout().is_terminal()`. Piped / redirected output should get raw bytes so downstream tools (grep, jq, less) see exactly what's on disk.
- Don't expose `--events` on `tg show` in this phase (Phase 5) — keeping surfaces separable.

**Followups:**

- [ ] [Low] Consider column widths in `print_events_human` — long author emails or task IDs may wrap awkwardly. Defer unless real-world usage shows ugliness.

---

### Phase 5: Doctor checks, sugar flags, docs

> Integrity checks for events, `--blocked` alias on `tg list`, `--events` flag on `tg show`, and documentation in README + CLAUDE.md.

**Phase Status:** not_started

**Complexity:** Low-Med

**Goal:** `tg doctor` reports events-side integrity issues. `tg list --blocked` works. `tg show <id> --events` includes the event log in the detail view. README and CLAUDE.md document the new workflow.

**Files:**

- `src/cli/commands/doctor.rs` — modify — add five checks:
  1. `events_malformed` (error) — malformed JSON lines in `events.jsonl` / `events.archive.jsonl`. Report file + line number. Repair: none automatic.
  2. `events_drift_status_mismatch` (warning) — most-recent `status_transition` event for a task disagrees with current `task.status`. Report: expected status, actual status, event timestamp. Repair: none automatic (suggest running the verb to reconcile).
  3. `events_orphan` (warning) — event whose `task_id` is absent from active + archive + `archive-pruned.jsonl`. The pruned set MUST be loaded into the "known" set to avoid false positives. Report: task_id, event count. Repair: none automatic.
  4. `events_in_active_for_archived_task` (warning) — event in `events.jsonl` for a task that's in `archive.jsonl`. Report: task_id, event count. Repair (fix mode): move the events via `events::archive::move_for_task`.
  5. `events_dup_across_active_and_archive` (warning) — event line (matching on `task_id` + `ts` + `text`) present in both `events.jsonl` and `events.archive.jsonl`. This indicates a crash during `move_for_task` between the archive-append and active-rewrite steps. Repair (fix mode): drop duplicates from active.
- `src/cli/args.rs` — modify — add `--blocked` flag on `List` (sugar for `--status blocked`), add `--events` flag on `Show`.
- `src/cli/commands/list.rs` — modify — honor `--blocked` as `--status blocked` (reject combining both with `InvalidInput`).
- `src/cli/commands/show.rs` — modify — when `--events` is set, after the existing detail output, call `output::print_events_human` (the shared helper introduced in Phase 4). No refactoring needed — Phase 4 designed the helper for this reuse.
- `tests/doctor_test.rs` — modify (or create `doctor_events_test.rs`) — seed each of the five conditions via a new test helper `TestProject::seed_raw_events(active: &[Event], archive: &[Event])` that writes fixtures directly (driving the CLI cannot reliably produce the drift/dup states). Assert each check fires. Assert `--fix` repairs `events_in_active_for_archived_task` (moves to archive) and `events_dup_across_active_and_archive` (drops dups from active), and leaves the others untouched.
- `tests/list_test.rs` — modify — assert `tg list --blocked` and `tg list --status blocked` produce identical output; assert combining both errors.
- `tests/show_test.rs` — modify — assert `tg show <id>` does NOT include events by default; `tg show <id> --events` includes them in the output.
- `README.md` — modify — add a short "Events" section to the feature list / quickstart, covering `tg note`, `tg events`, the `TG_AUTHOR` convention, and a one-line "don't paste secrets into notes — `events.jsonl` is part of the repo's durable state" warning.
- `CLAUDE.md` — modify — add agent-facing guidance: when to use `tg note` (stalls, verification ceilings, context-for-next-session), the `TG_AUTHOR` env var, the "notes on archived tasks are rejected" rule, the 2048-byte cap, and the "don't paste secrets" warning.
- `Cargo.toml` — modify — bump version 0.2.0 → 0.3.0 (committed decision — user-facing feature addition warrants a minor bump).

**Patterns:**

- Follow `src/cli/commands/doctor.rs:30-196` for check-and-fix structure, `Issue` struct, and backup-on-fix.
- Follow `src/cli/commands/list.rs`'s existing `--status` handling.
- Follow the existing README feature section for style.

**Tasks:**

- [ ] Implement the five doctor checks. Explicit sub-task: load `archive-pruned.jsonl` IDs into the orphan-check "known" set.
- [ ] Wire `--fix` repairs for `events_in_active_for_archived_task` (move via `events::archive::move_for_task`) and `events_dup_across_active_and_archive` (drop dups from active).
- [ ] Add `--blocked` flag + list.rs handling (reject `--blocked` combined with `--status`).
- [ ] Add `--events` flag + show.rs handling (calls shared `print_events_human` from Phase 4 — no refactor).
- [ ] Add `TestProject::seed_raw_events` helper for doctor test fixturing.
- [ ] Write doctor integration tests (seed each of the 5 conditions; assert `--fix` repairs the two that have repairs).
- [ ] Write list / show integration tests (including byte-identical output between `tg events <id>` and the events section of `tg show <id> --events`).
- [ ] Update README.md (including secrets warning).
- [ ] Update CLAUDE.md (including secrets warning).
- [ ] Bump Cargo.toml version 0.2.0 → 0.3.0 in a separate final commit for clean revert.
- [ ] Run `just check` → green.

**Verification:**

- [ ] `cargo test` passes; new doctor/list/show tests pass.
- [ ] `just check` green.
- [ ] Manual smoke: corrupt `events.jsonl` with a malformed middle line → `tg doctor` reports it.
- [ ] Manual smoke: `tg list --blocked` matches `tg list --status blocked`.
- [ ] Manual smoke: `tg show <id>` unchanged from before this change (no events); `tg show <id> --events` appends event log.
- [ ] Docs updated: README has `tg note`/`tg events` examples; CLAUDE.md has agent guidance.
- [ ] Code review passes.

**Commit:** `[TG-008][P5] Feature: Doctor events checks, --blocked/--events flags, docs`

**Notes:**

- The drift check requires scanning all events for each task's most-recent `status_transition`; cost scales with total event count. For typical active-task populations (<100 tasks, <500 events) this is fine. If doctor wall-clock becomes a concern, index events by task_id in-memory during the check rather than re-scanning.
- Orphan check tolerance: per DESIGN, events for pruned tasks (in `archive-pruned.jsonl`) are NOT orphans. Load pruned IDs into the "known" set.
- Terminal sanitization already lives in Phase 4; no new sanitization work here.

**Followups:**

- [ ] [Low] Consider a `tg doctor --events-only` scope flag if the full `tg doctor` pass becomes too slow. Defer until real-world usage.

---

## Final Verification

- [ ] All 5 phases complete and committed.
- [ ] All PRD must-have success criteria satisfied:
  - [ ] `events.jsonl` co-located with `tasks.jsonl`, append-only in normal operation.
  - [ ] Event schema v1 matches PRD (verified by roundtrip test).
  - [ ] Single chokepoint enforced by `StatusChange` witness + regression test.
  - [ ] Event-first ordering with fsync.
  - [ ] `tg note <id> <text>` with 2048-byte cap, appends `note` event without status change.
  - [ ] `tg events <id>` shows chronological log, `--json` emits NDJSON.
  - [ ] Author resolution: `TG_AUTHOR` → git → "unknown".
  - [ ] Concurrent appends safe under `O_APPEND` + `PIPE_BUF` (verified by concurrency test).
  - [ ] Archive integration: events move on `tg done` / `tg archive` recovery.
  - [ ] Default UX unchanged: `tg show` / `tg list` output identical without the new flags.
- [ ] Should-have items:
  - [ ] `tg show --events` works.
  - [ ] `tg doctor` has the four new checks.
  - [ ] `tg list --blocked` sugar works.
  - [ ] README + CLAUDE.md updated.
- [ ] `just check` passes cleanly on the final commit.
- [ ] No regressions in existing tests.
- [ ] Code reviewed.

## Execution Log

| Phase | Status | Commit | Notes |
|-------|--------|--------|-------|

## Followups Summary

### Critical

<!-- None expected pre-implementation. -->

### High

<!-- None expected pre-implementation. -->

### Medium

<!-- None expected pre-implementation. -->

### Low

- [ ] `print_events_human` column widths may need tuning after real-world use.
- [ ] `tg doctor --events-only` scope flag if full-doctor becomes slow.
- [ ] `xtask` nightly stress run of the concurrency test at N=1000 × 10 iterations.
- [ ] `tg doctor` size warning when `events.jsonl` exceeds 10MB (revisit after real-world usage).
- [ ] PRD Nice-to-Have: `tg unblock <id> [text]` — optional post-unblock note event. Trivial after `append_note` ships.

## Design Details

See `TG-008_blocked-status-and-task-events_DESIGN.md` for:

- Full component breakdown (`events::record`, `events::append`, `events::read`, `events::archive`, `events::author`, `StatusChange` witness).
- Data flow diagrams for `tg block`, `tg note`, `tg done`, `tg events`.
- Tradeoffs table (event-first ordering, separate events file, lock-free notes, witness enforcement, `O_APPEND` + 2048-byte cap, header-less events file, no cache integration).
- Alternatives considered and rejected (inline `events: Vec<Event>` on `Item`, SQLite-backed events in the cache).
- Security posture (no `O_NOFOLLOW`, TTY sanitization at display time only, explicit "don't paste secrets into notes" documentation).

### Key Types

```rust
// src/events/record.rs
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub v: u32,                          // always 1 on write; unknown v ignored on read
    pub task_id: String,
    pub ts: DateTime<Utc>,               // RFC3339 UTC, microsecond precision
    pub author: String,
    #[serde(rename = "type")]
    pub event_type: EventType,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<Status>,          // present iff event_type == StatusTransition
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum EventType { StatusTransition, Note }

// src/events/witness.rs
#[must_use = "StatusChange must be redeemed via Store::commit_status_change or commit_done"]
pub struct StatusChange {
    pub(crate) task_id: String,
    pub(crate) new_status: Status,
    pub(crate) text: String,
}
```

---

## Retrospective

[Fill in after completion]

### What worked well?

### What was harder than expected?

### What would we do differently next time?
