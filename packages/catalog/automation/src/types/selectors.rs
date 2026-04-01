use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Eq, Default)]
pub enum SelectorKind {
    #[default]
    Css,
    Xpath,
    Text,
    TextExact,
    Role,
    TestId,
    AriaLabel,
    Placeholder,
    AltText,
    Title,
    Image,
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

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, Default)]
pub struct SelectorSet {
    pub selectors: Vec<Selector>,
    pub fallback_order: Vec<usize>,
}

impl SelectorSet {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(mut self, selector: Selector) -> Self {
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
