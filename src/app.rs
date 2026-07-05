//! Pure input state machine: keys go in, an [`Outcome`] comes out.
//! No terminal, no socket — fully unit-testable.

use crossterm::event::{KeyCode, KeyModifiers};

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
    /// True when the snapshot itself had nothing to show (as opposed to a
    /// filter that currently matches nothing).
    tree_is_empty: bool,
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
            tree_is_empty,
        }
    }

    pub fn rows(&self) -> &[Row] {
        &self.rows
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
        self.rows = self.tree.visible_rows_filtered(&self.query);
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
        if self.rows.is_empty() {
            // The filter matches nothing: only cancel means anything.
            return match action {
                Action::Cancel => Outcome::Cancel,
                _ => Outcome::Continue,
            };
        }
        let last = self.rows.len() - 1;
        let page = (self.viewport_height as usize).max(1);
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
            Action::Cancel => return Outcome::Cancel,
            Action::SearchStart => self.mode = Mode::Search,
            // Bound only in the search table; nothing to do in normal mode.
            Action::SearchClear | Action::SearchExit => {}
        }
        Outcome::Continue
    }

    /// Recomputes the rows for the current query and drops the cursor on
    /// the first real match (not an ancestor shown only for context).
    fn refresh_filter(&mut self) {
        self.rows = self.tree.visible_rows_filtered(&self.query);
        self.cursor = if self.query.is_empty() {
            self.tree.initial_cursor()
        } else {
            self.rows
                .iter()
                .position(|row| crate::search::label_matches(&row.label, &self.query))
                .unwrap_or(0)
        };
    }

    /// Rebuilds the visible rows and parks the cursor on `path` (which is
    /// always still visible after our mutations).
    fn refresh_keeping(&mut self, path: NodePath) {
        self.rows = self.tree.visible_rows_filtered(&self.query);
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
            label: None,
            title: None,
            custom_status: None,
            terminal_id: format!("term_{id}"),
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
        press(&mut app, &keymaps, "ctrl+u");
        assert_eq!(app.cursor, 3, "page up moves by viewport height");
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
        assert_eq!(labels, vec!["alpha", "a-two"]);
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
        assert_eq!(app.rows().len(), 2, "backspace re-widens the filter");

        press(&mut app, &keymaps, "ctrl+u");
        assert_eq!(app.query, "");
        assert_eq!(app.rows().len(), 7, "clear restores the full tree");
        assert_eq!(app.mode, Mode::Search, "clear keeps the prompt focused");
    }

    #[test]
    fn esc_exits_search_keeping_the_filter_then_cancels() {
        let keymaps = default_keymaps();
        let mut app = app();

        press(&mut app, &keymaps, "/");
        type_text(&mut app, &keymaps, "two");
        assert_eq!(press(&mut app, &keymaps, "esc"), Outcome::Continue);
        assert_eq!(app.mode, Mode::Normal);
        assert_eq!(app.query, "two", "filter survives exiting the prompt");
        assert_eq!(app.rows().len(), 2);

        assert_eq!(press(&mut app, &keymaps, "esc"), Outcome::Cancel);
    }

    #[test]
    fn nonprintable_normal_keys_work_inside_search() {
        let keymaps = default_keymaps();
        let mut app = app();

        press(&mut app, &keymaps, "/");
        type_text(&mut app, &keymaps, "pane");
        // Matches: pane 1, pane 2, pane 1 (in beta) plus ancestors.
        assert_eq!(cursor_label(&app), "pane 1");
        press(&mut app, &keymaps, "ctrl+n");
        assert_ne!(cursor_label(&app), "pane 1", "ctrl+n moved the cursor");

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
        assert_eq!(app.rows().len(), 2, "filter reveals the match");
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
        assert_eq!(app.rows().len(), 2);

        app.replace_tree(tree(InitialExpansion::All));
        let labels: Vec<&str> = app.rows().iter().map(|r| r.label.as_str()).collect();
        assert_eq!(labels, vec!["alpha", "a-two"], "filter survives refresh");
        assert_eq!(app.query, "two");
    }

    #[test]
    fn enter_on_branch_parses_known_values_only() {
        assert_eq!(EnterOnBranch::parse("jump"), Some(EnterOnBranch::Jump));
        assert_eq!(EnterOnBranch::parse("expand"), Some(EnterOnBranch::Expand));
        assert_eq!(EnterOnBranch::parse("teleport"), None);
    }
}
