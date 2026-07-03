# herdr-configurable-picker

Tree-based goto picker for [herdr](https://herdr.dev), with **fully configurable keybindings**.

## Motivation

The built-in herdr goto (`prefix+g`, `Mode::Navigator` internally) has hard-coded navigation keys:

- `j` / `k` / arrows for movement.
- `Ctrl+n` / `Ctrl+p` only when the search field is focused.
- No expand / collapse of the tree.

This plugin binds to a separate key and lets you rebind every action — `up`, `down`, `expand`, `collapse`, `accept`, `cancel`, `search`, and more — from a plugin-local config file.

## Status

**Design phase.** Manifest, spec, and repository scaffolding are in place. Implementation is not started. See [SPEC.md](./SPEC.md) for the full design.

## Planned features (v0.1)

- Tree of `workspace → tab → pane`, matching herdr's built-in goto information model.
- No external runtime dependencies (single Rust binary; TUI via [`ratatui`](https://ratatui.rs/)).
- All keys user-configurable, including chords like `g g`.
- Optional expand / collapse per branch.
- Substring search that keeps ancestors of matches visible.

Milestones and open questions are tracked in [SPEC.md](./SPEC.md#milestones).

## Install (planned)

```bash
herdr plugin install yoshiori/herdr-configurable-picker
```

Then bind a key in your herdr config (the exact `type` will be finalized once M1 is verified against herdr's `keys.command` API — see SPEC Open Question #2):

```toml
[[keys.command]]
key = "prefix+alt+g"
type = "plugin_action"
command = "yoshiori.herdr-configurable-picker.picker"
description = "configurable goto picker"
```

## Plugin config (planned)

Written to `$HERDR_PLUGIN_CONFIG_DIR/config.toml` on first run. Every key is bindable:

```toml
[keys]
down     = ["down", "ctrl+n", "j"]
up       = ["up", "ctrl+p", "k"]
expand   = ["right", "l"]
collapse = ["left", "h"]
accept   = ["enter"]
cancel   = ["esc", "ctrl+c"]
search_start = ["/"]
```

Full config schema in [SPEC.md](./SPEC.md#plugin-config).

## Development

Requires Rust 1.75+.

```bash
cargo build --release --locked
```

For local iteration on an installed herdr, use `herdr plugin link` on the repo checkout instead of `install`; `link` skips build commands so you control the build cycle.

## License

MIT — see [LICENSE](./LICENSE).
