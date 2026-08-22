use crate::utils::pure_scores;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};
use unicode_segmentation::UnicodeSegmentation;

/// Shortens `input` so the result is at most `max_length` graphemes, ending in
/// `ellipsis`.
///
/// The budget covers the ellipsis, so the result never runs past what the caller
/// asked for — a label given twenty characters stays inside twenty. Graphemes,
/// not chars, because cutting a flag or a combining accent in half produces a
/// broken glyph rather than a shorter string.
pub fn ellipsize(input: &str, max_length: i64, ellipsis: &str) -> String {
    if max_length <= 0 {
        return String::new();
    }
    let max_length = max_length as usize;

    let graphemes: Vec<&str> = input.graphemes(true).collect();
    if graphemes.len() <= max_length {
        return input.to_string();
    }

    let ellipsis_length = ellipsis.graphemes(true).count();
    if ellipsis_length >= max_length {
        // No room for any of the string: hand back as much of the marker as fits.
        return ellipsis.graphemes(true).take(max_length).collect();
    }

    let kept: String = graphemes[..max_length - ellipsis_length].concat();
    format!("{}{}", kept.trim_end(), ellipsis)
}

#[crate::register_node]
#[derive(Default)]
pub struct StringEllipsisNode {}

impl StringEllipsisNode {
    pub fn new() -> Self {
        StringEllipsisNode {}
    }
}

#[async_trait]
impl NodeLogic for StringEllipsisNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "string_ellipsis",
            "Ellipsis",
            "Shortens a string that is longer than the given number of characters and marks the cut with an ellipsis. A string that already fits is returned unchanged",
            "Utils/String",
        );
        node.add_icon("/flow/icons/string.svg");
        node.set_scores(pure_scores());

        node.add_input_pin("string", "String", "Input String", VariableType::String);
        node.add_input_pin(
            "max_length",
            "Max Length",
            "Longest the result may be, counted in characters and including the ellipsis itself",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(100)));
        node.add_input_pin(
            "ellipsis",
            "Ellipsis",
            "Appended in place of what was cut",
            VariableType::String,
        )
        .set_default_value(Some(json!("…")));

        node.add_output_pin(
            "result",
            "Result",
            "The shortened string, or the input unchanged when it already fits",
            VariableType::String,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let string: String = context.evaluate_pin("string").await?;
        let max_length: i64 = context.evaluate_pin("max_length").await?;
        let ellipsis: String = context.evaluate_pin("ellipsis").await?;

        context
            .set_pin_value("result", json!(ellipsize(&string, max_length, &ellipsis)))
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_string_that_fits_is_returned_untouched() {
        assert_eq!(ellipsize("hello", 5, "…"), "hello");
        assert_eq!(ellipsize("hello", 50, "…"), "hello");
        assert_eq!(ellipsize("", 5, "…"), "");
    }

    /// The budget is the whole result, so nothing downstream has to know the
    /// ellipsis was going to be added.
    #[test]
    fn the_result_never_runs_past_the_budget() {
        for max in 1..20i64 {
            let result = ellipsize("the quick brown fox jumps", max, "…");
            assert!(
                result.graphemes(true).count() <= max as usize,
                "{max} produced {result:?}"
            );
        }
    }

    #[test]
    fn a_longer_string_is_cut_and_marked() {
        assert_eq!(ellipsize("hello world", 8, "…"), "hello w…");
        assert_eq!(ellipsize("hello world", 10, "..."), "hello w...");
    }

    /// Cutting mid-space would leave "hello …".
    #[test]
    fn trailing_space_is_dropped_before_the_marker() {
        assert_eq!(ellipsize("hello world", 7, "…"), "hello…");
    }

    /// The cut must land between glyphs, not inside one.
    #[test]
    fn multi_codepoint_glyphs_are_never_split() {
        let flags = "🇩🇪🇫🇷🇮🇹🇪🇸";
        assert_eq!(ellipsize(flags, 3, "…"), "🇩🇪🇫🇷…");
        assert_eq!(ellipsize("éclair", 4, "…"), "écl…");
    }

    #[test]
    fn a_budget_too_small_for_the_marker_yields_what_fits() {
        assert_eq!(ellipsize("hello world", 3, "..."), "...");
        assert_eq!(ellipsize("hello world", 2, "..."), "..");
        assert_eq!(ellipsize("hello world", 0, "…"), "");
        assert_eq!(ellipsize("hello world", -5, "…"), "");
    }

    #[test]
    fn an_empty_marker_just_truncates() {
        assert_eq!(ellipsize("hello world", 5, ""), "hello");
    }
}
