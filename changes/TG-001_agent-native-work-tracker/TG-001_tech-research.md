# TG-001 Technical Research: Agent-Native Work Tracker

**Date:** 2026-02-24
**Mode:** Medium
**Status:** Complete
**PRD:** TG-001_agent-native-work-tracker_PRD.md

---

## 1. Competitive Landscape

### 1.1 Beads (steveyegge/beads) — Go, Dolt

The most mature tool in this space (~17k stars, v0.56.1). Originally SQLite + JSONL, now fully Dolt-backed.

- **Storage:** Dolt (version-controlled SQL database). SQLite removed in v0.51+. Server mode on port 3307 for multi-writer
- **Schema:** ~81 fields in the `issues` table. 19 dependency types (blocks, parent-child, conditional-blocks, waits-for, related, etc.)
- **IDs:** SHA-256 of (title + description + timestamp + salt + actor). Adaptive 4-8 hex chars. Configurable prefix
- **Agent integration:** Full MCP server (`beads-mcp`). `bd prime` generates a ~1-2k token context summary. `bd ready` for ready queue. `--json` on all commands. 97% token reduction via BriefIssue models. Actor attribution via env vars
- **Concurrency:** Dolt ACID transactions. Atomic claim via `bd update --claim`

**Relevance to Task Golem:** Beads validates the problem space (17k stars proves demand) but occupies a very different position. It is heavyweight (requires Dolt server, ~30MB binary, 81-field schema), Go-based, and network-adjacent. It is the "Jira for agents" — powerful but complex. Task Golem is the "lightweight tracker for agents" — zero-config, no database, single binary.

### 1.2 Tracer (Abil-Shrestha/tracer) — Rust, JSONL

Closest philosophical match (~18 stars, v0.2.0, Oct 2025).

- **Storage:** JSONL at `.trace/issues.jsonl`. Project-scoped
- **Features:** `--json` flag, `tracer ready` command, `--actor` parameter, auto-assign, dependency types (blocks, parent-child, related, discovered-from), ~5ms per operation
- **Gaps:** No documented concurrency handling, no extension metadata, no claim semantics, no archive, no state machine enforcement

**Relevance to Task Golem:** Validates the design direction (Rust + JSONL + agent-native) but is too immature and incomplete to be a competitor. Task Golem's PRD is substantially more thorough.

### 1.3 Workgraph (graphwork/workgraph) — Rust, JSONL

Closest feature match (moderate maturity).

- **Storage:** JSONL at `.workgraph/graph.jsonl`
- **States:** open, in-progress, done, failed
- **Features:** DAG dependency graph, ready-task computation, background service mode with agent dispatch (`wg service start --max-agents 4`), skill-based assignment, loop edges, recursive task spawning, agent evolution scoring
- **Philosophy:** "Git-friendly. No server. No account. Just a binary and a directory"

**Relevance to Task Golem:** Workgraph bundles orchestration features (agent dispatch, scoring, evolution) that Task Golem explicitly excludes. If Workgraph gains adoption, Task Golem differentiates on simplicity, extension metadata (`x-*` fields), and the explicit "tracker not orchestrator" boundary.

### 1.4 beads_rust (Dicklesworthstone/beads_rust) — Rust, SQLite+JSONL

A Rust port that freezes the classic Beads architecture (~620 commits).

- **Storage:** Hybrid SQLite (`beads.db`) + JSONL (`issues.jsonl`). Sync is explicit (`br sync`)
- **Binary size:** ~5-8 MB (vs Beads Go ~30+ MB)
- **Features:** Ready view, dependency tracking, `--json`, assignee, priority 0-4, labels

**Relevance to Task Golem:** Closer in spirit than Go Beads, but Task Golem's PRD rejects SQLite. The hybrid approach (SQLite for speed, JSONL for git) is interesting but adds complexity.

### 1.5 Claude Code Built-In Tasks

- **Storage:** `~/.claude/tasks/<id>/tasks.json` — user-scoped, not project-scoped
- **Model:** id, subject, description, status (pending/in_progress/completed/failed), blockedBy/blocks, owner, metadata, timestamps
- **Limitations:** Claude Code-only. Not CLI-accessible. No priority. No tags. Sequential integer IDs. Not committable to git. Limited to 10 visible pending tasks

