//! Key-string parsing and keybinding resolution.
//!
//! Key syntax mirrors herdr's own bindings: `ctrl+n`, `shift+g`, named keys
//! like `enter`/`pageup`, and space-separated chords like `"g g"`. Parsing is
//! case-insensitive; uppercase characters canonicalize to lowercase + shift so
//! `"G"` and `"shift+g"` are the same binding.

use std::fmt;

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

/// Everything the picker can do in normal mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Action {
    Up,
    Down,
    PageUp,
    PageDown,
    Top,
    Bottom,
    Expand,
    Collapse,
    Toggle,
    Accept,
    Cancel,
}

/// A single canonicalized key press.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct KeyPress {
    pub code: KeyCode,
    pub mods: KeyModifiers,
}

/// One binding: a sequence of presses. Length > 1 means a chord like `"g g"`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeySpec(pub Vec<KeyPress>);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyParseError(pub String);

impl fmt::Display for KeyParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for KeyParseError {}

pub fn parse_key_spec(spec: &str) -> Result<KeySpec, KeyParseError> {
    let presses: Vec<KeyPress> = spec
        .split_whitespace()
        .map(parse_key_press)
        .collect::<Result<_, _>>()?;
    if presses.is_empty() {
        return Err(KeyParseError("empty key spec".to_string()));
    }
    Ok(KeySpec(presses))
}

fn parse_key_press(token: &str) -> Result<KeyPress, KeyParseError> {
    let mut mods = KeyModifiers::NONE;
    let mut parts = token.split('+').peekable();
    let mut key_part = None;
    while let Some(part) = parts.next() {
        if parts.peek().is_some() {
            mods |= match part.to_ascii_lowercase().as_str() {
                "ctrl" => KeyModifiers::CONTROL,
                "alt" => KeyModifiers::ALT,
                "shift" => KeyModifiers::SHIFT,
                "super" => KeyModifiers::SUPER,
                other => {
                    return Err(KeyParseError(format!(
                        "unknown modifier {other:?} in key {token:?}"
                    )))
                }
            };
        } else {
            key_part = Some(part);
        }
    }
    let key_part = key_part
        .filter(|k| !k.is_empty())
        .ok_or_else(|| KeyParseError(format!("missing key after modifier in {token:?}")))?;
    let code = parse_key_code(key_part, &mut mods)
        .ok_or_else(|| KeyParseError(format!("unknown key {key_part:?} in key {token:?}")))?;
    Ok(canonicalize(KeyPress { code, mods }))
}

fn parse_key_code(name: &str, mods: &mut KeyModifiers) -> Option<KeyCode> {
    let lower = name.to_ascii_lowercase();
    let code = match lower.as_str() {
        "enter" => KeyCode::Enter,
        "esc" => KeyCode::Esc,
        "tab" => KeyCode::Tab,
        "space" => KeyCode::Char(' '),
        "backspace" => KeyCode::Backspace,
        "delete" => KeyCode::Delete,
        "up" => KeyCode::Up,
        "down" => KeyCode::Down,
        "left" => KeyCode::Left,
        "right" => KeyCode::Right,
        "home" => KeyCode::Home,
        "end" => KeyCode::End,
        "pageup" => KeyCode::PageUp,
        "pagedown" => KeyCode::PageDown,
        _ => {
            if let Some(n) = lower.strip_prefix('f').and_then(|n| n.parse::<u8>().ok()) {
                if (1..=12).contains(&n) {
                    return Some(KeyCode::F(n));
                }
                return None;
            }
            let mut chars = name.chars();
            let (c, rest) = (chars.next()?, chars.next());
            if rest.is_some() {
                return None;
            }
            // A bare uppercase letter ("G") means shift+g, but with explicit
            // modifiers ("Ctrl+N") the case is just spelling, not shift.
            if c.is_uppercase() && mods.is_empty() {
                *mods |= KeyModifiers::SHIFT;
            }
            KeyCode::Char(c.to_ascii_lowercase())
        }
    };
    Some(code)
}

