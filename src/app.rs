//! Pure input state machine: keys go in, an [`Outcome`] comes out.
//! No terminal, no socket — fully unit-testable.

use crossterm::event::{KeyCode, KeyModifiers};

use crate::herdr_client::AgentStatus;
use crate::keymap::{Action, KeyPress, Keymaps, Resolution};
use crate::tree::{FocusTarget, NodePath, Row, RowKind, Tree};

/// What the event loop should do after a key press.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    Continue,
    Focus(FocusTarget),
    Cancel,
}

/// `[behavior] enter_on_branch` values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnterOnBranch {
    /// Accept on a workspace/tab jumps straight to it.
    Jump,
    /// Accept on a workspace/tab toggles its subtree instead.
    Expand,
}

impl EnterOnBranch {
    pub fn parse(text: &str) -> Option<EnterOnBranch> {
        match text {
            "jump" => Some(EnterOnBranch::Jump),
            "expand" => Some(EnterOnBranch::Expand),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Normal,
    /// The search prompt is focused: printable keys type into the query.
    Search,
}

/// Mouse events, translated from crossterm by the event loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseInput {
    Move { x: u16, y: u16 },
    Click { x: u16, y: u16 },
    ScrollUp,
    ScrollDown,
}

/// Rows the mouse wheel moves per notch, like the built-in.
const WHEEL_STEP: usize = 3;

/// Columns from the list's left edge that count as the expand/collapse
/// caret when clicking a branch row (the built-in's navigator_row_caret_at).
const CARET_ZONE: u16 = 3;

#[derive(Debug)]
pub struct App {
    tree: Tree,
    /// Cache of the visible rows under the current filter, rebuilt after
    /// every mutation.
    rows: Vec<Row>,
    pub cursor: usize,
    /// Keys buffered while a chord is in flight (normal mode only).
    pub pending: Vec<KeyPress>,
    /// Rows the list area can show; set by the UI on each draw so that
    /// page movements track the real terminal size.
    pub viewport_height: u16,
    /// Redraw counter driving the working-status spinner (~8/s).
    pub tick: u32,
    enter_on_branch: EnterOnBranch,
    pub mode: Mode,
    pub query: String,
    /// Active state filter (the built-in's b/w/i/d keys). Mutually
    /// exclusive with the text query: setting one drops the other.
    pub state_filter: Option<AgentStatus>,
    /// True when the snapshot itself had nothing to show (as opposed to a
    /// filter that currently matches nothing).
    tree_is_empty: bool,
    /// Screen y of the search prompt line as of the last draw; a click
    /// there focuses the search, like the built-in.
    pub prompt_row: u16,
    /// The list's screen rectangle `(x, y, w, h)` as of the last draw,
    /// for mouse hit-testing.
    pub list_rect: (u16, u16, u16, u16),
    /// Index of the first visible row as of the last draw.
    pub list_offset: usize,
}

impl App {
    pub fn new(tree: Tree, enter_on_branch: EnterOnBranch) -> App {
        let rows = tree.visible_rows();
        let cursor = tree.initial_cursor();
        let tree_is_empty = rows.is_empty();
        App {
            tree,
            rows,
            cursor,
            pending: Vec::new(),
            viewport_height: 0,
            tick: 0,
            enter_on_branch,
            mode: Mode::Normal,
            query: String::new(),
            state_filter: None,
            tree_is_empty,
            prompt_row: 0,
            list_rect: (0, 0, 0, 0),
            list_offset: 0,
        }
    }

    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    /// Total pane count for the header, unaffected by filters.
    pub fn pane_count(&self) -> usize {
        self.tree.pane_count()
    }

    /// Swaps in a freshly fetched snapshot (the built-in recomputes its
    /// rows from live state every frame; polling is our equivalent).
    /// Preserves the user's expansion choices, the node under the cursor,
    /// and the active search filter.
    pub fn replace_tree(&mut self, mut tree: Tree) {
        tree.adopt_expansion_from(&self.tree);
        let cursor_target = self
            .rows
            .get(self.cursor)
            .map(|row| row.focus_target.clone());
        self.tree = tree;
        self.tree_is_empty = self.tree.visible_rows().is_empty();
        self.refresh_rows();
        self.cursor = cursor_target
            .and_then(|target| self.rows.iter().position(|row| row.focus_target == target))
            .unwrap_or_else(|| self.cursor.min(self.rows.len().saturating_sub(1)));
    }

    pub fn handle_key(&mut self, keymaps: &Keymaps, key: KeyPress) -> Outcome {
        if self.tree_is_empty {
            // SPEC "Empty tree": show the message, close on any key.
            return Outcome::Cancel;
        }
        match self.mode {
            Mode::Normal => self.handle_normal_key(keymaps, key),
            Mode::Search => self.handle_search_key(keymaps, key),
        }
    }

