use std::fs;
use std::process::Command;

use criterion::{Criterion, criterion_group, criterion_main};

fn generate_items_jsonl(n: usize) -> String {
    let mut lines = Vec::new();
    lines.push("{\"schema_version\":1}".to_string());

    let now = "2026-02-24T12:00:00Z";
    for i in 0..n {
        let id = format!("tg-{:05x}", i);
        let item = serde_json::json!({
            "id": id,
            "title": format!("Item {}", i),
            "status": "todo",
            "priority": (i % 10) as i64,
            "description": null,
            "tags": [],
            "dependencies": [],
            "created_at": now,
            "updated_at": now,
            "blocked_reason": null,
            "blocked_from_status": null,
            "claimed_by": null,
            "claimed_at": null,
        });
        lines.push(serde_json::to_string(&item).unwrap());
    }

    lines.join("\n") + "\n"
}

fn bench_ready_500(c: &mut Criterion) {
    let tmp = tempfile::tempdir().unwrap();
    let project_dir = tmp.path().join(".task-golem");
    fs::create_dir_all(&project_dir).unwrap();

    // Write 500 items
    let content = generate_items_jsonl(500);
    fs::write(project_dir.join("tasks.jsonl"), content).unwrap();
    fs::write(
        project_dir.join("archive.jsonl"),
        "{\"schema_version\":1}\n",
    )
    .unwrap();
    fs::File::create(project_dir.join("tasks.lock")).unwrap();

    c.bench_function("tg ready --json (500 items)", |b| {
        b.iter(|| {
            let output = Command::new(assert_cmd::cargo::cargo_bin!("tg"))
                .current_dir(tmp.path())
                .args(["--json", "ready"])
                .output()
                .expect("failed to execute tg");
            assert!(output.status.success());
        });
    });

    c.bench_function("tg list --json (500 items)", |b| {
        b.iter(|| {
            let output = Command::new(assert_cmd::cargo::cargo_bin!("tg"))
                .current_dir(tmp.path())
                .args(["--json", "list"])
                .output()
                .expect("failed to execute tg");
            assert!(output.status.success());
        });
    });

    c.bench_function("tg next --json (500 items)", |b| {
        b.iter(|| {
            let output = Command::new(assert_cmd::cargo::cargo_bin!("tg"))
                .current_dir(tmp.path())
                .args(["--json", "next"])
                .output()
                .expect("failed to execute tg");
            assert!(output.status.success());
        });
    });

    c.bench_function("tg show --json (500 items)", |b| {
        b.iter(|| {
            let output = Command::new(assert_cmd::cargo::cargo_bin!("tg"))
                .current_dir(tmp.path())
                .args(["--json", "show", "tg-000fa"])
                .output()
                .expect("failed to execute tg");
            assert!(output.status.success());
        });
    });
}

criterion_group!(benches, bench_ready_500);
criterion_main!(benches);
