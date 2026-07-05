//! ratatui rendering: bordered tree list with status icons, reverse-video
//! cursor row, detail panel, and a footer hint line built from the *actual*
//! keymap so it never lies about bindings.

use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};
use ratatui::Frame;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::app::{App, Mode};
use crate::config::DisplayConfig;
use crate::herdr_client::AgentStatus;
use crate::icons::IconSet;
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
            (Action::SearchStart, "search"),
            (Action::Accept, "accept"),
            (Action::Cancel, "cancel"),
        ] {
            if let Some(key) = keymap.first_binding_label(action) {
                entries.push((key, label.to_string()));
            }
        }
        FooterHints { entries }
    }

    /// Keys pop (bold), action words recede (dim): the keys are what the
    /// eye is hunting for in a hint line.
    fn line(&self, view: &ViewOptions) -> Line<'static> {
        let mut spans = Vec::new();
        for (i, (key, action)) in self.entries.iter().enumerate() {
            if i > 0 {
                spans.push(Span::raw("   "));
            }
            spans.push(Span::styled(
                key.clone(),
                Style::new().add_modifier(Modifier::BOLD),
            ));
            spans.push(Span::styled(format!(" {action}"), dim_style(view)));
        }
        Line::from(spans)
    }
}

/// Everything the renderer needs from `[display]` config plus environment.
#[derive(Debug, Clone)]
pub struct ViewOptions {
    /// `None` hides status icons entirely (`show_agent_status = false`).
    pub icon_set: Option<IconSet>,
    pub show_pane_count: bool,
    pub show_cwd: bool,
    /// False under NO_COLOR: keep the layout, drop the colors.
    pub color: bool,
    /// For `~`-shortening cwd values.
    pub home: Option<String>,
}

impl ViewOptions {
    pub fn from_config(
        display: &DisplayConfig,
        no_color: bool,
        home: Option<String>,
    ) -> (ViewOptions, Vec<String>) {
        let mut warnings = Vec::new();
        let icon_set = if display.show_agent_status {
            Some(IconSet::parse(&display.icon_set).unwrap_or_else(|| {
                warnings.push(format!(
                    "unknown icon_set {:?}; using \"nerd\"",
                    display.icon_set
                ));
                IconSet::Nerd
            }))
        } else {
            None
        };
        (
            ViewOptions {
                icon_set,
                show_pane_count: display.show_pane_count,
                show_cwd: display.show_cwd,
                color: !no_color,
                home,
            },
            warnings,
        )
    }
}

/// The detail panel needs at least this much total width to be worth
/// splitting off; below it the list gets the whole canvas.
const DETAIL_MIN_TOTAL_WIDTH: u16 = 60;

pub fn draw(frame: &mut Frame, app: &mut App, hints: &FooterHints, view: &ViewOptions) {
    // No frame of our own: herdr already draws pane chrome (border + the
    // manifest pane title) around this canvas, and a second box inside it
    // just wastes two rows and two columns. Accent color is reserved for
    // the internal separators.
    let border_style = if view.color {
        Style::new().fg(Color::Cyan)
    } else {
        Style::new()
    };
    let inner = frame.area();

    // The search prompt line exists while the prompt is focused or a filter
    // is still applied, so the user can always see why rows are missing.
    let show_search = app.mode == Mode::Search || !app.query.is_empty();
    let (search_area, main_area, footer_area) = if show_search {
        let [search, main, footer] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(2),
        ])
        .areas(inner);
        (Some(search), main, footer)
    } else {
        let [main, footer] =
            Layout::vertical([Constraint::Min(1), Constraint::Length(2)]).areas(inner);
        (None, main, footer)
    };

    if let Some(search_area) = search_area {
        // The trailing bar marks the prompt as focused (typing goes here);
        // the query is the content, the "/" just furniture.
        let focused = app.mode == Mode::Search;
        let query_style = if focused {
            Style::new().add_modifier(Modifier::BOLD)
        } else {
            Style::new().add_modifier(Modifier::DIM)
        };
        let mut spans = vec![
            Span::styled(" / ", dim_style(view)),
            Span::styled(app.query.clone(), query_style),
        ];
        if focused {
            spans.push(Span::raw("▏"));
        }
        frame.render_widget(Paragraph::new(Line::from(spans)), search_area);
    }

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
        let placeholder = if app.query.is_empty() {
            "No workspaces found."
        } else {
            "No matches."
        };
        frame.render_widget(Paragraph::new(placeholder), list_area);
    } else {
        let width = list_area.width as usize;
        let items: Vec<ListItem> = app
            .rows()
            .iter()
            .map(|row| ListItem::new(row_line(row, width, view)))
            .collect();
        let list = List::new(items).highlight_style(Style::new().add_modifier(Modifier::REVERSED));
        let mut state = ListState::default().with_selected(Some(app.cursor));
        frame.render_stateful_widget(list, list_area, &mut state);
    }

    if let Some(detail_area) = detail_area {
        render_detail(frame, app, detail_area, view);
    }

    let footer = Paragraph::new(hints.line(view)).block(
        Block::new()
            .borders(Borders::TOP)
            .border_style(border_style),
    );
    frame.render_widget(footer, footer_area);
}

