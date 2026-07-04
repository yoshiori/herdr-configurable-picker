# Changelog

All notable changes to this project are documented here. The format is based
on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project
adheres to [Semantic Versioning](https://semver.org/).

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
