//! Integration tests for the SQLite cache module.
//!
//! The cache module is inert from the CLI side in Phase 3 — no `tg query` command
//! yet — so these tests drive the library API directly via the `task_golem::cache`
//! module. Phase 4 will add the command wiring.

mod common;

use std::collections::BTreeMap;
use std::fs;

use chrono::Utc;
use rusqlite::params;
use task_golem::cache;
use task_golem::errors::TgError;
use task_golem::model::item::Item;
use task_golem::model::status::Status;
use task_golem::store::{Store, jsonl};

const ID_A: &str = "018f2b1c-4d5e-7abc-8123-000000000001";
const ID_B: &str = "018f2b1c-4d5e-7abc-8123-000000000002";
const ID_C: &str = "018f2b1c-4d5e-7abc-8123-000000000003";
const ID_D: &str = "018f2b1c-4d5e-7abc-8123-000000000004";

/// Build an `Item` with explicit `id` / `title` and optional parent + deps.
///
/// Small helper so each test stays a single readable block.
fn make_item(id: &str, title: &str) -> Item {
    let now = Utc::now();
    Item {
        id: id.to_string(),
        title: title.to_string(),
        status: Status::Todo,
        priority: 0,
        description: None,
        tags: vec![],
        dependencies: vec![],
        created_at: now,
        updated_at: now,
        blocked_reason: None,
        blocked_from_status: None,
        claimed_by: None,
        claimed_at: None,
        parent: None,
        extensions: BTreeMap::new(),
    }
}

fn store_of(project: &common::TestProject) -> Store {
    Store::new(project.project_dir())
}

fn write_items(store: &Store, items: &[Item]) {
    jsonl::write_atomic(&store.tasks_path(), items).unwrap();
}

#[test]
fn rebuild_from_empty_jsonl() {
    let project = common::TestProject::new().unwrap();
    let store = store_of(&project);

    let conn = cache::open_or_rebuild(&store, false).unwrap();

    let count: i64 = conn
        .query_row("SELECT count(*) FROM tasks", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 0);

    // Schema tables all present.
    let tables: Vec<String> = conn
        .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
        .unwrap()
        .query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert!(tables.contains(&"tasks".to_string()));
    assert!(tables.contains(&"task_tags".to_string()));
    assert!(tables.contains(&"task_deps".to_string()));
    assert!(tables.contains(&"task_view".to_string()));
    assert!(tables.contains(&"_cache_meta".to_string()));
}

#[test]
fn rebuild_populates_all_tables() {
    let project = common::TestProject::new().unwrap();
    let store = store_of(&project);

    let mut a = make_item(ID_A, "Parent");
    a.tags = vec!["backend".to_string(), "urgent".to_string()];

    let mut b = make_item(ID_B, "Child");
    b.parent = Some(ID_A.to_string());
    b.dependencies = vec![ID_A.to_string()];
    b.tags = vec!["backend".to_string()];

    write_items(&store, &[a, b]);

    let conn = cache::open_or_rebuild(&store, false).unwrap();

    let tasks: i64 = conn
        .query_row("SELECT count(*) FROM tasks", [], |r| r.get(0))
        .unwrap();
    assert_eq!(tasks, 2);

    let tags: i64 = conn
        .query_row("SELECT count(*) FROM task_tags", [], |r| r.get(0))
        .unwrap();
    assert_eq!(tags, 3);

    let deps: i64 = conn
        .query_row("SELECT count(*) FROM task_deps", [], |r| r.get(0))
        .unwrap();
    assert_eq!(deps, 1);

    let view: i64 = conn
        .query_row("SELECT count(*) FROM task_view", [], |r| r.get(0))
        .unwrap();
    assert_eq!(view, 2);
}

