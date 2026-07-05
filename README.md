# herdr-configurable-picker

Tree-based goto picker for [herdr](https://herdr.dev), with **fully configurable keybindings**.

## Motivation

The built-in herdr goto (`prefix+g`, `Mode::Navigator` internally) has hard-coded navigation keys:

- `j` / `k` / arrows for movement.
- `Ctrl+n` / `Ctrl+p` only when the search field is focused.
- No expand / collapse of the tree.

This plugin binds to a separate key and lets you rebind every action — `up`, `down`, `expand`, `collapse`, `accept`, `cancel`, `search`, and more — from a plugin-local config file.

## Status

**v0.4 (M4): tree + search + polish.** Marketplace publication (M5) is next; see [SPEC.md](./SPEC.md#milestones) for the roadmap and full design.

```
┌ goto ──────────────────────────────────────────────┬────────────────────────┐
│  ▼ · picker                                3 panes │ picker/tab2/pane 2     │
│    ▶ · 1                                    1 pane │                        │
│    ▼ ● tab2                                2 panes │ id      w4:p2          │
│      ● pane 2                               claude │ agent   claude         │
│      ○ pane 3                                shell │ status  ⠋ working      │
│  ▶ ○ herdr                                  1 pane │ cwd     ~/src/picker   │
│→     ○ pane 1                                shell │ branch  main           │
│                                                    │                        │
├────────────────────────────────────────────────────┴────────────────────────┤
│ ↑/↓ move   → expand   ← collapse   / search   enter accept   esc cancel     │
└──────────────────────────────────────────────────────────────────────────────┘
```

## Features

- Tree of `workspace → tab → pane` with expand / collapse per branch (`initial_expansion` configurable) and tree-command style guide rails.
- `/` search over labels *and* meta (agent names, states, pane counts) with multi-word AND — `/moth work` intersects; `ctrl+n`/`ctrl+p`/arrows/`enter` keep working inside the prompt.
- State filters (`b`/`w`/`i`/`d`, rebindable): show only blocked / working / idle / done agents; `a` clears.
- Live view: statuses, labels, and panes refresh about once a second while open, with an animated spinner for working agents and per-branch activity summaries (`2 working · 1 blocked`).
- Jumps to any node: workspaces, tabs, and panes — including agentless panes, via the socket-only `pane.focus` (with a `tab.focus` fallback for herdr ≤ 0.7.1).
- A detail panel shows the selected node's id, agent, status (with its colored icon and the working spinner), cwd, and — inside a git repository — the current branch, read straight from `.git/HEAD` (no `git` subprocess; linked worktrees and detached HEADs included). Worktree workspaces also show their repo and branch.
- Status icons in three sets (`nerd` / `ascii` / `emoji`), status colors, `NO_COLOR` support, and `[display]` toggles for icons, pane counts, and cwd.
- No external runtime dependencies (single Rust binary; TUI via [`ratatui`](https://ratatui.rs/)).
- All keys user-configurable, including chords like `g g`.
- Talks directly to herdr's API socket — no subprocess per call.

## vs the built-in goto (`prefix+g`)

| | built-in goto | this plugin |
| --- | --- | --- |
| Movement keys | hard-coded (`j`/`k`/arrows; `ctrl+n`/`ctrl+p` only while searching) | fully rebindable, multiple keys per action, chords |
| Structure | workspace/tab list | workspace → tab → pane tree with expand/collapse |
| Jump to a pane | via its tab | any pane directly (incl. agentless, via socket `pane.focus`) |
| Search keys | fixed | rebindable (`search_start`/`search_clear`/`search_exit`) |
| Rendering | true floating modal | full-canvas pane with a detail panel |
| Ships with herdr | yes | `herdr plugin install yoshiori/herdr-configurable-picker` |

## Install

```bash
herdr plugin install yoshiori/herdr-configurable-picker
```

Then bind a key in your herdr config. The plugin ships an `open` action (herdr keybindings can trigger plugin actions, not panes) that opens the picker overlay:

```toml
[[keys.command]]
key = "prefix+alt+g"
type = "plugin_action"
command = "yoshiori.herdr-configurable-picker.open"
description = "configurable goto picker"
```

## Plugin config

Written to `$HERDR_PLUGIN_CONFIG_DIR/config.toml` on first run. Every key is bindable (multiple keys per action; chords like `"g g"` work):

```toml
[keys]
down      = ["down", "ctrl+n", "j"]
up        = ["up", "ctrl+p", "k"]
page_down = ["ctrl+d", "pagedown"]
page_up   = ["ctrl+u", "pageup"]
top       = ["home"]
bottom    = ["end", "shift+g"]
expand    = ["right", "l"]
collapse  = ["left", "h"]
toggle    = ["space"]
accept    = ["enter"]
cancel    = ["esc", "ctrl+c", "ctrl+g"]

search_start = ["/"]
search_clear = ["ctrl+u"]
search_exit  = ["esc"]

filter_blocked = ["b"]
filter_working = ["w"]
filter_idle    = ["i"]
filter_done    = ["d"]
filter_clear   = ["a"]
```

Broken or conflicting keys are disabled with a warning (also logged to `$HERDR_PLUGIN_STATE_DIR/picker.log`); the rest keep working. Full config schema in [SPEC.md](./SPEC.md#plugin-config-herdr_plugin_config_dirconfigtoml).

## Development

Requires Rust 1.85+.

```bash
cargo build --release --locked
```

For local iteration on an installed herdr, use `herdr plugin link` on the repo checkout instead of `install`; `link` skips build commands so you control the build cycle.

## License

MIT — see [LICENSE](./LICENSE).
