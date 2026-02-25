use std::fs::OpenOptions;
use std::path::Path;
use std::time::{Duration, Instant};

use fd_lock::RwLock;

use crate::errors::TgError;

const INITIAL_BACKOFF_MS: u64 = 10;
const MAX_BACKOFF_MS: u64 = 500;
const TOTAL_TIMEOUT: Duration = Duration::from_secs(5);
const JITTER_FACTOR: f64 = 0.5; // 0-50% random jitter

/// Execute a callback while holding an exclusive file lock.
///
/// Backoff schedule: 10ms, 20ms, 40ms, ..., 500ms (cap), with 0-50% random jitter.
/// Total timeout: 5 seconds.
pub fn with_lock<F, R>(lock_path: &Path, callback: F) -> Result<R, TgError>
where
    F: FnOnce() -> Result<R, TgError>,
{
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(lock_path)
        .map_err(TgError::IoError)?;

    let mut lock = RwLock::new(file);

    let start = Instant::now();
    let mut backoff_ms = INITIAL_BACKOFF_MS;

    loop {
        match lock.try_write() {
            Ok(_lock_guard) => {
                // Lock acquired — callback runs while guard is alive.
                // _lock_guard is dropped (releasing lock) when this arm exits.
                return callback();
            }
            Err(_) => {
                if start.elapsed() >= TOTAL_TIMEOUT {
                    return Err(TgError::LockTimeout(TOTAL_TIMEOUT));
                }

                use rand::Rng;
                let jitter = rand::thread_rng().gen_range(0.0..JITTER_FACTOR);
                let sleep_ms = (backoff_ms as f64 * (1.0 + jitter)) as u64;
                std::thread::sleep(Duration::from_millis(sleep_ms));

                backoff_ms = (backoff_ms * 2).min(MAX_BACKOFF_MS);
            }
        }
    }
}

/// Calculate backoff delays for testing purposes (without jitter).
#[cfg(test)]
pub fn backoff_delays(count: usize) -> Vec<u64> {
    let mut delays = Vec::with_capacity(count);
    let mut backoff_ms = INITIAL_BACKOFF_MS;
    for _ in 0..count {
        delays.push(backoff_ms);
        backoff_ms = (backoff_ms * 2).min(MAX_BACKOFF_MS);
    }
    delays
}

/// The number of retry iterations before total backoff exceeds 5s (without jitter).
/// Used to verify the backoff schedule stays within timeout bounds.
#[cfg(test)]
fn max_retries_within_timeout() -> usize {
    let mut total_ms = 0u64;
    let mut backoff_ms = INITIAL_BACKOFF_MS;
    let mut count = 0;
    while total_ms + backoff_ms <= TOTAL_TIMEOUT.as_millis() as u64 {
        total_ms += backoff_ms;
        backoff_ms = (backoff_ms * 2).min(MAX_BACKOFF_MS);
        count += 1;
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn acquisition_succeeds_uncontended() {
        let tmp = tempfile::tempdir().unwrap();
        let lock_path = tmp.path().join("test.lock");
        let result = with_lock(&lock_path, || Ok(42));
        assert_eq!(result.unwrap(), 42);
    }

    #[test]
    fn raii_drop_releases_lock() {
        let tmp = tempfile::tempdir().unwrap();
        let lock_path = tmp.path().join("test.lock");

        // First lock
        let r1 = with_lock(&lock_path, || Ok("first"));
        assert!(r1.is_ok());

        // Second lock should succeed after first is released
        let r2 = with_lock(&lock_path, || Ok("second"));
        assert!(r2.is_ok());
    }

    #[test]
    fn backoff_calculation() {
        let delays = backoff_delays(10);
        assert_eq!(delays[0], 10);
        assert_eq!(delays[1], 20);
        assert_eq!(delays[2], 40);
        assert_eq!(delays[3], 80);
        assert_eq!(delays[4], 160);
        assert_eq!(delays[5], 320);
        assert_eq!(delays[6], 500); // capped
        assert_eq!(delays[7], 500);
        assert_eq!(delays[8], 500);
        assert_eq!(delays[9], 500);
    }

    #[test]
    fn backoff_fits_within_timeout() {
        let max = max_retries_within_timeout();
        let delays = backoff_delays(max);
        let total: u64 = delays.iter().sum();
        assert!(
            total <= TOTAL_TIMEOUT.as_millis() as u64,
            "Total backoff of {}ms exceeds timeout of {}ms with {} retries",
            total,
            TOTAL_TIMEOUT.as_millis(),
            max
        );
    }
}
