# Change: Agent-Native Work Tracker (Task Golem)

**Status:** Proposed
**Created:** 2026-02-24
**Author:** sirhamy + Claude

## Problem Statement

AI coding agents (Claude Code, Cursor, Copilot, custom orchestrators) need a way to track, query, and update work items in a project. Today there is no lightweight, standardized, project-scoped work tracker that agents can interact with natively.

The current landscape forces developers into one of three bad options:

1. **No tracking** -- Work lives in the developer's head or in scattered TODO comments. Agents start every session from scratch with no memory of what needs doing.

2. **Heavyweight external tools** -- Jira, Linear, GitHub Issues. These tools require network access, API tokens, and complex integrations. They don't live in the repo and aren't accessible to agents without custom tool definitions.

3. **Tool-coupled formats** -- Phase-golem's BACKLOG.yaml, Beads' Dolt database. These work well within their respective orchestrators but can't be adopted independently. You must buy into the full system to get basic work tracking.

This creates two concrete adoption problems:

- **No on-ramp**: There's no way to start tracking work before committing to an orchestrator. A developer who wants "just a task list that agents can read" has no good option.
- **Migration cliff**: Moving to an orchestrator like phase-golem means starting your backlog from scratch in a proprietary format, rather than layering orchestration on top of existing tracked work.

The problem is worth solving now because agent-assisted development is becoming mainstream, multi-agent workflows are emerging (Claude Code teams, Cursor subagents), and the ecosystem lacks a standard substrate for work tracking that is both human-usable and agent-native.

## User Stories / Personas

- **Solo Developer with Agents** -- Uses Claude Code or Cursor daily. Wants to track work items that persist across agent sessions. Currently loses context between sessions because there's no structured work queue the agent can read. Needs: `tg init && tg add "Fix login timeout on /auth endpoint"` and the agent can pick it up next session.

- **AI Coding Agent** -- An LLM-based agent (Claude Code, Cursor, custom) that needs to know what to work on, report progress, create follow-up items, and mark work complete. Needs: structured `--json` output, collision-resistant IDs for concurrent creation, and a `ready` command that returns the prioritized unblocked work queue.

- **Orchestrator (Phase-Golem)** -- An autonomous development orchestrator that manages pipelines of work (PRD, research, design, spec, build, review). Currently owns its own backlog format. Wants to consume a standalone tracker as its work queue substrate, storing pipeline-specific metadata in extension fields. Needs: extension metadata via `x-*` fields, dependency graph, ready-queue computation, and a CLI API for all operations.

- **Multi-Agent Team** -- Multiple agents working on the same project simultaneously (e.g., one doing code review while another implements). Need collision-resistant IDs, serialized writes, and atomic state transitions so two agents don't claim the same work item.

## Desired Outcome

A developer can install Task Golem (`tg`) and immediately start tracking work in any project. The tool is:

1. **Zero-config to start** -- `tg init && tg add "Fix login timeout on /auth endpoint"` works with no configuration files, no accounts, no setup wizard.

2. **Agent-native** -- Every command supports `--json` for structured output. Hash-based IDs prevent multi-agent collisions. A `tg ready --json` command returns the prioritized unblocked work queue in a single call. An LLM agent can discover available work, claim it, and mark it done using only `tg` CLI commands and `--json` output, without human intervention.

3. **Orchestrator-friendly** -- Extension metadata (`x-*` fields) lets orchestrators store their domain-specific data (phases, pipelines, commit SHAs) without polluting the core schema. The 4-state model (todo/doing/done/blocked) is simple enough for standalone use and extensible enough for orchestrators via `x-*` sub-status fields.

4. **Concurrent-safe** -- Multiple agents can create, query, and update items simultaneously without data loss, corruption, or collision.

5. **Git-friendly** -- The backing store lives in the project directory and can be committed to version control. JSONL format ensures modifying a single item changes at most one line, producing clean and reviewable diffs.

## Success Criteria

### Must Have

