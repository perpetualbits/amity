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
#   Lines whose first non-whitespace character, outside any string literal, opens
#   a // or /// line comment. Block comments (/* ... */) are not counted — they
#   are rare in this codebase and multi-line blocks are error-prone in bash.
#
# What counts as a code line — the token rule:
#   A line counts as code only if it contains at least one non-whitespace
#   character that lies OUTSIDE a string literal (and before any line comment).
#   The gate exists to keep production *logic* explained, and string-literal
#   content is data, not logic. Concretely this means:
#     • The body lines of a multi-line string literal — most importantly embedded
#       SQL (the INSERT/SELECT blocks in the storage layer) — do NOT count. They
#       are the query text, not Rust that needs a comment beside it.
#     • A line that is only a string delimiter (a lone opening or closing quote)
#       does NOT count either.
#     • A line with real Rust tokens still counts even if it also holds a string:
#       `.route("/x", get(h))` and `"native" => Kind::Native,` are code, because
#       the `.route`/`=>` tokens live outside the quotes.
#   The scanner tracks string state across lines and honours backslash escapes
#   (so `"%\"{tag}\"%"` is one string, not two). It assumes production code has no
#   char literal containing a quote such as '"' — the sole occurrence lives in a
#   #[cfg(test)] block, which is stripped before scanning (see below).
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

    # One awk pass counts comment and code lines. It does two things: it skips any
    # `#[cfg(test)]` block so unit tests do not count toward the ratio, and it
    # scans each surviving line character by character to apply the token rule
    # above (string content is not code). String state carries across lines so a
    # multi-line SQL literal is excluded in full.
    read -r comment_lines code_lines < <(awk '
        # ── Test-block skipping ──────────────────────────────────────────────
        # A #[cfg(test)] attribute starts a test block we skip entirely. Brace
        # depth is tracked from the attributes following item (a `mod` or `fn`)
        # until it closes. This relies on the codebase convention that
        # `#[cfg(test)]` introduces a braced item (`#[cfg(test)] mod tests { … }`).
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
        # ── Per-line character scan ──────────────────────────────────────────
        # `in_str` persists across lines: 1 while inside a multi-line string. For
        # each line we compute whether it opens a full-line comment, and whether
        # it has any real code character outside string context.
        {
            n = length($0);
            i = 1;
            code_char = 0;      # saw a non-ws token outside string/comment?
            comment_start = 0;  # line begins (outside string) with // or ///?
            seen_nonws = 0;     # have we met the first non-ws char yet?
            while (i <= n) {
                c = substr($0, i, 1);
                if (in_str) {
                    # Inside a string: a backslash escapes the next char (so \" does
                    # not close), a bare quote closes, everything else is data.
                    if (c == "\\") { i += 2; continue }
                    if (c == "\"") { in_str = 0; i += 1; continue }
                    i += 1; continue
                }
                # Outside a string. Whitespace never classifies a line.
                if (c == " " || c == "\t") { i += 1; continue }
                is_first = (seen_nonws == 0);
                seen_nonws = 1;
                # A // outside a string starts a comment; if it is the lines first
                # token the whole line is a comment, otherwise it is a trailing
                # comment on a code line. Either way, stop scanning here.
                if (c == "/" && substr($0, i + 1, 1) == "/") {
                    if (is_first) comment_start = 1;
                    break
                }
                # An opening quote begins a string literal; the quote itself is not
                # a code token, so scanning it does not set code_char.
                if (c == "\"") { in_str = 1; i += 1; continue }
                # Any other non-ws char outside a string is a real code token.
                code_char = 1;
                i += 1;
            }
            # Classify: a full-line comment counts as comment; a line with a real
            # token counts as code; anything else (blank, or pure string body /
            # delimiter) counts as neither.
            if (comment_start) { comment++; next }
            if (code_char) { code++; next }
            next
        }
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
