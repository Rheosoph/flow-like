use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Eq, Default)]
pub enum SelectorKind {
    #[default]
    Css,
    Xpath,
    /// Visible text contained in the element; script, style and head content is ignored.
    Text,
    TextExact,
    /// ARIA role, optionally filtered by accessible name: `button`, `button|Sign in`
    /// (case-insensitive substring), `button|=Sign in` (exact) or `button|/^sign/i` (regex).
    Role,
    TestId,
    AriaLabel,
    Placeholder,
    AltText,
    Title,
    Image,
    /// Element ref from the latest Browser Snapshot, such as `e12`.
    Ref,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct Selector {
    pub kind: SelectorKind,
    pub value: String,
    pub confidence: Option<f64>,
    pub scope: Option<String>,
}

impl Selector {
    pub fn css(value: impl Into<String>) -> Self {
        Self {
            kind: SelectorKind::Css,
            value: value.into(),
            confidence: Some(1.0),
            scope: None,
        }
    }

    pub fn xpath(value: impl Into<String>) -> Self {
        Self {
            kind: SelectorKind::Xpath,
            value: value.into(),
            confidence: Some(1.0),
            scope: None,
        }
    }

    pub fn text(value: impl Into<String>) -> Self {
        Self {
            kind: SelectorKind::Text,
            value: value.into(),
            confidence: Some(0.9),
            scope: None,
        }
    }

    pub fn role(value: impl Into<String>) -> Self {
        Self {
            kind: SelectorKind::Role,
            value: value.into(),
            confidence: Some(0.8),
            scope: None,
        }
    }

    pub fn element_ref(value: impl Into<String>) -> Self {
        Self {
            kind: SelectorKind::Ref,
            value: value.into(),
            confidence: Some(1.0),
            scope: None,
        }
    }

    /// Treats `e12` / `ref=e12` as a snapshot ref and anything else as CSS.
    pub fn from_css_or_ref(value: &str) -> Self {
        match normalize_ref(value) {
            Some(reference) => Self::element_ref(reference),
            None => Self::css(value),
        }
    }

    pub fn test_id(value: impl Into<String>) -> Self {
        Self {
            kind: SelectorKind::TestId,
            value: value.into(),
            confidence: Some(1.0),
            scope: None,
        }
    }

    pub fn with_confidence(mut self, confidence: f64) -> Self {
        self.confidence = Some(confidence);
        self
    }

    pub fn with_scope(mut self, scope: impl Into<String>) -> Self {
        self.scope = Some(scope.into());
        self
    }
}

/// Accepts `e12`, `ref=e12`, `[ref=e12]` and `@e12`; returns the bare ref.
pub fn normalize_ref(value: &str) -> Option<String> {
    let trimmed = value.trim();
    let trimmed = trimmed
        .strip_prefix('[')
        .and_then(|inner| inner.strip_suffix(']'))
        .unwrap_or(trimmed)
        .trim();
    let bare = trimmed
        .strip_prefix("ref=")
        .or_else(|| trimmed.strip_prefix('@'))
        .unwrap_or(trimmed);
    let digits = bare.strip_prefix('e')?;
    (!digits.is_empty() && digits.len() <= 9 && digits.bytes().all(|b| b.is_ascii_digit()))
        .then(|| bare.to_string())
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, Default)]
pub struct SelectorSet {
    pub selectors: Vec<Selector>,
    pub fallback_order: Vec<usize>,
}

impl SelectorSet {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_selector(mut self, selector: Selector) -> Self {
        let idx = self.selectors.len();
        self.selectors.push(selector);
        self.fallback_order.push(idx);
        self
    }

    pub fn with_fallback_order(mut self, order: Vec<usize>) -> Self {
        self.fallback_order = order;
        self
    }

    pub fn primary(&self) -> Option<&Selector> {
        self.fallback_order
            .first()
            .and_then(|&idx| self.selectors.get(idx))
    }

    pub fn iter_by_priority(&self) -> impl Iterator<Item = &Selector> {
        self.fallback_order
            .iter()
            .filter_map(|&idx| self.selectors.get(idx))
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct RankedSelector {
    pub selector: Selector,
    pub rank: usize,
    pub score: f64,
    pub reason: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct RankedSelectorSet {
    pub ranked: Vec<RankedSelector>,
    pub context_hash: Option<String>,
}

impl RankedSelectorSet {
    pub fn new(ranked: Vec<RankedSelector>) -> Self {
        Self {
            ranked,
            context_hash: None,
        }
    }

    pub fn best(&self) -> Option<&RankedSelector> {
        self.ranked.first()
    }

    pub fn to_selector_set(&self) -> SelectorSet {
        let selectors: Vec<Selector> = self.ranked.iter().map(|r| r.selector.clone()).collect();
        let fallback_order: Vec<usize> = (0..selectors.len()).collect();
        SelectorSet {
            selectors,
            fallback_order,
        }
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct SelectorBuildOptions {
    pub from: SelectorSource,
    pub include_text: bool,
    pub include_attributes: bool,
    pub include_position: bool,
    pub max_depth: Option<usize>,
    pub preferred_attributes: Option<Vec<String>>,
}

impl Default for SelectorBuildOptions {
    fn default() -> Self {
        Self {
            from: SelectorSource::Dom,
            include_text: true,
            include_attributes: true,
            include_position: false,
            max_depth: Some(5),
            preferred_attributes: Some(vec![
                "data-testid".to_string(),
                "id".to_string(),
                "name".to_string(),
                "aria-label".to_string(),
            ]),
        }
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, Default)]
pub enum SelectorSource {
    #[default]
    Dom,
    Accessibility,
    Role,
    Text,
    Xpath,
    Css,
    Image,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ref_values_are_normalized() {
        assert_eq!(normalize_ref("e12").as_deref(), Some("e12"));
        assert_eq!(normalize_ref(" ref=e3 ").as_deref(), Some("e3"));
        assert_eq!(normalize_ref("[ref=e7]").as_deref(), Some("e7"));
        assert_eq!(normalize_ref("@e1").as_deref(), Some("e1"));
        assert_eq!(normalize_ref("e"), None);
        assert_eq!(normalize_ref("e1a"), None);
        assert_eq!(normalize_ref("#e12"), None);
        assert_eq!(Selector::from_css_or_ref("e5").kind, SelectorKind::Ref);
        assert_eq!(Selector::from_css_or_ref("div.e5").kind, SelectorKind::Css);
    }

    #[test]
    fn selector_kind_serde_names_are_stable() {
        let kinds = [
            (SelectorKind::Css, "Css"),
            (SelectorKind::TextExact, "TextExact"),
            (SelectorKind::Image, "Image"),
            (SelectorKind::Ref, "Ref"),
        ];
        for (kind, name) in kinds {
            assert_eq!(flow_like_types::json::to_value(&kind).unwrap(), name);
        }
    }
}
