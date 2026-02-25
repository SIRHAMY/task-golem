use std::fs::OpenOptions;
use std::io::{Read, Write};
use std::process::Command;
use std::time::Duration;

use assert_cmd::cargo::cargo_bin;

/// Cross-process lock PoC test.
///
/// Spawns a child process that uses fcntl/flock to hold the lock,
/// then verifies the parent process cannot acquire it via fd-lock.
#[test]
fn cross_process_lock_mutual_exclusion() {
    let tmp = tempfile::tempdir().unwrap();
    let lock_path = tmp.path().join("tasks.lock");

    // Create the lock file
    OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(&lock_path)
        .unwrap();

    // Use a Python script that uses fcntl.flock (same syscall fd-lock uses on Linux)
    let script_path = tmp.path().join("hold_lock.py");
    {
        let mut script = std::fs::File::create(&script_path).unwrap();
        writeln!(
            script,
            r#"import fcntl, time, sys
f = open(sys.argv[1], 'r+')
fcntl.flock(f.fileno(), fcntl.LOCK_EX)
sys.stdout.write("locked\n")
sys.stdout.flush()
time.sleep(10)"#
        )
        .unwrap();
    }

    // Start child process that holds the lock
    let mut child = Command::new("python3")
        .arg(&script_path)
        .arg(&lock_path)
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("Failed to spawn child process");

    // Wait for child to signal it has the lock
    let stdout = child.stdout.as_mut().unwrap();
    let mut buf = [0u8; 7]; // "locked\n"
    stdout.read_exact(&mut buf).unwrap();
    assert_eq!(&buf, b"locked\n");

    // Try to acquire the lock from the parent — should fail since child holds it
    {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&lock_path)
            .unwrap();
        let mut lock = fd_lock::RwLock::new(file);

        let result = lock.try_write();
        assert!(
            result.is_err(),
            "Parent should NOT be able to acquire lock while child holds it"
        );
    }

    // Kill child process to release the lock
    child.kill().ok();
    child.wait().ok();

    // Small delay for OS to clean up
    std::thread::sleep(Duration::from_millis(100));

    // After child is killed, lock should be released
    {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&lock_path)
            .unwrap();
        let mut lock = fd_lock::RwLock::new(file);

        let result = lock.try_write();
        assert!(
            result.is_ok(),
            "Parent should be able to acquire lock after child is killed"
        );
    }
}

/// Test that `tg init` creates a working lock file that can be acquired.
#[test]
fn init_creates_lockable_file() {
    let tmp = tempfile::tempdir().unwrap();

    let output = Command::new(cargo_bin("tg"))
        .current_dir(tmp.path())
        .args(["init"])
        .output()
        .unwrap();
    assert!(output.status.success());

    let lock_path = tmp.path().join(".task-golem").join("tasks.lock");
    assert!(lock_path.exists());

    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&lock_path)
        .unwrap();
    let mut lock = fd_lock::RwLock::new(file);
    let result = lock.try_write();
    assert!(
        result.is_ok(),
        "Should be able to acquire lock on init'd project"
    );
}
