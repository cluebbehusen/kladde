# Configuration

Kladde is configured via a single TOML file. Every key is optional, and so is
the file itself. A missing file is an empty config.

## Location

The config file is `$XDG_CONFIG_HOME/kladde/config.toml`, falling back to
`$HOME/.config/kladde/config.toml` when `XDG_CONFIG_HOME` is unset.

`kladde config path` prints the resolved path.

## Format

```toml
# Notebook used when a command is not given an explicit notebook.
default-notebook = "/home/me/notes"

# Command that opens files.
editor = "code --wait"

# Folder that holds daily notes. Unset means the notebook root.
daily-folder = "Daily Notes"

# strftime format for daily note file names. Unset means "%Y-%m-%d".
daily-date-format = "%Y-%m-%d"

# Note that seeds a daily note kladde creates. Unset means none.
daily-template = "templates/Daily.md"

# Whether writes stamp created/updated properties. Unset means true.
stamp = true

# Property name for the created stamp. Unset means "created".
stamp-created-key = "created"

# Property name for the updated stamp. Unset means "updated".
stamp-updated-key = "updated"

# strftime format for stamp values. Unset means "%Y-%m-%dT%H:%M:%S".
stamp-format = "%Y-%m-%dT%H:%M:%S"

# Notebook-relative paths whose notes are never stamped. Unset means none.
stamp-exclude = ["templates"]

# Indent unit for entries nested under a childless bullet. Unset means "tab".
bullet-indent = "tab"
```

Unknown keys are errors, so a typo fails loudly instead of silently doing
nothing.

## Keys

### `default-notebook`

Absolute path of a notebook: a folder of markdown files. Commands that read or
write notes take an explicit `--notebook` option; `--notebook` wins over
`default-notebook`, and when neither is present those commands fail.

### `editor`

Command that opens a file. The value follows shell quoting rules: unquoted words
split on whitespace, and quotes hold a word together, so a program path
containing spaces is written quoted, like
`editor = "'/Applications/My Editor.app/bin/editor' --wait"`. The first word is
the program, the rest are its arguments, and the file to open is appended. An
unquoted backslash escapes the character after it, so a Windows program path
must be quoted to keep its backslashes.

### `daily-folder`

Folder that holds daily notes, relative to the notebook root. Unset means daily
notes live at the notebook root.

### `daily-date-format`

strftime format for daily note file names; `.md` is appended. Unset means
`%Y-%m-%d`, so a note for August 4th, 2026 is `2026-08-04.md`. The format may
contain `/` to spread daily notes across nested folders, for example `%Y/%m/%d`.

### `daily-template`

Notebook-relative path of the note whose contents seed a daily note. When a
write creates a daily note, the template's contents become the note's starting
text, with template variables rendered: `{{title}}` is the note's file name
without `.md`, `{{date}}` the note's own date as `YYYY-MM-DD`, `{{time}}` the
current time as `HH:mm`, and `{{date:FORMAT}}` or `{{time:FORMAT}}` render
Moment-style format tokens. Anything else in `{{...}}` passes through untouched.
Reading commands never create notes, so the template only applies on a write: an
append or property edit targeting a missing daily note, or `kladde new`. A
template named here but missing fails the write rather than seeding nothing
silently. Unset means a new daily note starts empty. Add the template's folder
to `stamp-exclude` to keep the template note itself unstamped.

### `stamp`

Whether kladde stamps the notes it writes: a created property added when kladde
creates a note's frontmatter block, and an updated property refreshed on every
write that changes the note. Unset means `true`; set it to `false` to turn
stamping off on this machine.

### `stamp-created-key`

Property name for the created stamp. Unset means `created`.

### `stamp-updated-key`

Property name for the updated stamp. Unset means `updated`.

### `stamp-format`

strftime format for stamp values, rendered in local time. Unset means
`%Y-%m-%dT%H:%M:%S`, a shape markdown editors read as a date and time. Any other
format still stamps; an editor may then treat the values as plain text.

### `stamp-exclude`

Notebook-relative paths whose notes are never stamped, meant for template
folders and other notes that must stay bare. A note is excluded when its path
inside the notebook starts with an entry. `config set` takes the entries
comma-separated, so `kladde config set stamp-exclude "templates,archive"`
excludes two folders; an entry containing a comma can only be written by editing
the file directly.

### `bullet-indent`

Indent unit used when `kladde append --under-bullet` nests an entry under a
bullet that has no child bullet yet. Both units indent to the column where the
bullet's own text starts, the indent CommonMark asks of a nested item: `tab`
gets there with tabs, `spaces` with spaces. A bullet that already has a child is
not affected by this key: kladde copies that child's indent, so a note keeps its
own style. Unset means `tab`.

## Commands

`kladde config get <key>` prints the value and exits 0, or prints nothing and
exits 1 when the key is unset.

`kladde config set <key> <value>` writes the value, creating the config file and
its directory if needed. A relative `default-notebook` is made absolute against
the current directory, and the directory must exist. An `editor` value must
contain a command and parse under its shell quoting rules.

`kladde config unset <key>` removes the key. Removing a key that is not set is a
success that never creates the file.

`kladde config open` opens the config file in an editor and waits for it to
exit. The editor is the `editor` config key if set, otherwise the `VISUAL` or
`EDITOR` environment variable; an environment value naming no program, blank or
otherwise, counts as unset. The config directory is created first so the editor
can save on a first run. An invalid config file does not stop `config open`: it
warns, falls back to the environment, and lets you open the file to fix it.

## Editing by hand

The config file is meant to be hand-edited; `set` and `unset` preserve the
layout and comments of everything they do not touch. A comment on the lines
above a key belongs to that key and is removed with it. A comment on the same
line as a value is dropped when `set` replaces that value. `set` and `unset` do
not validate other keys, so they can repair a config that `get` rejects, but the
file must still parse as TOML.
