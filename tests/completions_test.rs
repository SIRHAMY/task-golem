mod common;

use common::TestProject;

#[test]
fn completions_bash_contains_subcommands() -> Result<(), Box<dyn std::error::Error>> {
    let project = TestProject::new()?;

    let output = project.run_tg(&["completions", "bash"]);
    assert!(output.status.success());

    let stdout = String::from_utf8_lossy(&output.stdout);

    // Verify all subcommand names appear in the completion script
    let expected_commands = [
        "init", "add", "list", "show", "edit", "rm", "do", "done", "todo", "block", "unblock",
        "ready", "next", "dep", "doctor", "archive", "dump", "completions",
    ];

    for cmd in &expected_commands {
        assert!(
            stdout.contains(cmd),
            "Completion script should contain subcommand '{}'",
            cmd
        );
    }

    Ok(())
}

#[test]
fn completions_zsh_produces_output() -> Result<(), Box<dyn std::error::Error>> {
    let project = TestProject::new()?;

    let output = project.run_tg(&["completions", "zsh"]);
    assert!(output.status.success());

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!stdout.is_empty());
    // zsh completions should contain the binary name
    assert!(stdout.contains("tg"));

    Ok(())
}

#[test]
fn completions_fish_produces_output() -> Result<(), Box<dyn std::error::Error>> {
    let project = TestProject::new()?;

    let output = project.run_tg(&["completions", "fish"]);
    assert!(output.status.success());

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!stdout.is_empty());
    assert!(stdout.contains("tg"));

    Ok(())
}

#[test]
fn completions_invalid_shell() -> Result<(), Box<dyn std::error::Error>> {
    let project = TestProject::new()?;

    let output = project.run_tg(&["completions", "notashell"]);
    assert!(!output.status.success());

    Ok(())
}
