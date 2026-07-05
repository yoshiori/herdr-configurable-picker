//! Status icon sets (`[display] icon_set`), matching the built-in goto's
//! mapping (`agent_icon` in herdr's ui/status.rs):
//! blocked ◉ · working spinner · done ● · idle ✓ · unknown ○.

use crate::herdr_client::AgentStatus;

/// The built-in's braille spinner frames; `tick` advances one frame per
/// redraw (~8/s under the poll-driven event loop).
const NERD_SPINNER: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
const ASCII_SPINNER: &[&str] = &["|", "/", "-", "\\"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IconSet {
    Nerd,
    Ascii,
    Emoji,
}

impl IconSet {
    pub fn parse(text: &str) -> Option<IconSet> {
        match text {
            "nerd" => Some(IconSet::Nerd),
            "ascii" => Some(IconSet::Ascii),
            "emoji" => Some(IconSet::Emoji),
            _ => None,
        }
    }

    pub fn icon(self, status: AgentStatus, tick: u32) -> &'static str {
        match (self, status) {
            (IconSet::Nerd, AgentStatus::Blocked) => "◉",
            (IconSet::Nerd, AgentStatus::Working) => {
                NERD_SPINNER[tick as usize % NERD_SPINNER.len()]
            }
            (IconSet::Nerd, AgentStatus::Done) => "●",
            (IconSet::Nerd, AgentStatus::Idle) => "✓",
            (IconSet::Nerd, AgentStatus::Unknown) => "○",
            (IconSet::Ascii, AgentStatus::Blocked) => "!",
            (IconSet::Ascii, AgentStatus::Working) => {
                ASCII_SPINNER[tick as usize % ASCII_SPINNER.len()]
            }
            (IconSet::Ascii, AgentStatus::Done) => "*",
            (IconSet::Ascii, AgentStatus::Idle) => "v",
            (IconSet::Ascii, AgentStatus::Unknown) => "o",
            (IconSet::Emoji, AgentStatus::Blocked) => "🔴",
            (IconSet::Emoji, AgentStatus::Working) => "🟡",
            (IconSet::Emoji, AgentStatus::Done) => "🔵",
            (IconSet::Emoji, AgentStatus::Idle) => "✅",
            (IconSet::Emoji, AgentStatus::Unknown) => "⚪",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_known_sets_only() {
        assert_eq!(IconSet::parse("nerd"), Some(IconSet::Nerd));
        assert_eq!(IconSet::parse("ascii"), Some(IconSet::Ascii));
        assert_eq!(IconSet::parse("emoji"), Some(IconSet::Emoji));
        assert_eq!(IconSet::parse("comic-sans"), None);
    }

    #[test]
    fn every_set_covers_every_status() {
        for set in [IconSet::Nerd, IconSet::Ascii, IconSet::Emoji] {
            for status in [
                AgentStatus::Idle,
                AgentStatus::Working,
                AgentStatus::Blocked,
                AgentStatus::Done,
                AgentStatus::Unknown,
            ] {
                assert!(!set.icon(status, 0).is_empty(), "{set:?}/{status:?}");
            }
        }
    }

    #[test]
    fn nerd_set_matches_the_builtin_agent_icons() {
        assert_eq!(IconSet::Nerd.icon(AgentStatus::Blocked, 0), "◉");
        assert_eq!(IconSet::Nerd.icon(AgentStatus::Done, 0), "●");
        assert_eq!(IconSet::Nerd.icon(AgentStatus::Idle, 0), "✓");
        assert_eq!(IconSet::Nerd.icon(AgentStatus::Unknown, 0), "○");
    }

    #[test]
    fn working_spins_with_the_tick() {
        assert_eq!(IconSet::Nerd.icon(AgentStatus::Working, 0), "⠋");
        assert_eq!(IconSet::Nerd.icon(AgentStatus::Working, 1), "⠙");
        assert_eq!(
            IconSet::Nerd.icon(AgentStatus::Working, 10),
            "⠋",
            "wraps around"
        );
        assert_eq!(IconSet::Ascii.icon(AgentStatus::Working, 1), "/");
        // Static everywhere else.
        assert_eq!(
            IconSet::Nerd.icon(AgentStatus::Idle, 3),
            IconSet::Nerd.icon(AgentStatus::Idle, 0)
        );
    }
}
