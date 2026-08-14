## Preparation attempt 2026-08-13

**Route:** RAISE
**Verify contract:** `just check` plus focused integration tests and `git diff --check`

### Stack grounding

- **PB-EXISTS:** VERIFIED - no workflow CLI/module exists in `src/lib.rs` or `src/cli/args.rs`.
- **PB-SENSE:** VERIFIED - Done-when remains unmet; existing TG hierarchy, dependencies, extensions, transitions, and Store locking provide the intended substrate.
- **PB-PLAN:** UNRESOLVED - blocked retry and same-Campaign exclusion contradicted current transition/claim behavior.
- **PB-CONTRACT:** UNRESOLVED - input, digest, path, status, and malformed-metadata behavior were underspecified.
- **PB-SHAPE:** RESOLVED-BY-SEARCH - pure projection/selection helpers can sit inside `src/workflow`; process and Store effects remain in the runner shell.

### Reconciliation required

- Retry through `tg todo`, not `tg unblock`, because blocking a `doing` item clears its claim while unblock restores unclaimed `doing` (`src/model/item.rs:137-177`).
- Exclude concurrent local runners with a Campaign-lifetime process lock; stable TG claims alone cannot distinguish two runners (`src/cli/commands/transition.rs:57-73`).
- Make ai-dotfiles WRK-048 a TG-009 template/plugin consumer instead of a second scheduler.
- Close the minimum template, digest, path, status, metadata, runtime-ignore, and verification contracts needed for an independent implementer.

The RFC was revised and its prior review/authorization markers removed. A fresh independent review and authorization are required before preparation restarts.

## Preparation attempt 2026-08-13 (revised RFC)

**Route:** PROCEED
**Verify contract:** `cargo test`; `cargo clippy`; `cargo fmt --check`; `just check`; focused Task tests; `git diff --check`

### Stack grounding

- **PB-EXISTS:** VERIFIED - workflow modules and commands remain absent.
- **PB-SENSE:** VERIFIED - the revised thin pilot remains useful and Done-when remains unmet.
- **PB-PLAN:** VERIFIED - retry uses the valid stopped-runner unblock-then-todo sequence, Campaign exclusion has its own process lock, and WRK-048 consumes rather than duplicates the runner.
- **PB-CONTRACT:** VERIFIED - `CHANGE.md` now owns the required node metadata, graph/context invariants, plugin request/result shape, public-data boundary, and bounded-note behavior.
- **PB-SHAPE:** VERIFIED - pure template/projection/selection logic is separated from the Store/process shell without adding another persistence layer.

### Readiness

- Fresh independent review marker and separate Hamilton authorization match the current behavioral fingerprint.
- The secret-handling safety acknowledgement is ticked and recorded in Decision-12 (approval).
- No material open question remains. The trusted-local, sequential, non-sandboxed limits are explicit.
