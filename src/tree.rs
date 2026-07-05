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
    /// `tab_id` is the fallback target: herdr only grew the socket-side
    /// `pane.focus` after 0.7.1, so older servers focus the pane's tab.
    Pane {
        pane_id: String,
        tab_id: String,
    },
}

/// One visible line of the tree.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub path: NodePath,
    pub kind: RowKind,
    pub depth: u8,
    pub label: String,
    /// The label with its ancestors ("ws/tab/pane"), for the detail panel
    /// header — a bare tab label like "1" identifies nothing on its own.
    /// Follows the *displayed* hierarchy: single-tab workspaces skip the
    /// tab segment, like their rows skip the tab level.
    pub title: String,
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
    /// Pane rows: user/plugin-set status text; wins over the state name.
    pub custom_status: Option<String>,
    /// True when this row is the last *visible* child of its parent —
    /// tree-command style guides close it with `└──` instead of `├──`.
    pub last_child: bool,
    /// One entry per ancestor level between the workspace and this row
    /// (deepest rows only): true when that ancestor still has visible
    /// siblings below, i.e. the guide column needs a `│` continuation.
    pub ancestor_continues: Vec<bool>,
    /// Branch rows: "N blocked · M working · K done" over contained panes
    /// (the built-in's activity summary); empty when nothing is going on.
    pub activity: String,
    /// What text search matches against: label plus the meta column,
    /// lowercased — mirrors the built-in's `search_text`.
    pub search_text: String,
}