fn status_color(status: AgentStatus) -> Option<Color> {
    match status {
        AgentStatus::Idle => None,
        AgentStatus::Working => Some(Color::Green),
        AgentStatus::Blocked => Some(Color::Red),
        AgentStatus::Done => Some(Color::Blue),
        AgentStatus::Unknown => Some(Color::DarkGray),
    }
}

fn dim_style(view: &ViewOptions) -> Style {
    if view.color {
        Style::new().fg(Color::DarkGray)
    } else {
        Style::new()
    }
}

/// Right-hand panel describing the row under the cursor.
fn render_detail(frame: &mut Frame, app: &App, area: ratatui::layout::Rect, view: &ViewOptions) {
    let border_style = if view.color {
        Style::new().fg(Color::Cyan)
    } else {
        Style::new()
    };
    let block = Block::new()
        .borders(Borders::LEFT)
        .border_style(border_style);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let Some(row) = app.rows().get(app.cursor) else {
        return;
    };
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
            shorten_home(value, view.home.as_deref())
        } else {
            value.clone()
        };
        lines.push(Line::from(vec![
            Span::styled(format!(" {key:<key_width$}  "), dim_style(view)),
            Span::raw(value),
        ]));
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

/// One row: current marker, indentation, expansion glyph, status icon, and
/// label on the left; pane count (branches) or agent name plus optional cwd
/// (panes) right-aligned and dimmed. Drops the right column on narrow
/// terminals rather than wrapping.
fn row_line(row: &Row, width: usize, view: &ViewOptions) -> Line<'static> {
    let marker = if row.is_current { "→" } else { " " };
    let marker_style = if row.is_current && view.color {
        Style::new().fg(Color::Cyan)
    } else {
        Style::new()
    };
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

    let mut spans = vec![
        Span::styled(marker.to_string(), marker_style),
        Span::raw(format!(" {indent}{glyph}")),
    ];
    if let Some(set) = view.icon_set {
        let icon = set.icon(row.agent_status);
        let style = match status_color(row.agent_status) {
            Some(color) if view.color => Style::new().fg(color),
            _ => Style::new(),
        };
        spans.push(Span::styled(format!("{icon} "), style));
    }
    // Bold workspaces anchor the hierarchy visually.
    let label_style = if row.kind == RowKind::Workspace {
        Style::new().add_modifier(Modifier::BOLD)
    } else {
        Style::new()
    };
    spans.push(Span::styled(row.label.clone(), label_style));

    let right = right_column(row, view);
    let left_width: usize = spans
        .iter()
        .map(|span| UnicodeWidthStr::width(span.content.as_ref()))
        .sum();
    let right_width = UnicodeWidthStr::width(right.as_str());

    if !right.is_empty() && left_width + right_width + 2 <= width {
        let padding = width - left_width - right_width - 1;
        spans.push(Span::raw(" ".repeat(padding)));
        spans.push(Span::styled(format!("{right} "), dim_style(view)));
        return Line::from(spans);
    }
    if left_width <= width {
        return Line::from(spans);
    }
    // Overflow: flatten and cut on a column boundary (styles are a luxury
    // a 20-column terminal can live without).
    let text: String = spans.iter().map(|span| span.content.as_ref()).collect();
    Line::raw(truncate_to_width(&text, width))
}

