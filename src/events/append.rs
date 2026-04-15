//! Single-syscall append for event records.
//!
//! # Concurrency contract
//!
//! Correctness under concurrent appends depends on POSIX `O_APPEND`
//! atomicity for writes that fit in a single `write(2)` call under the
//! kernel's `PIPE_BUF` (4096 bytes on Linux). If we split the payload into
//! multiple `write(2)` calls — which is what `BufWriter`, `write_all`, or
//! `writeln!` would do for a long line — two concurrent processes can
//! interleave their output and tear a line.
//!
//! Therefore this module **must not** use:
//!
//! - `BufWriter` (buffers until drop/flush; usually one write, but not
//!   guaranteed and easy to break later).
//! - `write_all` (loops on short writes, producing multiple syscalls).
//! - `writeln!` / `write!` (expand into `write_fmt`, which calls `write_all`
//!   internally).
//!
//! The public entry point serializes the event, enforces a 2048-byte
//! (well under `PIPE_BUF`) line cap, and calls `std::io::Write::write`
//! **exactly once**. A short write is treated as a hard error (not retried),
//! because retrying would violate the single-syscall contract and could
//! produce torn output under concurrent load.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;

use crate::errors::TgError;
use crate::events::record::Event;

/// Maximum serialized line length, including the trailing newline.
///
/// Set well under Linux `PIPE_BUF` (4096) to preserve headroom and match
/// the PRD's 2048-byte contract. Lines exceeding this cap are rejected
/// with `InvalidInput` before any I/O is attempted.
pub const MAX_EVENT_LINE_BYTES: usize = 2048;

/// Append a single event to `path`. The file is created if missing.
///
/// The event is serialized to JSON with a trailing newline, the 2048-byte
/// cap is enforced on the full line, a single `write(2)` is issued under
/// `O_APPEND`, and `sync_data()` is called before returning.
///
/// See the module-level docs for why `BufWriter`, `write_all`, and `writeln!`
/// are forbidden on this path.
pub fn write(path: &Path, event: &Event) -> Result<(), TgError> {
    let mut line = serde_json::to_string(event)
        .expect("Event serialization cannot fail: all fields are safely typed");
    line.push('\n');

    if line.len() > MAX_EVENT_LINE_BYTES {
        let overflow = line.len() - MAX_EVENT_LINE_BYTES;
        return Err(TgError::InvalidInput(format!(
            "Event line is {} bytes, exceeding the {}-byte cap by {} bytes. \
             Shorten the text by at least {} bytes.",
            line.len(),
            MAX_EVENT_LINE_BYTES,
            overflow,
            overflow
        )));
    }

    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(TgError::IoError)?;

    write_with_writer(&mut file, line.as_bytes())?;

    // sync_data() durably persists file content; metadata sync is not
    // required for correctness of the append (see PRD "fsync" clause).
    file.sync_data().map_err(TgError::IoError)?;

    Ok(())
}

