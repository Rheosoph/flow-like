use flow_like_storage::databases::showcase::{ShowcaseObjectLink, ShowcaseTable};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

pub const MAX_LISTED: usize = 500;
const MAX_ID_LEN: usize = 256;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(default)]
pub struct ShowcaseToggle {
    pub enabled: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(default)]
pub struct ShowcaseDataSettings {
    pub enabled: bool,
    /// Table names the owner ticked. A table created later stays hidden until ticked.
    pub tables: Vec<String>,
    pub row_counts: bool,
    pub schema: bool,
    /// Up to five rows per table. Personal columns are never read.
    pub samples: bool,
    pub ontology: bool,
    pub sources: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(default)]
pub struct ShowcaseTriggerSettings {
    pub enabled: bool,
    /// Event ids the owner ticked. An event created later stays hidden until ticked.
    pub events: Vec<String>,
    /// HTTP method + path and cron schedules.
    pub technical: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(default)]
pub struct ShowcasePageSettings {
    pub enabled: bool,
    /// A wireframe from each page's component tree, without text or data.
    pub previews: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(default)]
pub struct ShowcaseHoodSettings {
    pub enabled: bool,
    pub runtime: bool,
    pub models: bool,
    pub packages: bool,
    pub accounts: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(default)]
pub struct ShowcaseActivitySettings {
    pub enabled: bool,
    pub runs: bool,
    pub versions: bool,
}

/// What the public store page shows beyond the description and reviews. Every switch starts
/// off; a missing field reads as off.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(default)]
pub struct ShowcaseSettings {
    pub map: bool,
    pub data: ShowcaseDataSettings,
    pub triggers: ShowcaseTriggerSettings,
    pub pages: ShowcasePageSettings,
    pub widgets: ShowcaseToggle,
    pub templates: ShowcaseToggle,
    pub flows: ShowcaseToggle,
    pub hood: ShowcaseHoodSettings,
    pub activity: ShowcaseActivitySettings,
}

impl ShowcaseSettings {
    /// The stored settings; a missing or unreadable value shows nothing.
    pub fn from_column(value: Option<&serde_json::Value>) -> Self {
        value
            .and_then(|value| serde_json::from_value(value.clone()).ok())
            .unwrap_or_default()
    }

    /// Trimmed, deduplicated and bounded id lists. Table names that could never name a table
    /// are dropped here so the public route never hands them to the database.
    pub fn normalized(mut self) -> Self {
        self.data.tables = clean_ids(self.data.tables, |name| {
            crate::routes::app::db::validate_table_name(name).is_ok()
        });
        self.triggers.events = clean_ids(self.triggers.events, |_| true);
        self
    }
}

fn clean_ids(ids: Vec<String>, valid: impl Fn(&str) -> bool) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for id in ids {
        let id = id.trim();
        if id.is_empty() || id.len() > MAX_ID_LEN || !valid(id) || out.iter().any(|seen| seen == id)
        {
            continue;
        }
        out.push(id.to_string());
        if out.len() == MAX_LISTED {
            break;
        }
    }
    out
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct ShowcaseSource {
    pub id: String,
    pub name: String,
    pub alias: String,
    pub driver: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, ToSchema)]
pub struct ShowcaseEventTechnical {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cron: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timezone: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct ShowcaseEvent {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub event_type: String,
    /// Present when flows are shown.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub board_id: Option<String>,
    /// Present when the page is shown.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page_id: Option<String>,
    /// Shown tables this event's flow opens by name, also while flows are hidden.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tables: Vec<String>,
    /// Shown pages of this event's flow, also while flows are hidden.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pages: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub technical: Option<ShowcaseEventTechnical>,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct ShowcaseScores {
    pub security: u8,
    pub privacy: u8,
    pub performance: u8,
    pub governance: u8,
    pub reliability: u8,
    pub cost: u8,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct ShowcaseFlow {
    pub id: String,
    pub name: String,
    pub description: String,
    pub node_count: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scores: Option<ShowcaseScores>,
    /// Shown tables this flow opens by name.
    pub tables: Vec<String>,
    /// Shown pages that belong to this flow.
    pub pages: Vec<String>,
}

/// A page's component tree reduced to component types: no text, data or styling.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct ShowcaseOutlineNode {
    pub t: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schema(no_recursion)]
    pub c: Vec<ShowcaseOutlineNode>,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct ShowcasePage {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub route: Option<String>,
    /// Present when flows are shown.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub board_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outline: Option<ShowcaseOutlineNode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub component_count: Option<u32>,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct ShowcaseWidget {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thumbnail: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct ShowcaseTemplate {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct ShowcaseModel {
    pub id: String,
    pub name: String,
    pub kind: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct ShowcasePackage {
    pub id: String,
    pub name: String,
    pub version: String,
    pub permissions: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct ShowcaseOAuth {
    pub provider: String,
    pub scopes: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, ToSchema)]
pub struct ShowcaseHood {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub execution_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requires_local: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub models: Option<Vec<ShowcaseModel>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub packages: Option<Vec<ShowcasePackage>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub oauth: Option<Vec<ShowcaseOAuth>>,
    pub refuse_bots: bool,
    pub allow_forking: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct ShowcaseRunDay {
    /// `YYYY-MM-DD` (UTC).
    pub day: String,
    pub runs: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct ShowcaseVersion {
    pub version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, ToSchema)]
pub struct ShowcaseActivity {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runs: Option<Vec<ShowcaseRunDay>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub versions: Option<Vec<ShowcaseVersion>>,
}

/// The store page's view of an app. Everything here passed the owner's showcase settings;
/// an absent field is hidden, not missing.
#[derive(Clone, Debug, Default, Serialize, Deserialize, ToSchema)]
pub struct AppShowcase {
    pub app_id: String,
    pub map: bool,
    #[schema(value_type = Vec<Object>)]
    pub tables: Vec<ShowcaseTable>,
    #[schema(value_type = Vec<Object>)]
    pub links: Vec<ShowcaseObjectLink>,
    pub sources: Vec<ShowcaseSource>,
    pub events: Vec<ShowcaseEvent>,
    pub flows: Vec<ShowcaseFlow>,
    pub pages: Vec<ShowcasePage>,
    pub widgets: Vec<ShowcaseWidget>,
    pub templates: Vec<ShowcaseTemplate>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hood: Option<ShowcaseHood>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub activity: Option<ShowcaseActivity>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_or_broken_settings_show_nothing() {
        assert_eq!(
            ShowcaseSettings::from_column(None),
            ShowcaseSettings::default()
        );
        assert_eq!(
            ShowcaseSettings::from_column(Some(&serde_json::json!("nope"))),
            ShowcaseSettings::default()
        );
        let partial =
            ShowcaseSettings::from_column(Some(&serde_json::json!({"data": {"enabled": true}})));
        assert!(partial.data.enabled);
        assert!(!partial.data.samples);
        assert!(!partial.map);
    }

    #[test]
    fn id_lists_are_trimmed_deduplicated_and_validated() {
        let settings = ShowcaseSettings {
            data: ShowcaseDataSettings {
                tables: vec![
                    " orders ".into(),
                    "orders".into(),
                    "../etc".into(),
                    "".into(),
                    "__graph_overlays__".into(),
                ],
                ..Default::default()
            },
            triggers: ShowcaseTriggerSettings {
                events: vec!["a".into(), "a".into(), " b".into()],
                ..Default::default()
            },
            ..Default::default()
        }
        .normalized();
        assert_eq!(settings.data.tables, vec!["orders".to_string()]);
        assert_eq!(
            settings.triggers.events,
            vec!["a".to_string(), "b".to_string()]
        );
    }
}