fn right_column(row: &Row, view: &ViewOptions) -> String {
    if row.kind == RowKind::Pane {
        let agent = row.agent.as_deref().unwrap_or("shell");
        match (&row.cwd, view.show_cwd) {
            (Some(cwd), true) => {
                format!("{agent}  {}", shorten_home(cwd, view.home.as_deref()))
            }
            _ => agent.to_string(),
        }
    } else if view.show_pane_count {
        let panes = if row.pane_count == 1 { "pane" } else { "panes" };
        format!("{} {panes}", row.pane_count)
    } else {
        String::new()
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
    use crate::herdr_client::{PaneInfo, TabInfo, WorkspaceInfo};
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
            cwd: Some("/home/u/src/repo".to_string()),
            label: None,
            title: None,
            terminal_id: format!("term_{id}"),
        }
    }

    fn sample_app() -> App {
        // Two tabs so the tab level actually renders (single-tab
        // workspaces skip it, like the built-in goto).
        let tree = Tree::build(
            vec![workspace("w1", 1, "mothership", true)],
            vec![
                tab("w1:t1", "w1", 1, "main", true),
                tab("w1:t2", "w1", 2, "logs", false),
            ],
            vec![
                pane("w1:p1", "w1:t1", "w1", true, Some("claude")),
                pane("w1:p2", "w1:t1", "w1", false, None),
                pane("w1:p3", "w1:t2", "w1", false, None),
            ],
            InitialExpansion::All,
        );
        App::new(tree, EnterOnBranch::Jump)
    }

    #[test]
    fn single_tab_workspace_renders_panes_directly_under_it() {
        let tree = Tree::build(
            vec![workspace("w1", 1, "mothership", true)],
            vec![tab("w1:t1", "w1", 1, "main", true)],
            vec![
                pane("w1:p1", "w1:t1", "w1", true, Some("claude")),
                pane("w1:p2", "w1:t1", "w1", false, None),
            ],
            InitialExpansion::All,
        );
        let mut app = App::new(tree, EnterOnBranch::Jump);
        let terminal = render(80, 24, &mut app);
        let screen = screen(&terminal);

        assert!(!screen.contains("main"), "no tab row:\n{screen}");
        assert!(
            screen.contains("  ○ claude"),
            "panes at depth 1, right under the workspace:\n{screen}"
        );
    }

    fn default_hints() -> FooterHints {
        let (keymap, _) = Keymap::from_bindings(&KeysConfig::default().to_bindings());
        FooterHints::from_keymap(&keymap)
    }

    /// Colorless nerd icons: stable text assertions.
    fn plain_view() -> ViewOptions {
        ViewOptions {
            icon_set: Some(IconSet::Nerd),
            show_pane_count: true,
            show_cwd: false,
            color: false,
            home: Some("/home/u".to_string()),
        }
    }

    fn render_with(
        width: u16,
        height: u16,
        app: &mut App,
        view: &ViewOptions,
    ) -> Terminal<TestBackend> {
        let hints = default_hints();
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| draw(frame, app, &hints, view))
            .unwrap();
        terminal
    }

    fn render(width: u16, height: u16, app: &mut App) -> Terminal<TestBackend> {
        render_with(width, height, app, &plain_view())
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
    fn tree_rows_show_glyphs_icons_indentation_and_right_columns() {
        let mut app = sample_app();
        let terminal = render(80, 24, &mut app);
        let screen = screen(&terminal);

        // ws=unknown "·", tab=working "●", panes=idle "○".
        assert!(screen.contains("▼ · mothership"), "screen:\n{screen}");
        assert!(screen.contains("  ▼ ● main"), "indented tab:\n{screen}");
        assert!(screen.contains("    ○ claude"), "indented pane:\n{screen}");
        assert!(screen.contains("2 panes"), "tab pane count:\n{screen}");
        let lines = buffer_lines(&terminal);
        let pane2 = lines.iter().find(|l| l.contains("pane 2")).unwrap();
        assert!(pane2.contains("shell"), "agentless column: {pane2:?}");
    }

    #[test]
    fn ascii_icon_set_renders_ascii() {
        let mut app = sample_app();
        let view = ViewOptions {
            icon_set: Some(IconSet::Ascii),
            ..plain_view()
        };
        let terminal = render_with(80, 24, &mut app, &view);
        let screen = screen(&terminal);
        assert!(screen.contains("▼ - mothership"), "screen:\n{screen}");
        assert!(screen.contains("▼ + main"), "screen:\n{screen}");
        assert!(screen.contains("o claude"), "screen:\n{screen}");
    }

    #[test]
    fn display_toggles_hide_icons_and_pane_counts() {
        let mut app = sample_app();
        let view = ViewOptions {
            icon_set: None,
            show_pane_count: false,
            ..plain_view()
        };
        let terminal = render_with(80, 24, &mut app, &view);
        let screen = screen(&terminal);
        assert!(screen.contains("▼ mothership"), "no icon:\n{screen}");
        assert!(!screen.contains("panes"), "no pane counts:\n{screen}");
    }

    #[test]
    fn show_cwd_appends_the_shortened_path_to_pane_rows() {
        let mut app = sample_app();
        let view = ViewOptions {
            show_cwd: true,
            ..plain_view()
        };
        let terminal = render_with(100, 24, &mut app, &view);
        let lines = buffer_lines(&terminal);
        let pane2 = lines.iter().find(|l| l.contains("pane 2")).unwrap();
        assert!(
            pane2.contains("shell  ~/src/repo"),
            "pane row with cwd: {pane2:?}"
        );
    }

    #[test]
    fn working_status_icon_is_green_unless_no_color() {
        let mut app = sample_app();
        let colored = ViewOptions {
            color: true,
            ..plain_view()
        };
        let terminal = render_with(80, 24, &mut app, &colored);
        let lines = buffer_lines(&terminal);
        let y = lines.iter().position(|l| l.contains("● main")).unwrap() as u16;
        let x = lines[y as usize].chars().position(|c| c == '●').unwrap() as u16;
        let style = terminal.backend().buffer().cell((x, y)).unwrap().style();
        assert_eq!(style.fg, Some(Color::Green), "colored icon");

        let terminal = render_with(80, 24, &mut app, &plain_view());
        let lines = buffer_lines(&terminal);
        let y = lines.iter().position(|l| l.contains("● main")).unwrap() as u16;
        let x = lines[y as usize].chars().position(|c| c == '●').unwrap() as u16;
        let style = terminal.backend().buffer().cell((x, y)).unwrap().style();
        assert_ne!(style.fg, Some(Color::Green), "NO_COLOR keeps default fg");
    }

    #[test]
    fn view_options_from_config_warns_on_unknown_icon_set() {
        let mut display = DisplayConfig {
            icon_set: "comic-sans".to_string(),
            ..Default::default()
        };
        let (view, warnings) = ViewOptions::from_config(&display, false, None);
        assert_eq!(view.icon_set, Some(IconSet::Nerd));
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("comic-sans"));

        display.show_agent_status = false;
        let (view, warnings) = ViewOptions::from_config(&display, true, None);
        assert_eq!(view.icon_set, None, "hidden icons skip validation");
        assert!(warnings.is_empty());
        assert!(!view.color, "NO_COLOR disables color");
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
        assert!(
            screen.contains("~/src/repo"),
            "cwd shortened via view.home:\n{screen}"
        );
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
    fn cursor_row_is_reversed() {
        let mut app = sample_app(); // cursor starts on the focused pane row
        let terminal = render(80, 24, &mut app);

        let buffer = terminal.backend().buffer();
        let lines = buffer_lines(&terminal);
        // "○ claude" (indented) is the list row; the detail panel header
        // also says "pane 1" but without the icon.
        let cursor_y = lines
            .iter()
            .position(|line| line.contains("○ claude"))
            .expect("cursor row must be on screen") as u16;
        let x = lines[cursor_y as usize]
            .chars()
            .position(|c| c == 'c')
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

        let current = lines.iter().find(|l| l.contains("○ claude")).unwrap();
        assert!(current.contains("→"), "current row: {current:?}");
        let other = lines.iter().find(|l| l.contains("○ pane 2")).unwrap();
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
            .draw(|frame| draw(frame, &mut app, &hints, &plain_view()))
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
    fn footer_includes_the_search_hint() {
        let mut app = sample_app();
        let terminal = render(80, 24, &mut app);
        assert!(
            screen(&terminal).contains("/ search"),
            "screen:\n{}",
            screen(&terminal)
        );
    }

    #[test]
    fn search_prompt_shows_query_and_focus_state() {
        let mut app = sample_app();
        app.mode = Mode::Search;
        app.query = "two".to_string();
        let terminal = render(80, 24, &mut app);
        assert!(
            screen(&terminal).contains("/ two▏"),
            "focused prompt with cursor bar:\n{}",
            screen(&terminal)
        );

        app.mode = Mode::Normal;
        let terminal = render(80, 24, &mut app);
        let screen = screen(&terminal);
        assert!(
            screen.contains("/ two"),
            "kept filter stays visible:\n{screen}"
        );
        assert!(
            !screen.contains("▏"),
            "no cursor bar when unfocused:\n{screen}"
        );
    }

    #[test]
    fn no_search_prompt_without_a_query() {
        let mut app = sample_app();
        let terminal = render(80, 24, &mut app);
        let lines = buffer_lines(&terminal);
        // The first line is the list, not a prompt (the footer's
        // "/ search" hint is elsewhere).
        assert!(
            lines[0].contains("mothership"),
            "first line: {:?}",
            lines[0]
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
    fn empty_filter_result_says_no_matches() {
        use crate::keymap::{parse_key_spec, Keymaps};
        let keys = KeysConfig::default();
        let (normal, _) = Keymap::from_bindings(&keys.to_bindings());
        let (search, _) = Keymap::from_bindings(&keys.to_search_bindings());
        let keymaps = Keymaps { normal, search };

        let mut app = sample_app();
        for spec in ["/", "z", "z"] {
            let key = parse_key_spec(spec).unwrap().0[0];
            app.handle_key(&keymaps, key);
        }
        let terminal = render(80, 24, &mut app);
        let screen = screen(&terminal);
        assert!(screen.contains("No matches."), "screen:\n{screen}");
        assert!(
            !screen.contains("No workspaces found."),
            "empty filter is not an empty session:\n{screen}"
        );
    }

    #[test]
    fn narrow_terminal_truncates_without_panicking() {
        let mut app = sample_app();
        let terminal = render(20, 6, &mut app);
        assert!(!screen(&terminal).is_empty());
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
        let jp_body = jp.trim_end_matches([' ', '│']);
        let ascii_body = ascii.trim_end_matches([' ', '│']);
        assert!(jp_body.ends_with("0 panes"), "jp row: {jp:?}");
        assert!(ascii_body.ends_with("0 panes"), "ascii row: {ascii:?}");
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
        // 12 rows minus the footer (2) -> 10; herdr owns the pane border.
        assert_eq!(app.viewport_height, 10);
    }

    #[test]
    fn no_own_frame_but_separators_carry_accent_and_workspaces_bold() {
        let mut app = sample_app();
        let colored = ViewOptions {
            color: true,
            ..plain_view()
        };
        let terminal = render_with(80, 24, &mut app, &colored);
        let buffer = terminal.backend().buffer();
        let lines = buffer_lines(&terminal);

        // herdr draws the pane chrome; our canvas starts with content.
        assert!(
            lines[0].contains("mothership"),
            "no own border, first line is the list: {:?}",
            lines[0]
        );

        // The footer's top separator keeps the accent color.
        let sep_y = lines.iter().position(|l| l.starts_with('─')).unwrap() as u16;
        assert_eq!(
            buffer.cell((0, sep_y)).unwrap().style().fg,
            Some(Color::Cyan),
            "footer separator carries the accent color"
        );

        let y = lines
            .iter()
            .position(|l| l.contains("· mothership"))
            .unwrap() as u16;
        let x = lines[y as usize].chars().position(|c| c == 'm').unwrap() as u16;
        assert!(
            buffer
                .cell((x, y))
                .unwrap()
                .style()
                .add_modifier
                .contains(Modifier::BOLD),
            "workspace labels are bold"
        );

        // NO_COLOR: no cyan separator, structure modifiers stay.
        let terminal = render_with(80, 24, &mut app, &plain_view());
        assert_ne!(
            terminal
                .backend()
                .buffer()
                .cell((0, sep_y))
                .unwrap()
                .style()
                .fg,
            Some(Color::Cyan)
        );
    }

    #[test]
    fn footer_keys_are_bold_and_labels_dim() {
        let mut app = sample_app();
        let terminal = render(80, 24, &mut app);
        let buffer = terminal.backend().buffer();
        let lines = buffer_lines(&terminal);
        let y = lines
            .iter()
            .position(|l| l.contains("enter accept"))
            .unwrap() as u16;
        // Column index in chars, not bytes: "↑/↓" earlier in the line is
        // multi-byte.
        let key_x = lines[y as usize]
            .split("enter")
            .next()
            .unwrap()
            .chars()
            .count() as u16;
        let style = buffer.cell((key_x, y)).unwrap().style();
        assert!(
            style.add_modifier.contains(Modifier::BOLD),
            "hint keys are bold: {style:?}"
        );
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
}
