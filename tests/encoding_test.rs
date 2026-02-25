mod common;

use common::TestProject;

#[test]
fn utf8_emoji_title() -> Result<(), Box<dyn std::error::Error>> {
    let project = TestProject::new()?;

    let json = project.run_tg_json(&["add", "Fix bug 🐛 in parser"]);
    assert_eq!(json["title"], "Fix bug 🐛 in parser");

    // Verify it round-trips through show
    let id = json["id"].as_str().unwrap();
    let shown = project.run_tg_json(&["show", id]);
    assert_eq!(shown["title"], "Fix bug 🐛 in parser");

    Ok(())
}

#[test]
fn utf8_cjk_characters() -> Result<(), Box<dyn std::error::Error>> {
    let project = TestProject::new()?;

    let json = project.run_tg_json(&["add", "修正バグ：日本語テスト"]);
    assert_eq!(json["title"], "修正バグ：日本語テスト");

    let id = json["id"].as_str().unwrap();
    let shown = project.run_tg_json(&["show", id]);
    assert_eq!(shown["title"], "修正バグ：日本語テスト");

    Ok(())
}

#[test]
fn utf8_rtl_text() -> Result<(), Box<dyn std::error::Error>> {
    let project = TestProject::new()?;

    let json = project.run_tg_json(&["add", "مهمة اختبار"]);
    assert_eq!(json["title"], "مهمة اختبار");

    let id = json["id"].as_str().unwrap();
    let shown = project.run_tg_json(&["show", id]);
    assert_eq!(shown["title"], "مهمة اختبار");

    Ok(())
}

#[test]
fn utf8_zero_width_joiners() -> Result<(), Box<dyn std::error::Error>> {
    let project = TestProject::new()?;

    // Family emoji with ZWJ: 👨‍👩‍👧‍👦
    let title = "Task \u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}\u{200D}\u{1F466}";
    let json = project.run_tg_json(&["add", title]);
    assert_eq!(json["title"], title);

    let id = json["id"].as_str().unwrap();
    let shown = project.run_tg_json(&["show", id]);
    assert_eq!(shown["title"], title);

    Ok(())
}

#[test]
fn utf8_mixed_scripts() -> Result<(), Box<dyn std::error::Error>> {
    let project = TestProject::new()?;

    let json = project.run_tg_json(&["add", "Hello 世界 مرحبا мир"]);
    assert_eq!(json["title"], "Hello 世界 مرحبا мир");

    Ok(())
}

#[test]
fn newline_check_handles_unicode() -> Result<(), Box<dyn std::error::Error>> {
    let project = TestProject::new()?;

    // Regular newlines should be rejected
    let output = project.run_tg(&["add", "line1\nline2"]);
    assert!(!output.status.success());

    let output = project.run_tg(&["add", "line1\r\nline2"]);
    assert!(!output.status.success());

    let output = project.run_tg(&["add", "line1\rline2"]);
    assert!(!output.status.success());

    // But multi-byte single-line titles should be accepted
    let json = project.run_tg_json(&["add", "Ñoño café résumé naïve"]);
    assert_eq!(json["title"], "Ñoño café résumé naïve");

    Ok(())
}

#[test]
fn utf8_in_description_and_tags() -> Result<(), Box<dyn std::error::Error>> {
    let project = TestProject::new()?;

    let json = project.run_tg_json(&[
        "add",
        "Emoji task",
        "--description",
        "🎯 Goal: fix the 🐛",
        "--tag",
        "バグ修正",
    ]);

    let id = json["id"].as_str().unwrap();
    let shown = project.run_tg_json(&["show", id]);
    assert_eq!(shown["description"], "🎯 Goal: fix the 🐛");
    assert!(shown["tags"]
        .as_array()
        .unwrap()
        .iter()
        .any(|t| t == "バグ修正"));

    Ok(())
}