    /// Mouse semantics lifted from the built-in: hover follows, a click
    /// selects and accepts (or toggles, on a branch row's caret), the
    /// prompt line focuses search, and the wheel moves three rows.
    pub fn handle_mouse(&mut self, input: MouseInput) -> Outcome {
        if self.tree_is_empty {
            // Same as any key on an empty tree: close.
            return Outcome::Cancel;
        }
        match input {
            MouseInput::Move { x, y } => {
                if let Some(idx) = self.row_at(x, y) {
                    self.cursor = idx;
                }
                Outcome::Continue
            }
            MouseInput::Click { x, y } => {
                if y == self.prompt_row {
                    return self.apply(Action::SearchStart);
                }
                let Some(idx) = self.row_at(x, y) else {
                    return Outcome::Continue;
                };
                self.cursor = idx;
                let is_branch = self.rows[idx].kind != RowKind::Pane;
                if is_branch && x <= self.list_rect.0.saturating_add(CARET_ZONE) {
                    return self.apply(Action::Toggle);
                }
                self.apply(Action::Accept)
            }
            MouseInput::ScrollUp => {
                self.cursor = self.cursor.saturating_sub(WHEEL_STEP);
                Outcome::Continue
            }
            MouseInput::ScrollDown => {
                if !self.rows.is_empty() {
                    self.cursor = (self.cursor + WHEEL_STEP).min(self.rows.len() - 1);
                }
                Outcome::Continue
            }
        }
    }

    /// The row index under screen position `(x, y)`, if any.
    fn row_at(&self, x: u16, y: u16) -> Option<usize> {
        let (rx, ry, rw, rh) = self.list_rect;
        if x < rx || x >= rx.saturating_add(rw) || y < ry || y >= ry.saturating_add(rh) {
            return None;
        }
        let idx = self.list_offset + (y - ry) as usize;
        (idx < self.rows.len()).then_some(idx)
    }

    fn handle_normal_key(&mut self, keymaps: &Keymaps, key: KeyPress) -> Outcome {
        match keymaps.normal.resolve(&self.pending, key) {
            Resolution::Action(action) => {
                self.pending.clear();
                self.apply(action)
            }
            Resolution::Pending => {
                self.pending.push(key);
                Outcome::Continue
            }
            Resolution::NoMatch => {
                // A failed chord swallows the key: firing its standalone
                // binding instead would be a surprising double meaning.
                self.pending.clear();
                Outcome::Continue
            }
        }
    }

    /// Search prompt input: the search table wins, then editing keys, then
    /// non-printable normal-mode keys (ctrl+n, arrows, enter, ...) keep
    /// their meaning so moving and accepting work without leaving search.
    fn handle_search_key(&mut self, keymaps: &Keymaps, key: KeyPress) -> Outcome {
        if let Resolution::Action(action) = keymaps.search.resolve(&[], key) {
            match action {
                Action::SearchClear => {
                    self.query.clear();
                    self.refresh_filter();
                    return Outcome::Continue;
                }
                Action::SearchExit => {
                    // The filter result deliberately stays (SPEC): exit
                    // detaches the prompt, search_clear empties it.
                    self.mode = Mode::Normal;
                    return Outcome::Continue;
                }
                _ => {}
            }
        }
        match key.code {
            KeyCode::Backspace if key.mods.is_empty() => {
                self.query.pop();
                self.refresh_filter();
                Outcome::Continue
            }
            KeyCode::Char(c) if key.mods.difference(KeyModifiers::SHIFT).is_empty() => {
                if key.mods.contains(KeyModifiers::SHIFT) {
                    // Unicode-aware: canonicalization lowercased the char,
                    // shift restores case for display (matching ignores it).
                    self.query.extend(c.to_uppercase());
                } else {
                    self.query.push(c);
                }
                self.refresh_filter();
                Outcome::Continue
            }
            _ => match keymaps.normal.resolve(&[], key) {
                Resolution::Action(action) => self.apply(action),
                _ => Outcome::Continue,
            },
        }
    }

