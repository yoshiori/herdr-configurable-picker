//! Workspace → tab → pane tree with per-branch expansion state, flattened
//! into visible rows for the cursor and the renderer.

use crate::herdr_client::{AgentStatus, PaneInfo, TabInfo, WorkspaceInfo};

/// `[behavior] initial_expansion` values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InitialExpansion {
    All,
    CurrentWorkspace,
    None,
}

impl InitialExpansion {
    pub fn parse(text: &str) -> Option<InitialExpansion> {
        match text {
            "all" => Some(InitialExpansion::All),
            "current_workspace" => Some(InitialExpansion::CurrentWorkspace),
            "none" => Some(InitialExpansion::None),
            _ => Option::None,
        }
    }
}

/// Stable address of a node, independent of what is currently visible.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NodePath {
    pub ws: usize,
    pub tab: Option<usize>,
    pub pane: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowKind {
    Workspace,
    Tab,
    Pane,
}

/// What accepting a row asks herdr to focus.
#[derive(Debug, Clone, PartialEq)]
pub enum FocusTarget {
    Workspace(String),
    Tab(String),
    Pane(String),
}

/// One visible line of the tree.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub path: NodePath,
    pub kind: RowKind,
    pub depth: u8,
    pub label: String,
    pub expandable: bool,
    pub expanded: bool,
    /// Branch rows: pane count under this node. Pane rows: 0.
    pub pane_count: usize,
    /// Pane rows: agent name to display (`display_agent`/`agent`/"shell").
    pub agent: Option<String>,
    pub agent_status: AgentStatus,
    /// The deepest *visible* node on the focused chain — exactly one row
    /// (or none when the snapshot has no focused workspace).
    pub is_current: bool,
    pub focus_target: FocusTarget,
    /// Key/value pairs for the detail panel, in display order.
    pub detail: Vec<(&'static str, String)>,
    /// Pane rows: working directory, for the optional show_cwd column.
    pub cwd: Option<String>,
}

#[derive(Debug)]
struct PaneNode {
    info: PaneInfo,
}

#[derive(Debug)]
struct TabNode {
    info: TabInfo,
    expanded: bool,
    panes: Vec<PaneNode>,
}

#[derive(Debug)]
struct WsNode {
    /// None for a placeholder synthesized around orphan tabs whose
    /// workspace is missing from the snapshot.
    info: Option<WorkspaceInfo>,
    workspace_id: String,
    label: String,
    expanded: bool,
    tabs: Vec<TabNode>,
}

#[derive(Debug)]
pub struct Tree {
    workspaces: Vec<WsNode>,
}

impl Tree {
    /// Joins the snapshot, sorting workspaces and tabs by their numbers and
    /// keeping panes in API order within each tab.
    pub fn build(
        workspaces: Vec<WorkspaceInfo>,
        mut tabs: Vec<TabInfo>,
        panes: Vec<PaneInfo>,
        initial: InitialExpansion,
    ) -> Tree {
        let mut ws_nodes: Vec<WsNode> = workspaces
            .into_iter()
            .map(|info| WsNode {
                workspace_id: info.workspace_id.clone(),
                label: info.label.clone(),
                expanded: false,
                tabs: Vec::new(),
                info: Some(info),
            })
            .collect();
        ws_nodes.sort_by_key(|ws| ws.info.as_ref().map(|i| i.number).unwrap_or(usize::MAX));

        tabs.sort_by_key(|tab| tab.number);
        for tab in tabs {
            let ws = match ws_nodes
                .iter_mut()
                .find(|ws| ws.workspace_id == tab.workspace_id)
            {
                Some(ws) => ws,
                None => {
                    // Orphan tab: give it a placeholder workspace so the row
                    // is still reachable (better than silently dropping it).
                    ws_nodes.push(WsNode {
                        workspace_id: tab.workspace_id.clone(),
                        label: tab.workspace_id.clone(),
                        expanded: false,
                        tabs: Vec::new(),
                        info: None,
                    });
                    ws_nodes.last_mut().expect("just pushed")
                }
            };
            ws.tabs.push(TabNode {
                info: tab,
                expanded: false,
                panes: Vec::new(),
            });
        }

        for pane in panes {
            let Some(tab) = ws_nodes
                .iter_mut()
                .flat_map(|ws| ws.tabs.iter_mut())
                .find(|tab| tab.info.tab_id == pane.tab_id)
            else {
                continue; // pane without a tab in the snapshot: nothing to hang it on
            };
            tab.panes.push(PaneNode { info: pane });
        }

        let mut tree = Tree {
            workspaces: ws_nodes,
        };
        tree.apply_initial_expansion(initial);
        tree
    }

