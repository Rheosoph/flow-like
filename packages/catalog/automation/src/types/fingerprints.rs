use super::selectors::SelectorSet;
use chrono::{DateTime, Utc};
use flow_like_catalog_core::BoundingBox;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct ElementFingerprint {
    pub id: String,
    pub selectors: SelectorSet,
    pub role: Option<String>,
    pub name: Option<String>,
    pub text: Option<String>,
    pub bounding_box: Option<BoundingBox>,
    pub nearby_text: Vec<String>,
    pub template_ref: Option<String>,
    pub dom_path: Option<String>,
    pub ax_path: Option<String>,
    pub attributes: HashMap<String, String>,
    pub tag_name: Option<String>,
    pub inner_text: Option<String>,
    pub computed_styles: Option<HashMap<String, String>>,
    pub created_at: DateTime<Utc>,
    pub last_matched_at: Option<DateTime<Utc>>,
    pub match_count: u32,
}

impl ElementFingerprint {
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            selectors: SelectorSet::default(),
            role: None,
            name: None,
            text: None,
            bounding_box: None,
            nearby_text: Vec::new(),
            template_ref: None,
            dom_path: None,
            ax_path: None,
            attributes: HashMap::new(),
            tag_name: None,
            inner_text: None,
            computed_styles: None,
            created_at: Utc::now(),
            last_matched_at: None,
            match_count: 0,
        }
    }

    pub fn with_selectors(mut self, selectors: SelectorSet) -> Self {
        self.selectors = selectors;
        self
    }

    pub fn with_role(mut self, role: impl Into<String>) -> Self {
        self.role = Some(role.into());
        self
    }

    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    pub fn with_text(mut self, text: impl Into<String>) -> Self {
        self.text = Some(text.into());
        self
    }

    pub fn with_bounding_box(mut self, bbox: BoundingBox) -> Self {
        self.bounding_box = Some(bbox);
        self
    }

    pub fn with_template_ref(mut self, template_ref: impl Into<String>) -> Self {
        self.template_ref = Some(template_ref.into());
        self
    }

    pub fn record_match(&mut self) {
        self.last_matched_at = Some(Utc::now());
        self.match_count += 1;
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Eq, Default)]
pub enum MatchStrategy {
    Dom,
    Accessibility,
    Vision,
    #[default]
    Hybrid,
    LlmAssisted,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct TargetRef {
    pub fingerprint_id: String,
    pub resolved_selector: super::selectors::Selector,
    pub bounding_box: BoundingBox,
    pub confidence: f64,
    pub strategy_used: MatchStrategy,
    pub fallback_attempts: u32,
    pub resolution_time_ms: u64,
}

impl TargetRef {
    pub fn center(&self) -> (i32, i32) {
        (
            ((self.bounding_box.x1 + self.bounding_box.x2) / 2.0) as i32,
            ((self.bounding_box.y1 + self.bounding_box.y2) / 2.0) as i32,
        )
    }

    pub fn is_high_confidence(&self) -> bool {
        self.confidence >= 0.9
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct FingerprintMatchOptions {
    pub strategy: MatchStrategy,
    pub min_confidence: f64,
    pub max_fallback_attempts: u32,
    pub timeout_ms: u64,
    pub search_region: Option<BoundingBox>,
}

impl Default for FingerprintMatchOptions {
    fn default() -> Self {
        Self {
            strategy: MatchStrategy::Hybrid,
            min_confidence: 0.8,
            max_fallback_attempts: 3,
            timeout_ms: 10000,
            search_region: None,
        }
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, Default)]
pub struct ContextSignals {
    pub page_url: Option<String>,
    pub page_title: Option<String>,
    pub viewport_size: Option<(u32, u32)>,
    pub dom_snapshot: Option<String>,
    pub ax_snapshot: Option<String>,
    pub screenshot_ref: Option<String>,
    pub cursor_position: Option<(i32, i32)>,
    pub focus_element: Option<String>,
}