**Relevance to Task Golem:** Validates that agents benefit from structured task tracking. But it's tool-coupled (only Claude Code can use it) and not project-scoped — exactly the problems the PRD identifies.

### 1.6 Other Tools

- **TSK (dtormoen/tsk):** Rust, ~140 stars. Agent execution orchestrator with sandboxed Docker containers. SQLite storage. Not a standalone tracker
- **Block Agent Task Queue:** Python MCP server. Resource contention manager (serializes builds), not a work tracker
- **TAKT (nrslib/takt):** TypeScript, ~445 stars. Multi-agent workflow orchestrator. YAML definitions, NDJSON logs. Orchestrator, not tracker
- **GitHub Agentic Workflows:** Markdown-defined automations compiled to GitHub Actions. Not a task tracker — an automation layer that could consume a tracker

### 1.7 Landscape Summary

| | Task Golem | Beads | Tracer | Workgraph | beads_rust | CC Tasks |
|---|---|---|---|---|---|---|
| **Language** | Rust | Go | Rust | Rust | Rust | Built-in |
| **Storage** | JSONL | Dolt | JSONL | JSONL | SQLite+JSONL | JSONL |
| **Project-scoped** | Yes | Yes | Yes | Yes | Yes | No |
| **Git-friendly** | Yes | Via sync | Yes | Yes | Via export | No |
| **States** | 4 | 5 | ~3 | 4 | 4 | 4 |
| **Dep types** | 1 | 19 | 4 | DAG | blocking | 1 |
| **Extension metadata** | `x-*` fields | 81 built-in | None | None | Labels | metadata KV |
| **Concurrency** | File-lock | Dolt ACID | Undocumented | Undocumented | File-level | Shared JSONL |
| **MCP** | No | Yes | No | No | No | N/A |
| **Maturity** | Pre-impl | Very high | Very low | Moderate | Moderate | Production |

**Key finding:** No tool occupies Task Golem's exact niche — a lightweight, zero-config, project-scoped, JSONL-backed, agent-native tracker with extension metadata for orchestrator interop. The niche is validated by demand (Beads' 17k stars, Anthropic's blog post on agent work tracking) but unoccupied at this specific point in the design space.

---

## 2. Rust Crate Ecosystem

### 2.1 Recommended Dependencies (v1)

```toml
[dependencies]
# CLI
clap = { version = "4.5", features = ["derive"] }

# Serialization
serde = { version = "1.0", features = ["derive"] }
serde_json = { version = "1.0", features = ["preserve_order"] }

# File locking
fd-lock = "4.0"

# ID generation
rand = "0.10"
hex = "0.4"

# Timestamps
chrono = { version = "0.4", features = ["serde"] }

# Atomic file operations
tempfile = "3"

# Error handling
thiserror = "2.0"
anyhow = "1.0"

# Terminal output (Should-Have)
owo-colors = { version = "4.3", features = ["supports-colors"] }
tabled = "0.20"

# Shell completions (Nice-to-Have, optional)
clap_complete = { version = "4.5", optional = true }
```

### 2.2 Crate Rationale

**clap 4.5** (~103M weekly downloads): The unambiguous standard for Rust CLIs. Derive API handles the `tg <subcommand>` pattern natively. Supports repeated flags (`--dep ID`), global flags (`--json`), and custom value parsing (`--set x-foo.bar=value`). Binary impact ~200-400KB.

**serde + serde_json**: Non-negotiable. Use `preserve_order` feature (switches internal map from BTreeMap to IndexMap) for round-trip field order stability, producing minimal git diffs. Handle `x-*` extension fields via `#[serde(flatten)]` with `HashMap<String, serde_json::Value>`.

**fd-lock 4.0** (~5M weekly downloads): Advisory file descriptor locks. Cross-platform (Linux + macOS + Windows). Ownership-based API fits Rust RAII patterns. Locks auto-release on process death (flock semantics), eliminating the most common stale-lock scenario. Preferred over `fslock` (stale, last updated 2021) and `fs2` (supplanted by `fs4`).