    fn apply_initial_expansion(&mut self, initial: InitialExpansion) {
        for ws in &mut self.workspaces {
            let ws_focused = ws.info.as_ref().is_some_and(|i| i.focused);
            ws.expanded = match initial {
                InitialExpansion::All => true,
                InitialExpansion::CurrentWorkspace => ws_focused,
                InitialExpansion::None => false,
            };
            for tab in &mut ws.tabs {
                tab.expanded = match initial {
                    InitialExpansion::All => true,
                    InitialExpansion::CurrentWorkspace => ws_focused && tab.info.focused,
                    InitialExpansion::None => false,
                };
            }
        }
    }

    pub fn visible_rows(&self) -> Vec<Row> {
        self.build_rows(None)
    }

    /// Search view: a node is visible iff its own label matches or any
    /// descendant's does (ancestors of matches come along for context;
    /// children of a matching branch do not). Collapse state is ignored
    /// while a filter is active. Empty query = the normal expansion view.
    pub fn visible_rows_filtered(&self, query: &str) -> Vec<Row> {
        if query.is_empty() {
            self.build_rows(None)
        } else {
            self.build_rows(Some(query))
        }
    }

    fn build_rows(&self, filter: Option<&str>) -> Vec<Row> {
        // No current marker while filtering: the whole point of a filter is
        // navigating away, and the current chain may not even be visible.
        let current = if filter.is_none() {
            self.current_visible_path()
        } else {
            None
        };
        let mut rows = Vec::new();
        for (ws_idx, ws) in self.workspaces.iter().enumerate() {
            // (tab shown, per-pane shown) for this workspace under `filter`.
            let tab_states: Vec<(bool, Vec<bool>)> = ws
                .tabs
                .iter()
                .map(|tab| match filter {
                    Option::None => (
                        ws.expanded,
                        tab.panes
                            .iter()
                            .map(|_| ws.expanded && tab.expanded)
                            .collect(),
                    ),
                    Some(query) => {
                        let pane_shown: Vec<bool> = tab
                            .panes
                            .iter()
                            .map(|pane| {
                                crate::search::label_matches(&pane_label(&pane.info), query)
                            })
                            .collect();
                        let tab_shown = crate::search::label_matches(&tab.info.label, query)
                            || pane_shown.iter().any(|&shown| shown);
                        (tab_shown, pane_shown)
                    }
                })
                .collect();
            let ws_shown = match filter {
                Option::None => true,
                Some(query) => {
                    crate::search::label_matches(&ws.label, query)
                        || tab_states.iter().any(|(shown, _)| *shown)
                }
            };
            if !ws_shown {
                continue;
            }
            let any_tab_shown = tab_states.iter().any(|(shown, _)| *shown);

            let ws_path = NodePath {
                ws: ws_idx,
                tab: None,
                pane: None,
            };
            let ws_info = ws.info.as_ref();
            let ws_pane_count = ws_info
                .map(|i| i.pane_count)
                .unwrap_or_else(|| ws.tabs.iter().map(|t| t.info.pane_count).sum());
            let ws_status = ws_info
                .map(|i| i.agent_status)
                .unwrap_or(AgentStatus::Unknown);
            rows.push(Row {
                path: ws_path,
                kind: RowKind::Workspace,
                depth: 0,
                label: ws.label.clone(),
                expandable: !ws.tabs.is_empty(),
                expanded: match filter {
                    Option::None => ws.expanded,
                    Some(_) => any_tab_shown,
                },
                pane_count: ws_pane_count,
                agent: None,
                agent_status: ws_status,
                is_current: current == Some(ws_path),
                focus_target: FocusTarget::Workspace(ws.workspace_id.clone()),
                detail: vec![
                    ("id", ws.workspace_id.clone()),
                    (
                        "tabs",
                        ws_info
                            .map(|i| i.tab_count)
                            .unwrap_or(ws.tabs.len())
                            .to_string(),
                    ),
                    ("panes", ws_pane_count.to_string()),
                    ("status", ws_status.name().to_string()),
                ],
                cwd: None,
            });
            for (tab_idx, (tab, (tab_shown, pane_shown))) in
                ws.tabs.iter().zip(&tab_states).enumerate()
            {
                if !tab_shown {
                    continue;
                }
                let tab_path = NodePath {
                    ws: ws_idx,
                    tab: Some(tab_idx),
                    pane: None,
                };
                let any_pane_shown = pane_shown.iter().any(|&shown| shown);
                rows.push(Row {
                    path: tab_path,
                    kind: RowKind::Tab,
                    depth: 1,
                    label: tab.info.label.clone(),
                    expandable: !tab.panes.is_empty(),
                    expanded: match filter {
                        Option::None => tab.expanded,
                        Some(_) => any_pane_shown,
                    },
                    pane_count: tab.info.pane_count,
                    agent: None,
                    agent_status: tab.info.agent_status,
                    is_current: current == Some(tab_path),
                    focus_target: FocusTarget::Tab(tab.info.tab_id.clone()),
                    detail: vec![
                        ("id", tab.info.tab_id.clone()),
                        ("workspace", ws.label.clone()),
                        ("panes", tab.info.pane_count.to_string()),
                        ("status", tab.info.agent_status.name().to_string()),
                    ],
                    cwd: None,
                });
                for (pane_idx, pane) in tab.panes.iter().enumerate() {
                    if !pane_shown[pane_idx] {
                        continue;
                    }
                    let pane_path = NodePath {
                        ws: ws_idx,
                        tab: Some(tab_idx),
                        pane: Some(pane_idx),
                    };
                    let agent = pane
                        .info
                        .display_agent
                        .clone()
                        .or_else(|| pane.info.agent.clone());
                    let mut detail = vec![
                        ("id", pane.info.pane_id.clone()),
                        (
                            "agent",
                            agent.clone().unwrap_or_else(|| "shell".to_string()),
                        ),
                        ("status", pane.info.agent_status.name().to_string()),
                    ];
                    if let Some(cwd) = &pane.info.cwd {
                        detail.push(("cwd", cwd.clone()));
                    }
                    if let Some(title) = &pane.info.title {
                        if !title.is_empty() {
                            detail.push(("title", title.clone()));
                        }
                    }
                    rows.push(Row {
                        path: pane_path,
                        kind: RowKind::Pane,
                        depth: 2,
                        label: pane_label(&pane.info),
                        expandable: false,
                        expanded: false,
                        pane_count: 0,
                        agent,
                        agent_status: pane.info.agent_status,
                        is_current: current == Some(pane_path),
                        focus_target: FocusTarget::Pane(pane.info.pane_id.clone()),
                        detail,
                        cwd: pane.info.cwd.clone(),
                    });
                }
            }
        }
        rows
    }