#[test]
fn rebuild_stamp_round_trips() {
    let project = common::TestProject::new().unwrap();
    let store = store_of(&project);

    write_items(&store, &[make_item(ID_A, "a")]);
    let _ = cache::open_or_rebuild(&store, false).unwrap();

    // Stamp must match the JSONL file on disk.
    let on_disk = cache::compute_stamp(&store.tasks_path()).unwrap();
    let conn = rusqlite::Connection::open(store.cache_db_path()).unwrap();
    let stored_mtime: String = conn
        .query_row(
            "SELECT value FROM _cache_meta WHERE key='jsonl_mtime_nanos'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(stored_mtime, on_disk.mtime_nanos.to_string());
}

#[test]
fn rebuild_is_idempotent() {
    let project = common::TestProject::new().unwrap();
    let store = store_of(&project);

    let mut a = make_item(ID_A, "a");
    a.tags = vec!["x".to_string()];
    write_items(&store, &[a]);

    let _ = cache::open_or_rebuild(&store, false).unwrap();
    let first_mtime = fs::metadata(store.cache_db_path())
        .unwrap()
        .modified()
        .unwrap();

    // Second open with unchanged JSONL: no rebuild.
    let _ = cache::open_or_rebuild(&store, false).unwrap();
    let second_mtime = fs::metadata(store.cache_db_path())
        .unwrap()
        .modified()
        .unwrap();

    assert_eq!(
        first_mtime, second_mtime,
        "second open should not rebuild when stamp matches"
    );
}

#[test]
fn stamp_mismatch_triggers_rebuild() {
    let project = common::TestProject::new().unwrap();
    let store = store_of(&project);

    write_items(&store, &[make_item(ID_A, "a")]);
    let _ = cache::open_or_rebuild(&store, false).unwrap();

    // Mutate JSONL behind the cache's back.
    write_items(&store, &[make_item(ID_A, "a"), make_item(ID_B, "b")]);

    let conn = cache::open_or_rebuild(&store, false).unwrap();
    let count: i64 = conn
        .query_row("SELECT count(*) FROM tasks", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 2, "stamp mismatch should rebuild with the new row");
}

#[test]
fn task_view_depth_from_root_correct() {
    let project = common::TestProject::new().unwrap();
    let store = store_of(&project);

    // Tree: root -> mid -> leaf.
    let root = make_item(ID_A, "root");
    let mut mid = make_item(ID_B, "mid");
    mid.parent = Some(ID_A.to_string());
    let mut leaf = make_item(ID_C, "leaf");
    leaf.parent = Some(ID_B.to_string());
    write_items(&store, &[root, mid, leaf]);

    let conn = cache::open_or_rebuild(&store, false).unwrap();

    let depth = |id: &str| -> i64 {
        conn.query_row(
            "SELECT depth_from_root FROM task_view WHERE id = ?1",
            params![id],
            |r| r.get(0),
        )
        .unwrap()
    };

    assert_eq!(depth(ID_A), 0);
    assert_eq!(depth(ID_B), 1);
    assert_eq!(depth(ID_C), 2);
}

#[test]
fn task_view_is_ready_correct() {
    let project = common::TestProject::new().unwrap();
    let store = store_of(&project);

    // `done_dep` is done; `ready_task` depends on it → is_ready=1.
    // `blocked_task` depends on an undone `todo_dep` → is_ready=0.
    let mut done_dep = make_item(ID_A, "done dep");
    done_dep.status = Status::Done;

    let todo_dep = make_item(ID_B, "todo dep");

    let mut ready_task = make_item(ID_C, "ready");
    ready_task.dependencies = vec![ID_A.to_string()];

    let mut blocked_task = make_item(ID_D, "blocked");
    blocked_task.dependencies = vec![ID_B.to_string()];

    write_items(&store, &[done_dep, todo_dep, ready_task, blocked_task]);

    let conn = cache::open_or_rebuild(&store, false).unwrap();

    let ready_of = |id: &str| -> i64 {
        conn.query_row(
            "SELECT is_ready FROM task_view WHERE id = ?1",
            params![id],
            |r| r.get(0),
        )
        .unwrap()
    };
    let unmet_of = |id: &str| -> i64 {
        conn.query_row(
            "SELECT unmet_dep_count FROM task_view WHERE id = ?1",
            params![id],
            |r| r.get(0),
        )
        .unwrap()
    };

    assert_eq!(ready_of(ID_C), 1);
    assert_eq!(unmet_of(ID_C), 0);
    assert_eq!(ready_of(ID_D), 0);
    assert_eq!(unmet_of(ID_D), 1);
    // done dep itself is not `todo` so is_ready=0.
    assert_eq!(ready_of(ID_A), 0);
}

#[test]
fn task_view_rebuild_tracks_archived_completion_evidence() {
    // Arrange
    let project = common::TestProject::new().unwrap();
    let store = store_of(&project);
    let mut archived_target = make_item(ID_A, "archived target");
    archived_target.status = Status::Done;
    jsonl::write_atomic(
        &store.archive_path(),
        std::slice::from_ref(&archived_target),
    )
    .unwrap();
    let mut dependent = make_item(ID_B, "dependent");
    dependent.dependencies = vec![ID_A.to_string()];
    write_items(&store, std::slice::from_ref(&dependent));

    // Act
    let initial = cache::open_or_rebuild(&store, false).unwrap();
    let initially_ready: i64 = initial
        .query_row(
            "SELECT is_ready FROM task_view WHERE id = ?1",
            params![ID_B],
            |row| row.get(0),
        )
        .unwrap();
    drop(initial);
    archived_target.status = Status::Todo;
    jsonl::write_atomic(&store.archive_path(), &[archived_target]).unwrap();
    let rebuilt = cache::open_or_rebuild(&store, false).unwrap();
    let (ready_after_evidence_removal, unmet_after_evidence_removal): (i64, i64) = rebuilt
        .query_row(
            "SELECT is_ready, unmet_dep_count FROM task_view WHERE id = ?1",
            params![ID_B],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();

    // Assert
    assert_eq!(initially_ready, 1);
    assert_eq!(ready_after_evidence_removal, 0);
    assert_eq!(unmet_after_evidence_removal, 1);
}

#[test]
fn cyclic_parent_aborts_rebuild() {
    let project = common::TestProject::new().unwrap();
    let store = store_of(&project);

    // Introduce a parent cycle by writing directly (bypasses CLI validation).
    let mut a = make_item(ID_A, "a");
    a.parent = Some(ID_B.to_string());
    let mut b = make_item(ID_B, "b");
    b.parent = Some(ID_A.to_string());
    write_items(&store, &[a, b]);

    let err = cache::open_or_rebuild(&store, false).unwrap_err();
    match err {
        TgError::ParentCycle { ids } => {
            assert!(
                ids.iter().any(|id| id == ID_A) && ids.iter().any(|id| id == ID_B),
                "cycle should name the offending ids, got: {:?}",
                ids
            );
        }
        other => panic!("expected ParentCycle, got {:?}", other),
    }
}

#[test]
fn cyclic_dep_aborts_rebuild() {
    let project = common::TestProject::new().unwrap();
    let store = store_of(&project);

    let mut a = make_item(ID_A, "a");
    a.dependencies = vec![ID_B.to_string()];
    let mut b = make_item(ID_B, "b");
    b.dependencies = vec![ID_A.to_string()];
    write_items(&store, &[a, b]);

    let err = cache::open_or_rebuild(&store, false).unwrap_err();
    match err {
        TgError::CycleDetected(msg) => {
            assert!(
                msg.contains(ID_A) && msg.contains(ID_B),
                "message should name offending ids: {}",
                msg
            );
            assert!(
                msg.contains("tg doctor"),
                "message should suggest tg doctor: {}",
                msg
            );
        }
        other => panic!("expected CycleDetected, got {:?}", other),
    }
}

#[test]
fn duplicate_id_aborts_rebuild() {
    let project = common::TestProject::new().unwrap();
    let store = store_of(&project);

    // Hand-craft corruption because the public writer rejects duplicate IDs.
    let a = serde_json::to_string(&make_item(ID_A, "first")).unwrap();
    let b = serde_json::to_string(&make_item(ID_A, "second")).unwrap();
    fs::write(
        store.tasks_path(),
        format!("{{\"schema_version\":1}}\n{a}\n{b}\n"),
    )
    .unwrap();

    let err = cache::open_or_rebuild(&store, false).unwrap_err();
    match err {
        TgError::StorageCorruption(msg) => {
            assert!(msg.contains("duplicate"), "msg: {}", msg);
            assert!(msg.contains(ID_A), "msg: {}", msg);
        }
        other => panic!("expected StorageCorruption, got {:?}", other),
    }
}

#[test]
fn schema_version_mismatch_rebuilds() {
    let project = common::TestProject::new().unwrap();
    let store = store_of(&project);

    write_items(&store, &[make_item(ID_A, "a")]);
    let _ = cache::open_or_rebuild(&store, false).unwrap();

    // Poke the schema version to pretend the cache came from an older build.
    let conn = rusqlite::Connection::open(store.cache_db_path()).unwrap();
    conn.execute(
        "UPDATE _cache_meta SET value = '0' WHERE key = 'schema_version'",
        [],
    )
    .unwrap();
    drop(conn);

    let conn = cache::open_or_rebuild(&store, false).unwrap();
    let v: String = conn
        .query_row(
            "SELECT value FROM _cache_meta WHERE key='schema_version'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        v,
        cache::SCHEMA_VERSION.to_string(),
        "rebuild should restore current schema_version"
    );
}

#[test]
fn rebuild_writes_gitignore() {
    let project = common::TestProject::new().unwrap();
    let store = store_of(&project);

    // `tg init` already writes gitignore; remove it to prove rebuild restores it.
    let gi = store.gitignore_path();
    let _ = fs::remove_file(&gi);

    write_items(&store, &[make_item(ID_A, "a")]);
    let _ = cache::open_or_rebuild(&store, false).unwrap();

    let contents = fs::read_to_string(&gi).unwrap();
    assert!(contents.lines().any(|l| l == "cache.db"));
    assert!(contents.lines().any(|l| l == "cache.db-journal"));
    assert!(contents.lines().any(|l| l == "cache.db.tmp-*"));
}

/// When the cache path is unwritable, the rebuild falls back to an in-memory DB.
///
/// We simulate "unwritable" by `chmod 0555` on the project dir so opening
/// `cache.db` for write fails. The file lock (`tasks.lock`) remains readable
/// because it was created during `tg init` before we revoked write perms.
#[cfg(unix)]
#[test]
fn unwritable_cache_dir_falls_back_to_memory() {
    use std::os::unix::fs::PermissionsExt;

    let project = common::TestProject::new().unwrap();
    let store = store_of(&project);

    write_items(&store, &[make_item(ID_A, "a")]);

    let dir = project.project_dir();
    let original = fs::metadata(&dir).unwrap().permissions();
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o555)).unwrap();

    // Must always restore perms so the tempdir cleans up.
    let result = std::panic::catch_unwind(|| {
        let conn = cache::open_or_rebuild(&store, true).unwrap();
        let count: i64 = conn
            .query_row("SELECT count(*) FROM tasks", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1);
    });

    fs::set_permissions(&dir, original).unwrap();
    result.unwrap();

    // No cache.db should have been created.
    assert!(
        !store.cache_db_path().exists(),
        "unwritable fallback should not create cache.db"
    );
}

