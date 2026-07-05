//! Plugin configuration: `$HERDR_PLUGIN_CONFIG_DIR/config.toml`.
//!
//! The file is seeded verbatim from [`DEFAULT_CONFIG_TOML`] on first run and
//! never rewritten afterwards. Loading never fails hard: a broken file falls
//! back to defaults with a warning, because a picker that cannot open is
//! worse than one with default keys.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::keymap::Action;

/// Seeded into the config dir on first run. The round-trip unit test keeps
/// this file in sync with `Config::default()` forever.
pub const DEFAULT_CONFIG_TOML: &str = r##"# herdr-configurable-picker configuration.
#
# Every entry under [keys] is an array of key strings; all of them trigger
# the action. Syntax mirrors herdr's own bindings:
#   - modifiers: ctrl+, alt+, shift+, super+   (e.g. "ctrl+n")
#   - named keys: enter, esc, tab, space, backspace, delete, up, down,
#     left, right, home, end, pageup, pagedown, f1..f12
#   - single characters: "j", "G" (uppercase means shift)
#   - chords: space-separated, e.g. "g g"
# If two actions bind the same key, the earlier entry in this table wins;
# broken keys are disabled with a warning, the rest keep working.

[keys]
# Movement
down      = ["down", "ctrl+n", "j"]
up        = ["up", "ctrl+p", "k"]
page_down = ["ctrl+d", "pagedown"]
page_up   = ["ctrl+u", "pageup"]
top       = ["home"]
bottom    = ["end", "shift+g"]

# Tree expansion
expand   = ["right", "l"]
collapse = ["left", "h"]
toggle   = ["space"]

# Confirm / cancel
accept = ["enter"]
cancel = ["esc", "ctrl+c", "ctrl+g"]

# Search. While the search prompt is focused, printable keys type into the
# query; search_clear / search_exit and non-printable normal-mode keys
# (ctrl+n, arrows, enter, ...) still work.
search_start = ["/"]
search_clear = ["ctrl+u"]
search_exit  = ["esc"]

# State filters: show only nodes whose agents are in the given state.
# Mutually exclusive with text search (starting one drops the other).
filter_blocked = ["b"]
filter_working = ["w"]
filter_idle    = ["i"]
filter_done    = ["d"]
filter_clear   = ["a", "backspace"]

[display]
show_pane_count   = true
show_agent_status = true
show_cwd          = false

# "nerd" | "ascii" | "emoji"
icon_set = "nerd"

# Accent for the cursor row, current markers, and separators.
# "auto" follows the herdr theme; or set a named ANSI color ("cyan",
# "magenta", ...) or hex ("#bd93f9").
accent = "auto"

[behavior]
# "all" | "current_workspace" | "none"
initial_expansion = "all"

# Enter on a branch node: "expand" or "jump"
enter_on_branch = "jump"