    /// The deepest node on the focused chain whose row is visible: the
    /// focused pane if its tab is open, else that tab if its workspace is
    /// open, else the focused workspace itself.
    fn current_visible_path(&self) -> Option<NodePath> {
        let ws_idx = self
            .workspaces
            .iter()
            .position(|ws| ws.info.as_ref().is_some_and(|i| i.focused))?;
        let ws = &self.workspaces[ws_idx];
        if !ws.expanded {
            return Some(NodePath {
                ws: ws_idx,
                tab: None,
                pane: None,
            });
        }
        let Some(tab_idx) = ws.tabs.iter().position(|tab| tab.info.focused) else {
            return Some(NodePath {
                ws: ws_idx,
                tab: None,
                pane: None,
            });
        };
        let tab = &ws.tabs[tab_idx];
        if !tab.expanded {
            return Some(NodePath {
                ws: ws_idx,
                tab: Some(tab_idx),
                pane: None,
            });
        }
        match tab.panes.iter().position(|pane| pane.info.focused) {
            Some(pane_idx) => Some(NodePath {
                ws: ws_idx,
                tab: Some(tab_idx),
                pane: Some(pane_idx),
            }),
            None => Some(NodePath {
                ws: ws_idx,
                tab: Some(tab_idx),
                pane: None,
            }),
        }
    }

