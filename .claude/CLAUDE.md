# Task Golem

A lightweight, agent-native work tracker. Project-scoped task management with hash-based IDs, dependency graphs, and a CLI-first interface designed for AI agent interoperability.

## Tech Stack
- Rust
- JSONL backing store

## Commit Convention
- Format: `[WORK] Feature/Clean/Fix: MESSAGE`
  - `Feature:` for new functionality
  - `Clean:` for refactoring, cleanup, style
  - `Fix:` for bug fixes
- `[WORK]` is optional if there's no associated work item

## Verification
- Run `just check` before presenting any code change as ready. This runs formatting, lints, and tests.

## Project Structure
- `changes/` - PRDs, specs, designs for this project
- `src/` - Rust source

## Agent Guidance: Events

`tg note <id> "<text>"` appends a free-text event to a task's log. Use it to leave durable breadcrumbs whenever you:

- hit a verification ceiling ("can't reproduce locally; needs browser harness")
- stall on an external dependency ("waiting on API key from ops")
- reach a progress checkpoint mid-task ("phase 1 landed in abc1234, phase 2 next") — covers session handoffs
- leave a traceability pointer (commit, PR, or doc link with a one-liner)
- surface a surprising discovery worth stitching into the task history

Keep each note to one line where possible, prefer links over prose. Don't narrate every commit — status transitions and git log already tell that story.

Notes on **archived** tasks are rejected — archive is read-only once reached. If a task needs a note but has been archived prematurely, use `tg unblock`/`tg todo` flow to move it back to active before noting, or append via another active task.

Event text is capped at **2048 bytes** per event (including the trailing newline on disk). Keep notes short and specific; link out to commits, PRs, or other task IDs rather than pasting context.

Author resolution order: `TG_AUTHOR` env var → `git config user.email` → `"unknown"`. Set `TG_AUTHOR` for agent runs so notes are attributed stably.

**Never paste secrets (API keys, tokens, credentials) into notes.** `events.jsonl` is durable state that commits alongside the project.