    fn apply(&mut self, action: Action) -> Outcome {
        // Mode and filter switches must work even when the current filter
        // shows nothing — otherwise an empty result would trap the user
        // with cancel as the only way out.
        match action {
            Action::Cancel => {
                // Two-stage, like the built-in: a leftover query or state
                // filter is cleared first; only a clean esc closes.
                if self.query.is_empty() && self.state_filter.is_none() {
                    return Outcome::Cancel;
                }
                self.clear_filters();
                return Outcome::Continue;
            }
            Action::SearchStart => {
                // Text search and state filter are mutually exclusive.
                self.state_filter = None;
                self.mode = Mode::Search;
                self.refresh_filter();
                return Outcome::Continue;
            }
            Action::FilterBlocked => {
                self.set_state_filter(AgentStatus::Blocked);
                return Outcome::Continue;
            }
            Action::FilterWorking => {
                self.set_state_filter(AgentStatus::Working);
                return Outcome::Continue;
            }
            Action::FilterIdle => {
                self.set_state_filter(AgentStatus::Idle);
                return Outcome::Continue;
            }
            Action::FilterDone => {
                self.set_state_filter(AgentStatus::Done);
                return Outcome::Continue;
            }
            Action::FilterClear => {
                self.clear_filters();
                return Outcome::Continue;
            }
            // Bound only in the search table; nothing to do in normal mode.
            Action::SearchClear | Action::SearchExit => return Outcome::Continue,
            _ => {}
        }
        if self.rows.is_empty() {
            // Movement and accept mean nothing with no rows on screen.
            return Outcome::Continue;
        }
        let last = self.rows.len() - 1;
        // Half a viewport per page, like the built-in's ctrl+d/ctrl+u.
        let page = (self.viewport_height as usize / 2).max(1);
        let row = self.rows[self.cursor].clone();
        match action {
            Action::Down => self.cursor = (self.cursor + 1).min(last),
            Action::Up => self.cursor = self.cursor.saturating_sub(1),
            Action::PageDown => self.cursor = (self.cursor + page).min(last),
            Action::PageUp => self.cursor = self.cursor.saturating_sub(page),
            Action::Top => self.cursor = 0,
            Action::Bottom => self.cursor = last,
            Action::Expand => {
                if self.tree.expand(row.path) {
                    self.refresh_keeping(row.path);
                }
            }
            Action::Collapse => {
                if self.tree.collapse(row.path) {
                    self.refresh_keeping(row.path);
                } else if let Some(parent) = self.tree.parent_path(row.path) {
                    // Collapsing a leaf or an already-collapsed node walks
                    // up instead — the usual file-tree `h` behavior.
                    self.refresh_keeping(parent);
                }
            }
            Action::Toggle => {
                if self.tree.toggle(row.path) {
                    self.refresh_keeping(row.path);
                }
            }
            Action::Accept => {
                let is_branch = row.kind != RowKind::Pane;
                if is_branch && self.enter_on_branch == EnterOnBranch::Expand {
                    self.tree.toggle(row.path);
                    self.refresh_keeping(row.path);
                } else {
                    return Outcome::Focus(row.focus_target);
                }
            }
            // Handled before the empty-rows guard above.
            Action::Cancel
            | Action::SearchStart
            | Action::FilterBlocked
            | Action::FilterWorking
            | Action::FilterIdle
            | Action::FilterDone
            | Action::FilterClear
            | Action::SearchClear
            | Action::SearchExit => unreachable!("handled before the row-dependent actions"),
        }
        Outcome::Continue
    }

    /// Drops both the text query and the state filter (they are mutually
    /// exclusive, but a leftover query survives leaving search mode) and
    /// parks the cursor back on the current node.
    fn clear_filters(&mut self) {
        self.query.clear();
        self.state_filter = None;
        self.refresh_rows();
        self.cursor = self
            .rows
            .iter()
            .rposition(|row| row.is_current)
            .unwrap_or(0);
    }

    fn set_state_filter(&mut self, status: AgentStatus) {
        self.state_filter = Some(status);
        self.query.clear();
        self.refresh_rows();
        // Land on the first node actually in that state, not an ancestor
        // shown for context.
        self.cursor = self
            .rows
            .iter()
            .position(|row| row.agent_status == status)
            .unwrap_or(0);
    }

    /// Rows under the active filter (state filter wins; else text query).
    fn refresh_rows(&mut self) {
        self.rows = match self.state_filter {
            Some(status) => self.tree.visible_rows_state_filtered(status),
            None => self.tree.visible_rows_filtered(&self.query),
        };
    }

    /// Recomputes the rows for the current query and drops the cursor on
    /// the first real match (not an ancestor shown only for context).
    fn refresh_filter(&mut self) {
        self.refresh_rows();
        self.cursor = if self.query.is_empty() {
            self.tree.initial_cursor()
        } else {
            // Lowercase once; search_text is stored lowercased.
            let lowered = self.query.to_lowercase();
            self.rows
                .iter()
                .position(|row| crate::search::lowered_query_matches(&row.search_text, &lowered))
                .unwrap_or(0)
        };
    }

