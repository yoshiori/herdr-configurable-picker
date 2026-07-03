# herdr-configurable-picker — Specification

> Configurable, tree-based goto picker for [herdr](https://herdr.dev). Ships as a herdr plugin. Every key that moves the cursor, expands a node, accepts a choice, or starts a search is user-configurable — unlike the built-in `prefix+g` goto whose keys are hard-coded.

## Purpose

The built-in herdr goto (`prefix+g`, `Mode::Navigator` internally) has hard-coded navigation keys:
- `j` / `k` / arrows for movement.
- `Ctrl+n` / `Ctrl+p` only when the search field is focused (a workaround for the search-mode catch-all character handler, not a design choice).
- No expansion / collapse of the tree.

This plugin provides a **drop-in alternative** bound to a separate key (recommended: `prefix+alt+g`), with:

1. **No external dependencies** at runtime — one static binary, no `fzf`, no `jq`. TUI rendered by the plugin itself.
2. **Tree structure** — workspace / tab / pane hierarchy with collapse/expand, matching the built-in goto's information model.
3. **Fully user-configurable keybindings** — every action (`up` / `down` / `expand` / `collapse` / `accept` / `cancel` / `search` / `clear` / `page_up` / `page_down` / `top` / `bottom`) is bindable in the plugin's own config file.

## Non-goals

- Replacing the built-in goto entirely. The plugin binds to a separate key so both remain available.
- Duplicating features already covered by herdr core (pane split, workspace rename, agent send, etc.). Only navigation.
- Cross-instance goto (jumping into panes owned by a different herdr session).
- Mouse support in v0.1 (may come later).

## Distribution

- **Type**: herdr plugin (v1 manifest / `herdr-plugin.toml`).
- **Language**: Rust. Rationale: `ratatui` gives us the same TUI stack herdr itself uses; single static binary; no runtime dependencies; matches the herdr project's toolchain so contributors don't context-switch.
- **Install**: `herdr plugin install yoshiori/herdr-configurable-picker`.
- **License**: MIT.

## Manifest (`herdr-plugin.toml`)

```toml
id = "yoshiori.herdr-configurable-picker"
name = "herdr-configurable-picker"
version = "0.1.0"
min_herdr_version = "0.7.0"
description = "Tree-based goto picker with fully configurable keybindings"
platforms = ["linux", "macos"]

[[build]]
command = ["cargo", "build", "--release", "--locked"]

[[panes]]
id = "picker"
title = "Goto"
placement = "overlay"
command = ["target/release/herdr-configurable-picker"]
```

**`placement = "overlay"`**: matches the built-in goto's UX (centered popup, restores prior focus/zoom on close). `split` would leave a stray pane behind if the user is not careful.

**Windows in v0.1**: excluded from platforms. `ratatui` supports Windows fine, but we defer testing to v0.2.

## User keybinding (in herdr's config, not this plugin's)

Two paths depending on what herdr's `[[keys.command]]` supports for opening plugin panes:

**Path A — if `type = "plugin_action"` can open a plugin pane directly:**
```toml
[[keys.command]]
key = "prefix+alt+g"
type = "plugin_action"
command = "yoshiori.herdr-configurable-picker.picker"
description = "configurable goto picker"
```

**Path B (fallback) — if only actions (not panes) can be bound:**
The plugin declares an action `open` that internally calls `$HERDR_BIN_PATH plugin pane open --plugin yoshiori.herdr-configurable-picker --entrypoint picker`, and the user binds to that action.

The install docs will show whichever path is actually supported. See Open Question #2.

## Plugin config (`$HERDR_PLUGIN_CONFIG_DIR/config.toml`)

Owned by the plugin. Seeded on first run. Not validated by herdr.

```toml
[keys]
# Movement
down       = ["down", "ctrl+n", "j"]
up         = ["up", "ctrl+p", "k"]
page_down  = ["ctrl+d", "pagedown"]
page_up    = ["ctrl+u", "pageup"]
top        = ["home"]
bottom     = ["end", "shift+g"]

# Tree expansion
expand     = ["right", "l"]
collapse   = ["left", "h"]
toggle     = ["space"]        # expand/collapse the current branch node

# Confirm / cancel
accept     = ["enter"]
cancel     = ["esc", "ctrl+c", "ctrl+g"]

# Search
search_start = ["/"]
search_clear = ["ctrl+u"]     # only active while search mode is focused
search_exit  = ["esc"]        # returns to normal mode, keeps current filter result

[display]
show_pane_count   = true
show_agent_status = true
show_cwd          = false     # off by default; opt in for wide terminals

# "nerd" -> ○●✓✗·   "ascii" -> o+xv-   "emoji" -> ⚪🟢✅❌⚫
icon_set = "nerd"

[behavior]
initial_expansion = "current_workspace"   # "all" | "current_workspace" | "none"

# Enter on a branch node (workspace/tab with children):
# "expand" - toggle the subtree; user then moves to a leaf.
# "jump"   - jump immediately to the branch's active tab/pane.
enter_on_branch = "jump"
```

### Key syntax

Mirrors herdr's own binding syntax:

- Modifiers: `ctrl+`, `alt+`, `shift+`, `super+`.
- Named keys: `enter`, `esc`, `tab`, `space`, `backspace`, `delete`, `up`, `down`, `left`, `right`, `home`, `end`, `pageup`, `pagedown`, `f1`..`f12`.
- Character keys: `a`..`z`, `0`..`9`, punctuation.
- Sequences (chords): space-separated, e.g. `"g g"`.
- Case-insensitive; canonicalized internally.

### Key resolution

- Each action can be bound to multiple keys (array form).
- If two actions bind the same key, the earlier entry in the `[keys]` table wins.
- Config validator (run at plugin startup) prints warnings to stderr for conflicts and unknown modifiers, but does not fail — a broken key is disabled, the rest still work.
- Search mode has its own key table (`search_*`); when search is focused, normal-mode movement/accept/cancel keys are re-routed only through the search table.

## UI layout

Rendered inside the overlay pane herdr spawns. We do **not** read the outer terminal size — we query our own pty (`TIOCGWINSZ`) and re-layout on `SIGWINCH`.

```
┌ goto ─────────────────────────────────────────────────────────────────────┐
│                                                                            │
│ ▼ ○ mothership                                       1 pane   unknown     │
│   → ○ pane 1                                                  shell        │
│ ▼ ● mothership › tab2                                2 panes  working     │
│     ○ pane 2                                                  shell        │
│     ○ pane 3                                                  shell        │
│ ▶ ✓ herdr                                            1 pane   idle         │
│                                                                            │
├───────────────────────────────────────────────────────────────────────────┤
│  ↑↓ move   → expand   ← collapse   enter accept   / search   esc cancel   │
└───────────────────────────────────────────────────────────────────────────┘
```

- Header shows `/ search: <query>` only when search is active.
- Cursor row: reverse video.
- Tree glyphs: `▼` expanded / `▶` collapsed / `→` (or `»`) on the currently selected leaf.
- Right column: pane count (branch nodes) / agent info (leaf panes).
- Footer: single-line hint showing **currently bound** movement/accept/cancel keys (reads from user's config so it stays honest).
- Respects `NO_COLOR`. Color palette can be overridden in `[display]` later.

## Data model

Populated on open by calling herdr JSON APIs via `$HERDR_BIN_PATH`:

- `workspace list` → workspace `id`, `label`, `number`, `agent_status`.
- `tab list`       → tab `id`, `workspace_id`, `label`, `number`, `pane_count`, `agent_status`, `focused`.
- `pane list`      → pane `id`, `tab_id`, `workspace_id`, `agent`, `agent_status`, `cwd`, `focused`, `terminal_id`.

Note: the herdr CLI already emits JSON by default for these subcommands (no `--json` flag needed). Verify at implementation time that this remains stable.

Merged in memory into:

```rust
struct Workspace { id, label, number: u32, agent_status, tabs: Vec<Tab> }
struct Tab       { id, workspace_id, label, number: u32, pane_count: u32,
                   agent_status, focused: bool, panes: Vec<Pane> }
struct Pane      { id, tab_id, workspace_id, agent: Option<String>,
                   agent_status, cwd, focused: bool, terminal_id }
```

- **Snapshot semantics**: fetched once when the picker opens. No live subscription. Reopen for a fresh view.
- **HERDR_BIN_PATH deleted-inode fallback**: if `$HERDR_BIN_PATH` is not executable, fall back to `PATH` lookup for `herdr`. (Verified failure mode when the server binary was hot-swapped: env var still holds the pre-swap path with `(deleted)` suffix.)

## Focus / jump behavior

Selecting a node calls the appropriate herdr CLI subcommand:

| Node type              | Command                              | Notes                                                        |
| ---------------------- | ------------------------------------ | ------------------------------------------------------------ |
| Workspace              | `herdr workspace focus <ws_id>`      | Always works. Lands on the workspace's focused tab & pane.   |
| Tab                    | `herdr tab focus <tab_id>`           | Always works. Lands on the tab's focused pane.               |
| Pane with agent        | `herdr agent focus <pane_id>`        | Direct pane focus. Verified working via CLI.                 |
| Pane without agent     | `herdr tab focus <tab_id>` fallback  | herdr's CLI exposes no `pane.focus <pane_id>` for agentless panes. See Open Question #4. |

## Search

- Pressing `search_start` (default `/`) enters search mode.
- Typed characters go into the query buffer, not to key bindings.
- **Matching**: case-insensitive substring against each node's label. A node is visible if it or any descendant matches; ancestors of a match stay visible so context is preserved.
- Cursor auto-moves to the first visible match after each keystroke.
- `search_exit` returns to normal mode; the filter result stays. `search_clear` empties the query.
- Optional fuzzy mode (`search_mode = "fuzzy"`) deferred to v0.2.

## Behavior details

- **Initial cursor**: on the currently focused pane if visible; otherwise the first leaf of the first expanded workspace.
- **Exit code**: always 0 (both on selection and on cancel). herdr treats non-zero as a toast-worthy error.
- **Overlay close**: verify the overlay auto-closes on child exit; if not, call `herdr plugin pane close` before returning. See Open Question #3.
- **Terminal resize**: `SIGWINCH` → re-layout → redraw.
- **Empty tree**: display "No workspaces found." and wait for any key.

## Repository layout

```
herdr-configurable-picker/
├── Cargo.toml
├── Cargo.lock
├── herdr-plugin.toml         # the manifest herdr reads
├── README.md
├── LICENSE (MIT)
├── SPEC.md                   # this document
├── CHANGELOG.md              # to be added
└── src/
    ├── main.rs               # entry point, TUI event loop
    ├── config.rs             # plugin config load + key parsing
    ├── keymap.rs             # KeyMap resolution & conflict detection
    ├── herdr_client.rs       # calls out to $HERDR_BIN_PATH
    ├── model.rs              # Workspace/Tab/Pane structs
    ├── tree.rs               # tree rendering / cursor / expand-collapse
    ├── search.rs             # substring search + descendant-visibility
    └── ui.rs                 # ratatui layout, header, footer, colors
```

Dependencies (proposed):
- `ratatui` (TUI)
- `crossterm` (input events; herdr also uses it)
- `serde` + `serde_json` (parsing herdr CLI JSON output)
- `toml` (config file)
- `anyhow` (error handling)

## Milestones

**M1 — MVP**
- Flat tab list (no tree yet), no search.
- All movement / accept / cancel keys config-driven.
- `tab focus` on select.
- Ship as `v0.1.0`.

**M2 — Tree**
- Workspace → tab → pane hierarchy with expand / collapse.
- `initial_expansion` honored.
- Agent-aware focus (`agent focus <pane_id>` for agent panes).

**M3 — Search**
- `/` search with descendant-match visibility.
- Substring matching only.

**M4 — Polish**
- Icon sets, colors, `NO_COLOR`.
- CI (GitHub Actions: `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test`).
- README with screenshots and comparison to built-in goto.

**M5 — Publish**
- Add GitHub topic `herdr-plugin` so herdr's marketplace picks it up.
- Announce (optionally, via herdr Discussions).

## Open questions to resolve

1. **`--json` flag on herdr CLI**: `workspace list` / `tab list` / `pane list` already emit JSON by default. Confirm this is stable API.
2. **Path A vs B for keybinding to plugin pane**: does `[[keys.command]] type = "plugin_action"` accept a manifest-declared pane id, or is a wrapper action needed? Verify against herdr source before writing install docs.
3. **Overlay auto-close on child exit**: verify by test.
4. **`pane.focus <pane_id>` in CLI/socket API**: currently missing; agentless-pane focus falls back to `tab focus`. Consider raising an upstream Discussion (a small addition that would benefit any plugin doing navigation).