**rand 0.10 + hex 0.4**: For ID generation. No hash crate needed — IDs are random-based, not content-based. Generate 3 random bytes, hex-encode, truncate to 5 chars. Simple and correct.

**chrono 0.4** (~73M weekly downloads): Battle-tested for UTC timestamps and ISO 8601 (RFC 3339) formatting. Good serde integration. Preferred over `time` (similar but different API) and `jiff` (pre-1.0, overkill for UTC-only use).

**tempfile 3**: For atomic file writes. `NamedTempFile::new_in(data_dir)` + manual `sync_all()` + `persist()`. ~72M weekly downloads.

**thiserror 2.0 + anyhow 1.0**: Standard Rust error handling combo. `thiserror` for the structured error enum (maps to exit codes). `anyhow` for internal error propagation with `.context()`.

**owo-colors 4.3**: Zero-allocation terminal colors. Supports `NO_COLOR`/`FORCE_COLOR` env vars (important for agents). MIT licensed. Preferred over `colored` (MPL-2.0, allocates).

### 2.3 Notably Absent

- **tokio**: Not needed for v1 (CLI-only, no daemon). Would add 1-3MB binary size for zero benefit. Add later with minimal features (`rt`, `net`, `sync`, `signal`) for the daemon
- **blake3 / sha2**: Not needed — IDs are random, not hashed from content
- **serde-jsonlines**: The JSONL parsing pattern is ~3 lines of code. Not worth a dependency
- **petgraph**: The dependency graph for 500 items is trivially implementable (filter items where all deps are done). Not worth 2.5k+ lines of dependency

### 2.4 Binary Size

**Target:** Under 15MB stripped, goal 8-10MB.

Recommended Cargo release profile:
```toml
[profile.release]
opt-level = "z"        # Optimize for size
lto = true             # Fat LTO across all crates
codegen-units = 1      # Single codegen unit
strip = true           # Strip symbols
panic = "abort"        # Remove unwinding infrastructure
```

**Estimated v1 binary size:** 4-7MB stripped. Well within the 8-10MB target.

**Analysis tools:** `cargo-bloat --crates --release` for per-crate size contribution. `cargo-llvm-lines` for generic instantiation counts.

---

## 3. Implementation Patterns

### 3.1 JSONL Storage

**Format:** One JSON object per line. First line is `{"schema_version": 1}`. Each subsequent line is one task item.

**Schema versioning:** First-line header is sound. Newer `tg` versions auto-migrate older schemas. Older versions fail gracefully on unrecognized versions.

**Practical size limits:** 500 items at ~500 bytes/record = ~250KB. Trivially fast to load/rewrite. Well within 10MB RSS budget. The archive file growing large is acceptable since it's only scanned for ID collision checks and `tg show` fallback.

**Round-trip stability:** serde serializes struct fields in declaration order (deterministic). Extension fields in BTreeMap sort alphabetically. Same in-memory data always produces the same bytes.

**JSONL vs YAML for git:**
| | JSONL | YAML |
|---|---|---|
| Lines per record | 1 | 3-15+ |
| New record diff | 1 line | 3-15+ lines |
| Merge conflict granularity | Line-level | Block-level |
| Multiple representations | No | Yes (quoting, anchors, flow/block) |
| Round-trip stability | High | Low |

JSONL is the correct choice.

### 3.2 Atomic File Writes

**Pattern:** Write temp file → fsync → rename.

```
1. NamedTempFile::new_in(data_dir)     // Same dir = same filesystem
2. Write all JSONL lines to temp file
3. tmp.as_file().sync_all()            // fsync BEFORE rename
4. tmp.persist(target_path)            // Atomic rename
```

**Critical detail:** `tempfile::persist()` does NOT call fsync. You MUST call `sync_all()` before `persist()`. Without this, a crash can result in the renamed file being empty (ext4 delayed allocation reorders writes).

**Cross-platform:** Atomic on Linux ext4/btrfs (with fsync), macOS APFS (inherently safe due to CoW), and Windows NTFS (`MoveFileExW` with `MOVEFILE_REPLACE_EXISTING`).

