//! Entry point: wires env, config, socket client, and the TUI event loop.
//! All logic lives in the tested modules; this file only glues them.

mod app;
mod config;
mod herdr_client;
mod icons;
mod keymap;
mod search;
mod tree;
mod ui;

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use crossterm::event::{self, Event};

use app::{App, EnterOnBranch, Outcome};
use herdr_client::{HerdrApi, SocketClient};
use keymap::KeyPress;
use tree::{FocusTarget, InitialExpansion, Tree};

fn main() -> ExitCode {
    let Some(socket_path) = std::env::var_os("HERDR_SOCKET_PATH") else {
        eprintln!(
            "herdr-configurable-picker must run inside a herdr session \
             (HERDR_SOCKET_PATH is not set).\n\
             Install the plugin and open it via its \"open\" action; \
             see README.md."
        );
        return ExitCode::from(2);
    };

    let mut warnings = Vec::new();
    let config = match std::env::var_os("HERDR_PLUGIN_CONFIG_DIR") {
        Some(dir) => {
            let (config, mut config_warnings) = config::load_or_seed(Path::new(&dir));
            warnings.append(&mut config_warnings);
            config
        }
        None => {
            warnings.push("HERDR_PLUGIN_CONFIG_DIR is not set; using default config".to_string());
            config::Config::default()
        }
    };
    let (normal_keymap, mut keymap_warnings) =
        keymap::Keymap::from_bindings(&config.keys.to_bindings());
    warnings.append(&mut keymap_warnings);
    let (search_keymap, mut search_warnings) =
        keymap::Keymap::from_bindings(&config.keys.to_search_bindings());
    warnings.append(&mut search_warnings);
    let keymaps = keymap::Keymaps {
        normal: normal_keymap,
        search: search_keymap,
    };

    let initial_expansion = InitialExpansion::parse(&config.behavior.initial_expansion)
        .unwrap_or_else(|| {
            warnings.push(format!(
                "unknown initial_expansion {:?}; using \"all\"",
                config.behavior.initial_expansion
            ));
            InitialExpansion::All
        });
    let enter_on_branch =
        EnterOnBranch::parse(&config.behavior.enter_on_branch).unwrap_or_else(|| {
            warnings.push(format!(
                "unknown enter_on_branch {:?}; using \"jump\"",
                config.behavior.enter_on_branch
            ));
            EnterOnBranch::Jump
        });
    // NO_COLOR per https://no-color.org: present and non-empty disables color.
    let no_color = std::env::var_os("NO_COLOR").is_some_and(|value| !value.is_empty());
    let (view, mut view_warnings) =
        ui::ViewOptions::from_config(&config.display, no_color, std::env::var("HOME").ok());
    warnings.append(&mut view_warnings);
    report_warnings(&warnings);

    let mut client = match SocketClient::connect(Path::new(&socket_path)) {
        Ok(client) => client,
        Err(e) => return fail_visibly(&format!("{e:#}")),
    };
    let snapshot = client.list_workspaces().and_then(|workspaces| {
        let tabs = client.list_tabs()?;
        let panes = client.list_panes()?;
        Ok((workspaces, tabs, panes))
    });
    let (mut workspaces, mut tabs, mut panes) = match snapshot {
        Ok(snapshot) => snapshot,
        Err(e) => return fail_visibly(&format!("{e:#}")),
    };
    tree::drop_own_overlay_pane(
        &mut workspaces,
        &mut tabs,
        &mut panes,
        context_focused_pane_id().as_deref(),
    );

    let tree = Tree::build(workspaces, tabs, panes, initial_expansion);
    let mut app = App::new(tree, enter_on_branch);
    let hints = ui::FooterHints::from_keymap(&keymaps.normal);

    let mut terminal = ratatui::init();
    let selection = loop {
        if let Err(e) = terminal.draw(|frame| ui::draw(frame, &mut app, &hints, &view)) {
            ratatui::restore();
            return fail_visibly(&format!("failed to draw: {e}"));
        }
        match event::read() {
            Ok(Event::Key(key)) => {
                if let Some(press) = KeyPress::from_crossterm(&key) {
                    match app.handle_key(&keymaps, press) {
                        Outcome::Continue => {}
                        Outcome::Cancel => break None,
                        Outcome::Focus(target) => break Some(target),
                    }
                }
            }
            // Resize just needs the next draw; other events are ignored.
            Ok(_) => {}
            Err(_) => break None,
        }
    };
    ratatui::restore();

    if let Some(target) = selection {
        let focused = match &target {
            FocusTarget::Workspace(id) => client.focus_workspace(id),
            FocusTarget::Tab(id) => client.focus_tab(id),
            FocusTarget::Pane(id) => client.focus_pane(id),
        };
        if let Err(e) = focused {
            eprintln!("herdr-configurable-picker: {e:#}");
        }
    }
    // Exit 0 even on cancel: the overlay closing is the normal outcome, and
    // herdr raises a toast for non-zero exits.
    ExitCode::SUCCESS
}

/// The pane the user came from, out of HERDR_PLUGIN_CONTEXT_JSON. Needed to
/// tell the picker's own overlay pane apart from the user's pane in the
/// snapshot. Best effort: on any missing piece the overlay stays listed.
fn context_focused_pane_id() -> Option<String> {
    let context = std::env::var("HERDR_PLUGIN_CONTEXT_JSON").ok()?;
    let context: serde_json::Value = serde_json::from_str(&context).ok()?;
    Some(context.get("focused_pane_id")?.as_str()?.to_string())
}

/// Stderr flashes and vanishes with the overlay pane, so warnings also go
/// to $HERDR_PLUGIN_STATE_DIR/picker.log where they can be read later.
fn report_warnings(warnings: &[String]) {
    if warnings.is_empty() {
        return;
    }
    for warning in warnings {
        eprintln!("herdr-configurable-picker: {warning}");
    }
    if let Some(state_dir) = std::env::var_os("HERDR_PLUGIN_STATE_DIR") {
        let path = PathBuf::from(state_dir).join("picker.log");
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        {
            for warning in warnings {
                let _ = writeln!(file, "{warning}");
            }
        }
    }
}

/// Startup failure inside the overlay: the pane closes as soon as we exit,
/// so hold the message on screen briefly. Exit 0 to avoid a duplicate toast.
fn fail_visibly(message: &str) -> ExitCode {
    report_warnings(&[message.to_string()]);
    std::thread::sleep(std::time::Duration::from_secs(3));
    ExitCode::SUCCESS
}
