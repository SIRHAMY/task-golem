//! Compile-enforced witness for status mutations.
//!
//! Every status-mutating `Item::apply_*` method returns a [`StatusChange`].
//! The witness is `#[must_use]` and cannot be constructed outside this crate,
//! so the only way to discharge it is to hand it to
//! [`crate::store::Store::commit_status_change`] or
//! [`crate::store::Store::commit_done`]. Those methods consume the witness by
//! value — and on the way, they fsync an event to `events.jsonl` **before**
//! rewriting `tasks.jsonl`.
//!
//! Combined with `#![deny(unused_must_use)]` at the crate root, this turns a
//! dropped witness into a compile error: if you mutate status you *must* go
//! through the chokepoint, so there is no way for a transition to occur
//! without its corresponding event.
//!
//! In tests that exercise `apply_*` directly (without a `Store`), consume the
//! witness via [`StatusChange::consume_for_test`]. Do **not** use `let _ =
//! apply_foo(...)` — that silences the lint and defeats the whole mechanism.
//!
//! # Contract surface
//!
//! Fields are fully private. The only readers are the `Store::commit_*`
//! methods via [`StatusChange::fields`] (`pub(crate)`). Construction is also
//! `pub(crate)` so callers outside the crate cannot forge a witness.

use crate::model::status::Status;

/// Proof that a status mutation was performed on an `Item` and has not yet
/// been durably committed. Must be redeemed via
/// [`crate::store::Store::commit_status_change`] or
/// [`crate::store::Store::commit_done`].
#[must_use = "StatusChange must be redeemed via Store::commit_status_change or commit_done"]
#[derive(Debug)]
pub struct StatusChange {
    task_id: String,
    new_status: Status,
    text: String,
}

impl StatusChange {
    /// Construct a witness. Crate-private so external callers cannot forge
    /// one; `Item::apply_*` methods are the only legitimate constructors.
    pub(crate) fn new(
        task_id: impl Into<String>,
        new_status: Status,
        text: impl Into<String>,
    ) -> Self {
        Self {
            task_id: task_id.into(),
            new_status,
            text: text.into(),
        }
    }

    /// Expose the captured fields for `Store::commit_*` to build the event.
    /// Crate-private: no external caller has a legitimate reason to inspect
    /// a witness without also consuming it.
    pub(crate) fn fields(&self) -> (&str, Status, &str) {
        (&self.task_id, self.new_status, &self.text)
    }

    /// Test-only consumer. Use this in tests that drive `apply_*` without a
    /// `Store`. Explicitly named so reviewers can spot inappropriate uses.
    ///
    /// Do **not** substitute `let _ = ...`: under `#![deny(unused_must_use)]`
    /// `let _ =` silences the lint and bypasses the chokepoint enforcement.
    /// `consume_for_test` keeps the discharge explicit and greppable.
    ///
    /// This method is intentionally always-available (not `#[cfg(test)]`) so
    /// integration tests in `tests/` can use it too. Non-test production
    /// callers have no reason to call it — `Store::commit_*` is the only
    /// legitimate discharge path — and the name itself flags misuse at
    /// review time.
    pub fn consume_for_test(self) {
        // Intentionally empty — the witness is dropped here, which is fine
        // because tests are not durable and own their own assertion paths.
        let _ = self;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn witness_fields_roundtrip() {
        let change = StatusChange::new("tg-abc00", Status::Doing, "starting work");
        let (id, status, text) = change.fields();
        assert_eq!(id, "tg-abc00");
        assert_eq!(status, Status::Doing);
        assert_eq!(text, "starting work");
    }

    #[test]
    fn consume_for_test_drops_witness() {
        let change = StatusChange::new("tg-abc00", Status::Todo, "");
        change.consume_for_test();
        // If we reach this line, `consume_for_test` took ownership cleanly.
    }
}