**Core Commands:**
- [ ] `tg init` creates a `.task-golem/` directory with an empty backing store. Errors if already initialized (use `--force` to reinitialize)
- [ ] `tg add <title> [--description "..."] [--priority N] [--dep ID] [--tag TAG] [--set x-foo.bar=value]` creates a new item with a 5-hex-char hash-based ID (e.g., `tg-a3f82`) and returns the ID
- [ ] `tg list [--status STATUS] [--tag TAG] [--json]` lists items filtered by status and/or tag. Default: all non-done items, sorted by priority desc then created_at asc
- [ ] `tg show <ID> [--json]` displays a single item with all fields including extension metadata. Searches archive if not found in active store
- [ ] `tg do <ID> [--claim <agent-id>]` transitions an item to `doing` state. `--claim` records who is working on it
- [ ] `tg done <ID>` transitions an item to `done` state, archives it, and clears `claimed_by`
- [ ] `tg block <ID> [--reason "..."]` transitions to `blocked` state, stores `blocked_from_status` for restore on unblock
- [ ] `tg unblock <ID>` restores to previous state (using stored `blocked_from_status`)
- [ ] `tg edit <ID> [--title "..."] [--priority N] [--add-dep ID] [--rm-dep ID] [--add-tag TAG] [--rm-tag TAG] [--set x-foo.bar=value]` modifies item fields
- [ ] `tg rm <ID>` hard-deletes an item. Warns and requires `--force` if other items depend on it
- [ ] `tg ready [--json]` returns unblocked todo items sorted by priority (desc) then created_at (asc/FIFO)

**State Machine:**
- [ ] Four states: `todo`, `doing`, `done`, `blocked`
- [ ] Valid transitions: `todo`→`doing`, `todo`→`done`, `todo`→`blocked`, `doing`→`done`, `doing`→`blocked`, `doing`→`todo`, `blocked`→(restored via `blocked_from_status`)
- [ ] `done` is terminal -- items cannot leave `done` state. To reopen work, create a new item and reference the old one via dependency or description for context
- [ ] Invalid transitions return exit code 1 with a clear error message
- [ ] `blocked_from_status` field stored on every block transition so unblock restores correctly

**Claim Semantics (multi-agent coordination):**
- [ ] `claimed_by`: optional string field, set via `tg do <ID> --claim <agent-id>`
- [ ] `claimed_at`: ISO 8601 UTC timestamp, auto-set when `claimed_by` is set
- [ ] `claimed_by` is cleared on `tg done`, `tg block`, and `tg todo` (any transition out of `doing`)
- [ ] `tg do <ID> --claim <agent-id>` fails with exit code 1 if already claimed by a DIFFERENT agent. Same agent re-claiming is a no-op that updates `claimed_at`
- [ ] `tg ready --include-stale=<duration>` (e.g., `--include-stale=4h`) also returns `doing` items whose `updated_at` is older than the specified duration, regardless of claim. This surfaces items where the working agent may have died
- [ ] `tg list --status doing --json` includes `claimed_by` and `claimed_at` fields for visibility into who is working on what
- [ ] `--claim` is optional -- `tg do <ID>` without `--claim` transitions to `doing` with no claim (backwards compatible, works for solo use)

**Data Model:**
- [ ] Every command supports `--json` flag producing valid JSON to stdout (diagnostics/warnings to stderr only)
- [ ] Error JSON output: `{"error": "<message>", "exit_code": N}` on stderr when `--json` is set
- [ ] Hash-based IDs generated from random bytes + timestamp (not content-based), with collision detection against active + archived IDs and rehash on conflict
- [ ] Dependency graph: items can depend on other items; a dependency is "met" when the depended-on item is `done` or archived (absent from active store). Dependencies on non-existent IDs produce a warning on stderr
- [ ] Dependency cycles rejected at insertion time with a clear error
- [ ] Self-referential dependencies (`tg edit X --add-dep X`) rejected
- [ ] `created_at` and `updated_at` timestamps on all items (ISO 8601 UTC)
- [ ] Priority: integer, higher = more important, default 0. Unbounded range
- [ ] All text fields are UTF-8. Titles are single-line (newlines rejected). Descriptions may be multi-line

