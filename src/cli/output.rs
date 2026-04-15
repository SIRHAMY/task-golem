use std::borrow::Cow;
use std::io::{self, Write};

use owo_colors::OwoColorize;
use owo_colors::Stream::Stdout;
use serde::Serialize;

use task_golem::events::Event;
use task_golem::model::item::Item;
use task_golem::model::status::Status;

/// Output a value as JSON to stdout.
pub fn print_json<T: Serialize>(value: &T) {
    let json = serde_json::to_string_pretty(value).unwrap();
    let stdout = io::stdout();
    let mut handle = stdout.lock();
    writeln!(handle, "{}", json).ok();
}

/// Output a human-readable message to stdout.
pub fn print_human(message: &str) {
    let stdout = io::stdout();
    let mut handle = stdout.lock();
    writeln!(handle, "{}", message).ok();
}

/// Output a value — JSON if `json_mode` is true, otherwise the human message.
#[allow(dead_code)]
pub fn output<T: Serialize>(json_mode: bool, value: &T, human_message: &str) {
    if json_mode {
        print_json(value);
    } else {
        print_human(human_message);
    }
}

/// Output an error — JSON to stderr if json_mode, otherwise plain text to stderr.
pub fn print_error(json_mode: bool, error: &task_golem::errors::TgError) {
    let stderr = io::stderr();
    let mut handle = stderr.lock();
    if json_mode {
        writeln!(handle, "{}", error.to_json()).ok();
    } else {
        writeln!(handle, "Error: {}", error).ok();
    }
}

/// Format a status string with conditional color based on terminal support.
/// Respects NO_COLOR and FORCE_COLOR env vars via owo-colors supports-colors feature.
fn format_status(status: Status) -> String {
    let label = status.to_string();
    match status {
        Status::Todo => format!(
            "{}",
            label.if_supports_color(Stdout, |text| text.white().to_string())
        ),
        Status::Doing => format!(
            "{}",
            label.if_supports_color(Stdout, |text| text.yellow().to_string())
        ),
        Status::Done => format!(
            "{}",
            label.if_supports_color(Stdout, |text| text.green().to_string())
        ),
        Status::Blocked => format!(
            "{}",
            label.if_supports_color(Stdout, |text| text.red().to_string())
        ),
    }
}

/// Truncate a string to a max width, appending "..." if truncated.
fn truncate(s: &str, max_width: usize) -> String {
    if s.len() <= max_width {
        s.to_string()
    } else if max_width <= 3 {
        s[..max_width].to_string()
    } else {
        format!("{}...", &s[..max_width - 3])
    }
}

/// Print a table of items to stdout (for list, ready commands).
pub fn print_item_table(items: &[Item]) {
    if items.is_empty() {
        print_human("No items found.");
        return;
    }

    let title_width = 50;

    let stdout = io::stdout();
    let mut handle = stdout.lock();

    // Header
    writeln!(handle, "{:<10}  {:<7}  {:>4}  TITLE", "ID", "STATUS", "PRI").ok();

    for item in items {
        writeln!(
            handle,
            "{:<10}  {:<7}  {:>4}  {}",
            item.id,
            format_status(item.status),
            item.priority,
            truncate(&item.title, title_width),
        )
        .ok();
    }
}

/// Print a full item detail view (for show command).
pub fn print_item_detail(item: &Item) {
    let stdout = io::stdout();
    let mut handle = stdout.lock();

    writeln!(handle, "ID:          {}", item.id).ok();
    writeln!(handle, "Title:       {}", item.title).ok();
    writeln!(handle, "Status:      {}", format_status(item.status)).ok();
    writeln!(handle, "Priority:    {}", item.priority).ok();

    if let Some(ref desc) = item.description {
        writeln!(handle, "Description: {}", desc).ok();
    }

    if !item.tags.is_empty() {
        writeln!(handle, "Tags:        {}", item.tags.join(", ")).ok();
    }

    if !item.dependencies.is_empty() {
        writeln!(handle, "Deps:        {}", item.dependencies.join(", ")).ok();
    }

    if let Some(ref reason) = item.blocked_reason {
        writeln!(handle, "Block reason:{}", reason).ok();
    }

    if let Some(ref agent) = item.claimed_by {
        writeln!(handle, "Claimed by:  {}", agent).ok();
    }

    writeln!(handle, "Created:     {}", item.created_at).ok();
    writeln!(handle, "Updated:     {}", item.updated_at).ok();

    if !item.extensions.is_empty() {
        for (key, value) in &item.extensions {
            writeln!(handle, "{}: {}", key, value).ok();
        }
    }
}

/// Print a "Children:" section for an item's direct children.
///
/// Caps visible rows at 10 and appends `(N more)` when truncated so terminal
/// output stays scannable for epics with dozens of children.
pub fn print_children_section(children: &[Item]) {
    const MAX_VISIBLE: usize = 10;
    let stdout = io::stdout();
    let mut handle = stdout.lock();
    writeln!(handle, "Children:").ok();

    let visible = children.len().min(MAX_VISIBLE);
    for child in &children[..visible] {
        writeln!(
            handle,
            "  {:<10}  {:<7}  {:>4}  {}",
            child.id,
            format_status(child.status),
            child.priority,
            truncate(&child.title, 50),
        )
        .ok();
    }

    if children.len() > MAX_VISIBLE {
        writeln!(handle, "  ({} more)", children.len() - MAX_VISIBLE).ok();
    }
}

