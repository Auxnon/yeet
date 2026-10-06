# yeet

Throw files and text at another machine on your LAN, then pick them up from a TUI.

```sh
# on your laptop: pick a destination, send
yeet report.pdf photos/
yeet -t "the wifi password is hunter2"
git diff | yeet -t                  # -t with no value reads stdin

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
yeet -p -k > x       # same, but leave it in the warehouse
```

## Destinations

Sending opens a picker of your saved destinations. The last row is
**+ new destination**. The first time, when nothing is saved yet, the editor
opens straight away with three fields:

- **Host:** already filled in with your LAN prefix. On a `192.168.1.0/24`
  network you'll see `192.168.1.` and only need to type the last number.
  yeet finds the prefix from the interface on your default route and its
  netmask, and ignores Docker/VM bridges and VPN tunnels. An IP, a hostname,
  or a `~/.ssh/config` alias all work.
- **User:** optional. Leave it blank and ssh uses its default (your current
  user, or `User` from `~/.ssh/config`). Typing `me@host` into Host fills
  this in for you.
- **Nickname:** optional. Shown in the picker instead of the address.

`tab`/`↑↓` move between fields, `enter` goes to the next field and sends from
the last one, and `ctrl+u` clears a field. A new destination is saved only
after a send to it succeeds, so typos don't pile up. The most recently used
destination comes first.

In the picker, `e` edits a destination in place, `x` forgets it and `n` adds a
new one. Use `--to [user@]host` (or `YEET_TO`) to skip the picker, e.g. in
scripts.

## How it works

Every machine has a **warehouse** at `~/.yeet/warehouse` (override with
`YEET_WAREHOUSE`). `yeet FILE...` stages the items locally, uploads them over ssh
with **rsync** (or **scp**) into `<warehouse>/.incoming/`, then moves them into
place in one step. The receiver never sees a half-copied item.

There's no daemon and no open port, only ssh. If `ssh desktop` works, `yeet --to desktop`
works. Set up key auth (`ssh-copy-id desktop`) so there's no password prompt.
The receiving machine needs `yeet` itself to pick items up, and ideally `rsync`.

## Usage

| | |
|---|---|
| `yeet FILE...` | send files/folders |
| `yeet -t "text"` | send raw text (`-t` alone reads stdin) |
| `yeet` | TUI picker (or pop the newest item when piped) |
| `yeet -p` / `--pop` | write the newest item to stdout (`-k` to keep it) |
| `yeet -l` / `--list` | list pending items |
| `yeet --clear` | delete all pending items |

Files and `-t` can be combined in one send.

`--to local` drops items into your own warehouse, which is handy for testing.

### Picker keys

`↑/↓` or `j/k` move · `space` mark · `a` mark all · `enter` take marked (or the
highlighted item if nothing is marked) · `esc`/`q` cancel without taking anything.

## Config

`~/.config/yeet/config.toml` (all optional; yeet rewrites this file when it
saves destinations, so comments in it are not kept):

```toml
via = "auto"                    # auto | rsync | scp
remote_dir = ".yeet/warehouse"  # relative to the remote home

# destinations are managed by the picker
[[destinations]]
host = "192.168.1.20"
user = "me"        # optional
name = "desktop"   # optional
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