**Failure modes:** Power loss before fsync = data in buffer cache lost (original file safe). Power loss after fsync but before rename = orphaned temp file (original safe). Disk full = write fails (original safe). Cross-filesystem rename = fails with EXDEV (prevented by same-dir temp file).

### 3.3 File Locking

**Recommendation:** Use flock (via `fd-lock`) on a separate `.task-golem/tasks.lock` file.

**Why flock over PID-based locks:**
- flock auto-releases on process death — the kernel handles cleanup
- No PID recycling bugs
- No stale lock detection logic needed
- Simpler implementation

**Why a separate lock file (not the data file):**
- The data file gets replaced via atomic rename, which invalidates locks on the old inode
- A separate lock file persists across data file replacements
- Can store informational metadata (PID, timestamp, agent-id) without polluting the data format

**Read-modify-write pattern:**
```
1. Open/create .task-golem/tasks.lock
2. flock(LOCK_EX)  -- exclusive lock, block until acquired
3. Read tasks.jsonl into memory
4. Modify in memory
5. Atomic write (temp file, fsync, rename)
6. Close lock file (auto-releases flock)
```

**Lock timeout (Should-Have):** Non-blocking flock with exponential backoff + jitter. Start at 10ms, double each attempt, cap at 500ms, total timeout 5 seconds. Jitter 0-50% of delay to prevent thundering herd.

**PRD adjustment:** The PRD specifies PID + timestamp in the lock file with 30-second stale detection. With flock, this is unnecessary for correctness. The PID/timestamp can still be written to the lock file as diagnostic info (readable by `tg doctor`), but stale lock detection should rely on flock's auto-release semantics, not PID checking.

### 3.4 ID Generation

**Approach:** Pure random hex. No hash function needed since IDs are not content-derived.

```
1. Generate 3 random bytes via rand
2. Hex-encode → 6 hex chars
3. Truncate to 5 → prefix with "tg-" → "tg-a3f82"
4. Check against active + archived IDs
5. If collision, regenerate (max 10 attempts)
6. If 10 collisions, return exit code 2
```

**Birthday problem at 5 hex chars (20 bits, ~1M namespace):**

| Active+Archived Items | P(collision per generation) |
|---|---|
| 37 | 0.1% |
| 118 | 1% |
| 374 | 10% |
| 1,177 | 50% |

With collision detection + rehash, the actual failure threshold is the 1M namespace, not the birthday bound. 10 retries at 10% collision rate has a 10^-10 probability of failure.

**Archive scanning for collision checks:** Maintain an in-memory set of archived IDs loaded at startup, or scan the archive file. At 500 archived items, scanning is fast (<1ms). For larger archives, consider an ID index file.

### 3.5 Dependency Graph

**Ready-queue computation:** No graph library needed. For 500 items:

```
ready_items = items
    .filter(status == "todo")
    .filter(all deps are "done" or absent from active store)
    .sort_by(priority DESC, created_at ASC)
```

This is O(V + E) but typically O(V) since most items have 0-2 dependencies.

**Cycle detection at insertion time:** DFS from the dependency target, following dependency edges. If you reach the source node, reject the dependency.

**Edge cases:**
- Dependency on archived/absent item → dependency is met (item was done)
- Dependency on non-existent ID (never existed) → warning on stderr, dependency considered unmet
- Self-referential dependency (`--add-dep X` on item X) → reject immediately

### 3.6 State Machine

**Enum-based approach** (not typestate — status is loaded from files at runtime):

```rust
enum Status { Todo, Doing, Done, Blocked }

impl Status {
    fn can_transition_to(&self, target: &Status) -> bool {
        matches!((self, target),
            (Todo, Doing) | (Todo, Done) | (Todo, Blocked)
            | (Doing, Done) | (Doing, Blocked) | (Doing, Todo)
        )
    }
}
```

**Blocked restore:** Store `blocked_from_status: Option<Status>`. Set on `block`, consumed on `unblock`. Default to `Todo` if missing (defensive). Clear `claimed_by`/`claimed_at` on any transition out of `doing`.

### 3.7 Extension Metadata (`x-*` Fields)

**Serde pattern:** `#[serde(flatten)]` with `HashMap<String, serde_json::Value>`:

