#!/usr/bin/env bash
# comment-density.sh — audit the comment density of Rust *production* code.
#
# Usage:
#   scripts/comment-density.sh <file> [<file> ...]
#   scripts/comment-density.sh crates/amity-core/src/inbox.rs
#
# Exit codes:
#   0  All checked files meet the ≥50% threshold (or were skipped as test code).
#   1  One or more files are below threshold or an argument error occurred.
#
# Scope — TEST CODE IS NOT GATED:
#   The density target is about keeping production code well-explained. Test code
#   is verbose by nature (fixtures, table cases, assertions) and gating it pushes
#   toward padding rather than better comments, so two kinds of test code are
#   excluded from the measurement:
#     • Integration-test files — any path under a `tests/` directory is skipped
#       entirely (the whole file is a test crate).
#     • Unit-test modules — a `#[cfg(test)]` block (the conventional
#       `#[cfg(test)] mod tests { … }`, or a `#[cfg(test)]` item) is stripped
#       from a source file before its lines are counted.
#
# What counts as a comment line (in the remaining production lines):
#   Lines that, after stripping leading whitespace, start with // or ///.
#   Block comments (/* ... */) are not counted — they are rare in this codebase
#   and handling multi-line blocks correctly in bash is error-prone.
#
# What counts as a code line:
#   Non-empty, non-blank production lines that are not comment lines.
#
# The 50% target means: comment_lines / (comment_lines + code_lines) >= 0.5
# Blank lines are excluded from both counts — they carry no information either way.

set -euo pipefail

# Minimum ratio expressed as an integer percentage (0–100).
THRESHOLD=50

# Track whether any file failed so we can return a non-zero exit code at the end
# without stopping the loop early (we want to report all failing files, not just
# the first one).
any_failed=0

if [[ $# -eq 0 ]]; then
    echo "Usage: $0 <file> [<file> ...]" >&2
    exit 1
fi

for file in "$@"; do
    if [[ ! -f "$file" ]]; then
        echo "ERROR: not a file: $file" >&2
        any_failed=1
        continue
    fi

    # Integration-test files live under a `tests/` directory and are entirely
    # test code; they are not gated. Report and move on.
    if [[ "$file" == */tests/* ]]; then
        echo "SKIP  $file  (integration test file)"
        continue
    fi

    # One awk pass counts comment and code lines while skipping any `#[cfg(test)]`
    # block, so unit tests do not count toward the ratio. Brace depth is tracked
    # from the attribute's following item (a `mod` or `fn`) until it closes.
    # This relies on the codebase convention that `#[cfg(test)]` introduces a
    # braced item — which is true throughout (`#[cfg(test)] mod tests { … }`).
    read -r comment_lines code_lines < <(awk '
        # A #[cfg(test)] attribute starts a test block we skip entirely.
        /^[[:space:]]*#\[cfg\(test\)\]/ { in_test = 1; depth = 0; started = 0; next }
        in_test {
            # Count braces on this line to find where the test item closes.
            opens = gsub(/\{/, "&"); closes = gsub(/\}/, "&");
            depth += opens - closes;
            if (opens > 0) started = 1;
            # Once we have opened and returned to depth 0, the block is done.
            if (started && depth <= 0) in_test = 0;
            next
        }
        # Production comment line (// or ///).
        /^[[:space:]]*\/\// { comment++; next }
        # Blank line — ignored by both counts.
        /^[[:space:]]*$/ { next }
        # Anything else is a production code line.
        { code++ }
        END { printf "%d %d\n", comment + 0, code + 0 }
    ' "$file")

    # Guard against a file with no production lines (e.g. all test code).
    if [[ $(( comment_lines + code_lines )) -eq 0 ]]; then
        echo "SKIP  $file  (no non-blank production lines)"
        continue
    fi

    # Integer percentage: (comment_lines * 100) / (comment_lines + code_lines).
    # Integer division truncates, so 49.9% becomes 49 and fails the check.
    # This is intentional — the threshold is a floor, not a target.
    ratio=$(( (comment_lines * 100) / (comment_lines + code_lines) ))

    if [[ $ratio -ge $THRESHOLD ]]; then
        echo "OK    $file  (${ratio}% comments, ${comment_lines} comment / ${code_lines} code)"
    else
        echo "FAIL  $file  (${ratio}% comments, ${comment_lines} comment / ${code_lines} code — need ${THRESHOLD}%)"
        any_failed=1
    fi
done

exit "$any_failed"
