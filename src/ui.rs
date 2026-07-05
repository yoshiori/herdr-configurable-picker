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
        ] {
            if let Some(key) = keymap.first_binding_label(action) {
                entries.push((key, label.to_string()));
            }
        }
        // The state filters collapse into one "b/w/i/d/a states" hint.
        let filter_keys: Vec<String> = [
            Action::FilterBlocked,
            Action::FilterWorking,
            Action::FilterIdle,
            Action::FilterDone,
            Action::FilterClear,
        ]
        .into_iter()
        .filter_map(|action| keymap.first_binding_label(action))
        .collect();
        if !filter_keys.is_empty() {
            entries.push((filter_keys.join("/"), "states".to_string()));
        }
        for (action, label) in [(Action::Accept, "accept"), (Action::Cancel, "cancel")] {
            if let Some(key) = keymap.first_binding_label(action) {
                entries.push((key, label.to_string()));
            }
        }
        FooterHints { entries }
    }

    /// Keys pop (bold), action words recede (dim): the keys are what the
    /// eye is hunting for in a hint line. Borrows from `self` — the line
    /// is consumed within the same draw call.
    fn line<'a>(&'a self, view: &ViewOptions) -> Line<'a> {
        let mut spans = Vec::new();
        for (i, (key, action)) in self.entries.iter().enumerate() {
            if i > 0 {
                spans.push(Span::raw("   "));
            }
            spans.push(Span::styled(
                key.as_str(),
                Style::new().add_modifier(Modifier::BOLD),
            ));
            spans.push(Span::raw(" "));
            spans.push(Span::styled(action.as_str(), dim_style(view)));
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
    /// Agent/shell icon in front of the meta column and the detail
    /// panel's agent line; `None` when `show_agent_icon = false`.
    /// Deliberately separate from `icon_set` so `show_agent_status` and
    /// `show_agent_icon` toggle independently.
    pub agent_icon_set: Option<IconSet>,
    pub show_cwd: bool,
    /// False under NO_COLOR: keep the layout, drop the colors.
    pub color: bool,
    /// For `~`-shortening cwd values.
    pub home: Option<String>,
    /// Cursor-row background, current markers, separators. Configurable so
    /// it can match the herdr theme's accent.
    pub accent: Color,
}

impl ViewOptions {
    pub fn from_config(
        display: &DisplayConfig,
        no_color: bool,
        home: Option<String>,
        host_accent: Option<Color>,
    ) -> (ViewOptions, Vec<String>) {
        let mut warnings = Vec::new();
        let parsed_set = IconSet::parse(&display.icon_set).unwrap_or_else(|| {
            warnings.push(format!(
                "unknown icon_set {:?}; using \"nerd\"",
                display.icon_set
            ));
            IconSet::Nerd
        });
        // Independent toggles over the one configured set: hiding status
        // icons must not take the agent icons down with it (review
        // feedback on the coupled first version).
        let icon_set = display.show_agent_status.then_some(parsed_set);
        let agent_icon_set = display.show_agent_icon.then_some(parsed_set);
        // "auto" follows the herdr theme's accent (resolved from the host
        // config by the caller); an explicit color always wins.
        let accent = if display.accent == "auto" {
            host_accent.unwrap_or(Color::Cyan)
        } else {
            parse_color(&display.accent).unwrap_or_else(|| {
                warnings.push(format!(
                    "unknown accent {:?}; using \"cyan\"",
                    display.accent
                ));
                Color::Cyan
            })
        };
        (
            ViewOptions {
                icon_set,
                show_pane_count: display.show_pane_count,
                agent_icon_set,
                show_cwd: display.show_cwd,
                color: !no_color,
                home,
                accent,
            },
            warnings,
        )
    }
}

/// Named ANSI colors and `#rrggbb` hex.
pub(crate) fn parse_color(text: &str) -> Option<Color> {
    let lower = text.trim().to_lowercase();
    if let Some(hex) = lower.strip_prefix('#') {
        // is_ascii guards the byte slicing below: "#あい" is also 6 bytes,
        // and slicing it mid-character would panic.
        if hex.len() == 6 && hex.is_ascii() {
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            return Some(Color::Rgb(r, g, b));
        }
        return None;
    }
    match lower.as_str() {
        "black" => Some(Color::Black),
        "red" => Some(Color::Red),
        "green" => Some(Color::Green),
        "yellow" => Some(Color::Yellow),
        "blue" => Some(Color::Blue),
        "magenta" | "purple" => Some(Color::Magenta),
        "cyan" => Some(Color::Cyan),
        "white" => Some(Color::White),
        "gray" | "grey" => Some(Color::Gray),
        "darkgray" | "darkgrey" => Some(Color::DarkGray),
        "lightred" => Some(Color::LightRed),
        "lightgreen" => Some(Color::LightGreen),
        "lightyellow" => Some(Color::LightYellow),
        "lightblue" => Some(Color::LightBlue),
        "lightmagenta" => Some(Color::LightMagenta),
        "lightcyan" => Some(Color::LightCyan),
        _ => None,
    }
}

/// Black or white, whichever reads better on `background` — the built-in's
/// panel_contrast_fg idea.
fn contrast_fg(background: Color) -> Color {
    match background {
        Color::Rgb(r, g, b) => {
            // Standard luma approximation.
            let luma = 0.299 * r as f32 + 0.587 * g as f32 + 0.114 * b as f32;
            if luma > 140.0 {
                Color::Black
            } else {
                Color::White
            }
        }
        Color::Yellow
        | Color::Green
        | Color::Cyan
        | Color::White
        | Color::Gray
        | Color::LightRed
        | Color::LightGreen
        | Color::LightYellow
        | Color::LightBlue
        | Color::LightMagenta
        | Color::LightCyan => Color::Black,
        _ => Color::White,
    }
}