/// Shared canonical form for parsed specs and incoming terminal events:
/// character keys are lowercase, with uppercase input folded into SHIFT.
fn canonicalize(press: KeyPress) -> KeyPress {
    match press.code {
        KeyCode::Char(c) if c.is_uppercase() => KeyPress {
            code: KeyCode::Char(c.to_ascii_lowercase()),
            mods: press.mods | KeyModifiers::SHIFT,
        },
        _ => press,
    }
}

impl KeyPress {
    /// Canonicalize a crossterm event. Returns `None` for anything that is
    /// not a key press (Release/Repeat kinds from enhanced keyboards).
    pub fn from_crossterm(event: &KeyEvent) -> Option<KeyPress> {
        if event.kind != KeyEventKind::Press {
            return None;
        }
        Some(canonicalize(KeyPress {
            code: event.code,
            mods: event.modifiers,
        }))
    }
}

/// Ordered binding table. Built once from config at startup.
#[derive(Debug)]
pub struct Keymap {
    bindings: Vec<(KeySpec, Action)>,
}

/// What a key press means given the chord keys already pending.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolution {
    Action(Action),
    /// Prefix of a chord; caller should buffer the key and wait.
    Pending,
    NoMatch,
}

impl Keymap {
    /// Builds the table from `(action, key specs)` pairs in config order.
    /// Broken or conflicting keys are dropped with a warning; the rest of
    /// the table stays usable (SPEC: a broken key is disabled, not fatal).
    pub fn from_bindings(bindings: &[(Action, Vec<String>)]) -> (Keymap, Vec<String>) {
        let mut table: Vec<(KeySpec, Action)> = Vec::new();
        let mut warnings = Vec::new();

        for (action, specs) in bindings {
            for spec_str in specs {
                let spec = match parse_key_spec(spec_str) {
                    Ok(spec) => spec,
                    Err(e) => {
                        warnings.push(format!("{action:?}: disabled key {spec_str:?}: {e}"));
                        continue;
                    }
                };
                if let Some((_, earlier)) = table.iter().find(|(bound, _)| *bound == spec) {
                    warnings.push(format!(
                        "{action:?}: key {spec_str:?} is already bound to {earlier:?}; \
                         earlier entry wins"
                    ));
                    continue;
                }
                table.push((spec, *action));
            }
        }

        // A chord whose strict prefix is itself a complete binding could never
        // fire (the prefix resolves immediately), so disable it loudly instead
        // of leaving a dead binding.
        let mut kept = Vec::with_capacity(table.len());
        for (spec, action) in &table {
            let shadow = spec.0.len() > 1
                && table
                    .iter()
                    .any(|(other, _)| other.0.len() < spec.0.len() && spec.0.starts_with(&other.0));
            if shadow {
                warnings.push(format!(
                    "{action:?}: chord {:?} is shadowed by a shorter binding on its \
                     first key; chord disabled",
                    key_spec_label(spec)
                ));
            } else {
                kept.push((spec.clone(), *action));
            }
        }

        (Keymap { bindings: kept }, warnings)
    }

    pub fn resolve(&self, pending: &[KeyPress], next: KeyPress) -> Resolution {
        let mut candidate = pending.to_vec();
        candidate.push(next);
        for (spec, action) in &self.bindings {
            if spec.0 == candidate {
                return Resolution::Action(*action);
            }
        }
        let is_chord_prefix = self
            .bindings
            .iter()
            .any(|(spec, _)| spec.0.len() > candidate.len() && spec.0.starts_with(&candidate));
        if is_chord_prefix {
            Resolution::Pending
        } else {
            Resolution::NoMatch
        }
    }