    /// Index of the current row (deepest visible focused node), or 0.
    pub fn initial_cursor(&self) -> usize {
        self.visible_rows()
            .iter()
            .position(|row| row.is_current)
            .unwrap_or(0)
    }

    fn expanded_flag(&mut self, path: NodePath) -> Option<&mut bool> {
        let ws = self.workspaces.get_mut(path.ws)?;
        match (path.tab, path.pane) {
            (None, None) => Some(&mut ws.expanded),
            (Some(tab), None) => ws.tabs.get_mut(tab).map(|tab| &mut tab.expanded),
            _ => None, // panes have no expansion state
        }
    }

    /// Expands the branch at `path`. Returns false when nothing changed
    /// (already expanded, or a pane).
    pub fn expand(&mut self, path: NodePath) -> bool {
        match self.expanded_flag(path) {
            Some(expanded) if !*expanded => {
                *expanded = true;
                true
            }
            _ => false,
        }
    }

    /// Collapses the branch at `path`. Returns false when nothing changed.
    pub fn collapse(&mut self, path: NodePath) -> bool {
        match self.expanded_flag(path) {
            Some(expanded) if *expanded => {
                *expanded = false;
                true
            }
            _ => false,
        }
    }

    /// Toggles the branch at `path`. Returns false on panes.
    pub fn toggle(&mut self, path: NodePath) -> bool {
        match self.expanded_flag(path) {
            Some(expanded) => {
                *expanded = !*expanded;
                true
            }
            None => false,
        }
    }

    pub fn parent_path(&self, path: NodePath) -> Option<NodePath> {
        match (path.tab, path.pane) {
            (Some(tab), Some(_)) => Some(NodePath {
                ws: path.ws,
                tab: Some(tab),
                pane: None,
            }),
            (Some(_), None) => Some(NodePath {
                ws: path.ws,
                tab: None,
                pane: None,
            }),
            _ => None,
        }
    }
}

/// The picker's own overlay pane is part of the snapshot it fetches: herdr
/// focuses the overlay on open, so the raw data shows OUR pane as current
/// and lists it as a jump target. Drop it and hand "focused" back to the
/// pane the user came from (`focused_pane_id` in the invocation context),
/// fixing the branch pane counts on the way.
pub fn drop_own_overlay_pane(
    workspaces: &mut [WorkspaceInfo],
    tabs: &mut [TabInfo],
    panes: &mut Vec<PaneInfo>,
    context_pane_id: Option<&str>,
) {
    let Some(context_pane_id) = context_pane_id else {
        return; // no context to tell us apart from the user's pane
    };
    let Some(overlay_idx) = panes
        .iter()
        .position(|pane| pane.focused && pane.pane_id != context_pane_id)
    else {
        return;
    };
    let overlay = panes.remove(overlay_idx);
    for tab in tabs.iter_mut().filter(|t| t.tab_id == overlay.tab_id) {
        tab.pane_count = tab.pane_count.saturating_sub(1);
    }
    for ws in workspaces
        .iter_mut()
        .filter(|ws| ws.workspace_id == overlay.workspace_id)
    {
        ws.pane_count = ws.pane_count.saturating_sub(1);
    }
    for pane in panes.iter_mut() {
        pane.focused = pane.pane_id == context_pane_id;
    }
}

