use std::io::{self, Write};

use serde::Serialize;

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
