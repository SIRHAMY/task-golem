//! `O_APPEND` concurrency test for `events::append::write`.
//!
//! Correctness of the events log under concurrent writers rests on POSIX
//! `O_APPEND` atomicity for single-syscall writes under `PIPE_BUF`
//! (4096 bytes on Linux; our cap is 2048). This test spawns N concurrent
//! writers against the same events file and asserts:
//!
//! 1. Every line that was written appears exactly once (no loss).
//! 2. Every line parses as valid JSON (no torn interleaving).
//! 3. No line exceeds the 2048-byte cap.
//!
//! # Why threads, not `tg note` processes
//!
//! The SPEC's Phase 4 introduces `tg note`. Phase 3 cannot spawn
//! `tg note` processes because that verb does not yet exist. We exercise
//! the same code path at the library level: each thread opens its own
//! `OpenOptions::new().append(true).open(&path)` handle and calls
//! `events::append::write`. On Linux, `O_APPEND` atomicity applies
//! across *any* number of open file descriptions against the same inode
//! — threads within one process, or processes across the machine, behave
//! identically at the kernel level for `write(2)` under `PIPE_BUF`. A
//! follow-up Phase 4 test with real `tg note` subprocesses can layer on
//! top of this library-level guarantee.
//!
//! # Filesystem gating
//!
//! The `O_APPEND` contract holds on local POSIX filesystems (ext4, tmpfs,
//! xfs, btrfs, apfs). On NFS and some overlay filesystems it is weaker.
//! This test detects the filesystem type of the temp dir at startup and
//! skips with a visible `eprintln!` on unsupported filesystems. It never
//! silently `#[ignore]`s — a skip always prints the reason, so operators
//! running the test suite see what was (and was not) exercised.

use std::path::Path;
use std::sync::Arc;
use std::thread;

use task_golem::events::append as events_append;
use task_golem::events::record::Event;

/// Writers per concurrency pass. SPEC-mandated N=16 per-PR smoke.
const N_WRITERS: usize = 16;

/// Number of times the whole writer-fanout + assertion block runs in one
/// test invocation. Strengthens the signal without turning this into a
/// stress test (which is deferred to an `xtask` nightly per the SPEC).
const N_ITERATIONS: usize = 3;

/// Best-effort detection of whether the temp directory sits on a local
/// POSIX filesystem where `O_APPEND` atomicity is reliable.
///
/// Returns `Ok(())` if the FS is a supported local type, or `Err(reason)`
/// if the test should skip with a visible reason. On non-Linux platforms
/// this always errs toward skipping with a clear message, because the
/// concurrency contract is documented as Linux-local-POSIX in DESIGN.
///
/// Implementation note: we parse `/proc/self/mountinfo` rather than calling
/// `libc::statfs`, because the crate's implementation-boundary policy
/// (SPEC Approach section) forbids adding `libc` as a dependency.
/// mountinfo gives us the filesystem type as a string for the covering
/// mount, which is what we want here anyway.
fn detect_local_posix_fs(path: &Path) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    {
        // Canonicalize so symlinks inside /tmp, etc., resolve to the real
        // mount path.
        let canon = std::fs::canonicalize(path)
            .map_err(|e| format!("canonicalize {:?} failed: {}", path, e))?;
        let fs_type = covering_fs_type(&canon)
            .ok_or_else(|| format!("no mount covers {:?} in /proc/self/mountinfo", canon))?;

        // Accepted local POSIX filesystems — the ones that honor
        // O_APPEND + PIPE_BUF atomicity.
        match fs_type.as_str() {
            "ext2" | "ext3" | "ext4" | "tmpfs" | "xfs" | "btrfs" | "f2fs" => Ok(()),
            other => Err(format!(
                "skipping concurrency test: temp dir is on `{}`; \
                 only ext*, tmpfs, xfs, btrfs, f2fs guarantee the \
                 O_APPEND atomicity this test relies on",
                other
            )),
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = path;
        Err(format!(
            "skipping concurrency test: the O_APPEND + PIPE_BUF concurrency \
             contract is documented only for Linux local POSIX filesystems; \
             current target_os is `{}`",
            std::env::consts::OS
        ))
    }
}