```rust
struct Item {
    id: String,
    title: String,
    status: Status,
    // ... known fields ...

    #[serde(flatten)]
    extensions: HashMap<String, serde_json::Value>,
}
```

All unknown fields (including `x-*` keys) are captured in the HashMap. Preserved exactly through read/write cycles.

**Dot-path parsing for `--set x-foo.bar=value`:** Parse the key at the CLI layer, construct nested `serde_json::Value::Object` structures. `--set x-foo.bar=` (empty value) deletes the key.

---

## 4. PRD Assumption Validation

### 4.1 Confirmed Assumptions

- **The niche is real and unoccupied.** Beads' 17k stars prove demand. No existing tool matches Task Golem's exact position (lightweight, JSONL-only, no database, Rust, agent-native, orchestrator-friendly via `x-*`)
- **JSONL is the right storage format.** Superior git-friendliness over YAML/JSON. Adequate performance for 500-item target. Used by Tracer, Workgraph, and Claude Code's internal task system
- **4-state model is sufficient.** Beads has 5 states, most others have 3-4. The PRD's model covers all common workflows. Orchestrators can add sub-states via `x-*` fields
- **File-lock concurrency is adequate.** beads_rust uses it successfully. The PRD's target scale (sub-500 active items, sub-200ms operations) doesn't warrant a database. flock provides automatic cleanup on crash
- **`--json` on all commands is table stakes.** Every tool in this space provides it. It's the minimum for agent interoperability
- **The Anthropic harness blog validates the problem.** It describes exactly Task Golem's use case (persistent structured work tracking across agent sessions) implemented via ad-hoc file conventions. Task Golem formalizes the pattern

### 4.2 Adjustments Recommended

1. **ID generation: skip the hash function.** The PRD says "random bytes + timestamp" as hash input. Since the input is already random, the hash adds no entropy. Generate random hex directly. Saves a dependency (blake3/sha2)

2. **Stale lock detection: use flock, not PID+timestamp.** flock auto-releases on process death, which is the primary stale lock scenario. PID+timestamp adds complexity without benefit. Write PID info to lock file as diagnostic metadata only

3. **Consider `preserve_order` in serde_json.** The PRD doesn't mention field ordering, but deterministic serialization is critical for minimal git diffs. Either use `preserve_order` (IndexMap, preserves insertion order) or rely on struct field declaration order (which is deterministic with serde)

4. **Skip tokio entirely for v1.** The PRD lists it in dependencies but the daemon is Nice-to-Have. Every v1 operation is synchronous. Adding tokio later is straightforward

5. **No MCP.** CLIs with `--json` are the right interop layer — every tool in this space converges on it. MCP adds indirection without clear benefit over agents calling CLI commands directly or scripts hitting APIs. The CLI *is* the API

### 4.3 Risks Revalidated

| Risk | PRD Assessment | Research Finding |
|---|---|---|
| **JSONL merge conflicts** | Acknowledged, mitigated by small active store | Confirmed. Sort items by ID in JSONL file for predictable positioning. `tg merge-resolve` is a good Nice-to-Have |
| **Hash ID collision** | 50% at ~1,200 items | Confirmed mathematically. Collision detection + rehash makes this a non-issue in practice |
| **Extension field schema drift** | Namespace convention | Confirmed. Beads avoids this by having 81 built-in fields. Task Golem's approach (few built-in, flexible extensions) is better for an ecosystem tool, but namespace conventions must be documented clearly |
| **Scope creep toward orchestrator** | Strict boundary | Confirmed and reinforced. Workgraph shows what happens when a tracker adds orchestration (agent dispatch, scoring, evolution). Task Golem must resist this |
| **Phase-golem integration cost** | Acknowledged as large | Confirmed. A library crate API (v1.1) is the right approach for tight integration |

---

## 5. Open Questions — Research Recommendations

### 5.1 ID prefix: configurable or fixed `tg-`?

**Recommendation: Fixed `tg-` for v1, configurable later.** Beads makes it configurable (default `bd`). beads_rust uses configurable prefixes. The configurability is useful for multi-project contexts but adds complexity (config file parsing, validation). Start fixed and add `.task-golem/config.yaml` support as a Should-Have.

