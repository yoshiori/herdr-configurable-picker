//! Search matching, mirroring the built-in's `text_matches_query`:
//! whitespace-split needles, ALL of which must appear (case-insensitive)
//! in the haystack.

/// True when every whitespace-separated word of the query appears in the
/// text. Allocation-free for hot loops: both sides must already be
/// lowercase (`Row.search_text` is stored that way, and callers lowercase
/// the query once per pass). An empty query matches everything.
pub fn lowered_query_matches(lowered_text: &str, lowered_query: &str) -> bool {
    lowered_query
        .split_whitespace()
        .all(|needle| lowered_text.contains(needle))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Case-insensitive convenience for the tests below.
    fn query_matches(text: &str, query: &str) -> bool {
        lowered_query_matches(&text.to_lowercase(), &query.to_lowercase())
    }

    #[test]
    fn matches_case_insensitive_substrings() {
        assert!(query_matches("picker", "ick"));
        assert!(query_matches("Picker", "pick"));
        assert!(query_matches("picker", "ICK"));
        assert!(!query_matches("picker", "pique"));
    }

    #[test]
    fn empty_query_matches_everything() {
        assert!(query_matches("anything", ""));
        assert!(query_matches("", ""));
        assert!(query_matches("x", "   "));
    }

    #[test]
    fn multiple_words_all_must_match_in_any_order() {
        let text = "picker claude · working";
        assert!(query_matches(text, "pick work"));
        assert!(query_matches(text, "working claude"));
        assert!(!query_matches(text, "pick idle"));
    }

    #[test]
    fn works_on_non_ascii_labels() {
        assert!(query_matches("日本語ラベル claude", "ラベル claude"));
        assert!(!query_matches("日本語ラベル", "英語"));
    }
}
