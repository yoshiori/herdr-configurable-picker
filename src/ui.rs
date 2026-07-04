//! ratatui rendering: bordered tree list, reverse-video cursor row, and a
//! footer hint line built from the *actual* keymap so it never lies about
//! bindings.

use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};
use ratatui::Frame;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::app::App;
use crate::keymap::{Action, Keymap};
use crate::tree::{Row, RowKind};

/// Footer entries as `(key label, action label)` pairs, e.g. `("↑/↓", "move")`.
pub struct FooterHints {
    pub entries: Vec<(String, String)>,
}

impl FooterHints {
    /// Reads the first bound key per action out of the keymap, so the hint
    /// line reflects the user's config, not our defaults.
    pub fn from_keymap(keymap: &Keymap) -> FooterHints {
        let mut entries = Vec::new();
        let up = keymap.first_binding_label(Action::Up);
        let down = keymap.first_binding_label(Action::Down);
        match (up, down) {
            (Some(up), Some(down)) => entries.push((format!("{up}/{down}"), "move".to_string())),
            (Some(key), None) | (None, Some(key)) => entries.push((key, "move".to_string())),
            (None, None) => {}
        }
        for (action, label) in [
            (Action::Expand, "expand"),
            (Action::Collapse, "collapse"),
            (Action::Accept, "accept"),
            (Action::Cancel, "cancel"),
        ] {
            if let Some(key) = keymap.first_binding_label(action) {
                entries.push((key, label.to_string()));
            }
        }
        FooterHints { entries }
    }

    fn line(&self) -> String {
        self.entries
            .iter()
            .map(|(key, action)| format!("{key} {action}"))
            .collect::<Vec<_>>()
            .join("   ")
    }
}

/// The detail panel needs at least this much total width to be worth
/// splitting off; below it the list gets the whole canvas.
const DETAIL_MIN_TOTAL_WIDTH: u16 = 60;

pub fn draw(frame: &mut Frame, app: &mut App, hints: &FooterHints) {
    let outer = Block::bordered().title(" goto ");
    let inner = outer.inner(frame.area());
    frame.render_widget(outer, frame.area());

    let [main_area, footer_area] =
        Layout::vertical([Constraint::Min(1), Constraint::Length(2)]).areas(inner);

    let (list_area, detail_area) = if main_area.width >= DETAIL_MIN_TOTAL_WIDTH {
        let detail_width = (main_area.width / 3).clamp(24, 48);
        let [list, detail] =
            Layout::horizontal([Constraint::Min(1), Constraint::Length(detail_width)])
                .areas(main_area);
        (list, Some(detail))
    } else {
        (main_area, None)
    };
    app.viewport_height = list_area.height;

    if app.rows().is_empty() {
        frame.render_widget(Paragraph::new("No workspaces found."), list_area);
    } else {
        let width = list_area.width as usize;
        let items: Vec<ListItem> = app
            .rows()
            .iter()
            .map(|row| ListItem::new(Line::from(row_text(row, width))))
            .collect();
        let list = List::new(items).highlight_style(Style::new().add_modifier(Modifier::REVERSED));
        let mut state = ListState::default().with_selected(Some(app.cursor));
        frame.render_stateful_widget(list, list_area, &mut state);
    }

    if let Some(detail_area) = detail_area {
        render_detail(frame, app, detail_area);
    }

    let footer = Paragraph::new(hints.line()).block(Block::new().borders(Borders::TOP));
    frame.render_widget(footer, footer_area);
}

/// Right-hand panel describing the row under the cursor.
fn render_detail(frame: &mut Frame, app: &App, area: ratatui::layout::Rect) {
    let block = Block::new().borders(Borders::LEFT);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let Some(row) = app.rows().get(app.cursor) else {
        return;
    };
    let home = std::env::var("HOME").ok();
    let key_width = row
        .detail
        .iter()
        .map(|(key, _)| key.chars().count())
        .max()
        .unwrap_or(0);

    let mut lines = vec![
        Line::styled(
            format!(" {}", row.label),
            Style::new().add_modifier(Modifier::BOLD),
        ),
        Line::raw(""),
    ];
    for (key, value) in &row.detail {
        let value = if *key == "cwd" {
            shorten_home(value, home.as_deref())
        } else {
            value.clone()
        };
        lines.push(Line::raw(format!(" {key:<key_width$}  {value}")));
    }
    frame.render_widget(Paragraph::new(lines), inner);
}

