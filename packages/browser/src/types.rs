use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

macro_rules! cdp_id {
    ($($name:ident),*) => { $(
        #[derive(Clone, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub std::sync::Arc<str>);

        impl $name {
            pub fn new(value: impl Into<std::sync::Arc<str>>) -> Self {
                Self(value.into())
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl From<&str> for $name {
            fn from(value: &str) -> Self {
                Self(value.into())
            }
        }

        impl From<String> for $name {
            fn from(value: String) -> Self {
                Self(value.into())
            }
        }
    )* }
}

cdp_id!(SessionId, TargetId, FrameId, LoaderId, BrowserContextId);

#[macro_export]
macro_rules! cdp_enum {
    ($vis:vis $name:ident { $($variant:ident = $wire:literal),* $(,)? }) => {
        #[derive(Clone, Debug, PartialEq, Eq, Hash)]
        $vis enum $name {
            $($variant,)*
            Unrecognized(::std::string::String),
        }

        impl $name {
            pub fn as_str(&self) -> &str {
                match self {
                    $(Self::$variant => $wire,)*
                    Self::Unrecognized(value) => value,
                }
            }
        }

        impl ::std::default::Default for $name {
            fn default() -> Self {
                Self::Unrecognized(::std::string::String::new())
            }
        }

        impl ::std::fmt::Display for $name {
            fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl ::serde::Serialize for $name {
            fn serialize<S: ::serde::Serializer>(&self, serializer: S) -> ::std::result::Result<S::Ok, S::Error> {
                serializer.serialize_str(self.as_str())
            }
        }

        impl<'de> ::serde::Deserialize<'de> for $name {
            fn deserialize<D: ::serde::Deserializer<'de>>(deserializer: D) -> ::std::result::Result<Self, D::Error> {
                let value = <::std::string::String as ::serde::Deserialize>::deserialize(deserializer)?;
                Ok(match value.as_str() {
                    $($wire => Self::$variant,)*
                    _ => Self::Unrecognized(value),
                })
            }
        }
    };
}

cdp_enum!(pub TargetType {
    Page = "page",
    Iframe = "iframe",
    Worker = "worker",
    SharedWorker = "shared_worker",
    ServiceWorker = "service_worker",
    Browser = "browser",
    Tab = "tab",
    BrowserUi = "browser_ui",
    BackgroundPage = "background_page",
    Webview = "webview",
    Other = "other",
});
cdp_enum!(pub DialogType {
    Alert = "alert",
    Confirm = "confirm",
    Prompt = "prompt",
    BeforeUnload = "beforeunload",
});
cdp_enum!(pub DownloadState {
    InProgress = "inProgress",
    Completed = "completed",
    Canceled = "canceled",
});
cdp_enum!(pub NavigationType {
    SameDocument = "sameDocument",
    HistorySameDocument = "historySameDocument",
    DifferentDocument = "differentDocument",
    Reload = "reload",
    ReloadBypassingCache = "reloadBypassingCache",
    Restore = "restore",
    RestoreWithPost = "restoreWithPost",
    HistoryDifferentDocument = "historyDifferentDocument",
});

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TargetInfo {
    pub target_id: TargetId,
    #[serde(rename = "type")]
    pub type_: TargetType,
    pub subtype: Option<String>,
    pub title: String,
    pub url: String,
    pub attached: bool,
    pub opener_id: Option<TargetId>,
    pub browser_context_id: Option<BrowserContextId>,
    pub parent_frame_id: Option<FrameId>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AttachedToTarget {
    pub session_id: SessionId,
    pub target_info: TargetInfo,
    pub waiting_for_debugger: bool,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DetachedFromTarget {
    pub session_id: SessionId,
    pub target_id: Option<TargetId>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TargetCrashedEvent {
    pub target_id: TargetId,
    pub status: String,
    pub error_code: i64,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct FrameInfo {
    pub id: FrameId,
    pub parent_id: Option<FrameId>,
    pub loader_id: LoaderId,
    pub url: String,
    pub url_fragment: Option<String>,
    pub name: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct FrameTreeNode {
    pub frame: FrameInfo,
    pub child_frames: Vec<FrameTreeNode>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct LifecycleEvent {
    pub frame_id: FrameId,
    pub loader_id: LoaderId,
    pub name: String,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct FrameStartedNavigating {
    pub frame_id: FrameId,
    pub url: String,
    pub loader_id: LoaderId,
    pub navigation_type: NavigationType,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct FrameRequestedNavigation {
    pub frame_id: FrameId,
    pub reason: String,
    pub url: String,
    pub disposition: String,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct FrameIdParams {
    pub frame_id: FrameId,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct FrameDetached {
    pub frame_id: FrameId,
    pub reason: String,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct FrameAttached {
    pub frame_id: FrameId,
    pub parent_frame_id: FrameId,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct FrameNavigated {
    pub frame: FrameInfo,
    #[serde(rename = "type")]
    pub type_: String,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct NavigatedWithinDocument {
    pub frame_id: FrameId,
    pub url: String,
    pub navigation_type: String,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DialogOpening {
    pub url: String,
    pub frame_id: Option<FrameId>,
    pub message: String,
    #[serde(rename = "type")]
    pub type_: DialogType,
    pub default_prompt: String,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DialogClosed {
    pub frame_id: Option<FrameId>,
    pub result: bool,
    pub user_input: String,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DownloadWillBegin {
    pub frame_id: Option<FrameId>,
    pub guid: String,
    pub url: String,
    pub suggested_filename: String,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DownloadProgress {
    pub guid: String,
    pub total_bytes: f64,
    pub received_bytes: f64,
    pub state: DownloadState,
    pub file_path: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RequestPaused {
    pub request_id: String,
    pub network_id: Option<String>,
    pub frame_id: Option<FrameId>,
    pub resource_type: String,
    pub response_status_code: Option<i64>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AuthRequired {
    pub request_id: String,
    pub auth_challenge: Value,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RemoteObject {
    #[serde(rename = "type")]
    pub type_: String,
    pub subtype: Option<String>,
    pub class_name: Option<String>,
    pub value: Option<Value>,
    pub unserializable_value: Option<String>,
    pub description: Option<String>,
    pub object_id: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ExceptionDetails {
    pub text: String,
    pub line_number: i64,
    pub column_number: i64,
    pub url: Option<String>,
    pub exception: Option<RemoteObject>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ExecutionContextDescription {
    pub id: i64,
    pub origin: String,
    pub name: String,
    pub unique_id: String,
    pub aux_data: Value,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct NavigateResult {
    pub frame_id: FrameId,
    pub loader_id: Option<LoaderId>,
    pub error_text: Option<String>,
    pub is_download: bool,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct NavigationEntry {
    pub id: i64,
    pub url: String,
    pub title: String,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct NavigationHistory {
    pub current_index: i64,
    pub entries: Vec<NavigationEntry>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct WindowBounds {
    pub left: Option<i64>,
    pub top: Option<i64>,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub window_state: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct IoReadResult {
    pub data: String,
    pub eof: bool,
    pub base64_encoded: bool,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct VersionResult {
    pub protocol_version: String,
    pub product: String,
    pub revision: String,
    pub user_agent: String,
    pub js_version: String,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DescribedNode {
    pub node_id: i64,
    pub backend_node_id: i64,
    pub node_name: String,
    pub frame_id: Option<FrameId>,
    pub content_document: Option<Box<DescribedNode>>,
    pub attributes: Vec<String>,
}

fn check<T: DeserializeOwned>(value: &Value) -> Result<(), String> {
    T::deserialize(value)
        .map(drop)
        .map_err(|error| error.to_string())
}

fn check_field<T: DeserializeOwned>(value: &Value, field: &str) -> Result<(), String> {
    match value.get(field) {
        Some(inner) => check::<T>(inner).map_err(|error| format!("{field}: {error}")),
        None => Err(format!("missing field `{field}`")),
    }
}

fn check_optional_field<T: DeserializeOwned>(value: &Value, field: &str) -> Result<(), String> {
    match value.get(field) {
        Some(inner) => check::<T>(inner).map_err(|error| format!("{field}: {error}")),
        None => Ok(()),
    }
}

fn check_evaluation(result: &Value) -> Result<(), String> {
    check_field::<RemoteObject>(result, "result")?;
    check_optional_field::<ExceptionDetails>(result, "exceptionDetails")
}

pub fn typed_event_check(method: &str, params: &Value) -> Option<Result<(), String>> {
    let outcome = match method {
        "Target.targetCreated" | "Target.targetInfoChanged" => {
            check_field::<TargetInfo>(params, "targetInfo")
        }
        "Target.attachedToTarget" => check::<AttachedToTarget>(params),
        "Target.detachedFromTarget" => check::<DetachedFromTarget>(params),
        "Target.targetCrashed" => check::<TargetCrashedEvent>(params),
        "Page.frameAttached" => check::<FrameAttached>(params),
        "Page.frameDetached" => check::<FrameDetached>(params),
        "Page.frameNavigated" => check::<FrameNavigated>(params),
        "Page.navigatedWithinDocument" => check::<NavigatedWithinDocument>(params),
        "Page.lifecycleEvent" => check::<LifecycleEvent>(params),
        "Page.frameStartedNavigating" => check::<FrameStartedNavigating>(params),
        "Page.frameRequestedNavigation" => check::<FrameRequestedNavigation>(params),
        "Page.frameStartedLoading"
        | "Page.frameStoppedLoading"
        | "Page.frameClearedScheduledNavigation" => check::<FrameIdParams>(params),
        "Page.javascriptDialogOpening" => check::<DialogOpening>(params),
        "Page.javascriptDialogClosed" => check::<DialogClosed>(params),
        "Browser.downloadWillBegin" | "Page.downloadWillBegin" => {
            check::<DownloadWillBegin>(params)
        }
        "Browser.downloadProgress" | "Page.downloadProgress" => check::<DownloadProgress>(params),
        "Fetch.requestPaused" => check::<RequestPaused>(params),
        "Fetch.authRequired" => check::<AuthRequired>(params),
        "Runtime.executionContextCreated" => {
            check_field::<ExecutionContextDescription>(params, "context")
        }
        "Runtime.exceptionThrown" => check_field::<ExceptionDetails>(params, "exceptionDetails"),
        "Runtime.consoleAPICalled" => check_field::<Vec<RemoteObject>>(params, "args"),
        _ => return None,
    };
    Some(outcome)
}

pub fn typed_response_check(method: &str, result: &Value) -> Option<Result<(), String>> {
    let outcome = match method {
        "Target.getTargets" => check_field::<Vec<TargetInfo>>(result, "targetInfos"),
        "Target.getTargetInfo" => check_field::<TargetInfo>(result, "targetInfo"),
        "Page.getFrameTree" => check_field::<FrameTreeNode>(result, "frameTree"),
        "Page.navigate" => check::<NavigateResult>(result),
        "Page.getNavigationHistory" => check::<NavigationHistory>(result),
        "Browser.getWindowForTarget" | "Browser.getWindowBounds" => {
            check_field::<WindowBounds>(result, "bounds")
        }
        "IO.read" => check::<IoReadResult>(result),
        "Browser.getVersion" => check::<VersionResult>(result),
        "DOM.describeNode" => check_field::<DescribedNode>(result, "node"),
        "DOM.resolveNode" => check_field::<RemoteObject>(result, "object"),
        "Runtime.evaluate" | "Runtime.callFunctionOn" => check_evaluation(result),
        _ => return None,
    };
    Some(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn cdp_enum_keeps_unknown_wire_values() {
        let known: TargetType = serde_json::from_value(json!("iframe")).unwrap();
        let unknown: TargetType = serde_json::from_value(json!("auction_worklet")).unwrap();
        assert_eq!(known, TargetType::Iframe);
        assert_eq!(unknown, TargetType::Unrecognized("auction_worklet".into()));
        assert_eq!(unknown.as_str(), "auction_worklet");
        assert_eq!(
            serde_json::to_value(&unknown).unwrap(),
            json!("auction_worklet")
        );
        assert_eq!(
            DialogType::default(),
            DialogType::Unrecognized(String::new())
        );
    }

    #[test]
    fn ids_are_transparent_strings() {
        let id: SessionId = serde_json::from_value(json!("S1")).unwrap();
        assert_eq!(id, SessionId::from("S1"));
        assert_eq!(id.to_string(), "S1");
        assert_eq!(serde_json::to_value(&id).unwrap(), json!("S1"));
    }

    #[test]
    fn typed_structs_are_lenient() {
        let info: TargetInfo = serde_json::from_value(json!({
            "targetId": "T1",
            "type": "page",
            "subtype": "prerender",
            "extra": {"ignored": true}
        }))
        .unwrap();
        assert_eq!(info.target_id.as_str(), "T1");
        assert_eq!(info.type_, TargetType::Page);
        assert_eq!(info.subtype.as_deref(), Some("prerender"));
        assert!(info.url.is_empty());
    }

    #[test]
    fn typed_checks_cover_mapped_methods_only() {
        assert!(typed_event_check("Network.requestWillBeSent", &json!({})).is_none());
        assert_eq!(
            typed_event_check(
                "Page.frameNavigated",
                &json!({"frame": {"id": "F", "loaderId": "L"}})
            ),
            Some(Ok(()))
        );
        assert!(matches!(
            typed_event_check("Page.frameNavigated", &json!({"frame": {"id": 5}})),
            Some(Err(_))
        ));
        assert!(matches!(
            typed_response_check("Page.getFrameTree", &json!({})),
            Some(Err(_))
        ));
        assert_eq!(
            typed_response_check(
                "Runtime.evaluate",
                &json!({"result": {"type": "number", "value": 1}})
            ),
            Some(Ok(()))
        );
    }
}