/// The detail panel needs at least this much total width to be worth
/// splitting off; below it the list gets the whole canvas.
const DETAIL_MIN_TOTAL_WIDTH: u16 = 60;

/// Room for the right-aligned "N panes " header counter.
const COUNT_WIDTH: u16 = 12;

pub fn draw(frame: &mut Frame, app: &mut App, hints: &FooterHints, view: &ViewOptions) {
    // No frame of our own: herdr already draws pane chrome (border + the
    // manifest pane title) around this canvas, and a second box inside it
    // just wastes two rows and two columns. Accent color is reserved for
    // the internal separators.
    let border_style = if view.color {
        Style::new().fg(view.accent)
    } else {
        Style::new()
    };
    let inner = frame.area();

    // The header line is always there, like the built-in's: the search
    // prompt (a dim placeholder until it is used), or the active state
    // filter, with the row count at the right edge — separated from the
    // tree by a rule, mirroring the footer's.
    let [header_area, main_area, footer_area] = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(1),
        Constraint::Length(2),
    ])
    .areas(inner);

    {
        let header_block = Block::new()
            .borders(Borders::BOTTOM)
            .border_style(border_style);
        let header_inner = header_block.inner(header_area);
        frame.render_widget(header_block, header_area);
        // A click on this line focuses the search, like the built-in.
        app.prompt_row = header_inner.y;
        let [prompt_area, count_area] =
            Layout::horizontal([Constraint::Min(1), Constraint::Length(COUNT_WIDTH)])
                .areas(header_inner);
        if let Some(status) = app.state_filter {
            // Active state filter (b/w/i/d): "/ {icon} {name}" — the
            // built-in's chip, with the state's own icon (the spinner for
            // working) in the state color.
            let style = match status_color(status) {
                Some(color) if view.color => Style::new().fg(color),
                _ => Style::new(),
            };
            let mut spans = vec![Span::styled(" / ", dim_style(view))];
            if let Some(set) = &view.icon_set {
                spans.push(Span::styled(
                    set.icon(status, app.tick),
                    style.add_modifier(Modifier::BOLD),
                ));
                spans.push(Span::raw(" "));
            }
            spans.push(Span::styled(
                status.name(),
                style.add_modifier(Modifier::BOLD),
            ));
            frame.render_widget(Paragraph::new(Line::from(spans)), prompt_area);
        } else {
            // The trailing bar marks the prompt as focused (typing goes
            // here); the query is the content, the "/" just furniture.
            let focused = app.mode == Mode::Search;
            let query_style = if focused {
                Style::new().add_modifier(Modifier::BOLD)
            } else {
                Style::new().add_modifier(Modifier::DIM)
            };
            let mut spans = vec![
                Span::styled(" / ", dim_style(view)),
                Span::styled(app.query.as_str(), query_style),
            ];
            if focused {
                spans.push(Span::raw("▏"));
            } else if app.query.is_empty() {
                // Idle placeholder, like the built-in's "/ search panes".
                spans.push(Span::styled("search panes", dim_style(view)));
            }
            frame.render_widget(Paragraph::new(Line::from(spans)), prompt_area);
        }
        // The built-in shows the total pane count here, not the number of
        // visible rows — it stays put while filters narrow the list.
        let count = Paragraph::new(format!("{} panes ", app.pane_count()))
            .style(dim_style(view))
            .alignment(ratatui::layout::Alignment::Right);
        frame.render_widget(count, count_area);
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
    app.list_rect = (list_area.x, list_area.y, list_area.width, list_area.height);

    if app.rows().is_empty() {
        let placeholder = if app.query.is_empty() && app.state_filter.is_none() {
            "No workspaces found."
        } else {
            "No matches."
        };
        frame.render_widget(Paragraph::new(placeholder), list_area);
    } else {
        let width = list_area.width as usize;
        let tick = app.tick;
        let items: Vec<ListItem> = app
            .rows()
            .iter()
            .map(|row| ListItem::new(row_line(row, width, view, tick)))
            .collect();
        // A solid accent bar, like the built-in's selected row. REVERSED
        // would invert each cell's own color, leaving odd patches over the
        // status icons; overriding fg AND bg keeps the row uniform.
        let highlight = if view.color {
            Style::new().bg(view.accent).fg(contrast_fg(view.accent))
        } else {
            Style::new().add_modifier(Modifier::REVERSED)
        };
        let list = List::new(items).highlight_style(highlight);
        let mut state = ListState::default().with_selected(Some(app.cursor));
        frame.render_stateful_widget(list, list_area, &mut state);
        // The offset ratatui actually used — mouse hit-testing needs it.
        app.list_offset = state.offset();
        render_scrollbar(frame, list_area, app.rows().len(), app.list_offset, view);
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

/// Status colors matching the built-in's agent_icon: blocked red, working
/// yellow, done teal(ish), idle green, unknown muted.
fn status_color(status: AgentStatus) -> Option<Color> {
    match status {
        AgentStatus::Idle => Some(Color::Green),
        AgentStatus::Working => Some(Color::Yellow),
        AgentStatus::Blocked => Some(Color::Red),
        AgentStatus::Done => Some(Color::Cyan),
        AgentStatus::Unknown => Some(Color::DarkGray),
    }
}

/// Thumb geometry for a proportional scrollbar: `(top, len)` cells within
/// a `track`-tall track, or None when everything already fits.
fn scrollbar_thumb(
    total: usize,
    viewport: usize,
    offset: usize,
    track: usize,
) -> Option<(usize, usize)> {
    if total <= viewport || track == 0 {
        return None;
    }
    let len = (((viewport * track) as f32 / total as f32).round() as usize).clamp(1, track);
    let max_top = track - len;
    let max_offset = total - viewport;
    let top = (((offset.min(max_offset) * max_top) as f32 / max_offset as f32).round() as usize)
        .min(max_top);
    Some((top, len))
}

/// The built-in navigator's scrollbar: a `▕` column overdrawn on the
/// list's right edge, dim track with the thumb standing out. Only appears
/// when the rows overflow the viewport.
fn render_scrollbar(
    frame: &mut Frame,
    area: ratatui::layout::Rect,
    total: usize,
    offset: usize,
    view: &ViewOptions,
) {
    if area.width == 0 {
        return;
    }
    let Some((top, len)) =
        scrollbar_thumb(total, area.height as usize, offset, area.height as usize)
    else {
        return;
    };
    let thumb = if view.color {
        Style::new().fg(view.accent)
    } else {
        Style::new().add_modifier(Modifier::BOLD)
    };
    let x = area.x + area.width - 1;
    let buf = frame.buffer_mut();
    for (i, y) in (area.y..area.y + area.height).enumerate() {
        let cell = &mut buf[(x, y)];
        cell.set_symbol("▕");
        // Patch fg only: the selected row's background continues under
        // the bar, exactly like the built-in's.
        cell.set_style(if i >= top && i < top + len {
            thumb
        } else {
            dim_style(view)
        });
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
        Style::new().fg(view.accent)
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
    // Columns available for a value: panel minus the leading space, the
    // key column, and the two-space gap.
    let value_width = (inner.width as usize)
        .saturating_sub(1 + key_width + 2)
        .max(1);

    let mut lines = vec![
        Line::styled(
            format!(
                " {}",
                middle_elide(&row.title, (inner.width as usize).saturating_sub(1))
            ),
            Style::new().add_modifier(Modifier::BOLD),
        ),
        Line::raw(""),
    ];
    for (key, value) in &row.detail {
        let key_span = Span::styled(format!(" {key:<key_width$}  "), dim_style(view));
        // The agent line carries the same agent/shell icon as the meta
        // column in the tree.
        if *key == "agent" {
            if let Some(icon) = agent_icon(row, view) {
                // Emoji icons are two columns wide; measure instead of
                // assuming one.
                let budget = value_width
                    .saturating_sub(UnicodeWidthStr::width(icon) + 1)
                    .max(1);
                lines.push(Line::from(vec![
                    key_span,
                    Span::raw(format!("{icon} ")),
                    Span::raw(middle_elide(value, budget)),
                ]));
                continue;
            }
        }
        // The status line mirrors the tree rows: the status icon (the
        // animated spinner while working) and the status color.
        if *key == "status" {
            let style = match status_color(row.agent_status) {
                Some(color) if view.color => Style::new().fg(color),
                _ => Style::new(),
            };
            let mut spans = vec![key_span];
            let mut budget = value_width;
            if let Some(set) = &view.icon_set {
                let icon = set.icon(row.agent_status, app.tick);
                // Emoji status icons are two columns wide; measure
                // instead of assuming one.
                budget = budget
                    .saturating_sub(UnicodeWidthStr::width(icon) + 1)
                    .max(1);
                spans.push(Span::styled(icon, style));
                spans.push(Span::raw(" "));
            }
            spans.push(Span::styled(middle_elide(value, budget), style));
            lines.push(Line::from(spans));
            continue;
        }
        // Long values used to clip at the panel edge mid-path; elide them
        // instead. Paths keep their trailing segments (the part you scan
        // for), everything else keeps head and tail.
        let value = if *key == "cwd" {
            elide_path(&shorten_home(value, view.home.as_deref()), value_width)
        } else {
            middle_elide(value, value_width)
        };
        lines.push(Line::from(vec![key_span, Span::raw(value)]));
    }
    frame.render_widget(Paragraph::new(lines), inner);
}

/// Fits `text` into `max` columns by cutting the middle: "abc…xyz". Width
/// math in terminal columns, not chars.
fn middle_elide(text: &str, max: usize) -> String {
    if UnicodeWidthStr::width(text) <= max {
        return text.to_string();
    }
    if max <= 1 {
        return "…".to_string();
    }
    let head_budget = (max - 1) / 2;
    let tail_budget = max - 1 - head_budget;
    let head: String = truncate_to_width(text, head_budget);
    // Walk the tail backwards until it fits its budget.
    let mut tail_start = text.len();
    let mut tail_cols = 0;
    for (idx, c) in text.char_indices().rev() {
        let cols = UnicodeWidthChar::width(c).unwrap_or(0);
        if tail_cols + cols > tail_budget {
            break;
        }
        tail_cols += cols;
        tail_start = idx;
    }
    format!("{head}…{}", &text[tail_start..])
}

/// Fits a path into `max` columns fish-style: intermediate segments shrink
/// to their first character ("~/src/github.com/yoshiori/foo" ->
/// "~/s/g/y/foo", dot-dirs keep the dot: ".config" -> ".c"), the leaf stays
/// whole. Falls back to [`middle_elide`] when even that is too long.
fn elide_path(path: &str, max: usize) -> String {
    if UnicodeWidthStr::width(path) <= max {
        return path.to_string();
    }
    let segments: Vec<&str> = path.split('/').collect();
    let last = segments.len().saturating_sub(1);
    let shortened: Vec<String> = segments
        .iter()
        .enumerate()
        .map(|(i, segment)| {
            if i == last || *segment == "~" || segment.is_empty() {
                (*segment).to_string()
            } else {
                let keep = if segment.starts_with('.') { 2 } else { 1 };
                segment.chars().take(keep).collect()
            }
        })
        .collect();
    let shortened = shortened.join("/");
    if UnicodeWidthStr::width(shortened.as_str()) <= max {
        shortened
    } else {
        middle_elide(&shortened, max)
    }
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
fn row_line(row: &Row, width: usize, view: &ViewOptions, tick: u32) -> Line<'static> {
    // ◆ marks where you came from (workspace and pane), like the built-in;
    // the cursor itself is the reverse-video row.
    let marker = if row.is_current { "◆" } else { " " };
    let marker_style = if row.is_current && view.color {
        Style::new().fg(view.accent)
    } else {
        Style::new()
    };
    // Tree-command style guides: workspaces open with ▾/▸, children hang
    // off │ / ├── / └── rails. A collapsed branch keeps a ▸ on its rail so
    // hidden children stay discoverable.
    let glyph = if row.kind == RowKind::Workspace {
        if row.expanded { "▾ " } else { "▸ " }.to_string()
    } else {
        let mut rail = String::from("  ");
        for &continues in &row.ancestor_continues {
            rail.push_str(if continues { "│   " } else { "    " });
        }
        let collapsed = row.expandable && !row.expanded;
        rail.push_str(match (row.last_child, collapsed) {
            (false, false) => "├── ",
            (true, false) => "└── ",
            (false, true) => "├─▸ ",
            (true, true) => "└─▸ ",
        });
        rail
    };

    let mut spans = vec![
        Span::styled(marker.to_string(), marker_style),
        Span::raw(" "),
        Span::styled(glyph, dim_style(view)),
    ];
    if let Some(set) = view.icon_set {
        let icon = set.icon(row.agent_status, tick);
        let style = match status_color(row.agent_status) {
            Some(color) if view.color => Style::new().fg(color),
            _ => Style::new(),
        };
        spans.push(Span::styled(format!("{icon} "), style));
    }
    // Bold workspaces anchor the hierarchy visually. Like the built-in,
    // their pane count rides in the label ("picker (3)"); the display
    // suffix stays out of Row.label so search and the detail panel see the
    // clean name.
    let label_style = if row.kind == RowKind::Workspace {
        Style::new().add_modifier(Modifier::BOLD)
    } else {
        Style::new()
    };
    let label_text = if row.kind == RowKind::Workspace && view.show_pane_count {
        format!("{} ({})", row.label, row.pane_count)
    } else {
        row.label.clone()
    };
    spans.push(Span::styled(label_text, label_style));

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

/// The agent (or shell) icon for a pane row, honoring the icon set and the
/// `show_agent_icon` toggle. The same convention as the user's tmux
/// automatic-rename: 󰚩 for AI agents,  for plain shells.
fn agent_icon(row: &Row, view: &ViewOptions) -> Option<&'static str> {
    let set = view.agent_icon_set?;
    if row.agent.is_some() {
        set.agent_icon()
    } else {
        set.shell_icon()
    }
}

fn right_column(row: &Row, view: &ViewOptions) -> String {
    if row.kind == RowKind::Pane {
        // The built-in's pane meta: "{agent} · {status}" for agent panes,
        // bare "shell" otherwise. A custom status wins over the state name.
        let base = match &row.agent {
            Some(agent) => {
                let status = row
                    .custom_status
                    .clone()
                    .unwrap_or_else(|| row.agent_status.name().to_string());
                format!("{agent} · {status}")
            }
            None => "shell".to_string(),
        };
        let base = match agent_icon(row, view) {
            Some(icon) => format!("{icon} {base}"),
            None => base,
        };
        match (&row.cwd, view.show_cwd) {
            (Some(cwd), true) => {
                format!("{base}  {}", shorten_home(cwd, view.home.as_deref()))
            }
            _ => base,
        }
    } else if row.kind == RowKind::Tab {
        // "N panes · 1 blocked · 2 working", either part optional.
        let mut parts = Vec::new();
        if view.show_pane_count {
            let panes = if row.pane_count == 1 { "pane" } else { "panes" };
            parts.push(format!("{} {panes}", row.pane_count));
        }
        if !row.activity.is_empty() {
            parts.push(row.activity.clone());
        }
        parts.join(" · ")
    } else {
        // Workspace counts live in the label suffix, like the built-in;
        // the right column carries the activity summary.
        row.activity.clone()
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
            foreground_cwd: None,
            label: None,
            title: None,
            custom_status: None,
            terminal_id: format!("term_{id}"),
            branch: None,
        }
    }

    fn sample_app() -> App {
        // Two tabs so the tab level actually renders (single-tab
        // workspaces skip it, like the built-in goto).
        let mut ws = workspace("w1", 1, "picker", true);
        ws.pane_count = 3;
        let tree = Tree::build(
            vec![ws],
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
            vec![workspace("w1", 1, "picker", true)],
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
            screen.contains("  ├── ✓ claude"),
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
            agent_icon_set: Some(IconSet::Nerd),
            show_cwd: false,
            color: false,
            home: Some("/home/u".to_string()),
            accent: Color::Cyan,
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
        assert!(
            screen.contains("▾ ○ picker (3)"),
            "workspace pane count rides in the label:\n{screen}"
        );
        assert!(screen.contains("  ├── ⠋ main"), "indented tab:\n{screen}");
        assert!(
            screen.contains("│   ├── ✓ claude"),
            "pane on the rail:\n{screen}"
        );
        assert!(screen.contains("2 panes"), "tab pane count:\n{screen}");
        let ws_row = buffer_lines(&terminal)
            .into_iter()
            .find(|l| l.contains("picker"))
            .unwrap();
        assert!(
            !ws_row.contains("3 panes"),
            "no duplicate count in the workspace right column: {ws_row:?}"
        );
        let lines = buffer_lines(&terminal);
        let agent_row = lines.iter().find(|l| l.contains("✓ claude")).unwrap();
        assert!(
            agent_row.contains("claude · idle"),
            "agent pane meta like the built-in: {agent_row:?}"
        );
        let pane2 = lines.iter().find(|l| l.contains("pane 2")).unwrap();
        assert!(pane2.contains("shell"), "agentless column: {pane2:?}");
        assert!(
            !pane2.contains("· idle"),
            "shell panes carry no status text: {pane2:?}"
        );
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
        assert!(screen.contains("▾ o picker"), "screen:\n{screen}");
        assert!(screen.contains("├── | main"), "screen:\n{screen}");
        assert!(screen.contains("v claude"), "screen:\n{screen}");
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
        assert!(screen.contains("▾ picker"), "no icon:\n{screen}");
        assert!(!screen.contains("2 panes"), "no pane counts:\n{screen}");
        assert!(
            !screen.contains("picker ("),
            "no label suffix either:\n{screen}"
        );
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
    fn working_status_icon_is_yellow_unless_no_color() {
        let mut app = sample_app();
        let colored = ViewOptions {
            color: true,
            ..plain_view()
        };
        let terminal = render_with(80, 24, &mut app, &colored);
        let lines = buffer_lines(&terminal);
        let y = lines.iter().position(|l| l.contains("⠋ main")).unwrap() as u16;
        let x = lines[y as usize].chars().position(|c| c == '⠋').unwrap() as u16;
        let style = terminal.backend().buffer().cell((x, y)).unwrap().style();
        assert_eq!(
            style.fg,
            Some(Color::Yellow),
            "working spinner is yellow, like the built-in"
        );

        let terminal = render_with(80, 24, &mut app, &plain_view());
        let lines = buffer_lines(&terminal);
        let y = lines.iter().position(|l| l.contains("⠋ main")).unwrap() as u16;
        let x = lines[y as usize].chars().position(|c| c == '⠋').unwrap() as u16;
        let style = terminal.backend().buffer().cell((x, y)).unwrap().style();
        assert_ne!(style.fg, Some(Color::Yellow), "NO_COLOR keeps default fg");
    }

    #[test]
    fn view_options_from_config_warns_on_unknown_icon_set() {
        let mut display = DisplayConfig {
            icon_set: "comic-sans".to_string(),
            ..Default::default()
        };
        let (view, warnings) = ViewOptions::from_config(&display, false, None, None);
        assert_eq!(view.icon_set, Some(IconSet::Nerd));
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("comic-sans"));

        display.show_agent_status = false;
        let (view, warnings) = ViewOptions::from_config(&display, true, None, None);
        assert_eq!(view.icon_set, None, "status icons hidden");
        assert_eq!(
            view.agent_icon_set,
            Some(IconSet::Nerd),
            "agent icons keep the (fallback) set: the toggles are independent"
        );
        // The set still applies to agent icons, so the typo still warns.
        assert_eq!(warnings.len(), 1);
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
    fn meta_column_carries_agent_and_shell_icons() {
        let mut app = sample_app();
        let terminal = render(80, 24, &mut app);
        let lines = buffer_lines(&terminal);

        let agent_row = lines.iter().find(|l| l.contains("· idle")).unwrap();
        assert!(
            agent_row.contains("\u{f06a9} claude · idle"),
            "robot in front of the agent meta: {agent_row:?}"
        );
        let shell_row = lines.iter().find(|l| l.contains("pane 2")).unwrap();
        assert!(
            shell_row.contains("\u{f120} shell"),
            "terminal icon in front of shell meta: {shell_row:?}"
        );
    }

    #[test]
    fn agent_icons_respect_the_toggle_and_the_ascii_set() {
        let mut app = sample_app();
        let view = ViewOptions {
            agent_icon_set: None,
            ..plain_view()
        };
        let terminal = render_with(80, 24, &mut app, &view);
        let toggled_off = screen(&terminal);
        assert!(
            !toggled_off.contains('\u{f06a9}') && !toggled_off.contains('\u{f120}'),
            "toggle off drops the icons:\n{toggled_off}"
        );

        // Ascii has no one-column robot; the meta stays bare.
        let view = ViewOptions {
            icon_set: Some(IconSet::Ascii),
            agent_icon_set: Some(IconSet::Ascii),
            ..plain_view()
        };
        let terminal = render_with(80, 24, &mut app, &view);
        let ascii = screen(&terminal);
        assert!(
            ascii.contains("claude · idle") && !ascii.contains('\u{f06a9}'),
            "ascii keeps the bare meta:\n{ascii}"
        );
    }

    #[test]
    fn agent_icons_survive_hiding_the_status_icons() {
        // show_agent_status = false must not take the agent icons down
        // with it: the toggles are independent.
        let mut app = sample_app();
        let view = ViewOptions {
            icon_set: None,
            agent_icon_set: Some(IconSet::Nerd),
            ..plain_view()
        };
        let terminal = render_with(80, 24, &mut app, &view);
        let screen = screen(&terminal);
        assert!(
            screen.contains("\u{f06a9} claude · idle"),
            "agent icon without status icons:\n{screen}"
        );
        assert!(!screen.contains('✓'), "status icons stay hidden:\n{screen}");
    }

    #[test]
    fn detail_agent_line_carries_the_icon() {
        let mut app = sample_app(); // cursor on the claude pane
        let terminal = render(80, 24, &mut app);
        let lines = buffer_lines(&terminal);

        let agent_line = lines
            .iter()
            .find(|l| l.contains("agent"))
            .expect("detail agent line");
        assert!(
            agent_line.contains("agent   \u{f06a9} claude")
                || agent_line.contains("\u{f06a9} claude"),
            "icon on the detail agent value: {agent_line:?}"
        );
    }

    #[test]
    fn detail_header_shows_the_ancestor_path() {
        let mut app = sample_app(); // cursor on the claude pane in tab "main"
        let terminal = render(80, 24, &mut app);
        let screen = screen(&terminal);

        assert!(
            screen.contains("picker/main/claude"),
            "path title, not the bare label:\n{screen}"
        );
    }

    #[test]
    fn detail_status_line_carries_the_status_icon() {
        let mut app = sample_app(); // cursor on the idle claude pane
        let terminal = render(80, 24, &mut app);
        let lines = buffer_lines(&terminal);

        let status_line = lines
            .iter()
            .find(|l| l.contains("status"))
            .expect("detail panel status line");
        assert!(
            status_line.contains("✓ idle"),
            "icon next to the status value: {status_line:?}"
        );
    }

    #[test]
    fn detail_status_line_is_painted_in_the_status_color() {
        let mut app = sample_app();
        let view = ViewOptions {
            color: true,
            ..plain_view()
        };
        let terminal = render_with(80, 24, &mut app, &view);
        let lines = buffer_lines(&terminal);
        let buffer = terminal.backend().buffer();

        let y = lines.iter().position(|l| l.contains("✓ idle")).unwrap();
        // Convert the byte offset to a column: every cell symbol on this
        // row is a single-width char, but tree glyphs are multi-byte.
        let byte = lines[y].find("✓ idle").unwrap();
        let x = lines[y][..byte].chars().count();
        for dx in 0.."✓ idle".chars().count() as u16 {
            let cell = buffer.cell((x as u16 + dx, y as u16)).unwrap();
            if cell.symbol().trim().is_empty() {
                continue;
            }
            assert_eq!(
                cell.style().fg,
                Some(Color::Green),
                "idle paints green at +{dx}: {cell:?}"
            );
        }
    }

    #[test]
    fn detail_status_line_stays_plain_without_icons() {
        let mut app = sample_app();
        let view = ViewOptions {
            icon_set: None,
            ..plain_view()
        };
        let terminal = render_with(80, 24, &mut app, &view);
        let lines = buffer_lines(&terminal);

        let status_line = lines
            .iter()
            .find(|l| l.contains("status"))
            .expect("detail panel status line");
        assert!(
            status_line.contains("idle") && !status_line.contains("✓"),
            "no icon when show_agent_status is off: {status_line:?}"
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
        assert!(screen.contains("picker"), "list still renders:\n{screen}");
    }

    #[test]
    fn cursor_row_is_reversed() {
        let mut app = sample_app(); // cursor starts on the focused pane row
        let terminal = render(80, 24, &mut app);

        let buffer = terminal.backend().buffer();
        let lines = buffer_lines(&terminal);
        // "✓ claude" (indented) is the list row; the detail panel header
        // also says "pane 1" but without the icon.
        let cursor_y = lines
            .iter()
            .position(|line| line.contains("✓ claude"))
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
    fn current_workspace_and_pane_carry_the_diamond_marker() {
        let mut app = sample_app();
        let terminal = render(80, 24, &mut app);
        let lines = buffer_lines(&terminal);

        let ws = lines.iter().find(|l| l.contains("picker")).unwrap();
        assert!(ws.contains("◆"), "current workspace row: {ws:?}");
        let current = lines.iter().find(|l| l.contains("✓ claude")).unwrap();
        assert!(current.contains("◆"), "current pane row: {current:?}");
        let other = lines.iter().find(|l| l.contains("✓ pane 2")).unwrap();
        assert!(!other.contains("◆"), "other row: {other:?}");
        let tab = lines.iter().find(|l| l.contains("⠋ main")).unwrap();
        assert!(!tab.contains("◆"), "tabs never carry the marker: {tab:?}");
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
    fn header_always_shows_the_prompt_placeholder_and_count() {
        let mut app = sample_app();
        let terminal = render(80, 24, &mut app);
        let lines = buffer_lines(&terminal);
        // Like the built-in: an idle "/ search panes" placeholder with the
        // row count at the right edge, before any key is pressed.
        assert!(
            lines[0].contains("/ search panes"),
            "header: {:?}",
            lines[0]
        );
        // The count is the total pane count, like the built-in — not the
        // number of visible rows.
        assert!(
            lines[0].trim_end().ends_with("3 panes"),
            "count at the right edge: {:?}",
            lines[0]
        );
        assert!(
            lines[2].contains("picker"),
            "the list starts below the header rule: {:?}",
            lines[2]
        );
    }

    #[test]
    fn pane_count_in_the_header_ignores_active_filters() {
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
        assert!(app.rows().is_empty());
        let terminal = render(80, 24, &mut app);
        let lines = buffer_lines(&terminal);
        assert!(
            lines[0].trim_end().ends_with("3 panes"),
            "the total stays put under a filter: {:?}",
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
            vec![workspace("w1", 1, "w", true)],
            // Wide-charactered TAB labels: tab rows carry the "N panes"
            // right column whose alignment the width math protects.
            vec![
                tab("w1:t1", "w1", 1, "日本語のラベル", true),
                tab("w1:t2", "w1", 2, "ascii", false),
            ],
            vec![],
            InitialExpansion::All,
        );
        let mut app = App::new(tree, EnterOnBranch::Jump);
        let terminal = render(50, 10, &mut app); // < 60 cols: list only
        let lines = buffer_lines(&terminal);

        // Continuation cells of wide glyphs read back as blanks, so search
        // by a single character.
        let jp = lines.iter().find(|l| l.contains('日')).unwrap();
        let ascii = lines.iter().find(|l| l.contains("ascii")).unwrap();
        let jp_body = jp.trim_end_matches(' ');
        let ascii_body = ascii.trim_end_matches(' ');
        assert!(jp_body.ends_with("2 panes"), "jp row: {jp:?}");
        assert!(ascii_body.ends_with("2 panes"), "ascii row: {ascii:?}");
    }

    #[test]
    fn cursor_far_down_stays_visible_and_viewport_height_is_recorded() {
        let panes: Vec<PaneInfo> = (1..=25)
            .map(|n| pane(&format!("w1:p{n}"), "w1:t1", "w1", n == 20, None))
            .collect();
        let tree = Tree::build(
            vec![workspace("w1", 1, "picker", true)],
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
        // 12 rows minus the header (2) and footer (2) -> 8; herdr owns the pane border.
        assert_eq!(app.viewport_height, 8);
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
            lines[2].contains("picker"),
            "no own border, the list starts under the header rule: {:?}",
            lines[2]
        );

        // The footer's top separator keeps the accent color.
        let sep_y = lines.iter().position(|l| l.starts_with('─')).unwrap() as u16;
        assert_eq!(
            buffer.cell((0, sep_y)).unwrap().style().fg,
            Some(Color::Cyan),
            "footer separator carries the accent color"
        );

        let y = lines.iter().position(|l| l.contains("○ picker")).unwrap() as u16;
        // Column of the label's first char: byte offset → char count (the
        // guide glyphs before it are multi-byte), plus the "○ " prefix.
        let byte = lines[y as usize].find("○ picker").unwrap();
        let x = (lines[y as usize][..byte].chars().count() + 2) as u16;
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
    fn state_filter_line_shows_status_and_count() {
        use crate::keymap::{parse_key_spec, Keymaps};
        let keys = KeysConfig::default();
        let (normal, _) = Keymap::from_bindings(&keys.to_bindings());
        let (search, _) = Keymap::from_bindings(&keys.to_search_bindings());
        let keymaps = Keymaps { normal, search };

        let mut app = sample_app(); // tab "main"/"logs" are Working
        let key = parse_key_spec("w").unwrap().0[0];
        app.handle_key(&keymaps, key);
        let terminal = render(80, 24, &mut app);
        let screen = screen(&terminal);

        // The chip carries the state's own icon, like the built-in's
        // push_state_chip — for working that is the spinner (tick 0).
        assert!(screen.contains("⠋ working"), "filter line:\n{screen}");
        let lines = buffer_lines(&terminal);
        assert!(
            lines[0].trim_end().ends_with("3 panes"),
            "pane total at the right edge: {:?}",
            lines[0]
        );
    }

    #[test]
    fn search_line_keeps_the_pane_total() {
        let mut app = sample_app();
        app.mode = Mode::Search;
        app.query = "x".to_string();
        let terminal = render(80, 24, &mut app);
        let lines = buffer_lines(&terminal);
        assert!(
            lines[0].trim_end().ends_with("3 panes"),
            "count next to the prompt: {:?}",
            lines[0]
        );
    }

    #[test]
    fn overflowing_list_grows_a_scrollbar_on_the_right_edge() {
        let mut app = sample_app(); // 6 rows
                                    // 8 rows tall: 2 header + 2 footer leave a 4-row list viewport,
                                    // and 50 wide keeps the detail panel away from the right edge.
        let terminal = render(50, 8, &mut app);
        let lines = buffer_lines(&terminal);
        for (y, line) in lines.iter().enumerate().take(6).skip(2) {
            assert!(
                line.ends_with('▕'),
                "scrollbar in the last column of line {y}: {line:?}"
            );
        }

        // Roomy viewport: no scrollbar.
        let terminal = render(50, 24, &mut app);
        let lines = buffer_lines(&terminal);
        assert!(
            !lines[2].ends_with('▕'),
            "no scrollbar when everything fits: {:?}",
            lines[2]
        );
    }

    #[test]
    fn draw_records_the_layout_for_mouse_hit_testing() {
        let mut app = sample_app();
        let _ = render(80, 24, &mut app);
        assert_eq!(app.prompt_row, 0, "prompt on the first line");
        assert_eq!(app.list_rect.1, 2, "list starts under the header rule");
        assert_eq!(app.list_rect.3, app.viewport_height);
        assert_eq!(app.list_offset, 0, "six rows fit without scrolling");
    }

    #[test]
    fn footer_collapses_state_filters_into_one_hint() {
        let mut app = sample_app();
        let terminal = render(80, 24, &mut app);
        assert!(
            screen(&terminal).contains("b/w/i/d/a states"),
            "screen:\n{}",
            screen(&terminal)
        );
    }

    #[test]
    fn workspace_right_column_shows_the_activity_summary() {
        let mut blocked = pane("w1:p3", "w1:t2", "w1", false, Some("claude"));
        blocked.agent_status = AgentStatus::Blocked;
        let mut ws = workspace("w1", 1, "picker", true);
        ws.pane_count = 3;
        let tree = Tree::build(
            vec![ws],
            vec![
                tab("w1:t1", "w1", 1, "main", true),
                tab("w1:t2", "w1", 2, "logs", false),
            ],
            vec![
                pane("w1:p1", "w1:t1", "w1", true, Some("claude")),
                pane("w1:p2", "w1:t1", "w1", false, None),
                blocked,
            ],
            InitialExpansion::All,
        );
        let mut app = App::new(tree, EnterOnBranch::Jump);
        let terminal = render(100, 24, &mut app);
        let lines = buffer_lines(&terminal);

        let ws_row = lines.iter().find(|l| l.contains("picker")).unwrap();
        assert!(ws_row.contains("1 blocked"), "ws activity: {ws_row:?}");
        let logs_row = lines.iter().find(|l| l.contains("logs")).unwrap();
        assert!(
            logs_row.contains("2 panes · 1 blocked"),
            "tab pane count with activity: {logs_row:?}"
        );
    }

    #[test]
    fn parse_color_accepts_names_and_hex() {
        assert_eq!(parse_color("cyan"), Some(Color::Cyan));
        assert_eq!(parse_color("Magenta"), Some(Color::Magenta));
        assert_eq!(parse_color("purple"), Some(Color::Magenta));
        assert_eq!(parse_color("#bd93f9"), Some(Color::Rgb(0xbd, 0x93, 0xf9)));
        assert_eq!(parse_color("#bad"), None, "short hex unsupported");
        assert_eq!(
            parse_color("#あい"),
            None,
            "6 BYTES of non-ASCII must not panic the byte slicing"
        );
        assert_eq!(parse_color("mauve-ish"), None);
    }

    #[test]
    fn cursor_row_is_a_solid_accent_bar_not_reversed() {
        let mut app = sample_app(); // cursor on the claude pane row
        let accent = Color::Rgb(0xbd, 0x93, 0xf9);
        let view = ViewOptions {
            color: true,
            accent,
            ..plain_view()
        };
        let terminal = render_with(80, 24, &mut app, &view);
        let buffer = terminal.backend().buffer();
        let lines = buffer_lines(&terminal);
        let y = lines.iter().position(|l| l.contains("✓ claude")).unwrap() as u16;

        // Every cell of the row — icon included — sits on the accent
        // background with the contrast foreground; no inverted patches.
        let icon_x = lines[y as usize].chars().position(|c| c == '✓').unwrap() as u16;
        for x in [0, icon_x, icon_x + 2] {
            let style = buffer.cell((x, y)).unwrap().style();
            assert_eq!(style.bg, Some(accent), "cell {x} bg");
            assert_eq!(style.fg, Some(contrast_fg(accent)), "cell {x} fg");
        }

        // The accent config reaches the warning path too.
        let display = DisplayConfig {
            accent: "mauve-ish".to_string(),
            ..Default::default()
        };
        let (view, warnings) = ViewOptions::from_config(&display, false, None, None);
        assert_eq!(view.accent, Color::Cyan);
        assert!(warnings.iter().any(|w| w.contains("mauve-ish")));
    }

    #[test]
    fn middle_elide_keeps_head_and_tail_within_budget() {
        assert_eq!(middle_elide("short", 10), "short");
        let elided = middle_elide("abcdefghijklmnop", 9);
        assert_eq!(elided, "abcd…mnop");
        assert!(UnicodeWidthStr::width(elided.as_str()) <= 9);
        // Double-width chars count as two columns.
        let elided = middle_elide("日本語のラベルです", 9);
        assert!(UnicodeWidthStr::width(elided.as_str()) <= 9, "{elided}");
        assert!(elided.contains('…'));
        assert_eq!(middle_elide("anything", 1), "…");
    }

    #[test]
    fn elide_path_shortens_segments_fish_style() {
        assert_eq!(elide_path("~/src/repo", 20), "~/src/repo", "fits as-is");
        assert_eq!(
            elide_path("~/src/github.com/yoshiori/picker", 20),
            "~/s/g/y/picker"
        );
        assert_eq!(elide_path("/home/u/src/repo", 12), "/h/u/s/repo");
        assert_eq!(
            elide_path("~/.config/herdr/scripts", 15),
            "~/.c/h/scripts",
            "dot-dirs keep the dot"
        );
        // Even fish-style is too long: fall back to the middle cut.
        let elided = elide_path("~/x/very-long-repository-name", 12);
        assert!(elided.contains('…'), "{elided}");
        assert!(UnicodeWidthStr::width(elided.as_str()) <= 12, "{elided}");
    }

    #[test]
    fn detail_panel_elides_long_cwds_keeping_the_tail() {
        let mut long_cwd = pane("w1:p1", "w1:t1", "w1", true, Some("claude"));
        long_cwd.cwd = Some("/home/u/src/github.com/yoshiori/picker-repo".to_string());
        let tree = Tree::build(
            vec![workspace("w1", 1, "picker", true)],
            vec![
                tab("w1:t1", "w1", 1, "main", true),
                tab("w1:t2", "w1", 2, "logs", false),
            ],
            vec![long_cwd, pane("w1:p2", "w1:t1", "w1", false, None)],
            InitialExpansion::All,
        );
        let mut app = App::new(tree, EnterOnBranch::Jump);
        let terminal = render(100, 24, &mut app);
        let screen = screen(&terminal);

        assert!(
            screen.contains("~/s/g/y/picker-repo"),
            "cwd shortens fish-style, keeping the leaf:\n{screen}"
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
