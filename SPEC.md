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

[[actions]]
id = "open"
title = "Open goto picker"
description = "Open the configurable goto picker in an overlay pane"
command = ["sh", "-c", "exec \"${HERDR_BIN_PATH:-herdr}\" plugin pane open --plugin yoshiori.herdr-configurable-picker --entrypoint picker"]

[[panes]]
id = "picker"
title = "Goto"
placement = "overlay"
# ./ prefix required: bare relative paths go through PATH lookup, only
# ./-prefixed ones resolve against the plugin root (portable-pty semantics).
command = ["./target/release/herdr-configurable-picker"]
```

**The `open` action** exists because herdr keybindings cannot open plugin panes directly (see "User keybinding" below). Action argv is exec'd raw with no env-var expansion (`plugin_command.rs`), hence the `sh -c` wrapper; the `${HERDR_BIN_PATH:-herdr}` default falls back to a `PATH` lookup if the recorded binary path ever goes stale (e.g. hot-swapped server binary).

**`placement = "overlay"`**: matches the built-in goto's UX (centered popup, restores prior focus/zoom on close). `split` would leave a stray pane behind if the user is not careful.

**Windows in v0.1**: excluded from platforms. `ratatui` supports Windows fine, but we defer testing to v0.2.

## User keybinding (in herdr's config, not this plugin's)

Verified against herdr source: `[[keys.command]]` supports `type = "shell" | "pane" | "plugin_action"`, and `plugin_action` resolves manifest **actions only** — a keybinding cannot open a plugin pane directly. The plugin therefore declares an `open` action (see Manifest above) that runs `plugin pane open`, and the user binds that action:

```toml
[[keys.command]]
key = "prefix+alt+g"
type = "plugin_action"
command = "yoshiori.herdr-configurable-picker.open"
description = "configurable goto picker"
```

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

# Matches the built-in's agent icons (blocked/working/done/idle/unknown):
# "nerd" -> ◉ ⠋(spinner) ● ✓ ○   "ascii" -> ! |(spinner) * v o   "emoji" -> 🔴🟡🔵✅⚪
icon_set = "nerd"

[behavior]
initial_expansion = "all"   # "all" | "current_workspace" | "none"

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
- Config validator (run at plugin startup) prints warnings to stderr for conflicts and unknown modifiers, but does not fail — a broken key is disabled, the rest still work. Warnings are also appended to `$HERDR_PLUGIN_STATE_DIR/picker.log`, because stderr vanishes with the overlay.
- Search mode has its own key table (`search_*`); when search is focused, normal-mode movement/accept/cancel keys are re-routed only through the search table.

### Chord policy

- **No timeout.** The event loop blocks on the next key; a pending chord resolves whenever the next key arrives.
- A key that mismatches the pending chord is **swallowed** (pending cleared, no action) — firing its standalone binding instead would give one keypress two meanings.
- `esc` during a pending chord clears the chord; it does not cancel the picker.
- A chord whose strict prefix is itself a complete binding can never fire (the prefix resolves immediately); it is disabled with a startup warning.

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

Populated on open by talking **directly to herdr's API socket** (newline-delimited JSON over `$HERDR_SOCKET_PATH`) — not by shelling out to the CLI. The CLI's JSON output is just the raw socket response printed, so the parsed shapes are identical, but the socket gives us: no subprocess per call, no dependence on `$HERDR_BIN_PATH` staying valid, and access to socket-only methods (`pane.focus`) the CLI does not expose.

Env contract (set by herdr on plugin pane processes):

| Variable | Use |
| --- | --- |
| `HERDR_SOCKET_PATH` | API socket. Missing ⇒ not inside herdr ⇒ exit 2 with a hint. |
| `HERDR_PLUGIN_CONFIG_DIR` | `config.toml` location (seeded on first run). |
| `HERDR_PLUGIN_STATE_DIR` | `picker.log` for warnings (stderr vanishes with the overlay). |
| `HERDR_PLUGIN_CONTEXT_JSON` | invocation context; `tab_id` seeds the initial cursor. |

Wire protocol: one request line, one response line.

```
-> {"id":"1","method":"tab.list","params":{}}
<- {"id":"1","result":{"type":"tab_list","tabs":[...]}}
<- {"id":"1","error":{"code":"tab_not_found","message":"..."}}
```

`params` must always be present (herdr's Method enum is serde tag/content). **One request per connection**: herdr's server answers a single request and hangs up (only `events.subscribe` / `pane.wait_for_output` stream), so the client dials a fresh connection per call. Methods used: `workspace.list`, `tab.list`, `workspace.focus`, `tab.focus` (M1); `pane.list`, `pane.focus` (M2). IDs are strings: `w1`, `w1:t1`, `w1:p1`. Unknown response fields are ignored (serde default), so herdr adding fields is not a breaking change; the result `type` tag is checked before deserializing so drift fails with a clear error.

Merged in memory into:

```rust
struct Workspace { id, label, number: u32, agent_status, tabs: Vec<Tab> }
struct Tab       { id, workspace_id, label, number: u32, pane_count: u32,
                   agent_status, focused: bool, panes: Vec<Pane> }
