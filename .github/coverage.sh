#!/usr/bin/env bash
# Run a coverage gate, publish its table to the GitHub job summary, and emit
# workflow-command annotations for uncovered lines so they surface on the pull
# request diff. Runs locally too: the summary goes nowhere and annotations
# print as plain text.
set -uo pipefail

gate="$1"  # cov-unit | cov-int
title="$2" # heading for the job summary

out="$(mktemp)"
cargo "$gate" 2>&1 | tee "$out"
status=$?

{
    echo "### $title (${RUNNER_OS:-local})"
    echo '```'
    sed -n '/^Filename/,$p' "$out"
    echo '```'
} >>"${GITHUB_STEP_SUMMARY:-/dev/null}"

# llvm-cov prints absolute paths; annotations need repo-relative ones. On
# Windows the tool prints native paths while $PWD is the unix-style spelling,
# so try the Windows spelling of the repo root as well.
root="$PWD"
winroot="$(pwd -W 2>/dev/null || true)"
awk -v gate="$gate" -v root="$root/" -v winroot="${winroot:+$winroot/}" '
    /^Uncovered Lines:$/ { active = 1; next }
    active && /: / {
        split($0, parts, ": ")
        file = parts[1]
        gsub(/\\/, "/", file)
        if (index(file, root) == 1) file = substr(file, length(root) + 1)
        else if (winroot != "" && index(file, winroot) == 1) file = substr(file, length(winroot) + 1)
        n = split(parts[2], lines, ", ")
        for (i = 1; i <= n; i++)
            printf "::warning file=%s,line=%s::Not covered by %s\n", file, lines[i], gate
    }
' "$out"

rm -f "$out"
exit "$status"
