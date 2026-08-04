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
```

Unknown keys are errors, so a typo fails loudly instead of silently doing
nothing.

## Keys

### `default-notebook`

Absolute path of a notebook: a folder of markdown files. Commands that read or
write notes take an explicit `--notebook` option; `--notebook` wins over
`default-notebook`, and when neither is present those commands fail.

### `editor`

Command that opens a file. The value is split on whitespace: the first token is
the program, the rest are its arguments, and the file to open is appended.
Arguments like `--wait` work. A program path containing spaces does not.

### `daily-folder`

Folder that holds daily notes, relative to the notebook root. Unset means daily
notes live at the notebook root.

### `daily-date-format`

strftime format for daily note file names; `.md` is appended. Unset means
`%Y-%m-%d`, so a note for August 4th, 2026 is `2026-08-04.md`. The format may
contain `/` to spread daily notes across nested folders, for example `%Y/%m/%d`.

## Commands

`kladde config get <key>` prints the value and exits 0, or prints nothing and
exits 1 when the key is unset.

`kladde config set <key> <value>` writes the value, creating the config file and
its directory if needed. A relative `default-notebook` is made absolute against
the current directory, and the directory must exist. An `editor` value must
contain a command.

`kladde config unset <key>` removes the key. Removing a key that is not set is a
success that never creates the file.

`kladde config open` opens the config file in an editor and waits for it to
exit. The editor is the `editor` config key if set, otherwise the `VISUAL` or
`EDITOR` environment variable; blank environment values count as unset. The
config directory is created first so the editor can save on a first run. An
invalid config file does not stop `config open`: it warns, falls back to the
environment, and lets you open the file to fix it.

## Editing by hand

The config file is meant to be hand-edited; `set` and `unset` preserve the
layout and comments of everything they do not touch. A comment on the lines
above a key belongs to that key and is removed with it. A comment on the same
line as a value is dropped when `set` replaces that value. `set` and `unset` do
not validate other keys, so they can repair a config that `get` rejects, but the
file must still parse as TOML.
