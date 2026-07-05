//! Reads settings from the herdr host's own config.toml so the picker
//! blends in: the theme accent and the `[ui] mouse_capture` toggle.
//!
//! herdr (as of 0.7.1) exposes no theme/settings API or env var to
//! plugins, so this mirrors the host's own resolution. For the accent:
//! read `[theme]`, honor a `[theme.custom] accent` override, else look the
//! built-in palette's accent up by name (values lifted from herdr's
//! `Palette::from_name` table).

use std::path::Path;

use ratatui::style::Color;

/// The host `config.toml` contents. `plugin_config_dir` is
/// `$HERDR_PLUGIN_CONFIG_DIR` (= `<config_dir>/plugins/config/<id>`), from
/// which the host `config.toml` is three levels up.
fn read_host_config(plugin_config_dir: &Path) -> Option<String> {
    let host_config = plugin_config_dir
        .parent()?
        .parent()?
        .parent()?
        .join("config.toml");
    std::fs::read_to_string(host_config).ok()
}

/// The host accent, best effort.
pub fn host_accent(plugin_config_dir: &Path) -> Option<Color> {
    accent_from_config(&read_host_config(plugin_config_dir)?)
}

/// The host's `[ui] mouse_capture`, best effort — None only when the host
/// config cannot be read at all.
pub fn host_mouse_capture(plugin_config_dir: &Path) -> Option<bool> {
    Some(mouse_capture_from_config(&read_host_config(
        plugin_config_dir,
    )?))
}

/// `[ui] mouse_capture` with the host's own default (true) for anything
/// missing or malformed.
fn mouse_capture_from_config(config_toml: &str) -> bool {
    let Ok(doc) = config_toml.parse::<toml::Value>() else {
        return true;
    };
    doc.get("ui")
        .and_then(|ui| ui.get("mouse_capture"))
        .and_then(|v| v.as_bool())
        .unwrap_or(true)
}

fn accent_from_config(config_toml: &str) -> Option<Color> {
    let doc: toml::Value = config_toml.parse().ok()?;
    let theme = doc.get("theme");

    // A custom accent override wins, exactly like the host.
    if let Some(custom) = theme
        .and_then(|t| t.get("custom"))
        .and_then(|c| c.get("accent"))
        .and_then(|v| v.as_str())
    {
        if let Some(color) = crate::ui::parse_color(custom) {
            return Some(color);
        }
    }

    // auto_switch picks dark_name/light_name from the host appearance,
    // which a plugin cannot observe; prefer `name`, else assume dark.
    let name = theme
        .and_then(|t| t.get("name"))
        .and_then(|v| v.as_str())
        .or_else(|| {
            theme
                .and_then(|t| t.get("dark_name"))
                .and_then(|v| v.as_str())
        })
        .unwrap_or("catppuccin"); // the host default
    builtin_accent(name)
}