/// Which rows [`Tree::build_rows`] keeps.
#[derive(Debug, Clone, Copy)]
enum RowFilter<'a> {
    /// Expansion-based view.
    None,
    /// Multi-word AND text query over each node's search text.
    Text(&'a str),
    /// Only nodes whose (aggregate) agent status equals this.
    State(AgentStatus),
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
    /// Total pane count across all workspaces, for the header — the
    /// built-in counts every pane there, regardless of active filters.
    pub fn pane_count(&self) -> usize {
        self.workspaces
            .iter()
            .flat_map(|ws| ws.tabs.iter())
            .map(|tab| tab.panes.len())
            .sum()
    }

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

    /// Carries the user's expand/collapse choices over to a freshly
    /// fetched tree (matched by workspace/tab id). Nodes that are new to
    /// this snapshot keep whatever the initial expansion gave them.
    pub fn adopt_expansion_from(&mut self, previous: &Tree) {
        for ws in &mut self.workspaces {
            let Some(prev_ws) = previous
                .workspaces
                .iter()
                .find(|prev| prev.workspace_id == ws.workspace_id)
            else {
                continue;
            };
            ws.expanded = prev_ws.expanded;
            for tab in &mut ws.tabs {
                if let Some(prev_tab) = prev_ws
                    .tabs
                    .iter()
                    .find(|prev| prev.info.tab_id == tab.info.tab_id)
                {
                    tab.expanded = prev_tab.expanded;
                }
            }
        }
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
        self.build_rows(RowFilter::None)
    }

    /// Search view: a node is visible iff its own search text (label plus
    /// the meta column, like the built-in) matches or any descendant's does
    /// (ancestors of matches come along for context; children of a matching
    /// branch do not). Matching is multi-word AND. Collapse state is
    /// ignored while a filter is active. Empty query = the expansion view.
    pub fn visible_rows_filtered(&self, query: &str) -> Vec<Row> {
        if query.is_empty() {
            self.build_rows(RowFilter::None)
        } else {
            self.build_rows(RowFilter::Text(query))
        }
    }

    /// State-filter view (the built-in's b/w/i/d keys): only nodes whose
    /// (aggregate) agent status equals `status`, with the same
    /// ancestor-reveal rules as text search.
    pub fn visible_rows_state_filtered(&self, status: AgentStatus) -> Vec<Row> {
        self.build_rows(RowFilter::State(status))
    }

    fn build_rows(&self, filter: RowFilter) -> Vec<Row> {
        // Lowercase the query once; every node comparison below runs
        // against already-lowercased search text (review feedback: the
        // per-node to_lowercase() allocations add up on every keystroke).
        let lowered_query = match filter {
            RowFilter::Text(query) => query.to_lowercase(),
            _ => String::new(),
        };
        let query = lowered_query.as_str();
        let mut rows = Vec::new();
        for (ws_idx, ws) in self.workspaces.iter().enumerate() {
            // Like the built-in goto (navigator_child_rows: `multi_tab =
            // ws.tabs.len() > 1`), a single-tab workspace skips the tab
            // level entirely — its panes hang directly off the workspace.
            // Placeholder workspaces (orphan tabs) keep the tab row: there
            // the tab is the only real node.
            let single_tab = ws.info.is_some() && ws.tabs.len() == 1;
            // (tab row shown, per-pane shown) for this workspace under
            // `filter`. For single-tab workspaces the tab row is never
            // shown, and its label is not searchable either.
            // Meta/search text per tab, computed up front because both the
            // visibility pass and the row construction need them.
            let tab_metas: Vec<(String, String)> = ws
                .tabs
                .iter()
                .map(|tab| {
                    let activity =
                        activity_summary(tab.panes.iter().map(|pane| pane.info.agent_status));
                    let meta = if activity.is_empty() {
                        format!("{} panes", tab.info.pane_count)
                    } else {
                        format!("{} panes · {}", tab.info.pane_count, activity)
                    };
                    let search = format!("{} {}", tab.info.label, meta).to_lowercase();
                    (activity, search)
                })
                .collect();
            let ws_activity = activity_summary(
                ws.tabs
                    .iter()
                    .flat_map(|tab| tab.panes.iter())
                    .map(|pane| pane.info.agent_status),
            );
            let ws_search = format!("{} {}", ws.label, ws_activity).to_lowercase();

            let tab_states: Vec<(bool, Vec<bool>)> = ws
                .tabs
                .iter()
                .zip(&tab_metas)
                .map(|(tab, (_, tab_search))| match filter {
                    RowFilter::None => (
                        !single_tab && ws.expanded,
                        tab.panes
                            .iter()
                            .map(|_| ws.expanded && (single_tab || tab.expanded))
                            .collect(),
                    ),
                    RowFilter::Text(_) => {
                        // A tab that matches on its own shows all of its
                        // panes, like the built-in (navigator_child_rows:
                        // `Text if tab_matches => pane_rows`).
                        let tab_matches =
                            !single_tab && crate::search::lowered_query_matches(tab_search, query);
                        let pane_shown: Vec<bool> = tab
                            .panes
                            .iter()
                            .map(|pane| {
                                tab_matches
                                    || crate::search::lowered_query_matches(
                                        &pane_search_text(&pane.info),
                                        query,
                                    )
                            })
                            .collect();
                        let tab_shown =
                            tab_matches || (!single_tab && pane_shown.iter().any(|&shown| shown));
                        (tab_shown, pane_shown)
                    }
                    RowFilter::State(status) => {
                        let pane_shown: Vec<bool> = tab
                            .panes
                            .iter()
                            .map(|pane| pane.info.agent_status == status)
                            .collect();
                        let tab_shown = !single_tab
                            && (tab.info.agent_status == status
                                || pane_shown.iter().any(|&shown| shown));
                        (tab_shown, pane_shown)
                    }
                })
                .collect();
            let any_child_shown = tab_states
                .iter()
                .any(|(tab_shown, panes)| *tab_shown || panes.iter().any(|&shown| shown));
            let ws_status_agg = ws
                .info
                .as_ref()
                .map(|i| i.agent_status)
                .unwrap_or(AgentStatus::Unknown);
            let ws_shown = match filter {
                RowFilter::None => true,
                RowFilter::Text(_) => {
                    crate::search::lowered_query_matches(&ws_search, query) || any_child_shown
                }
                RowFilter::State(status) => ws_status_agg == status || any_child_shown,
            };
            if !ws_shown {
                continue;
            }

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
                title: ws.label.clone(),
                expandable: if single_tab {
                    !ws.tabs[0].panes.is_empty()
                } else {
                    !ws.tabs.is_empty()
                },
                expanded: match filter {
                    RowFilter::None => ws.expanded,
                    _ => any_child_shown,
                },
                pane_count: ws_pane_count,
                agent: None,
                agent_status: ws_status,
                // Like the built-in: the active workspace carries a current
                // marker of its own, filters included.
                is_current: ws_info.is_some_and(|i| i.focused),
                focus_target: FocusTarget::Workspace(ws.workspace_id.clone()),
                detail: {
                    let mut detail = vec![
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
                    ];
                    if let Some(worktree) = ws_info.and_then(|i| i.worktree.as_ref()) {
                        let repo = if worktree.is_linked_worktree {
                            format!("{} (worktree)", worktree.repo_name)
                        } else {
                            worktree.repo_name.clone()
                        };
                        detail.push(("repo", repo));
                    }
                    if let Some(branch) = ws_info.and_then(|i| i.branch.clone()) {
                        detail.push(("branch", branch));
                    }
                    detail
                },
                cwd: None,
                custom_status: None,
                last_child: false,
                ancestor_continues: Vec::new(),
                activity: ws_activity,
                search_text: ws_search,
            });
            for (tab_idx, ((tab, (tab_shown, pane_shown)), (tab_activity, tab_search))) in
                ws.tabs.iter().zip(&tab_states).zip(&tab_metas).enumerate()
            {
                if *tab_shown {
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
                        title: format!("{}/{}", ws.label, tab.info.label),
                        expandable: !tab.panes.is_empty(),
                        expanded: match filter {
                            RowFilter::None => tab.expanded,
                            _ => any_pane_shown,
                        },
                        pane_count: tab.info.pane_count,
                        agent: None,
                        agent_status: tab.info.agent_status,
                        // The built-in never marks tab rows as current.
                        is_current: false,
                        focus_target: FocusTarget::Tab(tab.info.tab_id.clone()),
                        detail: vec![
                            ("id", tab.info.tab_id.clone()),
                            ("workspace", ws.label.clone()),
                            ("panes", tab.info.pane_count.to_string()),
                            ("status", tab.info.agent_status.name().to_string()),
                        ],
                        cwd: None,
                        custom_status: None,
                        last_child: false,
                        ancestor_continues: Vec::new(),
                        activity: tab_activity.clone(),
                        search_text: tab_search.clone(),
                    });
                }
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
                        (
                            "status",
                            pane.info
                                .custom_status
                                .clone()
                                .unwrap_or_else(|| pane.info.agent_status.name().to_string()),
                        ),
                    ];
                    if let Some(cwd) = &pane.info.cwd {
                        detail.push(("cwd", cwd.clone()));
                    }
                    if let Some(branch) = &pane.info.branch {
                        detail.push(("branch", branch.clone()));
                    }
                    if let Some(title) = &pane.info.title {
                        if !title.is_empty() {
                            detail.push(("title", title.clone()));
                        }
                    }
                    let label = pane_label(&pane.info);
                    rows.push(Row {
                        path: pane_path,
                        kind: RowKind::Pane,
                        depth: if single_tab { 1 } else { 2 },
                        title: if single_tab {
                            format!("{}/{}", ws.label, label)
                        } else {
                            format!("{}/{}/{}", ws.label, tab.info.label, label)
                        },
                        label,
                        expandable: false,
                        expanded: false,
                        pane_count: 0,
                        agent,
                        agent_status: pane.info.agent_status,
                        // PaneInfo.focused is globally unique (active
                        // workspace + tab + pane), same as the built-in's
                        // is_active_pane.
                        is_current: pane.info.focused,
                        focus_target: FocusTarget::Pane {
                            pane_id: pane.info.pane_id.clone(),
                            tab_id: pane.info.tab_id.clone(),
                        },
                        detail,
                        cwd: pane.info.cwd.clone(),
                        custom_status: pane.info.custom_status.clone(),
                        last_child: false,
                        ancestor_continues: Vec::new(),
                        activity: String::new(),
                        search_text: pane_search_text(&pane.info),
                    });
                }
            }
        }
        annotate_guides(&mut rows);
        rows
    }

    /// Index of the starting row: the current pane if visible, else the
    /// current workspace (both carry `is_current`; the pane row is deeper
    /// and therefore later), or 0.
    pub fn initial_cursor(&self) -> usize {
        let rows = self.visible_rows();
        rows.iter().rposition(|row| row.is_current).unwrap_or(0)
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

    /// The nearest *visible* ancestor: panes in a single-tab workspace skip
    /// the (never rendered) tab level and report the workspace.
    pub fn parent_path(&self, path: NodePath) -> Option<NodePath> {
        let ws_path = NodePath {
            ws: path.ws,
            tab: None,
            pane: None,
        };
        match (path.tab, path.pane) {
            (Some(tab), Some(_)) => {
                let single_tab = self
                    .workspaces
                    .get(path.ws)
                    .is_some_and(|ws| ws.info.is_some() && ws.tabs.len() == 1);
                if single_tab {
                    Some(ws_path)
                } else {
                    Some(NodePath {
                        ws: path.ws,
                        tab: Some(tab),
                        pane: None,
                    })
                }
            }
            (Some(_), None) => Some(ws_path),
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

/// Fills in tree-guide info over the flattened rows: whether each row is
/// the last visible child of its parent (`└──` vs `├──`), and which
/// ancestor levels still have siblings below (`│` continuation columns).
fn annotate_guides(rows: &mut [Row]) {
    for i in 0..rows.len() {
        let depth = rows[i].depth;
        if depth == 0 {
            continue;
        }
        let mut last_child = true;
        for row in &rows[i + 1..] {
            if row.depth < depth {
                break;
            }
            if row.depth == depth {
                last_child = false;
                break;
            }
        }
        let ancestor_continues = (1..depth)
            .map(|level| {
                rows[i + 1..]
                    .iter()
                    .take_while(|row| row.depth >= level)
                    .any(|row| row.depth == level)
            })
            .collect();
        rows[i].last_child = last_child;
        rows[i].ancestor_continues = ancestor_continues;
    }
}

/// The built-in's activity summary: counts of blocked/working/done panes,
/// joined as "N blocked · M working · K done"; empty when all quiet
/// (idle-and-seen and unknown panes are not counted).
fn activity_summary(statuses: impl Iterator<Item = AgentStatus>) -> String {
    let (mut blocked, mut working, mut done) = (0usize, 0usize, 0usize);
    for status in statuses {
        match status {
            AgentStatus::Blocked => blocked += 1,
            AgentStatus::Working => working += 1,
            AgentStatus::Done => done += 1,
            AgentStatus::Idle | AgentStatus::Unknown => {}
        }
    }
    let mut parts = Vec::new();
    if blocked > 0 {
        parts.push(format!("{blocked} blocked"));
    }
    if working > 0 {
        parts.push(format!("{working} working"));
    }
    if done > 0 {
        parts.push(format!("{done} done"));
    }
    parts.join(" · ")
}

/// The pane's meta column: "{agent} · {status}" or bare "shell".
fn pane_meta(info: &PaneInfo) -> String {
    let agent = info.display_agent.as_deref().or(info.agent.as_deref());
    match agent {
        Some(agent) => {
            let status = info
                .custom_status
                .as_deref()
                .unwrap_or_else(|| info.agent_status.name());
            format!("{agent} · {status}")
        }
        None => "shell".to_string(),
    }
}

/// What text search sees for a pane: label + meta, like the built-in.
fn pane_search_text(info: &PaneInfo) -> String {
    format!("{} {}", pane_label(info), pane_meta(info)).to_lowercase()
}

/// Pane display label, mirroring the built-in goto's chain
/// (`navigator_pane_rows_for_tab`): effective title -> manual label ->
/// agent label -> "pane N" from the public id suffix ("w1:p8" -> "pane 8").
/// (The built-in's final launch-command fallback is not exposed by the API.)
fn pane_label(info: &PaneInfo) -> String {
    for candidate in [&info.title, &info.label, &info.agent, &info.display_agent]
        .into_iter()
        .flatten()
    {
        if !candidate.is_empty() {
            return candidate.clone();
        }
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
            worktree: None,
            branch: None,
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
            foreground_cwd: None,
            label: None,
            title: None,
            custom_status: None,
            terminal_id: format!("term_{id}"),
            branch: None,
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
                pane("w2:p1", "w2:t1", "w2", false, None),
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
        // beta has a single tab, so (like the built-in goto) its tab row
        // is skipped and the pane hangs directly off the workspace.
        assert_eq!(
            labels(&rows),
            vec!["alpha", "a-one", "claude", "pane 2", "a-two", "pane 3", "beta", "pane 1"]
        );
        let depths: Vec<u8> = rows.iter().map(|r| r.depth).collect();
        assert_eq!(depths, vec![0, 1, 2, 2, 1, 2, 0, 1]);
        assert_eq!(rows[0].kind, RowKind::Workspace);
        assert_eq!(rows[1].kind, RowKind::Tab);
        assert_eq!(rows[2].kind, RowKind::Pane);
        assert_eq!(rows[7].kind, RowKind::Pane, "beta's pane, no tab between");
    }

    #[test]
    fn single_tab_workspace_skips_the_tab_level_like_the_builtin() {
        // Mirrors herdr's navigator_rows_show_tab_nodes_only_for_multi_tab_workspaces.
        let tree = fixture(InitialExpansion::All);
        let rows = tree.visible_rows();
        assert!(
            !rows
                .iter()
                .any(|r| r.kind == RowKind::Tab && r.label == "b-one"),
            "single-tab workspace must not render its tab row"
        );
        let beta_pane = rows
            .iter()
            .find(|r| r.kind == RowKind::Pane && r.path.ws == 1)
            .unwrap();
        assert_eq!(beta_pane.depth, 1, "pane sits directly under the workspace");
        assert_eq!(
            tree.parent_path(beta_pane.path),
            Some(NodePath {
                ws: 1,
                tab: None,
                pane: None
            }),
            "collapse from the pane walks to the workspace"
        );

        // Filtering by the hidden tab's label reveals nothing (the built-in
        // cannot match it either — the row does not exist).
        assert!(tree.visible_rows_filtered("b-one").is_empty());
        // A pane match still reveals the chain: workspace -> pane. With
        // meta included in search (like the built-in), "pane 1" also hits
        // a-two's "1 panes" meta — and a tab match reveals its panes.
        assert_eq!(
            labels(&tree.visible_rows_filtered("pane 1")),
            vec!["alpha", "a-two", "pane 3", "beta", "pane 1"]
        );
    }

    #[test]
    fn current_workspace_expansion_opens_focused_workspace_and_tab_only() {
        let rows = fixture(InitialExpansion::CurrentWorkspace).visible_rows();
        assert_eq!(
            labels(&rows),
            vec!["alpha", "a-one", "claude", "pane 2", "a-two", "beta"]
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
    fn is_current_marks_the_active_workspace_and_pane_like_the_builtin() {
        let tree = fixture(InitialExpansion::All);
        let rows = tree.visible_rows();
        let current: Vec<&str> = rows
            .iter()
            .filter(|r| r.is_current)
            .map(|r| r.label.as_str())
            .collect();
        assert_eq!(
            current,
            vec!["alpha", "claude"],
            "workspace AND pane carry the marker; tabs never do"
        );

        let mut tree = fixture(InitialExpansion::All);
        let tab_path = tree.visible_rows()[1].path;
        tree.collapse(tab_path);
        let rows = tree.visible_rows();
        let current: Vec<&str> = rows
            .iter()
            .filter(|r| r.is_current)
            .map(|r| r.label.as_str())
            .collect();
        assert_eq!(
            current,
            vec!["alpha"],
            "hidden pane leaves only the workspace marked"
        );

        // Unlike before, the marker survives an active filter.
        let tree = fixture(InitialExpansion::All);
        let rows = tree.visible_rows_filtered("alpha");
        assert!(rows.iter().any(|r| r.is_current), "marker under filter");
    }

    #[test]
    fn initial_cursor_sits_on_the_deepest_current_row() {
        // Both alpha (ws) and claude (pane) are current; the pane wins.
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
        assert_eq!(
            rows[2].focus_target,
            FocusTarget::Pane {
                pane_id: "w1:p1".to_string(),
                tab_id: "w1:t1".to_string()
            }
        );
        assert_eq!(rows[2].agent.as_deref(), Some("claude"));
        assert_eq!(rows[3].agent, None, "agentless pane");
        assert_eq!(rows[1].pane_count, 2, "tab pane count");
        assert!(rows[1].expandable);
        assert!(!rows[2].expandable, "panes are leaves");
    }

    #[test]
    fn pane_label_follows_the_builtin_chain() {
        // title > manual label > agent label > "pane N"
        let mut titled = pane("w1:p1", "w1:t1", "w1", false, Some("claude"));
        titled.title = Some("make -j8".to_string());
        titled.label = Some("builder".to_string());

        let mut labeled = pane("w1:p2", "w1:t1", "w1", false, Some("claude"));
        labeled.label = Some("builder".to_string());

        let agent_only = pane("w1:p3", "w1:t1", "w1", false, Some("claude"));

        let mut empty_title = pane("w1:p8", "w1:t1", "w1", false, None);
        empty_title.title = Some(String::new()); // empty strings do not count

        let tree = Tree::build(
            vec![workspace("w1", 1, "alpha", true)],
            vec![tab("w1:t1", "w1", 1, "a-one", true, 4)],
            vec![titled, labeled, agent_only, empty_title],
            InitialExpansion::All,
        );
        let rows = tree.visible_rows();
        // Single-tab workspace: panes at rows 1..=4, no tab row.
        assert_eq!(rows[1].label, "make -j8");
        assert_eq!(rows[2].label, "builder");
        assert_eq!(rows[3].label, "claude");
        assert_eq!(rows[4].label, "pane 8");
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
        assert_eq!(current, vec!["alpha", "pane 1"]);
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

        // A matching tab reveals all of its panes, like the built-in
        // (navigator_child_rows: `Text if tab_matches => pane_rows`).
        let rows = tree.visible_rows_filtered("two");
        assert_eq!(labels(&rows), vec!["alpha", "a-two", "pane 3"]);
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
    fn state_filter_shows_matching_nodes_with_ancestors() {
        let mut blocked = pane("w1:p1", "w1:t1", "w1", true, Some("claude"));
        blocked.agent_status = AgentStatus::Blocked;
        let tree = Tree::build(
            vec![
                workspace("w1", 1, "alpha", true),
                workspace("w2", 2, "beta", false),
            ],
            vec![
                tab("w1:t1", "w1", 1, "a-one", true, 2),
                tab("w1:t2", "w1", 2, "a-two", false, 1),
                tab("w2:t1", "w2", 1, "b-one", true, 1),
            ],
            vec![
                blocked,
                pane("w1:p2", "w1:t1", "w1", false, None),
                pane("w2:p1", "w2:t1", "w2", false, None),
            ],
            InitialExpansion::None, // collapse must not matter
        );

        let rows = tree.visible_rows_state_filtered(AgentStatus::Blocked);
        assert_eq!(labels(&rows), vec!["alpha", "a-one", "claude"]);
        assert!(tree
            .visible_rows_state_filtered(AgentStatus::Working)
            .is_empty());
    }

    #[test]
    fn activity_summaries_count_blocked_working_done() {
        let mut blocked = pane("w1:p1", "w1:t1", "w1", true, Some("claude"));
        blocked.agent_status = AgentStatus::Blocked;
        let mut working = pane("w1:p2", "w1:t1", "w1", false, Some("claude"));
        working.agent_status = AgentStatus::Working;
        let mut done = pane("w1:p3", "w1:t2", "w1", false, Some("claude"));
        done.agent_status = AgentStatus::Done;
        let idle = pane("w1:p4", "w1:t2", "w1", false, Some("claude"));

        let tree = Tree::build(
            vec![workspace("w1", 1, "alpha", true)],
            vec![
                tab("w1:t1", "w1", 1, "a-one", true, 2),
                tab("w1:t2", "w1", 2, "a-two", false, 2),
            ],
            vec![blocked, working, done, idle],
            InitialExpansion::All,
        );
        let rows = tree.visible_rows();

        assert_eq!(rows[0].activity, "1 blocked · 1 working · 1 done");
        assert_eq!(rows[1].activity, "1 blocked · 1 working", "tab a-one");
        let a_two = rows.iter().find(|r| r.label == "a-two").unwrap();
        assert_eq!(a_two.activity, "1 done", "idle-and-seen is not counted");
        // Search sees the activity too (meta is part of search_text).
        assert!(rows[0].search_text.contains("1 blocked"));
    }

    #[test]
    fn rows_carry_detail_pairs_for_the_detail_panel() {
        let mut with_meta = pane("w1:p1", "w1:t1", "w1", true, Some("claude"));
        with_meta.cwd = Some("/home/u/repo".to_string());
        with_meta.title = Some("make -j8".to_string());
        with_meta.custom_status = Some("reviewing".to_string());
        let tree = Tree::build(
            vec![workspace("w1", 1, "alpha", true)],
            vec![
                tab("w1:t1", "w1", 1, "a-one", true, 2),
                // Second tab keeps the workspace multi-tab so the tab row
                // (whose detail this test checks) actually renders.
                tab("w1:t2", "w1", 2, "a-two", false, 0),
            ],
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
                // custom_status wins over the state name ("idle").
                ("status", "reviewing".to_string()),
                ("cwd", "/home/u/repo".to_string()),
                ("title", "make -j8".to_string()),
            ]
        );
        assert_eq!(rows[2].custom_status.as_deref(), Some("reviewing"));

        let agentless = &rows[3].detail;
        assert!(agentless.contains(&("agent", "shell".to_string())));
        assert!(
            agentless.iter().all(|(k, _)| *k != "cwd" && *k != "title"),
            "absent metadata stays out of the panel: {agentless:?}"
        );
    }

    #[test]
    fn row_titles_carry_the_ancestor_path() {
        let rows_all = fixture(InitialExpansion::All);
        let rows = rows_all.visible_rows();
        // fixture: alpha (a-one: claude+pane, a-two: pane), beta single-tab.
        let by_label = |label: &str| rows.iter().find(|r| r.label == label).unwrap();

        assert_eq!(by_label("alpha").title, "alpha");
        assert_eq!(by_label("a-one").title, "alpha/a-one");
        assert_eq!(by_label("claude").title, "alpha/a-one/claude");
        // Single-tab workspaces skip the tab level in the rows, and the
        // title follows suit.
        let beta_pane = rows
            .iter()
            .find(|r| r.kind == RowKind::Pane && r.path.ws == by_label("beta").path.ws)
            .unwrap();
        assert_eq!(beta_pane.title, format!("beta/{}", beta_pane.label));
    }

    #[test]
    fn pane_detail_includes_the_branch_when_resolved() {
        let mut on_branch = pane("w1:p1", "w1:t1", "w1", true, Some("claude"));
        on_branch.cwd = Some("/home/u/repo".to_string());
        on_branch.branch = Some("feature/x".to_string());
        let tree = Tree::build(
            vec![workspace("w1", 1, "alpha", true)],
            vec![tab("w1:t1", "w1", 1, "a-one", true, 2)],
            vec![on_branch, pane("w1:p2", "w1:t1", "w1", false, None)],
            InitialExpansion::All,
        );
        let rows = tree.visible_rows();

        // Branch sits right under cwd — they describe the same place.
        let detail = &rows[1].detail;
        let cwd_idx = detail.iter().position(|(k, _)| *k == "cwd").unwrap();
        assert_eq!(detail[cwd_idx + 1], ("branch", "feature/x".to_string()));
        // No branch resolved (p2) → no branch line.
        assert!(rows[2].detail.iter().all(|(k, _)| *k != "branch"));
    }

    #[test]
    fn workspace_detail_includes_worktree_repo_and_branch() {
        let mut ws = workspace("w1", 1, "alpha", true);
        ws.worktree = Some(crate::herdr_client::WorkspaceWorktree {
            repo_name: "herdr".to_string(),
            checkout_path: "/home/u/src/herdr-wt/fix".to_string(),
            is_linked_worktree: true,
        });
        ws.branch = Some("fix/thing".to_string());
        let tree = Tree::build(
            vec![ws, workspace("w2", 2, "beta", false)],
            vec![
                tab("w1:t1", "w1", 1, "a-one", true, 1),
                tab("w2:t1", "w2", 1, "b-one", true, 1),
            ],
            vec![
                pane("w1:p1", "w1:t1", "w1", true, None),
                pane("w2:p1", "w2:t1", "w2", false, None),
            ],
            InitialExpansion::None,
        );
        let rows = tree.visible_rows();

        let detail = &rows[0].detail;
        assert!(detail.contains(&("repo", "herdr (worktree)".to_string())));
        assert!(detail.contains(&("branch", "fix/thing".to_string())));
        // Plain workspaces stay as before.
        let plain = &rows[1].detail;
        assert!(plain.iter().all(|(k, _)| *k != "repo" && *k != "branch"));
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
