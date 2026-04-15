//! Event log for task mutations and free-text notes.
//!
//! Events are stored in an append-only `events.jsonl` file co-located with
//! `tasks.jsonl`. Each event is one JSON line; the format is documented in
//! [`record::Event`].
//!
//! # Concurrency contract
//!
//! The append path ([`append::write`]) relies on POSIX `O_APPEND` atomicity
//! for single-syscall writes under `PIPE_BUF` (4096 bytes on Linux). See
//! [`append`] for the full rationale and the constraints this places on the
//! implementation.
//!
//! # Forward compatibility
//!
//! Readers ([`read::for_task`], [`read::all`]) tolerate records with a
//! schema version (`v`) they do not understand: unknown versions are skipped
//! silently. Malformed lines warn-once-to-stderr and are skipped.

pub mod append;
pub mod archive;
pub mod author;
pub mod read;
pub mod record;
pub mod witness;

pub use record::{Event, EventType};
pub use witness::StatusChange;
