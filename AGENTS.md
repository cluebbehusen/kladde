# kladde

CLI for a local markdown notebook. _Kladde_ (KLAH-duh) is German for a rough
notebook for quick first jotting. In older merchant usage, the Kladde was the
daybook where transactions are recorded as they happen before being posted to
the ledger.

Kladde is intended predominantly for use by AI agents, so that they can record
decisions made during a coding session. Without this tool, important
undocumented knowledge can become lost in a compaction or session cleanup. The
tool manages daily notes created from a template, generic notes in any folder,
and section-aware appends that stay correct when several processes write to the
same note at once.

## Copy within the Repo

The developer of this repo is the final arbiter on all copy. This includes the
README, this file, and any other documentation, including the documentation in
the `kladde` CLI itself. You can and should modify copy related to your work,
but any commit which includes copy changes must be _explicitly_ approved by the
developer. This means the developer must say something like "copy approved" or
"commit with that copy" before any commit with copy changes is made.

Note that this is a directive to agents _writing_ code. An agent only reading or
reviewing uncommitted code need not and should not tell the human "copy needs
approval" or "this code should not be committed until copy is approved". "Copy
has changed" should never be considered a review finding.

## Development

```
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo cov-unit
cargo cov-int
npx prettier@3.9.6 --check "**/*.md"
```

All six must pass before a change is complete. These are the same commands CI
runs, so a clean local run means a green build. `cargo cov-unit` and
`cargo cov-int` are aliases (see `.cargo/config.toml`) for `cargo llvm-cov`,
enforcing 100% line coverage from unit tests alone and from integration tests
alone. Clippy runs with `pedantic` warnings enabled and `-D warnings` so
warnings fail the build; fix the code rather than suppressing the lint, and give
any justified `#[allow]` a short reason on the same line.
