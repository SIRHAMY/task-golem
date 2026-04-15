//! Author resolution for events.
//!
//! Resolution order (first hit wins):
//!
//! 1. `TG_AUTHOR` environment variable, if set and non-empty after trim.
//! 2. `git config user.email`, if `git` is on `PATH`, exits successfully
//!    within [`DEFAULT_GIT_TIMEOUT`], and returns non-empty output.
//! 3. The literal string `"unknown"`.
//!
//! This function never errors and never panics. Missing `git`, a timeout,
//! a non-zero exit, or any other failure falls through to the next step.
//!
//! Author strings are *self-reported* and advisory — there is no
//! cryptographic attribution. Documented in the PRD.

use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

/// Default timeout for the `git config user.email` probe.
pub const DEFAULT_GIT_TIMEOUT: Duration = Duration::from_secs(2);

const FALLBACK: &str = "unknown";
const TG_AUTHOR_ENV: &str = "TG_AUTHOR";

/// Read environment variables. Abstracted so tests can inject values without
/// touching global process state (which would break test parallelism).
pub trait EnvReader {
    fn get(&self, key: &str) -> Option<String>;
}

/// Probe for the local git author. Abstracted so tests can simulate
/// timeouts, missing binaries, and non-zero exits deterministically.
pub trait GitProbe {
    /// Return the email reported by `git config user.email`, or `None` on
    /// timeout, missing binary, non-zero exit, or empty output.
    fn probe(&self, timeout: Duration) -> Option<String>;
}

/// Production [`EnvReader`] that reads from the real process environment.
pub struct RealEnv;

impl EnvReader for RealEnv {
    fn get(&self, key: &str) -> Option<String> {
        std::env::var(key).ok()
    }
}

/// Production [`GitProbe`] that spawns `git config user.email` and enforces a
/// timeout via a scratch thread + mpsc channel.
pub struct RealGit;

impl GitProbe for RealGit {
    fn probe(&self, timeout: Duration) -> Option<String> {
        probe_git_with_timeout(timeout)
    }
}

/// Convenience wrapper: resolve the author using [`RealEnv`] + [`RealGit`]
/// with the default timeout.
pub fn resolve() -> String {
    resolve_with(&RealEnv, &RealGit)
}

/// Testable core: resolve the author using injected env and git probes.
///
/// - `TG_AUTHOR` set to a non-whitespace value → returned (trimmed).
/// - Otherwise, `git.probe(DEFAULT_GIT_TIMEOUT)` returns `Some(email)` → returned.
/// - Otherwise, `"unknown"`.
pub(crate) fn resolve_with(env: &dyn EnvReader, git: &dyn GitProbe) -> String {
    if let Some(raw) = env.get(TG_AUTHOR_ENV) {
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }
    if let Some(email) = git.probe(DEFAULT_GIT_TIMEOUT) {
        let trimmed = email.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }
    FALLBACK.to_string()
}