/// Pane display label: the user-set label wins; otherwise "pane N" derived
/// from the public id suffix ("w1:p8" -> "pane 8").
fn pane_label(info: &PaneInfo) -> String {
    if let Some(label) = &info.label {
        return label.clone();
    }
    let suffix = info
        .pane_id
        .rsplit_once(":p")
        .map(|(_, n)| n)
        .unwrap_or(info.pane_id.as_str());
    format!("pane {suffix}")
}

#[cfg(test)]
mod tests {
    use super::*;

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

    fn tab(
        id: &str,
        ws_id: &str,
        number: usize,
        label: &str,
        focused: bool,
        pane_count: usize,
    ) -> TabInfo {
        TabInfo {
            tab_id: id.to_string(),
            workspace_id: ws_id.to_string(),
            number,
            label: label.to_string(),
            focused,
            pane_count,
            agent_status: AgentStatus::Idle,
        }
    }

    fn pane(id: &str, tab_id: &str, ws_id: &str, focused: bool, agent: Option<&str>) -> PaneInfo {
        PaneInfo {
            pane_id: id.to_string(),
            tab_id: tab_id.to_string(),
            workspace_id: ws_id.to_string(),
            focused,
            agent: agent.map(|a| a.to_string()),
            display_agent: None,
            agent_status: AgentStatus::Idle,
            cwd: None,
            label: None,
            title: None,
            terminal_id: format!("term_{id}"),
        }
    }

    /// w1 (focused): t1 (focused; p1 focused agent=claude, p2), t2 (p3)
    /// w2:           t3 (p4)
    fn fixture(initial: InitialExpansion) -> Tree {
        Tree::build(
            vec![
                workspace("w2", 2, "beta", false),
                workspace("w1", 1, "alpha", true),
            ],
            vec![
                tab("w1:t2", "w1", 2, "a-two", false, 1),
                tab("w1:t1", "w1", 1, "a-one", true, 2),
                tab("w2:t1", "w2", 1, "b-one", true, 1),
            ],
            vec![
                pane("w1:p1", "w1:t1", "w1", true, Some("claude")),
                pane("w1:p2", "w1:t1", "w1", false, None),
                pane("w1:p3", "w1:t2", "w1", false, None),
                pane("w2:p1", "w2:t1", "w2", true, None),
            ],
            initial,
        )
    }

    fn labels(rows: &[Row]) -> Vec<String> {
        rows.iter().map(|r| r.label.clone()).collect()
    }

    #[test]
    fn all_expansion_flattens_everything_in_order() {
        let rows = fixture(InitialExpansion::All).visible_rows();
        assert_eq!(
            labels(&rows),
            vec![
                "alpha", "a-one", "pane 1", "pane 2", "a-two", "pane 3", "beta", "b-one", "pane 1"
            ]
        );
        let depths: Vec<u8> = rows.iter().map(|r| r.depth).collect();
        assert_eq!(depths, vec![0, 1, 2, 2, 1, 2, 0, 1, 2]);
        assert_eq!(rows[0].kind, RowKind::Workspace);
        assert_eq!(rows[1].kind, RowKind::Tab);
        assert_eq!(rows[2].kind, RowKind::Pane);
    }

    #[test]
    fn current_workspace_expansion_opens_focused_workspace_and_tab_only() {
        let rows = fixture(InitialExpansion::CurrentWorkspace).visible_rows();
        assert_eq!(
            labels(&rows),
            vec!["alpha", "a-one", "pane 1", "pane 2", "a-two", "beta"]
        );
        assert!(rows[0].expanded, "focused workspace expanded");
        assert!(rows[1].expanded, "focused tab expanded");
        assert!(!rows[4].expanded, "unfocused tab collapsed");
        assert!(!rows[5].expanded, "unfocused workspace collapsed");
    }

