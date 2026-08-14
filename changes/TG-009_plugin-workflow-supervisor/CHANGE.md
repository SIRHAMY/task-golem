# Change: Add a template-driven agent runner

**ID:** TG-009
**Status:** Draft
**Created:** 2026-08-13

## Problem

Task Golem can represent hierarchical work, dependencies, readiness, claims, and status, but a human still has to inspect ready work and launch agents manually. We want to test whether a small foreground runner plus reusable workflow templates and agent plugins is useful before investing in durable execution protocols, sandboxing, worker pools, or perfect recovery.

## Why now

TG already has the checklist/DAG primitives, and current agents are capable enough that the first experiment only needs to provide durable work selection, concise instructions, optional verification, and session reuse. `(user)` `(src/model/deps.rs:319-369)` `(src/cli/commands/transition.rs:41-90)`

## Stories

| id | outcome | depends-on | validation |
|---|---|---|---|
| S1 | A reusable template can create one TG Campaign containing containers and executable Tasks with dependencies, plugins, context policy, and optional verification. | - | Instantiating the same template with the same instance key is idempotent and `tg workflow status` shows the intended graph. |
| S2 | One foreground runner can drain ready Tasks through trusted agent plugins, reuse one agent session where configured, and durably stop on failure. | S1 | A multi-Story fixture completes serially; Tasks sharing a Story reuse one session; fresh Tasks do not; failed plugins or verification block the Task with a useful note. |
| S3 | The pilot is easy to extend and honest about its limits. | S1, S2 | A second plugin and template work without runner branches, docs explain the plugin/template contracts, and deferred reliability/security machinery is absent. |

## Approach

Add repository-local workflow templates and plugin definitions plus three foreground commands: `tg workflow instantiate`, `tg workflow run`, and `tg workflow status`. A template creates ordinary TG container and Task items and copies each executable Task's resolved plugin, context, input, and verify configuration into `x-workflow`. One Campaign-scoped process lock excludes concurrent runners while the existing TG lock remains short-lived around state mutations. The runner resumes one interrupted Task when possible, otherwise claims the next ready executable Task, invokes its trusted plugin with a small JSON request, records the returned public summary/session reference as TG metadata and a bounded note, runs optional verify argv directly, then marks the Task done or blocked. After the runner exits, a blocked Task is retried only through `tg unblock <task-id>` followed by `tg todo <task-id>`; containers complete bottom-up when their descendants are done. TG status is the sole mutable progress source; templates define reusable work and plugins define how an agent performs it.

**Key decisions:**

- **Decision-1 (thin pilot):** Build the smallest useful runner over a generalized durable orchestration platform. Prove that templates, plugins, Task visibility, and session reuse help before hardening edge cases. `(user)`
- **Decision-2 (TG owns progress):** TG item status, claims, dependencies, notes, and `x-workflow` metadata are the only runtime state; do not add a second evidence ledger or synchronize Markdown checkboxes. `(user)`
- **Decision-3 (templates and plugins):** Templates own reusable DAG shape; trusted repository-local plugins own agent invocation. V1 requires exactly one root container, a parent tree, Task-only acyclic dependencies, and one executable descendant per container. Every node carries version, instance, node ID, kind, and digest in `x-workflow`; executable Tasks additionally carry resolved plugin argv, context, input, and verify argv. Containers are never dispatched. A shared context references one ancestor container and all Tasks sharing it use one plugin. Plugins receive the versioned Campaign/Task/workspace/input/context request path and atomically write a versioned `complete|blocked` result with public summary and optional session reference. `(user)`
- **Decision-4 (configurable context):** Each executable Task chooses `fresh`, `shared:<key>`, or `none`. The HAMY-shaped example uses one shared writer session per Story and fresh review Tasks. `(user)`
- **Decision-5 (simple completion):** Plugin `complete` plus optional verify exit code marks a Task done; plugin failure, malformed output, failed verification, or missing interrupted-session state blocks it. After the runner exits, retry requires `tg unblock <task-id>` and then `tg todo <task-id>` because blocked cannot transition directly to todo; the runner never treats the intermediate unclaimed `doing` state as resumable. This is deliberately weaker than independent reconciliation. `(user)` `(src/model/item.rs:137-177)` `(src/model/status.rs:18-31)`
- **Decision-6 (review is a Task):** Workflows that need review add a normal review Task with its own plugin and dependencies. The review plugin may approve, repair within its run, or block; TG gains no review status or generic gate engine. `(user)`
- **Decision-7 (trusted local execution):** Plugins run as the current user like other repository scripts. V1 does not sandbox, manage secrets, or claim protection from malicious plugins; users explicitly choose the template and plugins they run. `(user)`
- **Decision-8 (bounded execution):** One foreground runner handles one Campaign and one Task at a time. A Campaign-scoped OS advisory lock held for the command lifetime rejects a second local runner; this is a process mutex, not a lease or progress ledger. Defer worker concurrency, daemons, automatic Git commits, dynamic DAG changes, and cross-machine recovery. `(user)` `(src/store/lock.rs)`
- **Decision-9 (HAMY remains a consumer):** TG-009 ships the generic runner and HAMY-shaped fixture. Revised ai-dotfiles WRK-048 owns a HAMY plugin/template adapter over `tg workflow`, not a second scheduler or TG projection, and decides whether this runner is useful enough to harden. `(user)`
- **Decision-10 (small explicit contracts):** Template inputs are unique `--input key=value` strings used only as exact scalar `${key}` replacements; missing, extra, or duplicate inputs fail before writes. Template/plugin YAML files must canonicalize beneath the workspace root; argv stays literal and runs with that root as `current_dir`. The instance digest is SHA-256 over canonical resolved template, inputs, and plugin definitions. `(src/store/root.rs)`
- **Decision-11 (public workflow data):** Workflow inputs, resolved `x-workflow` metadata, plugin result summaries, and TG notes are public repository state. There is no secret input/result channel: plugins obtain credentials from process-local environment or provider configuration and must not copy them into requests, results, summaries, or notes. `(src/model/item.rs:73-75)` `(src/store/jsonl.rs)` `(src/git.rs)`
- **Decision-12 (approval):** Hamilton acknowledged the Decision-11 public-only workflow-data boundary and external credential handling for this trusted-local pilot. `(user)`