**Storage:**
- [ ] JSONL backing store at `.task-golem/tasks.jsonl` -- one JSON object per line, one line per item
- [ ] Archive file at `.task-golem/archive.jsonl` -- done items moved here on `tg done`
- [ ] Schema version as first line of JSONL file: `{"schema_version": 1}`
- [ ] Atomic file writes: write to temp file, fsync, rename. Original file preserved on failure
- [ ] File-lock concurrency control for serialized writes (read-modify-write under exclusive lock)
- [ ] Stale lock detection: lock file contains PID + timestamp, auto-clear if PID is dead after 30-second timeout
- [ ] Project root detection: walk parent directories looking for `.task-golem/` (like git walks for `.git/`)

**CLI Contract:**
- [ ] Exit codes: 0 = success, 1 = user error (bad ID, invalid args, invalid transition), 2 = system error (corruption, lock failure)
- [ ] Non-existent ID returns exit code 1 for all commands
- [ ] Empty results: `[]` for `--json`, human-readable "No items found" message otherwise

**Extension Metadata:**
- [ ] `x-*` fields are top-level keys on the item JSON object (not nested under an `extensions` sub-object)
- [ ] Values are arbitrary JSON (strings, numbers, objects, arrays, booleans, null)
- [ ] Dot-path syntax in `--set` creates nested objects: `--set x-phasegolem.phase=build` stores `{"x-phasegolem": {"phase": "build"}}`
- [ ] Extension fields are preserved exactly through read/write cycles
- [ ] `--set x-foo.bar=` (empty value) deletes the key

### Should Have

- [ ] `tg next [--json]` returns the single highest-priority ready item (sugar over `tg ready --limit 1`)
- [ ] Bare hex ID input accepted (e.g., `tg show a3f82` works, prefix optional). Exact match wins; error on ambiguous prefix match
- [ ] Human-readable colored table output when not using `--json` (color-coded states)
- [ ] `tg dep add <ID> <depends-on-ID>` / `tg dep rm <ID> <depends-on-ID>` for managing dependencies after creation
- [ ] Configurable ID prefix via `.task-golem/config.yaml` (default: `tg`)
- [ ] `tg doctor` command: validates JSONL syntax, checks for duplicate IDs, validates state machine integrity, detects dependency cycles, offers automated repair with backup
- [ ] Lock acquisition timeout with backoff: block and retry for up to 5 seconds before failing with exit code 2

### Nice to Have

- [ ] Unix socket daemon mode for high-frequency multi-agent access (in-memory actor, tokio multi-producer single-consumer channel). Socket protocol is an internal implementation detail, not a stable API
- [ ] File-lock fallback transparent when daemon is not running
- [ ] `tg archive [--before DATE]` for manual bulk archival
- [ ] `tg watch --json` event stream (newline-delimited JSON on state changes)
- [ ] Tab completion via clap_complete (bash, zsh, fish)
- [ ] `tg merge-resolve` for JSONL merge conflict resolution
- [ ] Idempotent state transitions (e.g., `tg done` on already-done item returns success with `"previous_state":"done"`)
- [ ] `tg dump --yaml` / `tg dump --json` for human-readable full export

## Scope

### In Scope

- Core CLI binary (`tg`) written in Rust
- CRUD operations for work items (add, list, show, edit, rm)
- State management (do, done, block, unblock) with explicit transition table and claim semantics for multi-agent coordination
- Dependency graph with ready-queue computation and cycle detection
- Hash-based collision-resistant ID generation (random bytes + timestamp)
- Extension metadata via `x-*` namespaced fields (preserved but not interpreted; arbitrary JSON values)
- File-lock concurrency control with stale lock detection
- JSONL backing store (active file + archive file) with atomic writes
- `--json` output mode on all commands with defined JSON schemas
- Project-scoped storage (`.task-golem/` directory) with parent-directory walking
- Item fields: id, title, status, priority, description, tags, dependencies, created_at, updated_at, blocked_reason, blocked_from_status, claimed_by, claimed_at, plus arbitrary x-* extensions

### Out of Scope