    #[test]
    fn none_expansion_shows_only_workspaces() {
        let rows = fixture(InitialExpansion::None).visible_rows();
        assert_eq!(labels(&rows), vec!["alpha", "beta"]);
        assert!(rows.iter().all(|r| r.kind == RowKind::Workspace));
    }

    #[test]
    fn expand_collapse_and_toggle_mutate_visibility() {
        let mut tree = fixture(InitialExpansion::None);
        let ws_path = tree.visible_rows()[0].path;

        assert!(tree.expand(ws_path), "expanding a collapsed branch");
        assert_eq!(
            labels(&tree.visible_rows()),
            vec!["alpha", "a-one", "a-two", "beta"]
        );
        assert!(!tree.expand(ws_path), "expanding again is a no-op");

        assert!(tree.collapse(ws_path));
        assert_eq!(labels(&tree.visible_rows()), vec!["alpha", "beta"]);
        assert!(!tree.collapse(ws_path), "collapsing again is a no-op");

        assert!(tree.toggle(ws_path));
        assert_eq!(tree.visible_rows().len(), 4);

        // Panes are not expandable.
        let mut tree = fixture(InitialExpansion::All);
        let pane_path = tree.visible_rows()[2].path;
        assert!(!tree.expand(pane_path));
        assert!(!tree.collapse(pane_path));
        assert!(!tree.toggle(pane_path));
    }

    #[test]
    fn is_current_marks_the_deepest_visible_focused_node() {
        let tree = fixture(InitialExpansion::All);
        let rows = tree.visible_rows();
        let current: Vec<&str> = rows
            .iter()
            .filter(|r| r.is_current)
            .map(|r| r.label.as_str())
            .collect();
        assert_eq!(current, vec!["pane 1"], "focused pane when visible");

        let mut tree = fixture(InitialExpansion::All);
        let tab_path = tree.visible_rows()[1].path;
        tree.collapse(tab_path);
        let rows = tree.visible_rows();
        let current: Vec<&str> = rows
            .iter()
            .filter(|r| r.is_current)
            .map(|r| r.label.as_str())
            .collect();
        assert_eq!(current, vec!["a-one"], "tab when its panes are hidden");

        let tree = fixture(InitialExpansion::None);
        let rows = tree.visible_rows();
        let current: Vec<&str> = rows
            .iter()
            .filter(|r| r.is_current)
            .map(|r| r.label.as_str())
            .collect();
        assert_eq!(current, vec!["alpha"], "workspace when all collapsed");
    }

    #[test]
    fn initial_cursor_sits_on_the_current_row() {
        assert_eq!(fixture(InitialExpansion::All).initial_cursor(), 2);
        assert_eq!(fixture(InitialExpansion::None).initial_cursor(), 0);
        let empty = Tree::build(vec![], vec![], vec![], InitialExpansion::All);
        assert_eq!(empty.initial_cursor(), 0);
        assert!(empty.visible_rows().is_empty());
    }

    #[test]
    fn pane_rows_carry_agent_and_focus_targets_match_kinds() {
        let rows = fixture(InitialExpansion::All).visible_rows();
        assert_eq!(
            rows[0].focus_target,
            FocusTarget::Workspace("w1".to_string())
        );
        assert_eq!(rows[1].focus_target, FocusTarget::Tab("w1:t1".to_string()));
        assert_eq!(rows[2].focus_target, FocusTarget::Pane("w1:p1".to_string()));
        assert_eq!(rows[2].agent.as_deref(), Some("claude"));
        assert_eq!(rows[3].agent, None, "agentless pane");
        assert_eq!(rows[1].pane_count, 2, "tab pane count");
        assert!(rows[1].expandable);
        assert!(!rows[2].expandable, "panes are leaves");
    }