# Mouse support: hover to select, click to jump (branch carets toggle),
# wheel to scroll. "auto" follows the herdr config's [ui] mouse_capture;
# true / false override it.
mouse = "auto"
"##;

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub keys: KeysConfig,
    pub display: DisplayConfig,
    pub behavior: BehaviorConfig,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct KeysConfig {
    pub down: Vec<String>,
    pub up: Vec<String>,
    pub page_down: Vec<String>,
    pub page_up: Vec<String>,
    pub top: Vec<String>,
    pub bottom: Vec<String>,
    pub expand: Vec<String>,
    pub collapse: Vec<String>,
    pub toggle: Vec<String>,
    pub accept: Vec<String>,
    pub cancel: Vec<String>,
    pub search_start: Vec<String>,
    pub search_clear: Vec<String>,
    pub search_exit: Vec<String>,
    pub filter_blocked: Vec<String>,
    pub filter_working: Vec<String>,
    pub filter_idle: Vec<String>,
    pub filter_done: Vec<String>,
    pub filter_clear: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DisplayConfig {
    pub show_pane_count: bool,
    pub show_agent_status: bool,
    pub show_cwd: bool,
    pub icon_set: String,
    pub accent: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BehaviorConfig {
    pub initial_expansion: String,
    pub enter_on_branch: String,
    /// Hover, click-to-jump, caret toggling, and wheel scrolling.
    pub mouse: MouseConfig,
}

/// `[behavior] mouse`: `"auto"` follows the host's `[ui] mouse_capture`
/// (like `accent = "auto"` follows the theme); plain booleans override it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MouseConfig {
    Enabled(bool),
    Mode(String),
}

impl MouseConfig {
    /// On/off given the host's `[ui] mouse_capture` (None when the host
    /// config is unreadable), plus a warning for unknown values.
    pub fn resolve(&self, host_mouse_capture: Option<bool>) -> (bool, Option<String>) {
        // The host itself defaults mouse_capture to true.
        let auto = host_mouse_capture.unwrap_or(true);
        match self {
            MouseConfig::Enabled(enabled) => (*enabled, None),
            MouseConfig::Mode(mode) if mode == "auto" => (auto, None),
            MouseConfig::Mode(other) => (
                auto,
                Some(format!("unknown mouse {other:?}; using \"auto\"")),
            ),
        }
    }
}

fn keys(specs: &[&str]) -> Vec<String> {
    specs.iter().map(|s| s.to_string()).collect()
}

impl Default for KeysConfig {
    fn default() -> Self {
        KeysConfig {
            down: keys(&["down", "ctrl+n", "j"]),
            up: keys(&["up", "ctrl+p", "k"]),
            page_down: keys(&["ctrl+d", "pagedown"]),
            page_up: keys(&["ctrl+u", "pageup"]),
            top: keys(&["home"]),
            bottom: keys(&["end", "shift+g"]),
            expand: keys(&["right", "l"]),
            collapse: keys(&["left", "h"]),
            toggle: keys(&["space"]),
            accept: keys(&["enter"]),
            cancel: keys(&["esc", "ctrl+c", "ctrl+g"]),
            search_start: keys(&["/"]),
            search_clear: keys(&["ctrl+u"]),
            search_exit: keys(&["esc"]),
            filter_blocked: keys(&["b"]),
            filter_working: keys(&["w"]),
            filter_idle: keys(&["i"]),
            filter_done: keys(&["d"]),
            filter_clear: keys(&["a", "backspace"]),
        }
    }
}

impl Default for DisplayConfig {
    fn default() -> Self {
        DisplayConfig {
            show_pane_count: true,
            show_agent_status: true,
            show_cwd: false,
            icon_set: "nerd".to_string(),
            accent: "auto".to_string(),
        }
    }
}

impl Default for BehaviorConfig {
    fn default() -> Self {
        BehaviorConfig {
            initial_expansion: "all".to_string(),
            enter_on_branch: "jump".to_string(),
            mouse: MouseConfig::Mode("auto".to_string()),
        }
    }
}

impl KeysConfig {
    /// Binding table in `[keys]` declaration order — the order is the
    /// conflict-resolution priority (earlier entry wins).
    pub fn to_bindings(&self) -> Vec<(Action, Vec<String>)> {
        vec![
            (Action::Down, self.down.clone()),
            (Action::Up, self.up.clone()),
            (Action::PageDown, self.page_down.clone()),
            (Action::PageUp, self.page_up.clone()),
            (Action::Top, self.top.clone()),
            (Action::Bottom, self.bottom.clone()),
            (Action::Expand, self.expand.clone()),
            (Action::Collapse, self.collapse.clone()),
            (Action::Toggle, self.toggle.clone()),
            (Action::Accept, self.accept.clone()),
            (Action::Cancel, self.cancel.clone()),
            (Action::SearchStart, self.search_start.clone()),
            (Action::FilterBlocked, self.filter_blocked.clone()),
            (Action::FilterWorking, self.filter_working.clone()),
            (Action::FilterIdle, self.filter_idle.clone()),
            (Action::FilterDone, self.filter_done.clone()),
            (Action::FilterClear, self.filter_clear.clone()),
        ]
    }

    /// The search-mode table. Kept separate from [`Self::to_bindings`] so a
    /// key like ctrl+u can mean page_up in normal mode and clear-query while
    /// searching without tripping conflict detection.
    pub fn to_search_bindings(&self) -> Vec<(Action, Vec<String>)> {
        vec![
            (Action::SearchClear, self.search_clear.clone()),
            (Action::SearchExit, self.search_exit.clone()),
        ]
    }
}

/// Loads `config.toml` from `config_dir`, seeding it with the defaults on
/// first run. Returns the effective config plus human-readable warnings.
pub fn load_or_seed(config_dir: &Path) -> (Config, Vec<String>) {
    let path = config_dir.join("config.toml");
    let mut warnings = Vec::new();

    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            if let Err(e) = std::fs::create_dir_all(config_dir)
                .and_then(|_| std::fs::write(&path, DEFAULT_CONFIG_TOML))
            {
                warnings.push(format!(
                    "could not seed {}: {e}; continuing with defaults",
                    path.display()
                ));
            }
            return (Config::default(), warnings);
        }
        Err(e) => {
            warnings.push(format!(
                "could not read {}: {e}; continuing with defaults",
                path.display()
            ));
            return (Config::default(), warnings);
        }
    };

    match toml::from_str(&text) {
        Ok(config) => (config, warnings),
        Err(e) => {
            warnings.push(format!(
                "invalid {}: {e}; continuing with defaults (file left untouched)",
                path.display()
            ));
            (Config::default(), warnings)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_spec() {
        let config = Config::default();
        assert_eq!(config.keys.down, vec!["down", "ctrl+n", "j"]);
        assert_eq!(config.keys.expand, vec!["right", "l"]);
        assert_eq!(config.keys.collapse, vec!["left", "h"]);
        assert_eq!(config.keys.toggle, vec!["space"]);
        assert_eq!(config.keys.accept, vec!["enter"]);
        assert_eq!(config.keys.cancel, vec!["esc", "ctrl+c", "ctrl+g"]);
        assert_eq!(config.display.icon_set, "nerd");
        assert!(config.display.show_pane_count);
        assert!(!config.display.show_cwd);
        assert_eq!(config.behavior.enter_on_branch, "jump");
        assert_eq!(config.behavior.initial_expansion, "all");
    }

    #[test]
    fn mouse_accepts_auto_bools_and_warns_on_nonsense() {
        // "auto" (the default) follows the host's [ui] mouse_capture.
        let auto = Config::default().behavior.mouse;
        assert_eq!(auto.resolve(Some(false)), (false, None));
        assert_eq!(auto.resolve(Some(true)), (true, None));
        assert_eq!(
            auto.resolve(None),
            (true, None),
            "host config unreadable: mouse on, like the host default"
        );

        // Plain booleans override the host.
        let config: Config = toml::from_str("[behavior]\nmouse = false\n").unwrap();
        assert_eq!(config.behavior.mouse.resolve(Some(true)), (false, None));

        // Anything else warns and behaves like "auto".
        let config: Config = toml::from_str("[behavior]\nmouse = \"sometimes\"\n").unwrap();
        let (on, warning) = config.behavior.mouse.resolve(Some(false));
        assert!(!on);
        assert!(warning.unwrap().contains("sometimes"));
    }

    #[test]
    fn default_config_toml_round_trips_to_default_config() {
        let parsed: Config =
            toml::from_str(DEFAULT_CONFIG_TOML).expect("DEFAULT_CONFIG_TOML must stay parseable");
        assert_eq!(
            parsed,
            Config::default(),
            "DEFAULT_CONFIG_TOML and Config::default() drifted apart"
        );
    }

    #[test]
    fn to_bindings_preserves_keys_table_order() {
        let bindings = KeysConfig::default().to_bindings();
        let actions: Vec<Action> = bindings.iter().map(|(a, _)| *a).collect();
        assert_eq!(
            actions,
            vec![
                Action::Down,
                Action::Up,
                Action::PageDown,
                Action::PageUp,
                Action::Top,
                Action::Bottom,
                Action::Expand,
                Action::Collapse,
                Action::Toggle,
                Action::Accept,
                Action::Cancel,
                Action::SearchStart,
                Action::FilterBlocked,
                Action::FilterWorking,
                Action::FilterIdle,
                Action::FilterDone,
                Action::FilterClear,
            ]
        );
        assert_eq!(bindings[0].1, vec!["down", "ctrl+n", "j"]);
    }

    #[test]
    fn search_mode_keys_live_in_their_own_table() {
        let bindings = KeysConfig::default().to_search_bindings();
        assert_eq!(
            bindings,
            vec![
                (Action::SearchClear, vec!["ctrl+u".to_string()]),
                (Action::SearchExit, vec!["esc".to_string()]),
            ]
        );
    }

    #[test]
    fn seeds_default_file_on_first_run() {
        let dir = tempfile::tempdir().unwrap();
        let config_dir = dir.path().join("nested").join("config");

        let (config, warnings) = load_or_seed(&config_dir);

        assert_eq!(config, Config::default());
        assert!(warnings.is_empty(), "warnings: {warnings:?}");
        let seeded = std::fs::read_to_string(config_dir.join("config.toml")).unwrap();
        assert_eq!(seeded, DEFAULT_CONFIG_TOML);
    }

    #[test]
    fn partial_file_merges_with_defaults() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("config.toml"),
            "[keys]\ndown = [\"ctrl+j\"]\n",
        )
        .unwrap();

        let (config, warnings) = load_or_seed(dir.path());

        assert!(warnings.is_empty(), "warnings: {warnings:?}");
        assert_eq!(config.keys.down, vec!["ctrl+j"]);
        assert_eq!(config.keys.up, KeysConfig::default().up);
        assert_eq!(config.display, DisplayConfig::default());
    }

    #[test]
    fn malformed_file_falls_back_to_defaults_without_touching_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "not [ valid toml").unwrap();

        let (config, warnings) = load_or_seed(dir.path());

        assert_eq!(config, Config::default());
        assert_eq!(warnings.len(), 1);
        assert!(
            warnings[0].contains("config.toml"),
            "warning should point at the file: {}",
            warnings[0]
        );
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "not [ valid toml",
            "broken file must be left for the user to fix"
        );
    }

    #[test]
    fn existing_valid_file_is_never_rewritten() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let custom = "[keys]\ndown = [\"ctrl+j\"]\n";
        std::fs::write(&path, custom).unwrap();

        let _ = load_or_seed(dir.path());

        assert_eq!(std::fs::read_to_string(&path).unwrap(), custom);
    }
}