- Pipelines, phases, phase pools (orchestrator concern -- stored as x-* metadata)
- Guardrails, risk assessment, triage logic (orchestrator concern)
- Agent execution, prompt building, subprocess management (orchestrator concern)
- Git operations (committing, staging, staleness checks -- orchestrator or user concern)
- Structured description schemas (context/problem/solution -- use x-* or freeform description)
- Model Context Protocol (MCP) server (future extension, not v1)
- Web UI / Terminal User Interface (TUI) dashboard (future extension)
- Cross-project sync / team features
- Import from external trackers (GitHub Issues, Linear, Jira)
- Status transition validation beyond the 4-state model (orchestrators enforce their own via x-* fields)
- Worklog / execution history narratives
- Library API (v1 is CLI-only; a Rust crate API may come in v1.1 for tighter orchestrator integration)

## Non-Functional Requirements

- **Performance:** All CLI commands complete in under 200ms (cold start, release build, SSD, 500-item JSONL store, measured via `hyperfine`). `tg ready` computes the dependency graph and returns results in under 50ms for 500 items
- **Binary Size:** Release binary under 15MB (stripped). Target 8-10MB with tokio feature minimization
- **Memory:** Under 10MB Resident Set Size (RSS) for a 500-item store loaded in memory. The 500-item target represents expected 95th-percentile active store size for a typical project
- **Platform Support:** Linux (primary), macOS (supported). Windows via WSL (best-effort). Native Windows file-lock support but no Unix socket daemon on Windows
- **Observability:** Diagnostic output to stderr only (never mixed with stdout). `--verbose` flag for debug-level output. Exit codes for programmatic error handling

## Constraints

- **Rust** -- Binary must be a single Rust binary for easy distribution and to share the ecosystem with phase-golem
- **No network** -- Task Golem is fully local. No cloud accounts, no API tokens, no network calls
- **No database** -- Backing store is JSONL flat files. No SQLite, no Dolt, no embedded database. The in-memory actor model (if daemon implemented) provides query performance; the file provides durability
- **Git-friendly** -- JSONL format: one JSON record per line. Modifying a single item changes exactly one line in the backing store
- **Backwards compatible with future versions** -- Schema version in first line of JSONL file. Newer `tg` versions auto-migrate older stores. Older `tg` versions fail gracefully with a clear error on newer schemas
- **CLI is the primary API** -- All operations go through the CLI for v1. The CLI contract (commands, flags, JSON output schema) is the public interface. If a daemon is implemented, its socket protocol is an internal implementation detail, not a stable API

## Dependencies

- **Depends On:** Nothing external. Task Golem is self-contained
- **Rust Crates (implementation):** clap (CLI), serde + serde_json (serialization), fslock or file-lock (concurrency), sha2 or blake3 (ID hashing), chrono (timestamps), tempfile (atomic writes), tokio (async runtime, if daemon implemented)
- **Blocks:** Phase-golem integration (phase-golem would consume Task Golem as its work queue substrate -- this is a separate large work item requiring an adapter layer, not a simple swap). Changes workflow integration (the `/changes` skill could create/close Task Golem items as it works)

## Risks

- [ ] **JSONL git merge conflicts** -- When two branches both modify the backing store, line-level merge conflicts can occur. JSONL is better than YAML (one changed line per item) but not immune. Mitigation: small active store (archive done items aggressively), `tg merge-resolve` command (Nice to Have)

- [ ] **Daemon orphan processes** -- If the daemon crashes or is killed without cleanup, the socket file persists and new clients fail to connect. Mitigation: PID file validation on connect, stale socket cleanup on startup, auto-exit after idle timeout

- [ ] **Data loss window in debounced writes** -- Between daemon mutation and flush, data exists only in memory. A 100ms debounce caps the maximum data loss at one in-flight write, which is acceptable because items can be re-created and the store is recoverable from git. Mitigation: explicit flush on state transitions (`do`, `done`, `block`), flush on SIGTERM/SIGINT. Write-ahead log (WAL) deferred unless data loss reports emerge