/// Accents of herdr's built-in palettes (src/app/state.rs), keyed with the
/// same name normalization as the host's `Palette::from_name`.
fn builtin_accent(name: &str) -> Option<Color> {
    let rgb = |r, g, b| Some(Color::Rgb(r, g, b));
    match name.to_lowercase().replace([' ', '_'], "-").as_str() {
        "catppuccin" | "catppuccin-mocha" => rgb(137, 180, 250),
        "catppuccin-latte" | "latte" | "light" => rgb(30, 102, 245),
        "terminal" => Some(Color::Blue),
        "tokyo-night" | "tokyonight" => rgb(122, 162, 247),
        "tokyo-night-day" | "tokyo-day" | "tokyonight-day" => rgb(46, 125, 233),
        "dracula" => rgb(189, 147, 249),
        "nord" => rgb(136, 192, 208),
        "gruvbox" | "gruvbox-dark" => rgb(215, 153, 33),
        "gruvbox-light" => rgb(7, 102, 120),
        "one-dark" | "onedark" => rgb(97, 175, 239),
        "one-light" | "onelight" => rgb(64, 120, 242),
        "solarized" | "solarized-dark" | "solarized-light" => rgb(38, 139, 210),
        "kanagawa" => rgb(126, 156, 216),
        "kanagawa-lotus" | "lotus" => rgb(77, 105, 155),
        "rose-pine" | "rosepine" => rgb(196, 167, 231),
        "rose-pine-dawn" | "rosepine-dawn" | "dawn" => rgb(144, 122, 169),
        "vesper" => rgb(255, 199, 153),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_builtin_theme_accents() {
        assert_eq!(
            accent_from_config("[theme]\nname = \"dracula\"\n"),
            Some(Color::Rgb(189, 147, 249))
        );
        assert_eq!(
            accent_from_config("[theme]\nname = \"Tokyo Night\"\n"),
            Some(Color::Rgb(122, 162, 247)),
            "same name normalization as the host"
        );
    }

    #[test]
    fn custom_accent_override_wins() {
        let config = "[theme]\nname = \"dracula\"\n\n[theme.custom]\naccent = \"#ff79c6\"\n";
        assert_eq!(
            accent_from_config(config),
            Some(Color::Rgb(0xff, 0x79, 0xc6))
        );
    }

    #[test]
    fn defaults_to_catppuccin_and_falls_back_to_dark_name() {
        assert_eq!(
            accent_from_config("# no theme section\n"),
            Some(Color::Rgb(137, 180, 250)),
            "the host defaults to catppuccin"
        );
        let auto = "[theme]\nauto_switch = true\ndark_name = \"nord\"\n";
        assert_eq!(accent_from_config(auto), Some(Color::Rgb(136, 192, 208)));
    }

    #[test]
    fn unknown_theme_or_broken_config_yields_none() {
        assert_eq!(accent_from_config("[theme]\nname = \"my-theme\"\n"), None);
        assert_eq!(accent_from_config("not [ toml"), None);
    }

    #[test]
    fn reads_the_host_mouse_capture_flag() {
        assert!(!mouse_capture_from_config("[ui]\nmouse_capture = false\n"));
        assert!(mouse_capture_from_config("[ui]\nmouse_capture = true\n"));
        assert!(
            mouse_capture_from_config("# nothing here\n"),
            "the host default is on"
        );
        assert!(mouse_capture_from_config("not [ toml"));
    }

    #[test]
    fn host_mouse_capture_walks_up_like_the_accent() {
        let root = tempfile::tempdir().unwrap();
        let plugin_dir = root.path().join("plugins/config/some.plugin");
        std::fs::create_dir_all(&plugin_dir).unwrap();
        std::fs::write(
            root.path().join("config.toml"),
            "[ui]\nmouse_capture = false\n",
        )
        .unwrap();

        assert_eq!(host_mouse_capture(&plugin_dir), Some(false));
        let elsewhere = tempfile::tempdir().unwrap();
        let orphan = elsewhere.path().join("plugins/config/some.plugin");
        std::fs::create_dir_all(&orphan).unwrap();
        assert_eq!(
            host_mouse_capture(&orphan),
            None,
            "no host config.toml at all"
        );
    }

    #[test]
    fn host_accent_walks_up_from_the_plugin_config_dir() {
        let root = tempfile::tempdir().unwrap();
        let plugin_dir = root.path().join("plugins/config/some.plugin");
        std::fs::create_dir_all(&plugin_dir).unwrap();
        std::fs::write(
            root.path().join("config.toml"),
            "[theme]\nname = \"dracula\"\n",
        )
        .unwrap();

        assert_eq!(host_accent(&plugin_dir), Some(Color::Rgb(189, 147, 249)));
        assert_eq!(
            host_accent(&root.path().join("plugins/config/missing")),
            Some(Color::Rgb(189, 147, 249)),
            "only the ancestor path matters"
        );
    }
}