## Checklist

- [x] [T1][S1] Define and parse the Decision-3 v1 template/plugin/request/result contracts with generic `container|task` nodes, exact scalar input substitution, full parent/dependency/context validation, and optional verify argv; canonicalize definition files beneath the discovered workspace root while leaving argv literal and reject malformed fields before writes.
- [x] [T2][S1] Instantiate the fully validated graph in one locked write, populate every node's required `x-workflow` identity/version/digest fields plus Task execution fields, and make the same instance key return its active or archived Campaign only when the canonical SHA-256 digest matches.
- [ ] [T3][S2] Implement the pure Campaign projection/selection/rollup rules and `tg workflow run` state shell: hold one Campaign process lock, scope active/archive items, exclude containers from dispatch, resume only the one valid claimed `doing` Task, otherwise claim the deterministic next ready Task under the short TG lock, and close eligible containers bottom-up.
- [ ] [T4][S2] Invoke trusted plugins and optional verify argv outside the TG lock; support `fresh`, container-owned `shared`, and `none` sessions; ingest an interrupted valid result or resumable shared session; UTF-8 truncate public summary/diagnostics so the complete serialized note stays within the 2,048-byte event limit; and re-read under lock before transitioning done or blocked. Unblock-then-todo retries delete stale IPC before launch.
- [ ] [T5][S2] Implement versioned `tg workflow status` output from active/archive TG items, including Campaign identity/rollup, one current Task, completed Tasks, blockers with reasons, and deterministic next-ready Tasks; malformed or incomplete workflow metadata fails rather than guessing.
- [ ] [T6][S3] Add command and fake-agent plugins plus a multi-Story HAMY-shaped template, then cover full pre-write/schema validation, matching/mismatched idempotency, deterministic scoped selection, same-Campaign exclusion, short TG lock scope, plugin/verify/result failures and oversized UTF-8 output, all context modes and session replacement, interrupted resume/no-state block, stale IPC cleanup, nested invocation CWD, status projection, and container closure.
- [ ] [T7][S3] Add a second template/plugin without runner branches; document public-only workflow data, external credential handling, trusted-local execution, unblock-then-todo recovery, TG progress authority, the WRK-048 consumer seam, and explicit v1 non-goals; keep workflow runtime files ignored through init/doctor and run `just check`, focused integration tests, and `git diff --check`.

## Done when

A user can instantiate a reusable multi-Story workflow, run it through trusted command/agent plugins one Task at a time, observe TG-native progress and blockers, reuse one configured session per Story, and add another workflow without changing runner code.

## Open Questions

- [x] SAFETY ACK: Workflow inputs, resolved metadata, plugin summaries, and notes are durable public repository state and must contain no credentials or secrets - acknowledge the Decision-11 public-only boundary and external credential handling before build.

## Followups

<!-- plan-critic: ran 2026-08-13 sha=5d9d98221959b2f341c74a48777c54e870a9e1524593bba5b85bf03069b0d3e2 -->
<!-- rfc-authorized: 2026-08-13 approver=Hamilton sha=5d9d98221959b2f341c74a48777c54e870a9e1524593bba5b85bf03069b0d3e2 -->

- **followup:** Add independent reconciliation, immutable run evidence, stronger crash recovery, or managed transition guards only after the pilot exposes false completion or recovery toil.
- **followup:** Add sandboxing, environment isolation, or untrusted-plugin support only if plugins need a stronger boundary than approved local scripts.
- **followup:** Add worker pools, leases, daemons, dynamic repair Tasks, or dashboards only after sequential Campaigns prove useful.
- **followup:** Revised ai-dotfiles WRK-048 owns the real HAMY adapter, comparative cost/latency evaluation, and any migration from `CHANGE.md` Task checkboxes.

## References

- `DESIGN.md`
- `src/model/item.rs`
- `src/model/deps.rs`
- `src/cli/commands/transition.rs`
- `src/store/mod.rs`
- `/home/sirhamy/Code/ai-dotfiles/changes/WRK-048_lights-on-build-supervisor/CHANGE.md`