- [ ] **Hash ID collision at scale** -- 5 hex chars gives ~1M namespace. Birthday problem (probability that any two items share an ID grows quadratically with item count): 50% collision probability at ~1,200 items including archived. Mitigation: collision detection + rehash loop (check active + archived IDs, re-hash with attempt counter, cap at 10 retries). For projects that exceed this scale, a future version can extend to 6+ chars with a migration

- [ ] **Extension field schema drift** -- Different orchestrators may use the same `x-*` key for different purposes. Mitigation: namespace convention (`x-phasegolem-*`, `x-myorchestrator-*`), document expected fields per integration

- [ ] **Scope creep toward orchestrator features** -- Natural temptation to add phase tracking, triage logic, pipeline management. Mitigation: strict scope boundary -- Task Golem is a tracker, not an orchestrator. If a feature requires knowledge of pipelines or phases, it belongs in the orchestrator

- [ ] **Phase-golem integration cost is high** -- Phase-golem's 6-state model, 20+ typed fields, and deep scheduler integration mean adopting Task Golem as substrate is a near-rewrite of phase-golem's persistence layer, not a simple swap. Mitigation: scope integration as a separate large work item; consider a Rust library crate API in v1.1 so phase-golem can depend on Task Golem directly rather than shelling out to the CLI

## Decisions Made

- **Backing store format:** JSONL -- agent-first, line-per-record for clean git diffs, faster parse, append-friendly for archive
- **`done` terminality:** Terminal. To reopen work, create a new item and reference the old one. Keeps archive flow simple and state machine clean
- **Phase-golem integration:** CLI-only for v1. Try it standalone first, then consider a Rust library crate in v1.1 for tighter orchestrator integration
- **Claim semantics:** Core `claimed_by` + `claimed_at` fields on `tg do --claim <agent-id>`. Staleness via `tg ready --include-stale=<duration>`. Optional -- solo users can ignore it entirely

## Open Questions

- [ ] **ID prefix: configurable or fixed `tg-`?** -- Configurable adds complexity but helps with multi-project disambiguation. Phase-golem uses configurable prefixes (WRK, HAMY). Could start fixed and add configurability later

- [ ] **Daemon lifecycle: auto-start or explicit?** -- Auto-start on first CLI invocation is more ergonomic but harder to debug. Explicit `tg daemon start` is cleaner but adds friction. Hybrid: agents/orchestrators can start the daemon explicitly, CLI falls back to file-lock mode when daemon isn't running

- [ ] **Task hierarchy: flat with deps or parent-child?** -- Flat list with blocking dependencies is simpler and covers most use cases. Parent-child (epics/subtasks) is expected by users from Jira/Linear but adds significant complexity. Leaning flat-only for v1

- [ ] **Ready-queue extension field filtering** -- Can `tg ready` filter by x-* fields? E.g., `tg ready --where x-phasegolem.phase=build`. Powerful for orchestrators but complicates the query engine. Could defer to orchestrators filtering `tg list` output themselves

- [ ] **Lock acquisition behavior under contention** -- When a write command cannot acquire the file lock, should it block with backoff (up to N seconds), fail immediately, or be configurable? Blocking is better for agents (they retry automatically) but could mask deadlocks

## References

- [Beads](https://github.com/steveyegge/beads) -- Steve Yegge's distributed issue tracker for AI agents (Dolt+JSONL, hash IDs, `bd ready`)
- [Tracer](https://github.com/Abil-Shrestha/tracer) -- Lightweight CLI issue tracker (JSONL, similar concept)
- [Claude Code Tasks](https://claudefa.st/blog/guide/development/task-management) -- Built-in task management (JSON filesystem, addBlockedBy)
- [Phase-Golem](https://github.com/sirhamy/phase-golem) -- Autonomous development orchestrator (YAML backlog, scheduler, pipelines)
- [GitHub Agentic Workflows](https://github.blog/changelog/2026-02-13-github-agentic-workflows-are-now-in-technical-preview/) -- Markdown-defined workflows that compile to GitHub Actions
- [Anthropic: Effective Harnesses for Long-Running Agents](https://www.anthropic.com/engineering/effective-harnesses-for-long-running-agents) -- Best practices for agent work tracking
