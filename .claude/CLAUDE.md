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