struct Pane      { id, tab_id, workspace_id, agent: Option<String>,
                   agent_status, cwd, focused: bool, terminal_id }
```

- **Refresh semantics**: fetched on open and re-fetched about once a second while the picker is open (no event subscription — three cheap list calls per refresh). The built-in recomputes its rows from live state every frame; polling is the snapshot-client equivalent. A refresh preserves the cursor's node, the user's expand/collapse choices, and the active search filter; statuses, labels, and appearing/disappearing panes update in place. A failed refresh keeps the last good snapshot and retries on the next interval.

## Focus / jump behavior

Selecting a node sends the matching socket method:

| Node type | Method | Notes |
| --------- | ------ | ----- |
| Workspace | `workspace.focus {workspace_id}` | Lands on the workspace's focused tab & pane. |
| Tab       | `tab.focus {tab_id}`             | Lands on the tab's focused pane. |
| Pane      | `pane.focus {pane_id}`, falling back to `tab.focus {tab_id}` | The socket-side `pane.focus` (never exposed by the CLI) only exists in herdr builds **after 0.7.1**; older servers reject the method, so the picker retries with the pane's tab, which lands on that tab's focused pane. |

herdr answers a request it cannot parse (e.g. an unknown method) with an `invalid_request` error whose `id` is **empty** — clients must check the error body before comparing ids, or the real message gets masked.

## Search

- Pressing `search_start` (default `/`) enters search mode.
- **Key routing while the prompt is focused** (in priority order):
  1. `search_clear` / `search_exit` (the search-mode table),
  2. `backspace` deletes the last query character,
  3. printable characters (no modifiers, or shift only) type into the query,
  4. everything else falls through to the *normal* table — so non-printable
     bindings like `ctrl+n`/`ctrl+p`, arrows, and `enter` keep moving,
     accepting, and cancelling without leaving the prompt (chords do not
     fire inside search mode).
- **Matching**: case-insensitive substring against each node's label. A node is visible if it or any descendant matches; ancestors of a match stay visible so context is preserved. Children of a matching branch are *not* revealed — jump to the branch itself. Collapse state is ignored while a filter is active; the user's expansion state is untouched and returns when the query clears.
- Cursor auto-moves to the first visible *match* (not a context-only ancestor) after each keystroke.
- `search_exit` returns to normal mode; the filter result stays. `search_clear` empties the query.
- No current-tab marker while a filter is active (the point of filtering is going somewhere else).
- Optional fuzzy mode (`search_mode = "fuzzy"`) deferred.

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
    ├── main.rs               # entry point, env contract, TUI event loop
    ├── config.rs             # plugin config load + first-run seeding
    ├── keymap.rs             # key parsing, chord resolution, conflicts
    ├── herdr_client.rs       # socket client + wire structs (HerdrApi trait)
    ├── app.rs                # pure input state machine (keys -> Outcome)
    ├── tree.rs               # workspace/tab/pane tree, expansion, visible rows
    ├── search.rs             # substring search + descendant-visibility (M3)
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

## Open questions — all resolved (2026-07-03, against herdr source)

1. ~~**`--json` flag on herdr CLI**~~ — Moot: the plugin talks to the socket directly. (The CLI output *is* the raw socket response, so the shapes are the same either way.)
2. ~~**Path A vs B for keybinding to plugin pane**~~ — Path B. `[[keys.command]] type = "plugin_action"` resolves manifest **actions** only (`CommandKeybindType` has no pane variant), so the manifest ships an `open` action wrapping `plugin pane open`. See "User keybinding".
3. ~~**Overlay auto-close on child exit**~~ — Confirmed in source: on child exit (any exit code) herdr removes the overlay pane and restores the previous focus and zoom state. Exiting 0 *is* the close mechanism.
4. ~~**`pane.focus <pane_id>`**~~ — Already exists in the **socket** API (`Method::PaneFocus`); only the CLI lacks a subcommand for it. Direct socket access makes agentless-pane focus work without any upstream change.