    #[test]
    fn pane_label_prefers_explicit_label_then_id_suffix() {
        let mut with_label = pane("w1:p7", "w1:t1", "w1", false, None);
        with_label.label = Some("builder".to_string());
        let tree = Tree::build(
            vec![workspace("w1", 1, "alpha", true)],
            vec![tab("w1:t1", "w1", 1, "a-one", true, 2)],
            vec![with_label, pane("w1:p8", "w1:t1", "w1", false, None)],
            InitialExpansion::All,
        );
        let rows = tree.visible_rows();
        assert_eq!(rows[2].label, "builder");
        assert_eq!(rows[3].label, "pane 8");
    }

    #[test]
    fn orphan_tab_gets_a_placeholder_workspace() {
        let tree = Tree::build(
            vec![],
            vec![tab("w9:t1", "w9", 1, "orphan", false, 1)],
            vec![],
            InitialExpansion::All,
        );
        let rows = tree.visible_rows();
        assert_eq!(labels(&rows), vec!["w9", "orphan"]);
        assert_eq!(rows[0].kind, RowKind::Workspace);
    }

    #[test]
    fn parent_path_walks_up_the_tree() {
        let tree = fixture(InitialExpansion::All);
        let rows = tree.visible_rows();
        let pane_path = rows[2].path;
        let tab_path = rows[1].path;
        let ws_path = rows[0].path;
        assert_eq!(tree.parent_path(pane_path), Some(tab_path));
        assert_eq!(tree.parent_path(tab_path), Some(ws_path));
        assert_eq!(tree.parent_path(ws_path), None);
    }

    #[test]
    fn drop_own_overlay_pane_removes_the_picker_and_restores_focus() {
        // Snapshot as the picker sees it: the overlay pane (w1:p9) stole
        // focus from the pane the user was in (w1:p1).
        let mut workspaces = vec![{
            let mut ws = workspace("w1", 1, "alpha", true);
            ws.pane_count = 3;
            ws
        }];
        let mut tabs = vec![tab("w1:t1", "w1", 1, "a-one", true, 3)];
        let mut panes = vec![
            pane("w1:p1", "w1:t1", "w1", false, None),
            pane("w1:p2", "w1:t1", "w1", false, None),
            {
                let mut overlay = pane("w1:p9", "w1:t1", "w1", true, None);
                overlay.label = Some("Goto".to_string());
                overlay
            },
        ];

        drop_own_overlay_pane(&mut workspaces, &mut tabs, &mut panes, Some("w1:p1"));

        assert_eq!(panes.len(), 2, "overlay pane removed");
        assert!(panes.iter().all(|p| p.pane_id != "w1:p9"));
        assert!(panes[0].focused, "focus handed back to the context pane");
        assert_eq!(tabs[0].pane_count, 2);
        assert_eq!(workspaces[0].pane_count, 2);

        let tree = Tree::build(workspaces, tabs, panes, InitialExpansion::All);
        let rows = tree.visible_rows();
        let current: Vec<&str> = rows
            .iter()
            .filter(|r| r.is_current)
            .map(|r| r.label.as_str())
            .collect();
        assert_eq!(current, vec!["pane 1"]);
    }

    #[test]
    fn drop_own_overlay_pane_without_context_changes_nothing() {
        let mut workspaces = vec![workspace("w1", 1, "alpha", true)];
        let mut tabs = vec![tab("w1:t1", "w1", 1, "a-one", true, 2)];
        let mut panes = vec![
            pane("w1:p1", "w1:t1", "w1", false, None),
            pane("w1:p9", "w1:t1", "w1", true, None),
        ];

        drop_own_overlay_pane(&mut workspaces, &mut tabs, &mut panes, None);

        assert_eq!(panes.len(), 2);
        assert_eq!(tabs[0].pane_count, 2);
    }