/// Strip C0 (0x00–0x1F except `\t`) and C1 (0x80–0x9F) control bytes from a
/// string when `is_tty` is true. When `is_tty` is false (piped/redirected
/// output), the input is returned unchanged so downstream tools (jq, grep,
/// less) see the on-disk bytes verbatim.
///
/// `is_tty` is taken as a parameter rather than detected internally so tests
/// can exercise both branches without process-level isolation.
///
/// `\n` cannot occur in a JSONL value (it would tear the line on read), so it
/// does not need a special carve-out beyond the C0 strip semantics.
pub fn sanitize_for_tty(input: &str, is_tty: bool) -> Cow<'_, str> {
    if !is_tty {
        return Cow::Borrowed(input);
    }
    // Fast path: scan for any byte that needs stripping. The vast majority of
    // notes contain only printable ASCII or UTF-8 multibyte sequences (whose
    // continuation bytes are all >= 0x80 but in the 0x80..=0xBF range — the
    // C1 strip would mangle them). To preserve UTF-8 correctness, iterate over
    // chars and strip by codepoint, not byte.
    let needs_strip = input.chars().any(is_control_to_strip);
    if !needs_strip {
        return Cow::Borrowed(input);
    }
    let cleaned: String = input.chars().filter(|c| !is_control_to_strip(*c)).collect();
    Cow::Owned(cleaned)
}

fn is_control_to_strip(c: char) -> bool {
    let cp = c as u32;
    // C0: 0x00–0x1F except 0x09 (\t).
    if cp <= 0x1F && cp != 0x09 {
        return true;
    }
    // 0x7F DEL — stripping is conventional for terminal-safe rendering.
    if cp == 0x7F {
        return true;
    }
    // C1: 0x80–0x9F.
    if (0x80..=0x9F).contains(&cp) {
        return true;
    }
    false
}

/// Render a chronological event log to stdout in fixed-column human format.
///
/// Shared between `tg events` and (in Phase 5) `tg show --events`. When
/// `is_tty` is true, text fields are sanitized via [`sanitize_for_tty`] to
/// neutralize embedded escape sequences. Empty event slices print nothing
/// (no header, no rows) — callers should branch on emptiness if they want a
/// "no events" message.
pub fn print_events_human(events: &[Event], is_tty: bool) {
    if events.is_empty() {
        return;
    }
    let stdout = io::stdout();
    let mut handle = stdout.lock();
    writeln!(
        handle,
        "{:<27}  {:<24}  {:<17}  {:<7}  TEXT",
        "TIMESTAMP", "AUTHOR", "TYPE", "STATUS"
    )
    .ok();
    for event in events {
        let ts = event.ts.format("%Y-%m-%dT%H:%M:%S%.6fZ").to_string();
        let author = sanitize_for_tty(&event.author, is_tty);
        let type_str = event.event_type.to_string();
        let status_str = event
            .status
            .map(|s| s.to_string())
            .unwrap_or_else(|| "-".to_string());
        let text = sanitize_for_tty(&event.text, is_tty);
        writeln!(
            handle,
            "{:<27}  {:<24}  {:<17}  {:<7}  {}",
            ts, author, type_str, status_str, text,
        )
        .ok();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_passthrough_when_not_tty() {
        let input = "hello\x1b[31mworld";
        let out = sanitize_for_tty(input, false);
        assert_eq!(out.as_ref(), input);
        // Cow::Borrowed branch — no allocation.
        assert!(matches!(out, Cow::Borrowed(_)));
    }

    #[test]
    fn sanitize_strips_c0_when_tty() {
        let input = "hello\x1b[31mworld";
        let out = sanitize_for_tty(input, true);
        assert_eq!(out.as_ref(), "hello[31mworld");
    }

    #[test]
    fn sanitize_preserves_tab() {
        let input = "a\tb";
        let out = sanitize_for_tty(input, true);
        assert_eq!(out.as_ref(), "a\tb");
    }

    #[test]
    fn sanitize_strips_c1() {
        // U+0085 (NEL) is in the C1 range.
        let input = "a\u{0085}b";
        let out = sanitize_for_tty(input, true);
        assert_eq!(out.as_ref(), "ab");
    }

    #[test]
    fn sanitize_strips_del() {
        let input = "a\x7fb";
        let out = sanitize_for_tty(input, true);
        assert_eq!(out.as_ref(), "ab");
    }

    #[test]
    fn sanitize_preserves_plain_ascii() {
        let input = "hello world";
        let out = sanitize_for_tty(input, true);
        assert_eq!(out.as_ref(), input);
        // No alloc when nothing to strip.
        assert!(matches!(out, Cow::Borrowed(_)));
    }

    #[test]
    fn sanitize_preserves_unicode() {
        // Multi-byte chars whose codepoints are > 0x9F must pass through.
        let input = "café 日本語 🦀";
        let out = sanitize_for_tty(input, true);
        assert_eq!(out.as_ref(), input);
    }
}