/// Return the filesystem type (e.g. `"ext4"`) for the longest-prefix mount
/// that covers `path`, by reading `/proc/self/mountinfo`. Returns `None` if
/// mountinfo cannot be read or no mount covers the path.
#[cfg(target_os = "linux")]
fn covering_fs_type(path: &Path) -> Option<String> {
    // /proc/self/mountinfo line format (man 5 proc):
    //   mount_id parent_id major:minor root mount_point mount_opts ... - fstype source super_opts
    // We split on " - " to separate the optional-fields side from the
    // fstype-bearing side, then split by whitespace on each half.
    let contents = std::fs::read_to_string("/proc/self/mountinfo").ok()?;

    let mut best: Option<(usize, String)> = None; // (mount_point length, fstype)

    for line in contents.lines() {
        // Skip lines that don't match the expected mountinfo format rather
        // than failing the whole lookup. Any malformed mountinfo line on an
        // unrelated mount should not mask our ability to classify the temp
        // dir's FS.
        let mut halves = line.splitn(2, " - ");
        let (Some(left), Some(right)) = (halves.next(), halves.next()) else {
            continue;
        };

        let left_fields: Vec<&str> = left.split_whitespace().collect();
        // mount_point is field index 4 (0-indexed) in mountinfo.
        let Some(mount_point) = left_fields.get(4) else {
            continue;
        };
        let right_fields: Vec<&str> = right.split_whitespace().collect();
        let Some(fstype) = right_fields.first() else {
            continue;
        };

        // Only consider mounts whose mount point is an ancestor of `path`.
        if path.starts_with(mount_point) {
            let mp_len = mount_point.len();
            match &best {
                Some((best_len, _)) if *best_len >= mp_len => {}
                _ => best = Some((mp_len, (*fstype).to_string())),
            }
        }
    }

    best.map(|(_, fstype)| fstype)
}

#[test]
fn concurrent_appends_no_loss_no_tears() {
    let tmp = tempfile::tempdir().expect("create tempdir");

    if let Err(reason) = detect_local_posix_fs(tmp.path()) {
        // Visible skip with reason — never silent `#[ignore]`.
        eprintln!("CONCURRENCY TEST SKIPPED: {}", reason);
        return;
    }
    eprintln!(
        "Concurrency test running: {} writers x {} iterations against a local POSIX FS",
        N_WRITERS, N_ITERATIONS
    );

    for iteration in 0..N_ITERATIONS {
        let events_path = tmp.path().join(format!("events-{}.jsonl", iteration));
        let events_path = Arc::new(events_path);

        let mut handles = Vec::with_capacity(N_WRITERS);
        for i in 0..N_WRITERS {
            let path = Arc::clone(&events_path);
            let handle = thread::spawn(move || {
                // Distinct text per writer so we can verify every write
                // survived. The text is short (well under the 2048 cap) so
                // the full serialized line fits in a single write(2).
                let text = format!("writer-{:03}-iter-{}", i, iteration);
                let event = Event::note("tg-race0", format!("thread-{}", i), &text);
                events_append::write(&path, &event).expect("append should succeed");
                text
            });
            handles.push(handle);
        }

        let mut expected_texts: Vec<String> = Vec::with_capacity(N_WRITERS);
        for h in handles {
            expected_texts.push(h.join().expect("writer thread panicked"));
        }

        // Read the file back and verify.
        let contents = std::fs::read_to_string(events_path.as_path())
            .expect("read events file after concurrent appends");

        let lines: Vec<&str> = contents.lines().collect();

        // (a) Line count equals number of writers (no loss, no duplicates
        //     from short-write retries, no lost writes from torn lines).
        assert_eq!(
            lines.len(),
            N_WRITERS,
            "iteration {}: expected {} lines, got {}",
            iteration,
            N_WRITERS,
            lines.len()
        );

        // (b)/(c) Every line is valid JSON and under the cap.
        let mut seen_texts = std::collections::HashSet::new();
        for (idx, line) in lines.iter().enumerate() {
            assert!(
                line.len() + 1 <= task_golem::events::append::MAX_EVENT_LINE_BYTES,
                "iteration {}: line {} is {} bytes, exceeding cap {}",
                iteration,
                idx,
                line.len() + 1,
                task_golem::events::append::MAX_EVENT_LINE_BYTES
            );
            let parsed: serde_json::Value = serde_json::from_str(line).unwrap_or_else(|e| {
                panic!(
                    "iteration {}: line {} did not parse as JSON (torn write?): {}\nline: {:?}",
                    iteration, idx, e, line
                );
            });
            let text = parsed["text"]
                .as_str()
                .expect("every event has a text field")
                .to_string();
            assert!(
                seen_texts.insert(text.clone()),
                "iteration {}: duplicate text {:?} (possible retry or double-write)",
                iteration,
                text
            );
        }

        // (d) Every expected text appears exactly once.
        for expected in &expected_texts {
            assert!(
                seen_texts.contains(expected),
                "iteration {}: expected text {:?} missing from file",
                iteration,
                expected
            );
        }
    }
}
