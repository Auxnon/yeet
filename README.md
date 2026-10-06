# yeet

Throw files and text at another machine on your LAN, then pick them up from a TUI.

```sh
# on your laptop
yeet send --to desktop report.pdf photos/
git diff | yeet send --to desktop
yeet send --to desktop -t "the wifi password is hunter2"

# on the desktop, in whatever folder you want things to land
yeet
```

`yeet` opens a picker of everything pending. Files and folders get **moved** into
the current directory, and text goes to the **system clipboard**. Press esc/q to
cancel and nothing leaves the warehouse.

When stdout is not a terminal, `yeet` skips the picker. It writes the most recent
item to stdout and removes it from the warehouse:

```sh
yeet | jq .          # most recent text/file, piped
yeet pop --keep > x  # same, but leave it in the warehouse
```

## How it works

Every machine has a **warehouse** at `~/.yeet/warehouse` (override with
`YEET_WAREHOUSE`). `yeet send` stages the items locally, uploads them over ssh
with **rsync** (or **scp**) into `<warehouse>/.incoming/`, then moves them into
place in one step. The receiver never sees a half-copied item.

There's no daemon and no open port, only ssh. If `ssh desktop` works, `yeet send --to desktop`
works. Set up key auth (`ssh-copy-id desktop`) so there's no password prompt.
The receiving machine needs `yeet` itself to pick items up, and ideally `rsync`.

## Commands

| command | |
|---|---|
| `yeet` | TUI picker (or pop the newest item when piped) |
| `yeet send [PATHS]... [-t TEXT] [--to HOST]` | send files/folders/text; reads stdin if no paths or text |
| `yeet pop [--keep]` | write the newest item to stdout |
| `yeet list` | list pending items |
| `yeet clear` | delete all pending items |

`--to local` drops items into your own warehouse, which is handy for testing.

### Picker keys

`↑/↓` or `j/k` move · `space` mark · `a` mark all · `enter` take marked (or the
highlighted item if nothing is marked) · `esc`/`q` cancel without taking anything.

## Config

`~/.config/yeet/config.toml` (all optional):

```toml
to = "desktop"                  # default --to (or set YEET_TO)
via = "auto"                    # auto | rsync | scp
remote_dir = ".yeet/warehouse"  # relative to the remote home
```

## Clipboard

On Linux yeet uses `wl-copy` (Wayland), `xclip` or `xsel` (X11), or
`termux-clipboard-set`. On macOS and Windows it uses the native clipboard. If
none of these work, yeet prints an error and leaves the text in the warehouse
so you can `yeet pop` it instead.

## Install

```sh
cargo install --git https://github.com/Auxnon/yeet
```

Built with [ratatui](https://ratatui.rs) and
[ratatui-cheese](https://github.com/shashanktomar/ratatui-cheese).