/// `/home/user/src/repo` -> `~/src/repo` for display.
fn shorten_home(path: &str, home: Option<&str>) -> String {
    match home {
        Some(home) if !home.is_empty() => match path.strip_prefix(home) {
            Some("") => "~".to_string(),
            Some(rest) if rest.starts_with('/') => format!("~{rest}"),
            _ => path.to_string(),
        },
        _ => path.to_string(),
    }
}

/// One row: current marker, indentation, expansion glyph, and label on the
/// left; pane count (branches) or agent name (panes) right-aligned. Status
/// details live in the detail panel. Drops the right column on narrow
/// terminals rather than wrapping.
fn row_text(row: &Row, width: usize) -> String {
    let marker = if row.is_current { "→" } else { " " };
    let indent = "  ".repeat(row.depth as usize);
    let glyph = if row.expandable {
        if row.expanded {
            "▼ "
        } else {
            "▶ "
        }
    } else if row.kind == RowKind::Pane {
        ""
    } else {
        "· " // childless workspace/tab: nothing to expand
    };
    let left = format!("{marker} {indent}{glyph}{}", row.label);

    let right = if row.kind == RowKind::Pane {
        row.agent.as_deref().unwrap_or("shell").to_string()
    } else {
        let panes = if row.pane_count == 1 { "pane" } else { "panes" };
        format!("{} {panes}", row.pane_count)
    };

    // Terminal columns, not chars: CJK labels are two columns per char and
    // would push the right column out of alignment otherwise.
    let left_cols = UnicodeWidthStr::width(left.as_str());
    let right_cols = UnicodeWidthStr::width(right.as_str());
    if left_cols + right_cols + 2 <= width {
        let padding = width - left_cols - right_cols - 1;
        format!("{left}{}{right} ", " ".repeat(padding))
    } else {
        // Not enough room for both: keep the labels, drop the right column.
        truncate_to_width(&left, width)
    }
}

