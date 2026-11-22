#!/usr/bin/env bash
# The repo's single check command: formatting, lints (warnings are errors), tests.
# Run by the pre-commit hook (.githooks/pre-commit) and CI (.github/workflows/check.yml).
set -euo pipefail
cd "$(dirname "$0")/.."

cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