    /// Display label of the first (= highest priority) key bound to
    /// `action`, for the footer hint line.
    pub fn first_binding_label(&self, action: Action) -> Option<String> {
        self.bindings
            .iter()
            .find(|(_, bound)| *bound == action)
            .map(|(spec, _)| key_spec_label(spec))
    }
}

fn key_spec_label(spec: &KeySpec) -> String {
    spec.0
        .iter()
        .map(key_press_label)
        .collect::<Vec<_>>()
        .join(" ")
}

fn key_press_label(press: &KeyPress) -> String {
    let mut label = String::new();
    if press.mods.contains(KeyModifiers::CONTROL) {
        label.push_str("C-");
    }
    if press.mods.contains(KeyModifiers::ALT) {
        label.push_str("M-");
    }
    if press.mods.contains(KeyModifiers::SHIFT) {
        label.push_str("S-");
    }
    if press.mods.contains(KeyModifiers::SUPER) {
        label.push_str("Super-");
    }
    let key = match press.code {
        KeyCode::Char(' ') => "space".to_string(),
        KeyCode::Char(c) => c.to_string(),
        KeyCode::Up => "↑".to_string(),
        KeyCode::Down => "↓".to_string(),
        KeyCode::Left => "←".to_string(),
        KeyCode::Right => "→".to_string(),
        KeyCode::Enter => "enter".to_string(),
        KeyCode::Esc => "esc".to_string(),
        KeyCode::Tab => "tab".to_string(),
        KeyCode::Backspace => "bksp".to_string(),
        KeyCode::Delete => "del".to_string(),
        KeyCode::Home => "home".to_string(),
        KeyCode::End => "end".to_string(),
        KeyCode::PageUp => "pgup".to_string(),
        KeyCode::PageDown => "pgdn".to_string(),
        KeyCode::F(n) => format!("f{n}"),
        other => format!("{other:?}").to_lowercase(),
    };
    label.push_str(&key);
    label
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(code: KeyCode, mods: KeyModifiers) -> KeyPress {
        KeyPress { code, mods }
    }

    fn single(spec: &str) -> KeyPress {
        let parsed = parse_key_spec(spec).unwrap_or_else(|e| panic!("{spec:?} failed: {e}"));
        assert_eq!(parsed.0.len(), 1, "{spec:?} should be a single key");
        parsed.0[0]
    }

    // --- Group A: parsing ---

    #[test]
    fn parses_single_character_key() {
        assert_eq!(single("j"), press(KeyCode::Char('j'), KeyModifiers::NONE));
    }

    #[test]
    fn parses_named_keys() {
        let cases: &[(&str, KeyCode)] = &[
            ("enter", KeyCode::Enter),
            ("esc", KeyCode::Esc),
            ("tab", KeyCode::Tab),
            ("space", KeyCode::Char(' ')),
            ("backspace", KeyCode::Backspace),
            ("delete", KeyCode::Delete),
            ("up", KeyCode::Up),
            ("down", KeyCode::Down),
            ("left", KeyCode::Left),
            ("right", KeyCode::Right),
            ("home", KeyCode::Home),
            ("end", KeyCode::End),
            ("pageup", KeyCode::PageUp),
            ("pagedown", KeyCode::PageDown),
            ("f1", KeyCode::F(1)),
            ("f12", KeyCode::F(12)),
        ];
        for (spec, code) in cases {
            assert_eq!(
                single(spec),
                press(*code, KeyModifiers::NONE),
                "spec {spec:?}"
            );
        }
    }

    #[test]
    fn parses_modifiers() {
        assert_eq!(
            single("ctrl+n"),
            press(KeyCode::Char('n'), KeyModifiers::CONTROL)
        );
        assert_eq!(
            single("alt+x"),
            press(KeyCode::Char('x'), KeyModifiers::ALT)
        );
        assert_eq!(
            single("shift+g"),
            press(KeyCode::Char('g'), KeyModifiers::SHIFT)
        );
        assert_eq!(
            single("super+k"),
            press(KeyCode::Char('k'), KeyModifiers::SUPER)
        );
        assert_eq!(
            single("ctrl+alt+x"),
            press(
                KeyCode::Char('x'),
                KeyModifiers::CONTROL | KeyModifiers::ALT
            )
        );
        assert_eq!(
            single("ctrl+pagedown"),
            press(KeyCode::PageDown, KeyModifiers::CONTROL)
        );
    }

    #[test]
    fn parsing_is_case_insensitive_and_trims() {
        assert_eq!(single("Ctrl+N"), single("ctrl+n"));
        assert_eq!(single("ENTER"), single("enter"));
        assert_eq!(single("  ctrl+d  "), single("ctrl+d"));
    }

    #[test]
    fn uppercase_char_canonicalizes_to_shift_lowercase() {
        assert_eq!(single("G"), press(KeyCode::Char('g'), KeyModifiers::SHIFT));
        assert_eq!(single("G"), single("shift+g"));
    }

    #[test]
    fn parses_chords() {
        assert_eq!(
            parse_key_spec("g g").unwrap(),
            KeySpec(vec![
                press(KeyCode::Char('g'), KeyModifiers::NONE),
                press(KeyCode::Char('g'), KeyModifiers::NONE),
            ])
        );
        assert_eq!(
            parse_key_spec("ctrl+x ctrl+s").unwrap(),
            KeySpec(vec![
                press(KeyCode::Char('x'), KeyModifiers::CONTROL),
                press(KeyCode::Char('s'), KeyModifiers::CONTROL),
            ])
        );
    }

    #[test]
    fn rejects_invalid_specs_with_messages() {
        let unknown_key = parse_key_spec("bogus").unwrap_err();
        assert!(unknown_key.0.contains("bogus"), "message: {unknown_key}");

        let unknown_mod = parse_key_spec("hyper+x").unwrap_err();
        assert!(unknown_mod.0.contains("hyper"), "message: {unknown_mod}");

        assert!(parse_key_spec("").is_err());
        assert!(parse_key_spec("   ").is_err());
        assert!(parse_key_spec("ctrl+").is_err());
    }

    // --- Group A: crossterm canonicalization ---

    #[test]
    fn from_crossterm_accepts_only_press_kind() {
        let mut event = KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE);
        event.kind = KeyEventKind::Press;
        assert_eq!(
            KeyPress::from_crossterm(&event),
            Some(press(KeyCode::Char('j'), KeyModifiers::NONE))
        );

        event.kind = KeyEventKind::Release;
        assert_eq!(KeyPress::from_crossterm(&event), None);

        event.kind = KeyEventKind::Repeat;
        assert_eq!(KeyPress::from_crossterm(&event), None);
    }

    #[test]
    fn from_crossterm_canonicalizes_shifted_characters() {
        let event = KeyEvent::new(KeyCode::Char('G'), KeyModifiers::SHIFT);
        assert_eq!(
            KeyPress::from_crossterm(&event),
            Some(press(KeyCode::Char('g'), KeyModifiers::SHIFT))
        );
        // Terminals may omit the SHIFT bit and just send the uppercase char.
        let event = KeyEvent::new(KeyCode::Char('G'), KeyModifiers::NONE);
        assert_eq!(
            KeyPress::from_crossterm(&event),
            Some(press(KeyCode::Char('g'), KeyModifiers::SHIFT))
        );
    }

    #[test]
    fn from_crossterm_passes_through_plain_keys() {
        let event = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(
            KeyPress::from_crossterm(&event),
            Some(press(KeyCode::Enter, KeyModifiers::NONE))
        );
        let event = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert_eq!(
            KeyPress::from_crossterm(&event),
            Some(press(KeyCode::Char('c'), KeyModifiers::CONTROL))
        );
    }

    // --- Group B: keymap resolution ---

    fn bindings(entries: &[(Action, &[&str])]) -> Vec<(Action, Vec<String>)> {
        entries
            .iter()
            .map(|(a, keys)| (*a, keys.iter().map(|s| s.to_string()).collect()))
            .collect()
    }

    fn keymap(entries: &[(Action, &[&str])]) -> Keymap {
        let (keymap, warnings) = Keymap::from_bindings(&bindings(entries));
        assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");
        keymap
    }

    #[test]
    fn resolves_single_bindings_and_rejects_unbound_keys() {
        let km = keymap(&[(Action::Down, &["j"]), (Action::Accept, &["enter"])]);
        assert_eq!(
            km.resolve(&[], single("j")),
            Resolution::Action(Action::Down)
        );
        assert_eq!(km.resolve(&[], single("x")), Resolution::NoMatch);
    }

    #[test]
    fn resolves_any_of_multiple_keys_per_action() {
        let km = keymap(&[(Action::Down, &["down", "ctrl+n", "j"])]);
        for spec in ["down", "ctrl+n", "j"] {
            assert_eq!(
                km.resolve(&[], single(spec)),
                Resolution::Action(Action::Down),
                "spec {spec:?}"
            );
        }
    }

    #[test]
    fn conflicting_key_goes_to_earlier_action_with_warning() {
        let (km, warnings) = Keymap::from_bindings(&bindings(&[
            (Action::Down, &["j"]),
            (Action::Up, &["j", "k"]),
        ]));
        assert_eq!(
            km.resolve(&[], single("j")),
            Resolution::Action(Action::Down)
        );
        assert_eq!(km.resolve(&[], single("k")), Resolution::Action(Action::Up));
        assert_eq!(warnings.len(), 1);
        assert!(
            warnings[0].contains("Down") && warnings[0].contains("Up"),
            "warning should name both actions: {}",
            warnings[0]
        );
    }

    #[test]
    fn invalid_key_string_is_disabled_but_rest_survive() {
        let (km, warnings) = Keymap::from_bindings(&bindings(&[(Action::Down, &["bogus", "j"])]));
        assert_eq!(
            km.resolve(&[], single("j")),
            Resolution::Action(Action::Down)
        );
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("bogus"), "warning: {}", warnings[0]);
    }

    #[test]
    fn chords_resolve_through_pending_state() {
        let km = keymap(&[(Action::Top, &["g g"]), (Action::Down, &["j"])]);
        let g = single("g");
        assert_eq!(km.resolve(&[], g), Resolution::Pending);
        assert_eq!(km.resolve(&[g], g), Resolution::Action(Action::Top));
        assert_eq!(km.resolve(&[g], single("x")), Resolution::NoMatch);
    }

    #[test]
    fn exact_binding_shadows_chord_prefix() {
        let (km, warnings) = Keymap::from_bindings(&bindings(&[
            (Action::Top, &["g g"]),
            (Action::Bottom, &["g"]),
        ]));
        assert_eq!(
            km.resolve(&[], single("g")),
            Resolution::Action(Action::Bottom)
        );
        assert_eq!(warnings.len(), 1);
        assert!(
            warnings[0].contains("Top"),
            "warning should name the disabled chord's action: {}",
            warnings[0]
        );
    }

    #[test]
    fn first_binding_label_reflects_config_order() {
        let km = keymap(&[
            (Action::Up, &["up", "ctrl+p", "k"]),
            (Action::Down, &["ctrl+n"]),
            (Action::Top, &["g g"]),
            (Action::Accept, &["enter"]),
        ]);
        assert_eq!(km.first_binding_label(Action::Up).as_deref(), Some("↑"));
        assert_eq!(km.first_binding_label(Action::Down).as_deref(), Some("C-n"));
        assert_eq!(km.first_binding_label(Action::Top).as_deref(), Some("g g"));
        assert_eq!(
            km.first_binding_label(Action::Accept).as_deref(),
            Some("enter")
        );
        assert_eq!(km.first_binding_label(Action::Cancel), None);
    }
}
