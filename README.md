# Task Golem

A minimal, agent-native work tracker for software projects. Stores items as JSONL with file-level locking, designed to be used both as a CLI tool and as a Rust library dependency.

Used by [phase-golem](https://github.com/SIRHAMY/phase-golem) as its storage backend.

## Installation

```bash
git clone https://github.com/SIRHAMY/task-golem.git
cd task-golem
cargo build --release
cp target/release/tg ~/.local/bin/
```

## Quick Start

```bash
# Initialize in your project root
tg init

# Add items
tg add "Build user authentication"
tg add "Fix login timeout bug"

# List items
tg list

# Show item details
tg show tg-a1b2c
```

## Commands

| Command | What it does |
|---------|-------------|
| `init` | Create `.task-golem/` directory with empty store |
| `add "<title>"` | Add a new item (generates a hex ID) |
| `list` | Show all active items (add `--blocked` for blocked items) |
| `show <ID>` | Show full details of an item (add `--events` for the event log) |
| `ready` | Show items with `todo` status |
| `archive` | Archive completed items |
| `note <ID> "<text>"` | Append a free-text note to a task's event log |
| `events <ID>` | Show the chronological event log for a task (`--json` emits NDJSON) |

## Events

Every status transition (`do`, `done`, `todo`, `block`, `unblock`) automatically appends a structured event to `.task-golem/events.jsonl`. Agents and humans can also append free-text notes:

```bash
tg note tg-a1b2c "tried approach X, hit verification ceiling on browser check"
tg events tg-a1b2c           # human table view
tg events tg-a1b2c --json    # NDJSON for downstream tools
```

Notes are intended as lightweight breadcrumbs for stalls, verification ceilings, and context-for-next-session handoffs. They are capped at 2048 bytes per event (including the trailing newline on disk).

Author resolution order: `TG_AUTHOR` env var → `git config user.email` → `"unknown"`. Set `TG_AUTHOR` for agent runs to keep author tags stable across invocations.

**Don't paste secrets into notes.** `events.jsonl` and `events.archive.jsonl` are part of the repo's durable state and travel with the project. Treat them like any other committed file.

## Storage

Items are stored in `.task-golem/tasks.jsonl` (one JSON object per line). Archived items go to `.task-golem/archive.jsonl`. A file lock (`.task-golem/tasks.lock`) provides safe concurrent access.

Items support an `extensions` field (`BTreeMap<String, Value>`) for consumer-specific metadata. Phase-golem uses `x-pg-*` extension fields to track its 6-state status model, phase progress, assessments, and more.

## Library Usage

Task-golem can be used as a Rust library dependency:

```toml
[dependencies]
task-golem = { path = "../task-golem" }
```

```rust
use task_golem::store::Store;
use task_golem::model::Item;

let store = Store::new(project_root.join(".task-golem"));
store.with_lock(|s| {
    let items = s.load_active()?;
    // ... work with items
    s.save_active(&items)
})?;
```

Key library exports: `model` (Item, Status, ID generation), `store` (Store with file locking), `errors` (TgError), `git` (stage_self, commit).

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or <http://opensource.org/licenses/MIT>)

at your option.
