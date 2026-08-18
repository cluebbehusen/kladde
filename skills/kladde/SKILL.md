---
name: kladde
description:
  "Use the kladde CLI to maintain a local markdown notebook: record decisions
  and discoveries in daily notes, append under headings or bullets, read or
  search notes, and edit frontmatter or configuration. Use when the user or
  project instructions ask for durable notes to be captured, retrieved, or
  organized with kladde."
---

# kladde

Kladde is a CLI over a notebook of markdown files. Writes take a notebook lock
and replace files atomically, so concurrent kladde writers never lose an entry.

This skill does not include the binary. If `kladde` is not on PATH or no
notebook is configured, commands fail with clear messages; do not install the
CLI or create configuration without authorization. When it is missing, you can
offer the user a choice: run it without durably installing it (`uvx kladde`), or
install it (see the
[README's Installation section](https://github.com/cluebbehusen/kladde#installation)).

## Targeting a note

Commands that work on one note take the target three ways:

- a relative path inside the notebook: `kladde read notes/api.md`
- `--name <NAME>`: by name, resolved the way a wikilink is; the note must
  already exist and match uniquely, so `--name` never creates a note
- `--date today|yesterday|tomorrow|YYYY-MM-DD`: a daily note

With no target, the note is today's daily note. `--notebook <DIR>` overrides the
configured default notebook.

## Appending

`kladde append "<text>"` appends the text verbatim as its own line, so a bullet
is whatever you type. Text spelled like an option needs a `--` separator first.

- `--under <HEADING>` places the text at the end of that heading's section. The
  value is a prefix of the heading as written, `#` marks excluded; exactly one
  heading must match. Repeat to descend into subsections.
- `--under-bullet <BULLET>` nests the text under a bullet, matched by a prefix
  of the bullet's first line past its marker; exactly one bullet must match.
  Repeat to descend a thread. With `--under`, the bullet is found inside that
  section. Kladde chooses the nesting indent itself; never indent by hand.

A write that creates a daily note seeds it from the configured template. Placed
appends fail loudly rather than guess: zero or several matches is an error, and
a placed append fails rather than create a note its target cannot match.

Group related entries as a parent bullet with children:

```sh
kladde append "- Chose per-key cache versioning" --under Decisions
kladde append "- Versioned keys permit rollback" --under Decisions --under-bullet "Chose per-key"
```

## Other commands

- `kladde new <TARGET>` creates a note; `kladde read` prints one; `kladde path`
  prints its absolute path; `kladde open` opens one in an editor
- `kladde list` and `kladde search <QUERY>` cover the whole notebook
- `kladde frontmatter` edits a note's properties: `set` writes a text value,
  `add` and `remove` edit list items, `get` prints and `unset` removes either
  kind; writes also stamp `created`/`updated` properties by default
- `kladde config get|set|unset|path|open` manages configuration; add
  `--notebook <DIR>` to target a notebook's own config

`search` and `frontmatter get` exit 1 with no output for an ordinary no-match;
that is a result, not a command failure.

Run `kladde <command> --help` for any command's full contract.
`kladde config set --help` lists every configuration key with a one-line
description; the
[configuration reference](https://github.com/cluebbehusen/kladde/blob/main/docs/config.md)
covers their full semantics.
