#!/bin/sh
# Run locally built Rust executables from an isolated, ad-hoc-signed directory.
# This avoids CoreText/CFBundle traversing target/debug/deps for font resources.
# The working directory is preserved for fixture-relative tests.
set -eu
binary=$1
shift
run_dir=$(mktemp -d "${TMPDIR:-/tmp}/yu-rust-test.XXXXXX")
trap 'rm -rf "$run_dir"' EXIT HUP INT TERM
cp "$binary" "$run_dir/test"
codesign --force --sign - "$run_dir/test" >/dev/null 2>&1
"$run_dir/test" "$@"