    #[test]
    fn drop_own_overlay_pane_leaves_a_genuinely_focused_context_pane_alone() {
        // Hypothetical --no-focus open: the context pane is still focused.
        let mut workspaces = vec![workspace("w1", 1, "alpha", true)];
        let mut tabs = vec![tab("w1:t1", "w1", 1, "a-one", true, 2)];
        let mut panes = vec![
            pane("w1:p1", "w1:t1", "w1", true, None),
            pane("w1:p9", "w1:t1", "w1", false, None),
        ];

        drop_own_overlay_pane(&mut workspaces, &mut tabs, &mut panes, Some("w1:p1"));

        assert_eq!(panes.len(), 2, "nothing looks like a focused overlay");
        assert!(panes[0].focused);
    }

    #[test]
    fn filter_shows_matches_and_their_ancestors_ignoring_collapse() {
        // Collapsed everywhere: the filter must reveal matches regardless.
        let tree = fixture(InitialExpansion::None);

        let rows = tree.visible_rows_filtered("two");
        assert_eq!(labels(&rows), vec!["alpha", "a-two"]);
        assert_eq!(rows[0].kind, RowKind::Workspace);
        assert!(rows[0].expanded, "ancestor renders as expanded");

        // A matching pane pulls in its whole ancestor chain.
        let rows = tree.visible_rows_filtered("pane 3");
        assert_eq!(labels(&rows), vec!["alpha", "a-two", "pane 3"]);
    }

    #[test]
    fn filter_is_case_insensitive_and_does_not_reveal_children_of_matches() {
        let tree = fixture(InitialExpansion::All);

        // Workspace matches: children stay hidden (they do not match).
        let rows = tree.visible_rows_filtered("ALPHA");
        assert_eq!(labels(&rows), vec!["alpha"]);
        assert!(!rows[0].expanded, "no visible children");

        let rows = tree.visible_rows_filtered("zzz");
        assert!(rows.is_empty());
    }

    #[test]
    fn empty_filter_falls_back_to_expansion_visibility() {
        let tree = fixture(InitialExpansion::None);
        assert_eq!(
            labels(&tree.visible_rows_filtered("")),
            labels(&tree.visible_rows())
        );
    }

    #[test]
    fn rows_carry_detail_pairs_for_the_detail_panel() {
        let mut with_meta = pane("w1:p1", "w1:t1", "w1", true, Some("claude"));
        with_meta.cwd = Some("/home/u/repo".to_string());
        with_meta.title = Some("make -j8".to_string());
        let tree = Tree::build(
            vec![workspace("w1", 1, "alpha", true)],
            vec![tab("w1:t1", "w1", 1, "a-one", true, 2)],
            vec![with_meta, pane("w1:p2", "w1:t1", "w1", false, None)],
            InitialExpansion::All,
        );
        let rows = tree.visible_rows();

        assert_eq!(rows[0].detail[0], ("id", "w1".to_string()));
        assert!(rows[0].detail.contains(&("status", "unknown".to_string())));

        assert_eq!(rows[1].detail[0], ("id", "w1:t1".to_string()));
        assert!(rows[1].detail.contains(&("workspace", "alpha".to_string())));
        assert!(rows[1].detail.contains(&("panes", "2".to_string())));

        assert_eq!(
            rows[2].detail,
            vec![
                ("id", "w1:p1".to_string()),
                ("agent", "claude".to_string()),
                ("status", "idle".to_string()),
                ("cwd", "/home/u/repo".to_string()),
                ("title", "make -j8".to_string()),
            ]
        );

        let agentless = &rows[3].detail;
        assert!(agentless.contains(&("agent", "shell".to_string())));
        assert!(
            agentless.iter().all(|(k, _)| *k != "cwd" && *k != "title"),
            "absent metadata stays out of the panel: {agentless:?}"
        );
    }

    #[test]
    fn initial_expansion_parses_known_values_only() {
        assert_eq!(InitialExpansion::parse("all"), Some(InitialExpansion::All));
        assert_eq!(
            InitialExpansion::parse("current_workspace"),
            Some(InitialExpansion::CurrentWorkspace)
        );
        assert_eq!(
            InitialExpansion::parse("none"),
            Some(InitialExpansion::None)
        );
        assert_eq!(InitialExpansion::parse("everything"), Option::None);
    }
}
