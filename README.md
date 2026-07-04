# herdr-configurable-picker

Tree-based goto picker for [herdr](https://herdr.dev), with **fully configurable keybindings**.

## Motivation

The built-in herdr goto (`prefix+g`, `Mode::Navigator` internally) has hard-coded navigation keys:

- `j` / `k` / arrows for movement.
- `Ctrl+n` / `Ctrl+p` only when the search field is focused.
- No expand / collapse of the tree.

This plugin binds to a separate key and lets you rebind every action — `up`, `down`, `expand`, `collapse`, `accept`, `cancel`, `search`, and more — from a plugin-local config file.

## Status

**v0.2 (M2): workspace → tab → pane tree with expand/collapse.** Search (M3) is next; see [SPEC.md](./SPEC.md#milestones) for the roadmap and full design.

## Features

- Tree of `workspace → tab → pane` with expand / collapse per branch (`initial_expansion` configurable).
- Jumps to any node: workspaces, tabs, and panes — including agentless panes, via the socket-only `pane.focus`.
- No external runtime dependencies (single Rust binary; TUI via [`ratatui`](https://ratatui.rs/)).
- All keys user-configurable, including chords like `g g`.
- Talks directly to herdr's API socket — no subprocess per call.

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