/// Internal single-`write` helper. Exposed (crate-private) as a test seam so
/// short-write behavior can be exercised without involving a real file.
///
/// Short writes are treated as `StorageCorruption`: retrying would break the
/// single-syscall contract under concurrent load. In practice, short writes
/// on a regular local file are vanishingly rare, but we must fail loudly
/// rather than silently tear lines.
pub(crate) fn write_with_writer<W: Write>(w: &mut W, bytes: &[u8]) -> Result<(), TgError> {
    let n = w.write(bytes).map_err(TgError::IoError)?;
    if n != bytes.len() {
        return Err(TgError::StorageCorruption(format!(
            "Short write on events append: wrote {} of {} bytes; \
             refusing to retry to avoid torn concurrent lines.",
            n,
            bytes.len()
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io::{self, Write};

    use super::*;
    use crate::events::record::Event;
    use crate::model::status::Status;

    /// Writer that always short-writes by `short_by` bytes.
    struct TrackingWriter {
        buf: Vec<u8>,
        short_by: usize,
    }

    impl Write for TrackingWriter {
        fn write(&mut self, data: &[u8]) -> io::Result<usize> {
            let to_write = data.len().saturating_sub(self.short_by);
            self.buf.extend_from_slice(&data[..to_write]);
            Ok(to_write)
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn append_creates_file_on_first_write() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("events.jsonl");
        assert!(!path.exists());

        let event = Event::note("tg-abc00", "alice", "first");
        write(&path, &event).unwrap();

        assert!(path.exists());
        let contents = fs::read_to_string(&path).unwrap();
        assert!(contents.contains("tg-abc00"));
        assert!(contents.ends_with('\n'));
    }

    #[test]
    fn second_append_preserves_first_line() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("events.jsonl");

        let first = Event::note("tg-abc00", "alice", "first");
        let second = Event::note("tg-abc00", "alice", "second");

        write(&path, &first).unwrap();
        write(&path, &second).unwrap();

        let contents = fs::read_to_string(&path).unwrap();
        let lines: Vec<_> = contents.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("first"));
        assert!(lines[1].contains("second"));
    }

    #[test]
    fn cap_accepts_line_at_limit_minus_one() {
        // Construct an event whose serialized form + newline is exactly
        // MAX_EVENT_LINE_BYTES - 1. We pad `text` to hit the target.
        let target = MAX_EVENT_LINE_BYTES - 1;
        let event = padded_event(target);
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("events.jsonl");
        write(&path, &event).expect("2047-byte line should be accepted");
    }

    #[test]
    fn cap_accepts_line_at_limit() {
        let target = MAX_EVENT_LINE_BYTES;
        let event = padded_event(target);
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("events.jsonl");
        write(&path, &event).expect("2048-byte line should be accepted");
    }

    #[test]
    fn cap_rejects_line_over_limit() {
        let target = MAX_EVENT_LINE_BYTES + 1;
        let event = padded_event(target);
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("events.jsonl");
        match write(&path, &event) {
            Err(TgError::InvalidInput(msg)) => {
                assert!(
                    msg.contains("2048"),
                    "error should mention the cap: {}",
                    msg
                );
            }
            other => panic!("Expected InvalidInput, got: {:?}", other),
        }
        // File should not have been created because the check runs first.
        assert!(!path.exists());
    }

    #[test]
    fn short_write_path_reports_storage_corruption() {
        let mut w = TrackingWriter {
            buf: Vec::new(),
            short_by: 1,
        };
        let bytes = b"hello world\n";
        let result = write_with_writer(&mut w, bytes);
        match result {
            Err(TgError::StorageCorruption(msg)) => {
                assert!(
                    msg.contains("Short write"),
                    "error should mention short write: {}",
                    msg
                );
            }
            other => panic!("Expected StorageCorruption, got: {:?}", other),
        }
    }

    #[test]
    fn full_write_path_succeeds_via_seam() {
        let mut w = TrackingWriter {
            buf: Vec::new(),
            short_by: 0,
        };
        let bytes = b"hello world\n";
        write_with_writer(&mut w, bytes).unwrap();
        assert_eq!(w.buf, bytes);
    }

    /// Build an event whose JSON + trailing newline is exactly `target_bytes`
    /// (or as close as possible without exceeding). Used for cap-boundary
    /// tests. We pad the `text` field to drive the total size.
    fn padded_event(target_bytes: usize) -> Event {
        // Build a baseline event and measure its serialized size with a
        // single-char text, then pad up to the target.
        let mut event =
            Event::status_transition("tg-abc00", "alice@example.com", Status::Blocked, "x");
        let baseline = serde_json::to_string(&event).unwrap().len() + 1; // +1 for newline
        assert!(
            baseline <= target_bytes,
            "baseline ({}) already exceeds target ({})",
            baseline,
            target_bytes
        );
        let padding = target_bytes - baseline;
        event.text = "x".repeat(1 + padding);
        event
    }
}