    /// Rebuilds the visible rows and parks the cursor on `path` (which is
    /// always still visible after our mutations).
    fn refresh_keeping(&mut self, path: NodePath) {
        self.refresh_rows();
        self.cursor = self
            .rows
            .iter()
            .position(|row| row.path == path)
            .unwrap_or_else(|| self.cursor.min(self.rows.len().saturating_sub(1)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::KeysConfig;
    use crate::herdr_client::{AgentStatus, PaneInfo, TabInfo, WorkspaceInfo};
    use crate::keymap::{parse_key_spec, Keymap};
    use crate::tree::InitialExpansion;

    fn workspace(id: &str, number: usize, label: &str, focused: bool) -> WorkspaceInfo {
        WorkspaceInfo {
            workspace_id: id.to_string(),
            number,
            label: label.to_string(),
            focused,
            pane_count: 0,
            tab_count: 0,
            active_tab_id: String::new(),
            agent_status: AgentStatus::Unknown,
            worktree: None,
            branch: None,
        }
    }

    fn tab(id: &str, ws_id: &str, number: usize, label: &str, focused: bool) -> TabInfo {
        TabInfo {
            tab_id: id.to_string(),
            workspace_id: ws_id.to_string(),
            number,
            label: label.to_string(),
            focused,
            pane_count: 1,
            agent_status: AgentStatus::Idle,
        }
    }

    fn pane(id: &str, tab_id: &str, ws_id: &str, focused: bool) -> PaneInfo {
        PaneInfo {
            pane_id: id.to_string(),
            tab_id: tab_id.to_string(),
            workspace_id: ws_id.to_string(),
            focused,
            agent: None,
            display_agent: None,
            agent_status: AgentStatus::Idle,
            cwd: None,
            foreground_cwd: None,
            label: None,
            title: None,
            custom_status: None,
            terminal_id: format!("term_{id}"),
            branch: None,
        }
    }

    /// Rows with All expansion (beta is single-tab, so its tab row is
    /// skipped):
    /// 0 alpha / 1 a-one / 2 pane 1(focused) / 3 a-two / 4 pane 2 / 5 beta / 6 pane 1
    fn tree(initial: InitialExpansion) -> Tree {
        Tree::build(
            vec![
                workspace("w1", 1, "alpha", true),
                workspace("w2", 2, "beta", false),
            ],
            vec![
                tab("w1:t1", "w1", 1, "a-one", true),
                tab("w1:t2", "w1", 2, "a-two", false),
                tab("w2:t1", "w2", 1, "b-one", true),
            ],
            vec![
                pane("w1:p1", "w1:t1", "w1", true),
                pane("w1:p2", "w1:t2", "w1", false),
                pane("w2:p1", "w2:t1", "w2", false),
            ],
            initial,
        )
    }

    fn app() -> App {
        let mut app = App::new(tree(InitialExpansion::All), EnterOnBranch::Jump);
        app.viewport_height = 3;
        app
    }

    fn default_keymaps() -> Keymaps {
        let keys = KeysConfig::default();
        let (normal, warnings) = Keymap::from_bindings(&keys.to_bindings());
        assert!(warnings.is_empty(), "default config must be warning-free");
        let (search, warnings) = Keymap::from_bindings(&keys.to_search_bindings());
        assert!(warnings.is_empty(), "default search keys must be clean");
        Keymaps { normal, search }
    }

    fn press(app: &mut App, keymaps: &Keymaps, spec: &str) -> Outcome {
        let keys = parse_key_spec(spec).unwrap();
        let mut outcome = Outcome::Continue;
        for key in keys.0 {
            outcome = app.handle_key(keymaps, key);
        }
        outcome
    }

    fn type_text(app: &mut App, keymaps: &Keymaps, text: &str) {
        for c in text.chars() {
            press(app, keymaps, &c.to_string());
        }
    }

    fn cursor_label(app: &App) -> &str {
        &app.rows()[app.cursor].label
    }

    #[test]
    fn cursor_starts_on_the_current_row() {
        let app = app();
        assert_eq!(cursor_label(&app), "pane 1");
        assert_eq!(app.cursor, 2);
    }

    #[test]
    fn movement_moves_over_visible_rows_and_clamps() {
        let keymaps = default_keymaps();
        let mut app = app();

        press(&mut app, &keymaps, "home");
        assert_eq!(app.cursor, 0);
        press(&mut app, &keymaps, "up");
        assert_eq!(app.cursor, 0, "up clamps at the top");
        press(&mut app, &keymaps, "j");
        press(&mut app, &keymaps, "ctrl+n");
        assert_eq!(app.cursor, 2);
        press(&mut app, &keymaps, "shift+g");
        assert_eq!(app.cursor, 6, "bottom hits the last visible row");
        press(&mut app, &keymaps, "down");
        assert_eq!(app.cursor, 6, "down clamps at the bottom");
        // Half a viewport per page, like the built-in's ctrl+d/ctrl+u.
        // (Paging DOWN defaults to ctrl+v — ctrl+d belongs to filter_done.)
        app.viewport_height = 4;
        press(&mut app, &keymaps, "ctrl+u");
        assert_eq!(app.cursor, 4, "page up moves by half the viewport");
        press(&mut app, &keymaps, "ctrl+v");
        assert_eq!(app.cursor, 6, "page down moves by half the viewport");
    }

    #[test]
    fn collapse_hides_the_subtree_and_keeps_cursor_on_the_branch() {
        let keymaps = default_keymaps();
        let mut app = app();

        press(&mut app, &keymaps, "home");
        assert_eq!(cursor_label(&app), "alpha");
        press(&mut app, &keymaps, "h");
        assert_eq!(cursor_label(&app), "alpha", "cursor stays on the branch");
        // alpha, beta, pane 1 — alpha's own subtree is hidden.
        assert_eq!(app.rows().len(), 3);
    }

    #[test]
    fn collapse_on_a_leaf_walks_up_to_the_parent() {
        let keymaps = default_keymaps();
        let mut app = app(); // cursor on "pane 1"

        press(&mut app, &keymaps, "h");
        assert_eq!(cursor_label(&app), "a-one", "pane -> its tab");
        press(&mut app, &keymaps, "h"); // collapses a-one (it is expanded)
        assert_eq!(cursor_label(&app), "a-one");
        press(&mut app, &keymaps, "h"); // now collapsed -> walks up
        assert_eq!(cursor_label(&app), "alpha");
    }

    #[test]
    fn expand_opens_a_collapsed_branch_in_place() {
        let keymaps = default_keymaps();
        let mut app = App::new(tree(InitialExpansion::None), EnterOnBranch::Jump);

        assert_eq!(app.rows().len(), 2);
        press(&mut app, &keymaps, "l");
        assert_eq!(cursor_label(&app), "alpha", "cursor stays put");
        assert_eq!(app.rows().len(), 4, "alpha's tabs appeared");
        press(&mut app, &keymaps, "l");
        assert_eq!(app.rows().len(), 4, "expanding again is a no-op");
    }

    #[test]
    fn toggle_flips_the_branch() {
        let keymaps = default_keymaps();
        let mut app = app();

        press(&mut app, &keymaps, "home");
        press(&mut app, &keymaps, "space");
        assert_eq!(app.rows().len(), 3);
        press(&mut app, &keymaps, "space");
        assert_eq!(app.rows().len(), 7);
    }

    #[test]
    fn accept_on_a_pane_focuses_the_pane() {
        let keymaps = default_keymaps();
        let mut app = app();

        assert_eq!(
            press(&mut app, &keymaps, "enter"),
            Outcome::Focus(FocusTarget::Pane {
                pane_id: "w1:p1".to_string(),
                tab_id: "w1:t1".to_string()
            })
        );
    }

    #[test]
    fn accept_on_branches_jumps_when_configured_to_jump() {
        let keymaps = default_keymaps();
        let mut app = app();

        press(&mut app, &keymaps, "home");
        assert_eq!(
            press(&mut app, &keymaps, "enter"),
            Outcome::Focus(FocusTarget::Workspace("w1".to_string()))
        );
        press(&mut app, &keymaps, "j");
        assert_eq!(
            press(&mut app, &keymaps, "enter"),
            Outcome::Focus(FocusTarget::Tab("w1:t1".to_string()))
        );
    }

    #[test]
    fn accept_on_branches_toggles_when_configured_to_expand() {
        let keymaps = default_keymaps();
        let mut app = App::new(tree(InitialExpansion::All), EnterOnBranch::Expand);

        press(&mut app, &keymaps, "home");
        assert_eq!(press(&mut app, &keymaps, "enter"), Outcome::Continue);
        assert_eq!(app.rows().len(), 3, "enter collapsed the workspace");

        // Panes still jump.
        let mut app2 = App::new(tree(InitialExpansion::All), EnterOnBranch::Expand);
        assert_eq!(
            press(&mut app2, &keymaps, "enter"),
            Outcome::Focus(FocusTarget::Pane {
                pane_id: "w1:p1".to_string(),
                tab_id: "w1:t1".to_string()
            })
        );
    }

    #[test]
    fn cancel_and_empty_tree_behave_like_m1() {
        let keymaps = default_keymaps();
        let mut app = app();
        assert_eq!(press(&mut app, &keymaps, "esc"), Outcome::Cancel);

        let mut empty = App::new(
            Tree::build(vec![], vec![], vec![], InitialExpansion::All),
            EnterOnBranch::Jump,
        );
        assert_eq!(press(&mut empty, &keymaps, "x"), Outcome::Cancel);
    }

    // --- Search mode ---

    #[test]
    fn slash_enters_search_and_typing_filters() {
        let keymaps = default_keymaps();
        let mut app = app();

        press(&mut app, &keymaps, "/");
        assert_eq!(app.mode, Mode::Search);

        type_text(&mut app, &keymaps, "two");
        assert_eq!(app.query, "two");
        let labels: Vec<&str> = app.rows().iter().map(|r| r.label.as_str()).collect();
        assert_eq!(
            labels,
            vec!["alpha", "a-two", "pane 2"],
            "tab match reveals its panes"
        );
        assert_eq!(cursor_label(&app), "a-two", "cursor lands on the match");
    }

    #[test]
    fn j_types_into_the_query_instead_of_moving() {
        let keymaps = default_keymaps();
        let mut app = app();

        press(&mut app, &keymaps, "/");
        press(&mut app, &keymaps, "j");
        assert_eq!(app.query, "j");
        assert!(app.rows().is_empty(), "nothing is labeled 'j'");
    }

    #[test]
    fn backspace_edits_and_clear_empties_the_query() {
        let keymaps = default_keymaps();
        let mut app = app();

        press(&mut app, &keymaps, "/");
        type_text(&mut app, &keymaps, "twoX");
        assert!(app.rows().is_empty());
        press(&mut app, &keymaps, "backspace");
        assert_eq!(app.query, "two");
        assert_eq!(app.rows().len(), 3, "backspace re-widens the filter");

        press(&mut app, &keymaps, "ctrl+u");
        assert_eq!(app.query, "");
        assert_eq!(app.rows().len(), 7, "clear restores the full tree");
        assert_eq!(app.mode, Mode::Search, "clear keeps the prompt focused");
    }

    #[test]
    fn esc_exits_search_then_clears_the_filter_then_cancels() {
        let keymaps = default_keymaps();
        let mut app = app();

        press(&mut app, &keymaps, "/");
        type_text(&mut app, &keymaps, "two");
        assert_eq!(press(&mut app, &keymaps, "esc"), Outcome::Continue);
        assert_eq!(app.mode, Mode::Normal);
        assert_eq!(app.query, "two", "filter survives exiting the prompt");
        assert_eq!(app.rows().len(), 3);

        // Like the built-in: esc with a leftover filter clears it first...
        assert_eq!(press(&mut app, &keymaps, "esc"), Outcome::Continue);
        assert_eq!(app.query, "");
        assert_eq!(app.rows().len(), 7, "full tree restored");

        // ...and only a clean esc closes the picker.
        assert_eq!(press(&mut app, &keymaps, "esc"), Outcome::Cancel);
    }

    #[test]
    fn filter_clear_also_drops_a_leftover_query() {
        let keymaps = default_keymaps();
        let mut app = app();

        press(&mut app, &keymaps, "/");
        type_text(&mut app, &keymaps, "two");
        press(&mut app, &keymaps, "esc");
        assert_eq!(app.query, "two");

        // The built-in's `a` clears the query as well as the state filter.
        press(&mut app, &keymaps, "a");
        assert_eq!(app.query, "");
        assert_eq!(app.rows().len(), 7);
    }

    #[test]
    fn nonprintable_normal_keys_work_inside_search() {
        let keymaps = default_keymaps();
        let mut app = app();

        press(&mut app, &keymaps, "/");
        type_text(&mut app, &keymaps, "pane");
        // With meta searchable, "pane" also hits the tabs' "N panes" meta;
        // the cursor lands on the first matching row (a-one).
        assert_eq!(cursor_label(&app), "a-one");
        press(&mut app, &keymaps, "ctrl+n");
        assert_eq!(cursor_label(&app), "pane 1", "ctrl+n moved the cursor");

        let outcome = press(&mut app, &keymaps, "enter");
        assert!(
            matches!(outcome, Outcome::Focus(_)),
            "enter accepts from search mode: {outcome:?}"
        );
    }

    #[test]
    fn search_with_no_matches_swallows_accept_and_movement() {
        let keymaps = default_keymaps();
        let mut app = app();

        press(&mut app, &keymaps, "/");
        type_text(&mut app, &keymaps, "zzz");
        assert!(app.rows().is_empty());
        assert_eq!(press(&mut app, &keymaps, "enter"), Outcome::Continue);
        assert_eq!(press(&mut app, &keymaps, "ctrl+n"), Outcome::Continue);
        press(&mut app, &keymaps, "backspace");
        press(&mut app, &keymaps, "backspace");
        press(&mut app, &keymaps, "backspace");
        assert_eq!(app.rows().len(), 7, "recovers once the query shrinks");
    }

    #[test]
    fn expansion_state_is_untouched_by_a_temporary_filter() {
        let keymaps = default_keymaps();
        let mut app = App::new(tree(InitialExpansion::None), EnterOnBranch::Jump);

        press(&mut app, &keymaps, "/");
        type_text(&mut app, &keymaps, "two");
        assert_eq!(
            app.rows().len(),
            3,
            "filter reveals the match and its panes"
        );
        press(&mut app, &keymaps, "ctrl+u");
        assert_eq!(
            app.rows().len(),
            2,
            "clearing goes back to the collapsed view (alpha, beta)"
        );
    }

    #[test]
    fn replace_tree_updates_data_but_keeps_cursor_expansion_and_filter() {
        let keymaps = default_keymaps();
        let mut app = app();

        // Park the cursor on "a-two" and collapse it.
        press(&mut app, &keymaps, "home");
        for _ in 0..3 {
            press(&mut app, &keymaps, "j");
        }
        assert_eq!(cursor_label(&app), "a-two");
        press(&mut app, &keymaps, "h");
        assert_eq!(app.rows().len(), 6, "pane 2 hidden under collapsed a-two");

        // Fresh snapshot: same session, but beta's pane got an agent and
        // started working.
        let mut beta_pane = pane("w2:p1", "w2:t1", "w2", false);
        beta_pane.agent = Some("claude".to_string());
        beta_pane.agent_status = AgentStatus::Working;
        let refreshed = Tree::build(
            vec![
                workspace("w1", 1, "alpha", true),
                workspace("w2", 2, "beta", false),
            ],
            vec![
                tab("w1:t1", "w1", 1, "a-one", true),
                tab("w1:t2", "w1", 2, "a-two", false),
                tab("w2:t1", "w2", 1, "b-one", true),
            ],
            vec![
                pane("w1:p1", "w1:t1", "w1", true),
                pane("w1:p2", "w1:t2", "w1", false),
                beta_pane,
            ],
            InitialExpansion::All,
        );
        app.replace_tree(refreshed);

        assert_eq!(cursor_label(&app), "a-two", "cursor stays on its node");
        assert_eq!(
            app.rows().len(),
            6,
            "user's collapse of a-two survives the refresh"
        );
        let beta_row = app.rows().last().unwrap();
        assert_eq!(beta_row.label, "claude", "refreshed label");
        assert_eq!(
            beta_row.agent_status,
            AgentStatus::Working,
            "refreshed status"
        );
    }

    #[test]
    fn replace_tree_reapplies_the_search_filter() {
        let keymaps = default_keymaps();
        let mut app = app();

        press(&mut app, &keymaps, "/");
        type_text(&mut app, &keymaps, "two");
        assert_eq!(app.rows().len(), 3);

        app.replace_tree(tree(InitialExpansion::All));
        let labels: Vec<&str> = app.rows().iter().map(|r| r.label.as_str()).collect();
        assert_eq!(
            labels,
            vec!["alpha", "a-two", "pane 2"],
            "filter survives refresh"
        );
        assert_eq!(app.query, "two");
    }

    #[test]
    fn state_filter_keys_filter_clear_and_exclude_search() {
        let keymaps = default_keymaps();
        let mut blocked = pane("w2:p1", "w2:t1", "w2", false);
        blocked.agent = Some("claude".to_string());
        blocked.agent_status = AgentStatus::Blocked;
        let tree = Tree::build(
            vec![
                workspace("w1", 1, "alpha", true),
                workspace("w2", 2, "beta", false),
            ],
            vec![
                tab("w1:t1", "w1", 1, "a-one", true),
                tab("w2:t1", "w2", 1, "b-one", true),
            ],
            vec![pane("w1:p1", "w1:t1", "w1", true), blocked],
            InitialExpansion::All,
        );
        let mut app = App::new(tree, EnterOnBranch::Jump);

        press(&mut app, &keymaps, "b");
        assert_eq!(app.state_filter, Some(AgentStatus::Blocked));
        let labels: Vec<&str> = app.rows().iter().map(|r| r.label.as_str()).collect();
        assert_eq!(labels, vec!["beta", "claude"], "blocked chain only");
        assert_eq!(cursor_label(&app), "claude", "cursor on the blocked node");

        press(&mut app, &keymaps, "a");
        assert_eq!(app.state_filter, None);
        assert_eq!(app.rows().len(), 4, "full tree restored");

        // Entering search drops an active state filter.
        press(&mut app, &keymaps, "w");
        assert!(app.rows().is_empty(), "nothing is working");
        press(&mut app, &keymaps, "/");
        assert_eq!(app.state_filter, None);
        assert_eq!(app.mode, Mode::Search);
        assert_eq!(app.rows().len(), 4);
        // ...and typing b/w/i/d in search mode edits the query instead.
        press(&mut app, &keymaps, "b");
        assert_eq!(app.query, "b");
        assert_eq!(app.state_filter, None);
    }

    #[test]
    fn esc_and_backspace_clear_the_state_filter_before_closing() {
        let keymaps = default_keymaps();
        let mut app = app();

        press(&mut app, &keymaps, "b");
        assert_eq!(app.state_filter, Some(AgentStatus::Blocked));

        // Backspace drops the state filter, like the built-in.
        press(&mut app, &keymaps, "backspace");
        assert_eq!(app.state_filter, None);
        assert_eq!(app.rows().len(), 7);

        // Esc with an active filter clears it instead of closing.
        press(&mut app, &keymaps, "w");
        assert_eq!(press(&mut app, &keymaps, "esc"), Outcome::Continue);
        assert_eq!(app.state_filter, None);
        assert_eq!(app.rows().len(), 7);

        assert_eq!(press(&mut app, &keymaps, "esc"), Outcome::Cancel);
    }

    #[test]
    fn enter_on_branch_parses_known_values_only() {
        assert_eq!(EnterOnBranch::parse("jump"), Some(EnterOnBranch::Jump));
        assert_eq!(EnterOnBranch::parse("expand"), Some(EnterOnBranch::Expand));
        assert_eq!(EnterOnBranch::parse("teleport"), None);
    }

    /// The list as drawn: rows at y 2..12, prompt line at y 0.
    fn app_with_layout() -> App {
        let mut app = app();
        app.prompt_row = 0;
        app.list_rect = (0, 2, 40, 10);
        app.list_offset = 0;
        app
    }

    #[test]
    fn hover_moves_the_cursor_over_rows_only() {
        let mut app = app_with_layout();

        assert_eq!(
            app.handle_mouse(MouseInput::Move { x: 5, y: 5 }),
            Outcome::Continue
        );
        assert_eq!(cursor_label(&app), "a-two", "row 3 sits at y 5");

        app.handle_mouse(MouseInput::Move { x: 5, y: 9 });
        assert_eq!(cursor_label(&app), "a-two", "y 9 is past the last row");
        app.handle_mouse(MouseInput::Move { x: 41, y: 5 });
        assert_eq!(cursor_label(&app), "a-two", "outside the list, no move");
    }

    #[test]
    fn click_selects_and_accepts_a_row() {
        let mut app = app_with_layout();

        let outcome = app.handle_mouse(MouseInput::Click { x: 10, y: 4 });
        assert_eq!(cursor_label(&app), "pane 1");
        assert!(
            matches!(outcome, Outcome::Focus(FocusTarget::Pane { .. })),
            "click = select + accept, like the built-in: {outcome:?}"
        );
    }

    #[test]
    fn click_on_a_branch_caret_toggles_instead_of_jumping() {
        let mut app = app_with_layout();

        // The caret zone is the first four columns, like the built-in.
        let outcome = app.handle_mouse(MouseInput::Click { x: 1, y: 2 });
        assert_eq!(outcome, Outcome::Continue);
        assert_eq!(cursor_label(&app), "alpha");
        assert_eq!(app.rows().len(), 3, "alpha collapsed");

        // Clicking the label part of a branch row accepts it.
        let outcome = app.handle_mouse(MouseInput::Click { x: 10, y: 2 });
        assert!(
            matches!(outcome, Outcome::Focus(FocusTarget::Workspace(_))),
            "label click jumps: {outcome:?}"
        );
    }

    #[test]
    fn click_on_the_prompt_row_enters_search() {
        let mut app = app_with_layout();

        assert_eq!(
            app.handle_mouse(MouseInput::Click { x: 3, y: 0 }),
            Outcome::Continue
        );
        assert_eq!(app.mode, Mode::Search);
    }

    #[test]
    fn wheel_scrolls_the_cursor_three_rows_at_a_time() {
        let mut app = app_with_layout();
        app.cursor = 0;

        app.handle_mouse(MouseInput::ScrollDown);
        assert_eq!(app.cursor, 3);
        app.handle_mouse(MouseInput::ScrollDown);
        app.handle_mouse(MouseInput::ScrollDown);
        assert_eq!(app.cursor, 6, "clamped at the last row");
        app.handle_mouse(MouseInput::ScrollUp);
        assert_eq!(app.cursor, 3);
    }
}
