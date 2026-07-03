//! Flat picker rows built from herdr's workspace/tab snapshot (M1 scope;
//! becomes a real tree in M2).

use crate::herdr_client::{AgentStatus, TabInfo, WorkspaceInfo};

/// One selectable row: a tab, labeled with its workspace.
#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub tab_id: String,
    pub workspace_label: String,
    pub tab_label: String,
    pub pane_count: usize,
    pub agent_status: AgentStatus,
    /// True for the tab the user came from (focused tab of the focused
    /// workspace) — marked in the UI and the default cursor position.
    pub is_current: bool,
}

/// Joins the snapshot into rows ordered by (workspace number, tab number).
pub fn build_flat_list(workspaces: &[WorkspaceInfo], tabs: &[TabInfo]) -> Vec<Item> {
    let mut rows: Vec<(usize, usize, Item)> = tabs
        .iter()
        .map(|tab| {
            let ws = workspaces
                .iter()
                .find(|ws| ws.workspace_id == tab.workspace_id);
            let item = Item {
                tab_id: tab.tab_id.clone(),
                // A tab whose workspace is missing from the snapshot still
                // deserves a row; its raw workspace id is better than nothing.
                workspace_label: ws
                    .map(|ws| ws.label.clone())
                    .unwrap_or_else(|| tab.workspace_id.clone()),
                tab_label: tab.label.clone(),
                pane_count: tab.pane_count,
                agent_status: tab.agent_status,
                is_current: tab.focused && ws.is_some_and(|ws| ws.focused),
            };
            (
                ws.map(|ws| ws.number).unwrap_or(usize::MAX),
                tab.number,
                item,
            )
        })
        .collect();
    rows.sort_by_key(|(ws_number, tab_number, _)| (*ws_number, *tab_number));
    rows.into_iter().map(|(_, _, item)| item).collect()
}

/// Where the cursor starts: the caller's context tab if it is in the list,
/// otherwise the current tab, otherwise the top.
pub fn initial_cursor(items: &[Item], context_tab_id: Option<&str>) -> usize {
    context_tab_id
        .and_then(|id| items.iter().position(|item| item.tab_id == id))
        .or_else(|| items.iter().position(|item| item.is_current))
        .unwrap_or(0)
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

    #[test]
    fn orders_by_workspace_number_then_tab_number() {
        let workspaces = [
            workspace("w2", 2, "beta", false),
            workspace("w1", 1, "alpha", false),
        ];
        let tabs = [
            tab("w2:t1", "w2", 1, "b-one", false),
            tab("w1:t2", "w1", 2, "a-two", false),
            tab("w1:t1", "w1", 1, "a-one", false),
        ];

        let items = build_flat_list(&workspaces, &tabs);

        let ids: Vec<&str> = items.iter().map(|i| i.tab_id.as_str()).collect();
        assert_eq!(ids, vec!["w1:t1", "w1:t2", "w2:t1"]);
        assert_eq!(items[0].workspace_label, "alpha");
        assert_eq!(items[0].tab_label, "a-one");
    }

    #[test]
    fn is_current_requires_focused_workspace_and_focused_tab() {
        // TabInfo.focused is per-workspace (each workspace has one focused
        // tab), so it must be ANDed with the workspace's own focused flag.
        let workspaces = [
            workspace("w1", 1, "alpha", false),
            workspace("w2", 2, "beta", true),
        ];
        let tabs = [
            tab("w1:t1", "w1", 1, "a-one", true),
            tab("w2:t1", "w2", 1, "b-one", true),
            tab("w2:t2", "w2", 2, "b-two", false),
        ];

        let items = build_flat_list(&workspaces, &tabs);

        let current: Vec<&str> = items
            .iter()
            .filter(|i| i.is_current)
            .map(|i| i.tab_id.as_str())
            .collect();
        assert_eq!(current, vec!["w2:t1"]);
    }

    #[test]
    fn tab_with_unknown_workspace_still_gets_a_row() {
        let tabs = [tab("w9:t1", "w9", 1, "orphan", false)];

        let items = build_flat_list(&[], &tabs);

        assert_eq!(items.len(), 1);
        assert_eq!(items[0].workspace_label, "w9");
    }

    fn items() -> Vec<Item> {
        let workspaces = [
            workspace("w1", 1, "alpha", true),
            workspace("w2", 2, "beta", false),
        ];
        let tabs = [
            tab("w1:t1", "w1", 1, "a-one", false),
            tab("w1:t2", "w1", 2, "a-two", true),
            tab("w2:t1", "w2", 1, "b-one", true),
        ];
        build_flat_list(&workspaces, &tabs)
    }

    #[test]
    fn initial_cursor_prefers_context_tab() {
        assert_eq!(initial_cursor(&items(), Some("w2:t1")), 2);
    }

    #[test]
    fn initial_cursor_falls_back_to_current_tab() {
        assert_eq!(initial_cursor(&items(), None), 1);
        assert_eq!(initial_cursor(&items(), Some("w9:t9")), 1);
    }

    #[test]
    fn initial_cursor_defaults_to_top() {
        let workspaces = [workspace("w1", 1, "alpha", false)];
        let tabs = [tab("w1:t1", "w1", 1, "a-one", false)];
        let no_current = build_flat_list(&workspaces, &tabs);
        assert_eq!(initial_cursor(&no_current, None), 0);
        assert_eq!(initial_cursor(&[], None), 0);
    }
}
