# Changelog

All notable changes to this project are documented here. The format is based
on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project
adheres to [Semantic Versioning](https://semver.org/).

## [1.0.0] - 2026-07-05

First stable release — at feature parity with the built-in goto, plus the
things it cannot do. Highlights since 0.4.0:

### Added
- Meta search (`/moth work` matches agent names, states, pane counts with
  multi-word AND), state filters (`b`/`w`/`i`/`d`, `a`/backspace clears)
  with the state's icon in the header chip, and per-branch activity
  summaries (`2 working · 1 blocked`).
- Live view: snapshots refresh about once a second while open; animated
  spinner for working agents.
- Mouse support like the built-in's (hover follows, click jumps, wheel
  scrolls, caret toggles), with `mouse = "auto"` following the host's
  `[ui] mouse_capture`.
- Detail panel: git branch for panes inside a repository (read from
  `.git/HEAD`, no subprocess; linked worktrees and detached HEADs
  included), worktree repo/branch on workspace rows, ancestor-path header
  (`ws/tab/pane`), and the status line with its colored icon and spinner.
- Agent icons in the meta column and detail panel (`󰚩 claude · idle`,
  `` for shells; `show_agent_icon` toggle).
- `accent = "auto"` resolves the herdr theme from the host config;
  fish-style cwd eliding; persistent header with the built-in's rule
  lines; proportional scrollbar; half-page scrolling; two-stage esc
  (clears the filter first, closes second).

### Changed
- Default keys grew IME-safe `ctrl+` aliases (with an IME active, bare
  letters get swallowed; ctrl+letter passes through): `ctrl+b`/`ctrl+w`/
  `ctrl+d`/`ctrl+a` on the state filters, `tab` for idle (`ctrl+i` IS tab
  to a terminal), and `ctrl+s` for search. `page_down` is `pagedown`/
  `ctrl+v` now — `ctrl+d` belongs to filter_done.
- No frame of its own — herdr's pane chrome (border + title) is the
  frame; the overlay pane is titled with the plugin name.
- Single-tab workspaces list their panes directly under the workspace,
  like the built-in.
- `pane.focus` falls back to `tab.focus` on herdr ≤ 0.7.1, where the
  socket method does not exist.

## [0.4.0] - 2026-07-04

### Added
- Status icons on every row, with three sets via `[display] icon_set`:
  `nerd` (○ ● ✗ ✓ ·), `ascii` (o + x v -), `emoji` (⚪ 🟢 ❌ ✅ ⚫).
- Colors: status-colored icons, accent on the current-row marker, dimmed
  right columns and detail keys. `NO_COLOR` (non-empty) disables all of it.
- `[display]` toggles honored: `show_agent_status` hides icons,
  `show_pane_count` hides pane counts, `show_cwd` appends the pane's
  `~`-shortened working directory to its row.
- GitHub Actions CI: fmt, clippy (deny warnings), tests, release build on
  Linux and macOS.

## [0.3.0] - 2026-07-04

### Added
- `/` search: case-insensitive substring filter; a node stays visible if it
  or any descendant matches, so ancestors give context. Collapse state is
  ignored while filtering and restored afterwards.
- Inside the prompt, non-printable normal-mode keys (`ctrl+n`/`ctrl+p`,
  arrows, `enter`) keep moving and accepting; `search_clear` (ctrl+u)
  empties the query and `search_exit` (esc) leaves the prompt but keeps the
  filter. Search keys live in their own binding table.
- "No matches." placeholder distinct from an empty session.

## [0.2.0] - 2026-07-04

### Added
- Workspace → tab → pane tree with ▼/▶ glyphs, per-branch expand/collapse
  (`right`/`l`, `left`/`h`, `space`), and `initial_expansion`
  (`all` default / `current_workspace` / `none`).
- Per-node focus dispatch: `workspace.focus`, `tab.focus`, and the
  socket-only `pane.focus`, which makes agentless panes directly focusable.
- `enter_on_branch = "jump" | "expand"`.
- Right-hand detail panel (id, agent, status, cwd, title) on wide terminals.
- Full-canvas layout; column widths measured in terminal cells so CJK
  labels stay aligned.

### Fixed
- The picker's own overlay pane no longer appears in (or steals the current
  marker from) the tree.

## [0.1.0] - 2026-07-04

### Added
- Initial release: flat tab list across all workspaces in a herdr overlay
  pane, opened via the plugin's `open` action.
- Fully configurable keybindings (multiple keys per action, chords like
  `"g g"`, conflict detection that disables broken keys with a warning) in
  `$HERDR_PLUGIN_CONFIG_DIR/config.toml`, seeded on first run.
- Direct newline-delimited-JSON client for herdr's API socket
  (one request per connection), no CLI subprocesses.
