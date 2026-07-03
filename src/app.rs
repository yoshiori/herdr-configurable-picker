//! Pure input state machine: keys go in, an [`Outcome`] comes out.
//! No terminal, no socket — fully unit-testable.

use crate::keymap::{Action, KeyPress, Keymap, Resolution};
use crate::model::Item;

/// What the event loop should do after a key press.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    Continue,
    FocusTab(String),
    Cancel,
}

#[derive(Debug)]
pub struct App {
    pub items: Vec<Item>,
    pub cursor: usize,
    /// Keys buffered while a chord is in flight.
    pub pending: Vec<KeyPress>,
    /// Rows the list area can show; set by the UI on each draw so that
    /// page movements track the real terminal size.
    pub viewport_height: u16,
}

impl App {
    pub fn new(items: Vec<Item>, cursor: usize) -> App {
        let cursor = cursor.min(items.len().saturating_sub(1));
        App {
            items,
            cursor,
            pending: Vec::new(),
            viewport_height: 0,
        }
    }

    pub fn handle_key(&mut self, keymap: &Keymap, key: KeyPress) -> Outcome {
        if self.items.is_empty() {
            // SPEC "Empty tree": show the message, close on any key.
            return Outcome::Cancel;
        }
        match keymap.resolve(&self.pending, key) {
            Resolution::Action(action) => {
                self.pending.clear();
                self.apply(action)
            }
            Resolution::Pending => {
                self.pending.push(key);
                Outcome::Continue
            }
            Resolution::NoMatch => {
                // A failed chord swallows the key: firing its standalone
                // binding instead would be a surprising double meaning.
                self.pending.clear();
                Outcome::Continue
            }
        }
    }