### 5.2 Daemon lifecycle: auto-start or explicit?

**Recommendation: Defer daemon entirely.** File-lock concurrency handles v1's use cases. If implemented later, explicit `tg daemon start` is cleaner. Agents/orchestrators start it explicitly; CLI falls back to file-lock when daemon isn't running.

### 5.3 Task hierarchy: flat with deps or parent-child?

**Recommendation: Flat-only for v1.** The dependency graph covers blocking relationships. Parent-child (epics/subtasks) adds significant complexity (display formatting, cascade operations, partial completion). Beads has hierarchical IDs (`bd-a3f8.1.2`) but this adds schema complexity. Flat + deps is simpler and sufficient.

### 5.4 Ready-queue extension field filtering?

**Recommendation: Defer to orchestrators.** `tg ready` returns all unblocked todo items. Orchestrators filter the `--json` output themselves (`tg ready --json | jq '.[] | select(."x-phasegolem".phase == "build")'`). Adding a query engine to Task Golem crosses into orchestrator territory.

### 5.5 Lock acquisition behavior?

**Recommendation: Block with backoff, 5-second timeout.** Fail-fast is hostile to agents (they'd need their own retry logic). Configurable adds complexity. Blocking with backoff + jitter is the standard approach per AWS best practices. 5 seconds is long enough for any realistic file-lock contention, short enough to detect deadlocks.

---

## 6. References

### Existing Tools
- [Beads](https://github.com/steveyegge/beads) — Go, Dolt, 17k stars
- [Tracer](https://github.com/Abil-Shrestha/tracer) — Rust, JSONL, 18 stars
- [Workgraph](https://graphwork.github.io/) — Rust, JSONL, DAG + agent dispatch
- [beads_rust](https://github.com/Dicklesworthstone/beads_rust) — Rust, SQLite+JSONL
- [TSK](https://github.com/dtormoen/tsk) — Rust, agent execution orchestrator
- [Block Agent Task Queue](https://github.com/block/agent-task-queue) — Python MCP, resource contention
- [TAKT](https://github.com/nrslib/takt) — TypeScript, multi-agent orchestrator

### Anthropic / Industry
- [Effective Harnesses for Long-Running Agents](https://www.anthropic.com/engineering/effective-harnesses-for-long-running-agents) — Validates persistent work tracking need
- [GitHub Agentic Workflows](https://github.blog/changelog/2026-02-13-github-agentic-workflows-are-now-in-technical-preview/) — Automation layer, not task tracker

### Rust Crates
- [clap](https://crates.io/crates/clap) 4.5 — CLI framework (~103M/week)
- [serde](https://crates.io/crates/serde) 1.0 + [serde_json](https://crates.io/crates/serde_json) 1.0 — Serialization
- [fd-lock](https://crates.io/crates/fd-lock) 4.0 — Advisory file locking (~5M/week)
- [rand](https://crates.io/crates/rand) 0.10 + [hex](https://crates.io/crates/hex) 0.4 — ID generation
- [chrono](https://crates.io/crates/chrono) 0.4 — Timestamps (~73M/week)
- [tempfile](https://crates.io/crates/tempfile) 3 — Atomic writes (~72M/week)
- [thiserror](https://crates.io/crates/thiserror) 2.0 + [anyhow](https://crates.io/crates/anyhow) 1.0 — Error handling
- [owo-colors](https://crates.io/crates/owo-colors) 4.3 — Terminal colors
- [tabled](https://crates.io/crates/tabled) 0.20 — Table output

### Technical Patterns
- [NDJSON Specification](https://ndjson.com/definition/)
- [Advisory File Locking on Linux](https://gavv.net/articles/file-locks/)
- [Lockfiles, the Right Way](https://apenwarr.ca/log/20101213)
- [Birthday Problem Calculator](https://preshing.com/20110504/hash-collision-probabilities/)
- [Rust State Machine Pattern](https://hoverbear.org/blog/rust-state-machine-pattern/)
- [min-sized-rust](https://github.com/johnthagen/min-sized-rust) — Binary size optimization
