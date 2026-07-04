//! Status icon sets (`[display] icon_set`).

use crate::herdr_client::AgentStatus;

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

    pub fn icon(self, status: AgentStatus) -> &'static str {
        match (self, status) {
            (IconSet::Nerd, AgentStatus::Idle) => "○",
            (IconSet::Nerd, AgentStatus::Working) => "●",
            (IconSet::Nerd, AgentStatus::Blocked) => "✗",
            (IconSet::Nerd, AgentStatus::Done) => "✓",
            (IconSet::Nerd, AgentStatus::Unknown) => "·",
            (IconSet::Ascii, AgentStatus::Idle) => "o",
            (IconSet::Ascii, AgentStatus::Working) => "+",
            (IconSet::Ascii, AgentStatus::Blocked) => "x",
            (IconSet::Ascii, AgentStatus::Done) => "v",
            (IconSet::Ascii, AgentStatus::Unknown) => "-",
            (IconSet::Emoji, AgentStatus::Idle) => "⚪",
            (IconSet::Emoji, AgentStatus::Working) => "🟢",
            (IconSet::Emoji, AgentStatus::Blocked) => "❌",
            (IconSet::Emoji, AgentStatus::Done) => "✅",
            (IconSet::Emoji, AgentStatus::Unknown) => "⚫",
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
                assert!(!set.icon(status).is_empty(), "{set:?}/{status:?}");
            }
        }
    }

    #[test]
    fn spec_examples_hold() {
        assert_eq!(IconSet::Nerd.icon(AgentStatus::Working), "●");
        assert_eq!(IconSet::Ascii.icon(AgentStatus::Blocked), "x");
        assert_eq!(IconSet::Emoji.icon(AgentStatus::Done), "✅");
    }
}