/// Performance fixture: rebuild a realistic-sized store and print timing.
///
/// Timing is recorded in Phase Notes. Not a pass/fail test — prints to stderr so
/// the author can read it; asserts only a generous ceiling (60s) to catch a
/// catastrophic regression.
#[test]
fn rebuild_perf_500_and_5000() {
    use std::time::Instant;

    for n in [500, 5000] {
        let project = common::TestProject::new().unwrap();
        let store = store_of(&project);

        let mut items = Vec::with_capacity(n);
        for i in 0..n {
            let id = format!("018f2b1c-4d5e-7abc-8123-{i:012x}");
            let mut item = make_item(&id, &format!("task #{}", i));
            // ~300-byte description so the JSONL hash is exercised realistically.
            item.description = Some("x".repeat(300));
            if i > 0 && i % 5 == 0 {
                item.parent = Some(format!("018f2b1c-4d5e-7abc-8123-{:012x}", i - 1));
            }
            if i > 1 && i % 7 == 0 {
                item.dependencies = vec![format!("018f2b1c-4d5e-7abc-8123-{:012x}", i - 1)];
            }
            items.push(item);
        }
        write_items(&store, &items);

        let start = Instant::now();
        let _ = cache::open_or_rebuild(&store, false).unwrap();
        let elapsed = start.elapsed();
        eprintln!("cache rebuild for {} tasks: {:?}", n, elapsed);
        assert!(elapsed.as_secs() < 60, "rebuild took {:?}", elapsed);
    }
}
