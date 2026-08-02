# kladde

_Kladde (KLAH-duh) - German: a rough notebook for quick first jottings, where a
merchant recorded transactions before posting them to the ledger._

A concurrency-safe markdown notebook CLI, intended predominantly for use by AI
agents. When agents generate code, it's easy to lose important tacit knowledge
in a compaction or a lost session. Some coding agents use memory, but this
generally isn't portable between Claude, Codex, Cursor, and the myriad other
coding agents on the market. Kladde is a simple CLI over a notebook made of
markdown files. Via skills or other directives, a coding agent can be instructed
to record decisions made during a coding session.

Kladde manages daily notes created from a template, generic notes in any folder,
and section-aware appends that stay correct when several processes write to the
same note at once.

Because Kladde writes pure markdown files, it is compatible with any markdown
editor. For example, Kladde can be used to write to your Obsidian vault simply
by pointing Kladde at the vault folder.