/// Cuts `text` down to at most `width` terminal columns on a char boundary.
fn truncate_to_width(text: &str, width: usize) -> String {
    let mut out = String::new();
    let mut used = 0;
    for c in text.chars() {
        let w = UnicodeWidthChar::width(c).unwrap_or(0);
        if used + w > width {
            break;
        }
        used += w;
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::EnterOnBranch;
    use crate::config::KeysConfig;
    use crate::herdr_client::{AgentStatus, PaneInfo, TabInfo, WorkspaceInfo};
    use crate::tree::{InitialExpansion, Tree};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

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
            pane_count: 2,
            agent_status: AgentStatus::Working,
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

    fn sample_app() -> App {
        let tree = Tree::build(
            vec![workspace("w1", 1, "mothership", true)],
            vec![tab("w1:t1", "w1", 1, "main", true)],
            vec![
                pane("w1:p1", "w1:t1", "w1", true, Some("claude")),
                pane("w1:p2", "w1:t1", "w1", false, None),
            ],
            InitialExpansion::All,
        );
        App::new(tree, EnterOnBranch::Jump)
    }

    fn default_hints() -> FooterHints {
        let (keymap, _) = Keymap::from_bindings(&KeysConfig::default().to_bindings());
        FooterHints::from_keymap(&keymap)
    }

    fn render(width: u16, height: u16, app: &mut App) -> Terminal<TestBackend> {
        let hints = default_hints();
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| draw(frame, app, &hints)).unwrap();
        terminal
    }

    fn buffer_lines(terminal: &Terminal<TestBackend>) -> Vec<String> {
        let buffer = terminal.backend().buffer();
        let area = buffer.area;
        (area.top()..area.bottom())
            .map(|y| {
                (area.left()..area.right())
                    .map(|x| buffer.cell((x, y)).unwrap().symbol())
                    .collect()
            })
            .collect()
    }

    fn screen(terminal: &Terminal<TestBackend>) -> String {
        buffer_lines(terminal).join("\n")
    }

    #[test]
    fn tree_rows_show_glyphs_indentation_and_right_columns() {
        let mut app = sample_app();
        let terminal = render(80, 24, &mut app);
        let screen = screen(&terminal);

        assert!(screen.contains("▼ mothership"), "screen:\n{screen}");
        assert!(screen.contains("  ▼ main"), "indented tab:\n{screen}");
        assert!(screen.contains("    pane 1"), "indented pane:\n{screen}");
        assert!(screen.contains("2 panes"), "tab pane count:\n{screen}");
        let lines = buffer_lines(&terminal);
        let pane2 = lines.iter().find(|l| l.contains("pane 2")).unwrap();
        assert!(pane2.contains("shell"), "agentless column: {pane2:?}");
    }

    #[test]
    fn detail_panel_describes_the_row_under_the_cursor() {
        let mut app = sample_app(); // cursor on the focused pane (pane 1)
        let terminal = render(100, 24, &mut app);
        let screen = screen(&terminal);

        assert!(screen.contains("w1:p1"), "selected pane id:\n{screen}");
        assert!(screen.contains("agent"), "screen:\n{screen}");
        assert!(screen.contains("claude"), "screen:\n{screen}");
        assert!(screen.contains("status"), "screen:\n{screen}");
        assert!(screen.contains("idle"), "screen:\n{screen}");
    }

    #[test]
    fn narrow_terminal_hides_the_detail_panel() {
        let mut app = sample_app();
        let terminal = render(50, 12, &mut app);
        let screen = screen(&terminal);

        assert!(
            !screen.contains("w1:p1"),
            "no detail ids on a narrow screen:\n{screen}"
        );
        assert!(
            screen.contains("mothership"),
            "list still renders:\n{screen}"
        );
    }

    #[test]
    fn wide_characters_keep_the_right_column_aligned() {
        let tree = Tree::build(
            vec![
                workspace("w1", 1, "日本語のラベル", true),
                workspace("w2", 2, "ascii", false),
            ],
            vec![
                tab("w1:t1", "w1", 1, "t", true),
                tab("w2:t1", "w2", 1, "t", true),
            ],
            vec![],
            InitialExpansion::None,
        );
        let mut app = App::new(tree, EnterOnBranch::Jump);
        let terminal = render(50, 10, &mut app); // < 60 cols: list only
        let lines = buffer_lines(&terminal);

        // Continuation cells of wide glyphs read back as blanks, so search
        // by a single character.
        let jp = lines.iter().find(|l| l.contains('日')).unwrap();
        let ascii = lines.iter().find(|l| l.contains("ascii")).unwrap();
        // Right-aligned pane counts must survive double-width labels; if the
        // width math counted chars instead of columns the count would be
        // pushed past the border and clipped.
        let jp_body = jp.trim_end_matches([' ', '│']);
        let ascii_body = ascii.trim_end_matches([' ', '│']);
        assert!(jp_body.ends_with("0 panes"), "jp row: {jp:?}");
        assert!(ascii_body.ends_with("0 panes"), "ascii row: {ascii:?}");
    }

    #[test]
    fn shorten_home_replaces_the_home_prefix() {
        assert_eq!(
            shorten_home("/home/u/src/repo", Some("/home/u")),
            "~/src/repo"
        );
        assert_eq!(shorten_home("/home/u", Some("/home/u")), "~");
        assert_eq!(
            shorten_home("/home/unrelated/x", Some("/home/u")),
            "/home/unrelated/x",
            "prefix must match a whole path component"
        );
        assert_eq!(shorten_home("/tmp/x", None), "/tmp/x");
    }

    #[test]
    fn collapsed_branch_shows_the_collapsed_glyph() {
        let tree = Tree::build(
            vec![workspace("w1", 1, "mothership", true)],
            vec![tab("w1:t1", "w1", 1, "main", true)],
            vec![pane("w1:p1", "w1:t1", "w1", true, None)],
            InitialExpansion::None,
        );
        let mut app = App::new(tree, EnterOnBranch::Jump);
        let terminal = render(80, 24, &mut app);
        assert!(
            screen(&terminal).contains("▶ mothership"),
            "screen:\n{}",
            screen(&terminal)
        );
    }

    #[test]
    fn cursor_row_is_reversed() {
        let mut app = sample_app(); // cursor starts on the focused pane row
        let terminal = render(80, 24, &mut app);

        let buffer = terminal.backend().buffer();
        let lines = buffer_lines(&terminal);
        // "    pane 1" (indented) is the list row; the detail panel header
        // also says "pane 1" but without the tree indentation.
        let cursor_y = lines
            .iter()
            .position(|line| line.contains("    pane 1"))
            .expect("cursor row must be on screen") as u16;
        let x = lines[cursor_y as usize]
            .chars()
            .position(|c| c == 'p')
            .unwrap() as u16;
        let style = buffer.cell((x, cursor_y)).unwrap().style();
        assert!(
            style.add_modifier.contains(Modifier::REVERSED),
            "cursor row must be reverse video, got {style:?}"
        );
    }

    #[test]
    fn current_row_carries_a_marker() {
        let mut app = sample_app();
        let terminal = render(80, 24, &mut app);
        let lines = buffer_lines(&terminal);

        let current = lines.iter().find(|l| l.contains("    pane 1")).unwrap();
        assert!(current.contains("→"), "current row: {current:?}");
        let other = lines.iter().find(|l| l.contains("    pane 2")).unwrap();
        assert!(!other.contains("→"), "other row: {other:?}");
    }

    #[test]
    fn footer_reflects_the_actual_keymap_not_the_defaults() {
        let (keymap, warnings) = Keymap::from_bindings(&[
            (Action::Down, vec!["ctrl+j".to_string()]),
            (Action::Up, vec!["ctrl+k".to_string()]),
            (Action::Expand, vec!["tab".to_string()]),
            (Action::Accept, vec!["ctrl+m".to_string()]),
            (Action::Cancel, vec!["q".to_string()]),
        ]);
        assert!(warnings.is_empty());
        let hints = FooterHints::from_keymap(&keymap);

        let mut app = sample_app();
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal
            .draw(|frame| draw(frame, &mut app, &hints))
            .unwrap();
        let screen = screen(&terminal);

        assert!(screen.contains("C-k/C-j move"), "screen:\n{screen}");
        assert!(screen.contains("tab expand"), "screen:\n{screen}");
        assert!(screen.contains("q cancel"), "screen:\n{screen}");
        assert!(
            !screen.contains("collapse"),
            "unbound actions stay out of the footer:\n{screen}"
        );
    }

    #[test]
    fn empty_tree_shows_placeholder() {
        let tree = Tree::build(vec![], vec![], vec![], InitialExpansion::All);
        let mut app = App::new(tree, EnterOnBranch::Jump);
        let terminal = render(80, 24, &mut app);
        assert!(
            screen(&terminal).contains("No workspaces found."),
            "screen:\n{}",
            screen(&terminal)
        );
    }

    #[test]
    fn narrow_terminal_truncates_without_panicking() {
        let mut app = sample_app();
        let terminal = render(20, 6, &mut app);
        assert!(!screen(&terminal).is_empty());
    }

    #[test]
    fn cursor_far_down_stays_visible_and_viewport_height_is_recorded() {
        let panes: Vec<PaneInfo> = (1..=25)
            .map(|n| pane(&format!("w1:p{n}"), "w1:t1", "w1", n == 20, None))
            .collect();
        let tree = Tree::build(
            vec![workspace("w1", 1, "mothership", true)],
            vec![tab("w1:t1", "w1", 1, "main", true)],
            panes,
            InitialExpansion::All,
        );
        let mut app = App::new(tree, EnterOnBranch::Jump); // cursor on pane 20
        let terminal = render(80, 12, &mut app);
        let screen = screen(&terminal);

        assert!(
            screen.contains("pane 20"),
            "row under cursor must be scrolled into view:\n{screen}"
        );
        // 12 rows minus top/bottom border (2) and footer (2) -> 8.
        assert_eq!(app.viewport_height, 8);
    }
}
