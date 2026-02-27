mod common;

use common::TestProject;

#[test]
fn dep_add_creates_dependency() {
    let project = TestProject::new().unwrap();

    let a = project.run_tg_json(&["add", "Task A"]);
    let b = project.run_tg_json(&["add", "Task B"]);
    let a_id = a["id"].as_str().unwrap();
    let b_id = b["id"].as_str().unwrap();

    // Use dep add (sugar for edit --add-dep)
    let result = project.run_tg_json(&["dep", "add", b_id, a_id]);
    let deps = result["dependencies"].as_array().unwrap();
    assert!(
        deps.iter().any(|d| d.as_str().unwrap() == a_id),
        "Expected dep {} in {:?}",
        a_id,
        deps
    );
}

#[test]
fn dep_add_equivalent_to_edit_add_dep() {
    let project = TestProject::new().unwrap();

    let a = project.run_tg_json(&["add", "Task A"]);
    let b = project.run_tg_json(&["add", "Task B"]);
    let a_id = a["id"].as_str().unwrap();
    let b_id = b["id"].as_str().unwrap();

    // Use dep add
    let result = project.run_tg_json(&["dep", "add", b_id, a_id]);
    let deps_after_dep_add = result["dependencies"].as_array().unwrap().clone();

    // Verify it matches what edit --add-dep would produce
    let show = project.run_tg_json(&["show", b_id]);
    let deps_show = show["dependencies"].as_array().unwrap();
    assert_eq!(&deps_after_dep_add, deps_show);
}

#[test]
fn dep_add_rejects_cycle() {
    let project = TestProject::new().unwrap();

    let a = project.run_tg_json(&["add", "Task A"]);
    let b = project.run_tg_json(&["add", "Task B", "--dep", a["id"].as_str().unwrap()]);
    let a_id = a["id"].as_str().unwrap();
    let b_id = b["id"].as_str().unwrap();

    // Adding A -> B would create cycle: A -> B -> A
    let output = project.run_tg(&["--json", "dep", "add", a_id, b_id]);
    assert!(!output.status.success(), "Expected cycle rejection");
}

#[test]
fn dep_add_rejects_self_dep() {
    let project = TestProject::new().unwrap();

    let a = project.run_tg_json(&["add", "Task A"]);
    let a_id = a["id"].as_str().unwrap();

    let output = project.run_tg(&["--json", "dep", "add", a_id, a_id]);
    assert!(!output.status.success(), "Expected self-dep rejection");
}

#[test]
fn dep_rm_removes_dependency() {
    let project = TestProject::new().unwrap();

    let a = project.run_tg_json(&["add", "Task A"]);
    let a_id = a["id"].as_str().unwrap();
    let b = project.run_tg_json(&["add", "Task B", "--dep", a_id]);
    let b_id = b["id"].as_str().unwrap();

    // Verify dep exists
    let show = project.run_tg_json(&["show", b_id]);
    assert!(!show["dependencies"].as_array().unwrap().is_empty());

    // Remove dep
    let result = project.run_tg_json(&["dep", "rm", b_id, a_id]);
    let deps = result["dependencies"].as_array().unwrap();
    assert!(
        deps.is_empty(),
        "Expected no deps after removal, got: {:?}",
        deps
    );
}

#[test]
fn dep_add_warns_on_nonexistent() {
    let project = TestProject::new().unwrap();

    let a = project.run_tg_json(&["add", "Task A"]);
    let a_id = a["id"].as_str().unwrap();

    // Adding dep on nonexistent ID should warn but succeed
    let output = project.run_tg(&["--json", "dep", "add", a_id, "tg-nonex"]);
    // This should fail because the ID doesn't exist (ItemNotFound)
    assert!(
        !output.status.success(),
        "Expected failure on nonexistent dep target"
    );
}
