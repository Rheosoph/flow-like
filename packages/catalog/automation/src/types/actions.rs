use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::fingerprints::ElementFingerprint;

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Eq)]
pub enum ActionType {
    Click,
    DoubleClick,
    RightClick,
    Type,
    Fill,
    Press,
    Hover,
    Drag,
    Scroll,
    Focus,
    Select,
    Check,
    Uncheck,
    Upload,
    Download,
    Navigate,
    Wait,
    Screenshot,
    Custom(String),
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct Action {
    pub action_type: ActionType,
    pub target: Option<ElementFingerprint>,
    pub value: Option<String>,
    pub options: ActionOptions,
    pub description: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, Default)]
pub struct ActionOptions {
    pub delay_ms: Option<u64>,
    pub timeout_ms: Option<u64>,
    pub force: bool,
    pub no_wait_after: bool,
    pub modifiers: Option<Vec<KeyModifier>>,
    pub button: Option<MouseButton>,
    pub click_count: Option<u32>,
    pub position: Option<(i32, i32)>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Eq)]
pub enum KeyModifier {
    Shift,
    Control,
    Alt,
    Meta,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Eq, Default)]
pub enum MouseButton {
    #[default]
    Left,
    Right,
    Middle,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct ActionPlan {
    pub plan_id: String,
    pub goal: String,
    pub actions: Vec<Action>,
    pub confidence: f64,
    pub reasoning: Option<String>,
    pub alternatives: Vec<ActionPlan>,
}

impl ActionPlan {
    pub fn new(plan_id: impl Into<String>, goal: impl Into<String>) -> Self {
        Self {
            plan_id: plan_id.into(),
            goal: goal.into(),
            actions: Vec::new(),
            confidence: 0.0,
            reasoning: None,
            alternatives: Vec::new(),
        }
    }

    pub fn add_action(mut self, action: Action) -> Self {
        self.actions.push(action);
        self
    }

    pub fn with_confidence(mut self, confidence: f64) -> Self {
        self.confidence = confidence;
        self
    }

    pub fn with_reasoning(mut self, reasoning: impl Into<String>) -> Self {
        self.reasoning = Some(reasoning.into());
        self
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Eq)]
pub enum ScreenState {
    Loading,
    Ready,
    Error,
    Dialog,
    Popup,
    Login,
    Captcha,
    Empty,
    Custom(String),
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct ScreenClassification {
    pub state: ScreenState,
    pub confidence: f64,
    pub indicators: Vec<String>,
    pub suggested_action: Option<String>,
}

impl ScreenClassification {
    pub fn new(state: ScreenState, confidence: f64) -> Self {
        Self {
            state,
            confidence,
            indicators: Vec::new(),
            suggested_action: None,
        }
    }

    pub fn with_indicator(mut self, indicator: impl Into<String>) -> Self {
        self.indicators.push(indicator.into());
        self
    }

    pub fn with_suggested_action(mut self, action: impl Into<String>) -> Self {
        self.suggested_action = Some(action.into());
        self
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct SelectorSetCandidate {
    pub selectors: super::selectors::SelectorSet,
    pub confidence: f64,
    pub reasoning: String,
    pub tested: bool,
    pub test_result: Option<bool>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct TemplateUpdateCandidate {
    pub template_ref: super::templates::TemplateRef,
    pub confidence: f64,
    pub reasoning: String,
    pub changes: Vec<String>,
}