/// Spawn `git config user.email`, wait up to `timeout`, and return the
/// trimmed output on success. `None` otherwise.
///
/// On timeout, we kill the child and reap it to avoid leaving zombies. We
/// steal `stdout` before handing the `Child` to a reader thread so the main
/// thread retains the `Child` and can call `kill()`/`wait()`. The reader
/// thread drains the `stdout` pipe into a byte buffer and signals
/// completion; the main thread then calls `wait()` to collect the exit
/// status and decide success vs. fall-through.
fn probe_git_with_timeout(timeout: Duration) -> Option<String> {
    let mut child = match Command::new("git")
        .args(["config", "user.email"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .stdin(Stdio::null())
        .spawn()
    {
        Ok(c) => c,
        Err(_) => return None, // git not on PATH, or spawn failed
    };

    // Steal stdout — the main thread keeps `child` for kill/wait.
    let stdout = match child.stdout.take() {
        Some(s) => s,
        None => {
            // No stdout pipe somehow. Reap and fall through.
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
    };

    // Reader thread drains stdout into a buffer and signals completion.
    let (tx, rx) = mpsc::channel::<std::io::Result<Vec<u8>>>();
    thread::spawn(move || {
        use std::io::Read;
        let mut buf = Vec::new();
        let mut stdout = stdout;
        let result = stdout.read_to_end(&mut buf).map(|_| buf);
        // Send may fail if the receiver timed out and was dropped; that's OK.
        let _ = tx.send(result);
    });

    let output = match rx.recv_timeout(timeout) {
        Ok(Ok(buf)) => buf,
        Ok(Err(_)) => {
            // Read failed; reap and fall through.
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        Err(_) => {
            // Timeout or reader thread died. Kill and reap.
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
    };

    // Reader succeeded; collect exit status (bounded — the pipe is already
    // closed, so wait() returns promptly).
    let status = match child.wait() {
        Ok(s) => s,
        Err(_) => return None,
    };
    if !status.success() {
        return None;
    }

    let text = String::from_utf8_lossy(&output).trim().to_string();
    if text.is_empty() { None } else { Some(text) }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Mutex;

    use super::*;

    struct FakeEnv {
        values: HashMap<String, String>,
    }

    impl FakeEnv {
        fn new() -> Self {
            Self {
                values: HashMap::new(),
            }
        }
        fn with(mut self, key: &str, value: &str) -> Self {
            self.values.insert(key.to_string(), value.to_string());
            self
        }
    }

    impl EnvReader for FakeEnv {
        fn get(&self, key: &str) -> Option<String> {
            self.values.get(key).cloned()
        }
    }

    enum GitBehavior {
        ReturnsEmail(String),
        Absent,
        Empty,
        TimesOut,
    }

    /// Fake git probe. Records the timeout it was called with so tests can
    /// assert it receives [`DEFAULT_GIT_TIMEOUT`]. `TimesOut` returns `None`
    /// immediately — it models the post-timeout state, not a real sleep, so
    /// the test suite stays fast.
    struct FakeGit {
        behavior: GitBehavior,
        observed_timeout: Mutex<Option<Duration>>,
    }

    impl FakeGit {
        fn new(behavior: GitBehavior) -> Self {
            Self {
                behavior,
                observed_timeout: Mutex::new(None),
            }
        }
    }

    impl GitProbe for FakeGit {
        fn probe(&self, timeout: Duration) -> Option<String> {
            *self.observed_timeout.lock().unwrap() = Some(timeout);
            match &self.behavior {
                GitBehavior::ReturnsEmail(e) => Some(e.clone()),
                GitBehavior::Absent => None,
                GitBehavior::Empty => Some(String::new()),
                GitBehavior::TimesOut => None,
            }
        }
    }

    #[test]
    fn tg_author_set_takes_precedence() {
        let env = FakeEnv::new().with("TG_AUTHOR", "alice@example.com");
        let git = FakeGit::new(GitBehavior::ReturnsEmail("bob@example.com".to_string()));
        let author = resolve_with(&env, &git);
        assert_eq!(author, "alice@example.com");
    }

    #[test]
    fn tg_author_is_trimmed() {
        let env = FakeEnv::new().with("TG_AUTHOR", "  alice@example.com  ");
        let git = FakeGit::new(GitBehavior::Absent);
        let author = resolve_with(&env, &git);
        assert_eq!(author, "alice@example.com");
    }

    #[test]
    fn tg_author_whitespace_only_falls_through() {
        let env = FakeEnv::new().with("TG_AUTHOR", "   ");
        let git = FakeGit::new(GitBehavior::ReturnsEmail("bob@example.com".to_string()));
        let author = resolve_with(&env, &git);
        assert_eq!(author, "bob@example.com");
    }

    #[test]
    fn tg_author_empty_falls_through() {
        let env = FakeEnv::new().with("TG_AUTHOR", "");
        let git = FakeGit::new(GitBehavior::ReturnsEmail("bob@example.com".to_string()));
        let author = resolve_with(&env, &git);
        assert_eq!(author, "bob@example.com");
    }

    #[test]
    fn git_email_used_when_tg_author_unset() {
        let env = FakeEnv::new();
        let git = FakeGit::new(GitBehavior::ReturnsEmail("bob@example.com".to_string()));
        let author = resolve_with(&env, &git);
        assert_eq!(author, "bob@example.com");
    }

    #[test]
    fn git_empty_output_falls_through_to_unknown() {
        let env = FakeEnv::new();
        let git = FakeGit::new(GitBehavior::Empty);
        let author = resolve_with(&env, &git);
        assert_eq!(author, "unknown");
    }

    #[test]
    fn git_absent_falls_through_to_unknown() {
        let env = FakeEnv::new();
        let git = FakeGit::new(GitBehavior::Absent);
        let author = resolve_with(&env, &git);
        assert_eq!(author, "unknown");
    }

    #[test]
    fn git_timeout_falls_through_to_unknown() {
        let env = FakeEnv::new();
        let git = FakeGit::new(GitBehavior::TimesOut);
        let author = resolve_with(&env, &git);
        assert_eq!(author, "unknown");
    }

    #[test]
    fn both_unset_returns_unknown() {
        let env = FakeEnv::new();
        let git = FakeGit::new(GitBehavior::Absent);
        let author = resolve_with(&env, &git);
        assert_eq!(author, "unknown");
    }

    #[test]
    fn default_timeout_is_passed_to_probe() {
        // Confirms the resolve_with -> probe call path uses
        // DEFAULT_GIT_TIMEOUT (and not some silently-introduced fallback).
        let env = FakeEnv::new();
        let git = FakeGit::new(GitBehavior::Absent);
        let _ = resolve_with(&env, &git);
        let observed = git.observed_timeout.lock().unwrap().unwrap();
        assert_eq!(observed, DEFAULT_GIT_TIMEOUT);
    }
}
