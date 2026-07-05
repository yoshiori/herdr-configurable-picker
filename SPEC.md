# herdr-configurable-picker — Specification

> Configurable, tree-based goto picker for [herdr](https://herdr.dev). Ships as a herdr plugin. Emacs-style movement and IME-safe keys out of the box, and every key that moves the cursor, expands a node, accepts a choice, or starts a search is user-configurable — unlike the built-in `prefix+g` goto whose keys are hard-coded.

## Purpose

The built-in herdr goto (`prefix+g`, `Mode::Navigator` internally) has hard-coded navigation keys:
- `j` / `k` / arrows for movement.
- `Ctrl+n` / `Ctrl+p` only when the search field is focused (a workaround for the search-mode catch-all character handler, not a design choice).
- No expansion / collapse of the tree.

This plugin provides a **drop-in alternative** bound to a separate key (recommended: `prefix+ctrl+g`), with:

1. **No external dependencies** at runtime — one static binary, no `fzf`, no `jq`. TUI rendered by the plugin itself.
2. **Tree structure** — workspace / tab / pane hierarchy with collapse/expand, matching the built-in goto's information model.
3. **Fully user-configurable keybindings** — every action (`up` / `down` / `expand` / `collapse` / `accept` / `cancel` / `search` / `clear` / `page_up` / `page_down` / `top` / `bottom` / the state filters) is bindable in the plugin's own config file, with Emacs-style and IME-safe defaults.

## Non-goals

- Replacing the built-in goto entirely. The plugin binds to a separate key so both remain available.
- Duplicating features already covered by herdr core (pane split, workspace rename, agent send, etc.). Only navigation.
- Cross-instance goto (jumping into panes owned by a different herdr session).

## Distribution

- **Type**: herdr plugin (v1 manifest / `herdr-plugin.toml`).
- **Language**: Rust. Rationale: `ratatui` gives us the same TUI stack herdr itself uses; single static binary; no runtime dependencies; matches the herdr project's toolchain so contributors don't context-switch.
- **Install**: `herdr plugin install yoshiori/herdr-configurable-picker`.
- **License**: MIT.

## Manifest (`herdr-plugin.toml`)

```toml
id = "yoshiori.herdr-configurable-picker"
name = "herdr-configurable-picker"
version = "1.0.0"
min_herdr_version = "0.7.0"
description = "Goto picker with Emacs-style movement, IME-safe keys, and fully configurable bindings"
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
title = "herdr-configurable-picker"
placement = "overlay"
# ./ prefix required: bare relative paths go through PATH lookup, only
# ./-prefixed ones resolve against the plugin root (portable-pty semantics).
command = ["./target/release/herdr-configurable-picker"]
```

**The `open` action** exists because herdr keybindings cannot open plugin panes directly (see "User keybinding" below). Action argv is exec'd raw with no env-var expansion (`plugin_command.rs`), hence the `sh -c` wrapper; the `${HERDR_BIN_PATH:-herdr}` default falls back to a `PATH` lookup if the recorded binary path ever goes stale (e.g. hot-swapped server binary).

**`placement = "overlay"`**: matches the built-in goto's UX (centered popup, restores prior focus/zoom on close). `split` would leave a stray pane behind if the user is not careful.

**Windows**: excluded from `platforms`. `ratatui` supports Windows fine, but it is untested here; revisit if someone asks.

## User keybinding (in herdr's config, not this plugin's)

Verified against herdr source: `[[keys.command]]` supports `type = "shell" | "pane" | "plugin_action"`, and `plugin_action` resolves manifest **actions only** — a keybinding cannot open a plugin pane directly. The plugin therefore declares an `open` action (see Manifest above) that runs `plugin pane open`, and the user binds that action:

```toml
[[keys.command]]
key = "prefix+ctrl+g"
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
page_down  = ["pagedown", "ctrl+v"]
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
search_start = ["/", "ctrl+s"]
search_clear = ["ctrl+u"]     # only active while search mode is focused
search_exit  = ["esc"]        # returns to normal mode, keeps current filter result

# State filters (the built-in's b/w/i/d/a): show only nodes whose agents
# are in the given state. Mutually exclusive with text search.
filter_blocked = ["b", "ctrl+b"]
filter_working = ["w", "ctrl+w"]
filter_idle    = ["i", "tab"]
filter_done    = ["d", "ctrl+d"]
filter_clear   = ["a", "backspace", "ctrl+a"]

[display]
show_pane_count   = true
show_agent_status = true
show_agent_icon   = true      # 󰚩 in front of agent meta,  for shells
show_cwd          = false     # off by default; opt in for wide terminals

# Matches the built-in's agent icons (blocked/working/done/idle/unknown):
# "nerd" -> ◉ ⠋(spinner) ● ✓ ○   "ascii" -> ! |(spinner) * v o   "emoji" -> 🔴🟡🔵✅⚪
icon_set = "nerd"

# Accent for the cursor row, current markers, and separators. "auto" reads
# the herdr theme out of the host config.toml ([theme] name + custom.accent
# override, mirroring the host's own resolution — 0.7.1 exposes no theme
# API to plugins) with a cyan fallback; or set a named ANSI color / #rrggbb.
accent = "auto"

[behavior]
initial_expansion = "all"   # "all" | "current_workspace" | "none"

# Enter on a branch node (workspace/tab with children):
# "expand" - toggle the subtree; user then moves to a leaf.
# "jump"   - jump immediately to the branch's active tab/pane.
enter_on_branch = "jump"

# Hover follows, click jumps, wheel scrolls, caret click toggles.
# "auto" follows the host's [ui] mouse_capture; booleans override.
mouse = "auto"
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

The plugin draws no frame of its own — herdr's pane chrome (border + the
manifest pane title) is the frame.

```
┌ herdr-configurable-picker ────────────────────────────────────────┬────────────────────────┐
│  ▼ · picker                                               3 panes │ picker/tab2/pane 2     │
│    ▶ · 1                                                   1 pane │                        │
│    ▼ ● tab2                                               2 panes │ id      w4:p2          │
│      ● pane 2                                            󰚩 claude │ agent   󰚩 claude       │
│      ○ pane 3                                               shell │ status  ⠋ working      │
│  ▶ ○ herdr                                                 1 pane │ cwd     ~/src/picker   │
│→     ○ pane 1                                               shell │ branch  main           │
│                                                                   │                        │
├───────────────────────────────────────────────────────────────────┴────────────────────────┤
│ ↑/↓ move   → expand   ← collapse   / search   b/w/i/d/a states   enter accept   esc cancel │
└────────────────────────────────────────────────────────────────────────────────────────────┘
```

- **Header** (always on, like the built-in's): the `/` search prompt (dim
  placeholder until used) or the active state-filter chip with the state's
  own icon, plus the total pane count right-aligned; a rule separates it
  from the tree.
- **Tree**: workspaces open with `▾`/`▸` and are bold; children hang off
  tree-command guide rails (`├──`, `└──`, `│` continuations). Single-tab
  workspaces list their panes directly (the tab level is skipped, like the
  built-in). `◆` marks where you came from (workspace and pane).
- **Cursor row**: a solid accent bar (fg + bg overridden); `REVERSED`
  under `NO_COLOR`.
- **Right column** (dim): pane counts and activity summaries
  (`2 working · 1 blocked`) on branches; `{agent icon} {agent} · {status}`
  on agent panes, `{icon} shell` otherwise; optionally the `~`-shortened
  cwd (`show_cwd`).
- **Detail panel** (right third, ≥ 60 total columns): the selected row's
  ancestor-path header (`ws/tab/pane`), then id / agent (with icon) /
  status (colored icon + working spinner) / cwd / git branch / title.
  Branches come from reading `.git/HEAD` locally (walking up from
  `foreground_cwd`, then `cwd`; linked worktrees and detached HEADs
  handled) — the API does not carry them.
- **Scrollbar**: a `▕` column overdrawn on the list's right edge when the
  rows overflow, dim track with an accent thumb.
- **Footer**: hint line built from the **currently bound** keys (reads the
  user's keymap so it never lies).
- **Mouse** (when enabled): hover moves the cursor, click jumps, wheel
  scrolls three rows, clicking the prompt focuses search.
- Respects `NO_COLOR`; the accent follows the herdr theme
  (`accent = "auto"`) or an explicit color.

## Data model

Populated on open by talking **directly to herdr's API socket** (newline-delimited JSON over `$HERDR_SOCKET_PATH`) — not by shelling out to the CLI. The CLI's JSON output is just the raw socket response printed, so the parsed shapes are identical, but the socket gives us: no subprocess per call, no dependence on `$HERDR_BIN_PATH` staying valid, and access to socket-only methods (`pane.focus`) the CLI does not expose.

Env contract (set by herdr on plugin pane processes):

| Variable | Use |
| --- | --- |
| `HERDR_SOCKET_PATH` | API socket. Missing ⇒ not inside herdr ⇒ exit 2 with a hint. |
| `HERDR_PLUGIN_CONFIG_DIR` | `config.toml` location (seeded on first run). |
| `HERDR_PLUGIN_STATE_DIR` | `picker.log` for warnings (stderr vanishes with the overlay). |
| `HERDR_PLUGIN_CONTEXT_JSON` | invocation context; `focused_pane_id` (the pane focused *before* the overlay opened) lets the picker drop its own overlay pane from the snapshot and restore the real current pane. |

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
struct Pane      { id, tab_id, workspace_id, agent, display_agent,
                   agent_status, custom_status, title, cwd, foreground_cwd,
                   focused: bool, terminal_id,
                   branch }  // resolved locally from .git/HEAD, not the API
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
- **Matching**: like the built-in's `text_matches_query` — the query is split on whitespace and every word must appear (case-insensitive) in the node's *search text*, which is the label plus the meta column (`claude · idle`, `2 panes · 1 working`, …). So `/blocked` finds stuck agents and `/pick work` intersects. A node is visible if it or any descendant matches; ancestors of a match stay visible so context is preserved. Children of a matching branch are *not* revealed — jump to the branch itself. Collapse state is ignored while a filter is active; the user's expansion state is untouched and returns when the query clears.
- **Match count**: shown at the right edge of the prompt line.
- **State filters**: `filter_blocked`/`_working`/`_idle`/`_done` (default `b`/`w`/`i`/`d`) show only nodes whose (aggregate) agent state matches, with the same ancestor-reveal rules; `filter_clear` (default `a`) drops the filter. Text search and state filters are mutually exclusive — starting one drops the other, and mode/filter keys keep working even when the current filter matches nothing.
- Cursor auto-moves to the first visible *match* (not a context-only ancestor) after each keystroke.
- `search_exit` returns to normal mode; the filter result stays. `search_clear` empties the query.
- No current-tab marker while a filter is active (the point of filtering is going somewhere else).
- Optional fuzzy mode (`search_mode = "fuzzy"`) deferred.

## Behavior details

- **Initial cursor**: on the currently focused pane if visible; otherwise the first leaf of the first expanded workspace.
- **Exit code**: always 0 (both on selection and on cancel). herdr treats non-zero as a toast-worthy error.
- **Overlay close**: confirmed in herdr source — on child exit (any code) herdr removes the overlay pane and restores the previous focus and zoom. Exiting 0 *is* the close mechanism.
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
├── CHANGELOG.md
├── assets/                   # README screenshot
├── tests/
│   └── manifest_sync.rs      # crate version == manifest version
└── src/
    ├── main.rs               # entry point, env contract, TUI event loop
    ├── config.rs             # plugin config load + first-run seeding
    ├── keymap.rs             # key parsing, chord resolution, conflicts
    ├── herdr_client.rs       # socket client + wire structs (HerdrApi trait)
    ├── host_config.rs        # host config.toml readers (theme accent, mouse)
    ├── app.rs                # pure input state machine (keys/mouse -> Outcome)
    ├── tree.rs               # workspace/tab/pane tree, expansion, visible rows
    ├── search.rs             # multi-word AND matching
    ├── git.rs                # .git/HEAD branch resolution (no subprocess)
    ├── icons.rs              # status/agent icon sets (nerd/ascii/emoji)
    └── ui.rs                 # ratatui layout, header, detail panel, footer
```

Dependencies:
- `ratatui` (TUI)
- `crossterm` (input events; herdr also uses it)
- `serde` + `serde_json` (socket wire structs)
- `toml` (config file)
- `anyhow` (error handling)
- `unicode-width` (column math for eliding)
- `tempfile` (dev only, git fixture tests)

## Milestones — all shipped

**M1 — MVP** ✅ `v0.1.0` (2026-07-04)
- Flat tab list (no tree yet), no search.
- All movement / accept / cancel keys config-driven.
- `tab.focus` on select.

**M2 — Tree** ✅ `v0.2.0` (2026-07-04)
- Workspace → tab → pane hierarchy with expand / collapse.
- `initial_expansion` honored.
- Direct pane focus — shipped as socket `pane.focus` with a `tab.focus`
  fallback for herdr ≤ 0.7.1 (not the CLI `agent focus` this originally
  proposed; the socket method works for agentless panes too).

**M3 — Search** ✅ `v0.3.0` (2026-07-04)
- `/` search with descendant-match visibility.
- Substring matching only.

**M4 — Polish** ✅ `v0.4.0` (2026-07-04)
- Icon sets, colors, `NO_COLOR`.
- CI (GitHub Actions: `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test`).
- README with the comparison to the built-in goto.

**M5 — Publish** ✅ `v1.0.0` (2026-07-05)
- Built-in parity work and beyond landed on the way (PR #5–#13: live
  refresh, meta search, state filters, mouse, detail panel with git
  branches, agent icons, IME-safe default keys).
- GitHub topic `herdr-plugin` added; clean `herdr plugin install`
  verified; announced in herdr Discussions
  ([#1047](https://github.com/ogulcancelik/herdr/discussions/1047)).

## Open questions — all resolved (2026-07-03, against herdr source)

1. ~~**`--json` flag on herdr CLI**~~ — Moot: the plugin talks to the socket directly. (The CLI output *is* the raw socket response, so the shapes are the same either way.)
2. ~~**Path A vs B for keybinding to plugin pane**~~ — Path B. `[[keys.command]] type = "plugin_action"` resolves manifest **actions** only (`CommandKeybindType` has no pane variant), so the manifest ships an `open` action wrapping `plugin pane open`. See "User keybinding".
3. ~~**Overlay auto-close on child exit**~~ — Confirmed in source: on child exit (any exit code) herdr removes the overlay pane and restores the previous focus and zoom state. Exiting 0 *is* the close mechanism.
4. ~~**`pane.focus <pane_id>`**~~ — Already exists in the **socket** API (`Method::PaneFocus`); only the CLI lacks a subcommand for it. Direct socket access makes agentless-pane focus work without any upstream change.
