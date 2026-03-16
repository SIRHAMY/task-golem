# Run all checks: formatting, lints, and tests
check:
    cargo fmt --check
    cargo clippy -- -D warnings
    cargo test
