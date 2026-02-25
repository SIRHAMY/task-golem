use std::io::{self, Write};

use owo_colors::OwoColorize;
use owo_colors::Stream::Stdout;
use serde::Serialize;

use crate::model::item::Item;
use crate::model::status::Status;

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
pub fn print_error(json_mode: bool, error: &crate::errors::TgError) {
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
