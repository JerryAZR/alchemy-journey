#!/usr/bin/env bash
# alchemy-journey's green gate: fmt + clippy + test, honest exit codes.
# (A piped `cargo test | grep` can mask a failing suite; don't hand-roll one.)
#
# Usage:
#   scripts/test.sh           fmt --check + clippy + test --workspace
#   scripts/test.sh --test    test only

set -u -o pipefail

fail=0

if [ "${1:-}" = "--test" ]; then
    cargo test --workspace || fail=1
    exit $fail
fi

echo "== cargo fmt --check =="
cargo fmt --all -- --check || fail=1

echo "== cargo clippy =="
cargo clippy --workspace --all-targets -- -D warnings || fail=1

echo "== cargo test --workspace =="
cargo test --workspace || fail=1

if [ "$fail" -ne 0 ]; then
    echo "GATE FAILED"
else
    echo "GATE PASSED"
fi
exit $fail