    fn apply(&mut self, action: Action) -> Outcome {
        let last = self.items.len() - 1;
        let page = (self.viewport_height as usize).max(1);
        match action {
            Action::Down => self.cursor = (self.cursor + 1).min(last),
            Action::Up => self.cursor = self.cursor.saturating_sub(1),
            Action::PageDown => self.cursor = (self.cursor + page).min(last),
            Action::PageUp => self.cursor = self.cursor.saturating_sub(page),
            Action::Top => self.cursor = 0,
            Action::Bottom => self.cursor = last,
            Action::Accept => return Outcome::FocusTab(self.items[self.cursor].tab_id.clone()),
            Action::Cancel => return Outcome::Cancel,
        }
        Outcome::Continue
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::KeysConfig;
    use crate::herdr_client::AgentStatus;
    use crate::keymap::parse_key_spec;

    fn item(tab_id: &str) -> Item {
        Item {
            tab_id: tab_id.to_string(),
            workspace_label: "ws".to_string(),
            tab_label: tab_id.to_string(),
            pane_count: 1,
            agent_status: AgentStatus::Idle,
            is_current: false,
        }
    }

    fn app() -> App {
        let items = (1..=5).map(|n| item(&format!("w1:t{n}"))).collect();
        let mut app = App::new(items, 0);
        app.viewport_height = 3;
        app
    }

    fn default_keymap() -> Keymap {
        let (keymap, warnings) = Keymap::from_bindings(&KeysConfig::default().to_bindings());
        assert!(warnings.is_empty(), "default config must be warning-free");
        keymap
    }

    fn press(app: &mut App, keymap: &Keymap, spec: &str) -> Outcome {
        let keys = parse_key_spec(spec).unwrap();
        let mut outcome = Outcome::Continue;
        for key in keys.0 {
            outcome = app.handle_key(keymap, key);
        }
        outcome
    }

    #[test]
    fn down_and_up_move_and_clamp() {
        let keymap = default_keymap();
        let mut app = app();

        assert_eq!(press(&mut app, &keymap, "up"), Outcome::Continue);
        assert_eq!(app.cursor, 0, "up clamps at the top");

        press(&mut app, &keymap, "j");
        press(&mut app, &keymap, "ctrl+n");
        assert_eq!(app.cursor, 2, "all down bindings move");

        for _ in 0..10 {
            press(&mut app, &keymap, "down");
        }
        assert_eq!(app.cursor, 4, "down clamps at the bottom");
    }

    #[test]
    fn top_and_bottom_jump() {
        let keymap = default_keymap();
        let mut app = app();

        press(&mut app, &keymap, "end");
        assert_eq!(app.cursor, 4);
        press(&mut app, &keymap, "home");
        assert_eq!(app.cursor, 0);
        press(&mut app, &keymap, "shift+g");
        assert_eq!(app.cursor, 4);
    }

    #[test]
    fn page_movement_uses_viewport_height() {
        let keymap = default_keymap();
        let mut app = app();

        press(&mut app, &keymap, "ctrl+d");
        assert_eq!(app.cursor, 3, "page down moves by viewport height");
        press(&mut app, &keymap, "ctrl+d");
        assert_eq!(app.cursor, 4, "page down clamps at the bottom");
        press(&mut app, &keymap, "ctrl+u");
        assert_eq!(app.cursor, 1);
        press(&mut app, &keymap, "ctrl+u");
        assert_eq!(app.cursor, 0, "page up clamps at the top");
    }

    #[test]
    fn accept_returns_the_selected_tab() {
        let keymap = default_keymap();
        let mut app = app();

        press(&mut app, &keymap, "j");
        assert_eq!(
            press(&mut app, &keymap, "enter"),
            Outcome::FocusTab("w1:t2".to_string())
        );
    }

    #[test]
    fn cancel_keys_cancel() {
        let keymap = default_keymap();
        let mut app = app();

        assert_eq!(press(&mut app, &keymap, "esc"), Outcome::Cancel);
        assert_eq!(press(&mut app, &keymap, "ctrl+c"), Outcome::Cancel);
    }

    #[test]
    fn chords_accumulate_and_mismatches_clear_pending() {
        let (keymap, warnings) = Keymap::from_bindings(&[
            (Action::Top, vec!["g g".to_string()]),
            (Action::Cancel, vec!["esc".to_string()]),
            (Action::Down, vec!["j".to_string()]),
        ]);
        assert!(warnings.is_empty());
        let mut app = app();
        press(&mut app, &keymap, "j");
        press(&mut app, &keymap, "j");
        assert_eq!(app.cursor, 2);

        // First "g" is pending, second completes the chord.
        assert_eq!(press(&mut app, &keymap, "g"), Outcome::Continue);
        assert_eq!(app.pending.len(), 1);
        assert_eq!(press(&mut app, &keymap, "g"), Outcome::Continue);
        assert_eq!(app.cursor, 0, "g g jumps to the top");
        assert!(app.pending.is_empty());

        // A mismatch swallows the key: "g j" neither tops nor moves down.
        press(&mut app, &keymap, "j"); // cursor -> 1
        press(&mut app, &keymap, "g");
        assert_eq!(press(&mut app, &keymap, "j"), Outcome::Continue);
        assert_eq!(
            app.cursor, 1,
            "the mismatched key must not fire its own binding"
        );
        assert!(app.pending.is_empty());

        // Esc during a pending chord clears the chord instead of cancelling.
        press(&mut app, &keymap, "g");
        assert_eq!(press(&mut app, &keymap, "esc"), Outcome::Continue);
        assert!(app.pending.is_empty());
        assert_eq!(press(&mut app, &keymap, "esc"), Outcome::Cancel);
    }

    #[test]
    fn unbound_keys_do_nothing() {
        let keymap = default_keymap();
        let mut app = app();
        assert_eq!(press(&mut app, &keymap, "x"), Outcome::Continue);
        assert_eq!(app.cursor, 0);
    }

    #[test]
    fn empty_list_cancels_on_any_key() {
        let keymap = default_keymap();
        let mut app = App::new(Vec::new(), 0);
        assert_eq!(press(&mut app, &keymap, "x"), Outcome::Cancel);
        assert_eq!(press(&mut app, &keymap, "enter"), Outcome::Cancel);
    }
}
