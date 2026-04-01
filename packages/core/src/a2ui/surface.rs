use flow_like_types::{
    Value,
    json::{Deserialize, Serialize, from_slice, to_vec},
    proto,
    sync::RwLock,
};
use schemars::JsonSchema;
use std::{collections::HashMap, sync::Arc};

use super::{CanvasSettings, DataModel, Style};

/// A component in the A2UI surface
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SurfaceComponent {
    pub id: String,
    pub style: Option<Style>,
    pub component: Value,
    /// When true, this component's current value is included in widget action event payloads
    #[serde(default)]
    pub event_relevant: bool,
}

impl SurfaceComponent {
    pub fn new(id: impl Into<String>, component: Value) -> Self {
        Self {
            id: id.into(),
            style: None,
            component,
            event_relevant: false,
        }
    }

    pub fn with_event_relevant(mut self, event_relevant: bool) -> Self {
        self.event_relevant = event_relevant;
        self
    }

    pub fn with_style(mut self, style: Style) -> Self {
        self.style = Some(style);
        self
    }

    /// Get the component type name from the component Value
    pub fn get_component_type_name(&self) -> String {
        if let Some(obj) = self.component.as_object() {
            // Component should be stored as { "type": "componentType", ...props }
            if let Some(type_val) = obj.get("type")
                && let Some(type_str) = type_val.as_str()
            {
                return type_str.to_string();
            }
            // Fallback: assume older format { "type_name": { ...props } }
            obj.keys()
                .next()
                .cloned()
                .unwrap_or_else(|| "unknown".to_string())
        } else {
            "unknown".to_string()
        }
    }
}

/// A surface represents an isolated UI region with its own component tree and data
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Surface {
    pub id: String,
    pub root_component_id: String,
    pub components: HashMap<String, SurfaceComponent>,
    pub catalog_id: Option<String>,
}

impl Surface {
    pub fn new(id: impl Into<String>, root_component_id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            root_component_id: root_component_id.into(),
            components: HashMap::new(),
            catalog_id: None,
        }
    }

    pub fn with_catalog(mut self, catalog_id: impl Into<String>) -> Self {
        self.catalog_id = Some(catalog_id.into());
        self
    }

    pub fn add_component(&mut self, component: SurfaceComponent) {
        self.components.insert(component.id.clone(), component);
    }

    pub fn get_component(&self, id: &str) -> Option<&SurfaceComponent> {
        self.components.get(id)
    }

    pub fn get_component_mut(&mut self, id: &str) -> Option<&mut SurfaceComponent> {
        self.components.get_mut(id)
    }

    pub fn remove_component(&mut self, id: &str) -> Option<SurfaceComponent> {
        self.components.remove(id)
    }

    pub fn get_root(&self) -> Option<&SurfaceComponent> {
        self.components.get(&self.root_component_id)
    }
}

/// Manages multiple surfaces and their data models
#[derive(Debug, Default)]
pub struct SurfaceManager {
    surfaces: RwLock<HashMap<String, Surface>>,
    data_models: RwLock<HashMap<String, DataModel>>,
}

impl SurfaceManager {
    pub fn new() -> Self {
        Self {
            surfaces: RwLock::new(HashMap::new()),
            data_models: RwLock::new(HashMap::new()),
        }
    }

    pub async fn create_surface(&self, surface: Surface, data_model: Option<DataModel>) {
        let id = surface.id.clone();
        self.surfaces.write().await.insert(id.clone(), surface);
        self.data_models
            .write()
            .await
            .insert(id, data_model.unwrap_or_default());
    }

    pub async fn get_surface(&self, id: &str) -> Option<Surface> {
        self.surfaces.read().await.get(id).cloned()
    }

    pub async fn update_surface(&self, surface: Surface) {
        self.surfaces
            .write()
            .await
            .insert(surface.id.clone(), surface);
    }

    pub async fn delete_surface(&self, id: &str) {
        self.surfaces.write().await.remove(id);
        self.data_models.write().await.remove(id);
    }

    pub async fn get_data_model(&self, surface_id: &str) -> Option<DataModel> {
        self.data_models.read().await.get(surface_id).cloned()
    }

    pub async fn update_data(&self, surface_id: &str, path: &str, value: Value) {
        if let Some(model) = self.data_models.write().await.get_mut(surface_id) {
            model.set(path, value);
        }
    }

    pub async fn update_data_model(&self, surface_id: &str, entries: Vec<super::DataEntry>) {
        let model = DataModel::from_entries(entries);
        self.data_models
            .write()
            .await
            .insert(surface_id.to_string(), model);
    }

    pub async fn list_surface_ids(&self) -> Vec<String> {
        self.surfaces.read().await.keys().cloned().collect()
    }

    pub async fn clear(&self) {
        self.surfaces.write().await.clear();
        self.data_models.write().await.clear();
    }
}

/// Messages sent from server to client
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum A2UIServerMessage {
    BeginRendering {
        surface_id: String,
        root_component_id: String,
        components: Vec<SurfaceComponent>,
        data_model: Vec<super::DataEntry>,
        catalog_id: Option<String>,
    },
    SurfaceUpdate {
        surface_id: String,
        components: Vec<SurfaceComponent>,
        parent_id: Option<String>,
    },
    SetCanvasSettings {
        surface_id: String,
        canvas_settings: CanvasSettings,
    },
    DataModelUpdate {
        surface_id: String,
        path: Option<String>,
        contents: Vec<super::DataEntry>,
    },
    DeleteSurface {
        surface_id: String,
    },
    RequestElements {
        element_ids: Vec<String>,
    },
    UpsertElement {
        element_id: String,
        value: Value,
    },
    NavigateTo {
        route: String,
        replace: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        query_params: Option<std::collections::HashMap<String, String>>,
    },
    CreateElement {
        surface_id: String,
        parent_id: String,
        component: SurfaceComponent,
        index: Option<usize>,
    },
    RemoveElement {
        surface_id: String,
        element_id: String,
    },
    SetGlobalState {
        key: String,
        value: Value,
    },
    SetPageState {
        page_id: String,
        key: String,
        value: Value,
    },
    ClearPageState {
        page_id: String,
    },
    SetQueryParam {
        key: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        value: Option<String>,
        replace: bool,
    },
    OpenDialog {
        route: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        query_params: Option<std::collections::HashMap<String, String>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        dialog_id: Option<String>,
    },
    CloseDialog {
        #[serde(skip_serializing_if = "Option::is_none")]
        dialog_id: Option<String>,
    },
}

impl A2UIServerMessage {
    pub fn begin_rendering(surface: &Surface, data_model: &DataModel) -> Self {
        Self::BeginRendering {
            surface_id: surface.id.clone(),
            root_component_id: surface.root_component_id.clone(),
            components: surface.components.values().cloned().collect(),
            data_model: data_model.to_entries(),
            catalog_id: surface.catalog_id.clone(),
        }
    }

    pub fn surface_update(
        surface_id: impl Into<String>,
        components: Vec<SurfaceComponent>,
    ) -> Self {
        Self::SurfaceUpdate {
            surface_id: surface_id.into(),
            components,
            parent_id: None,
        }
    }

    pub fn set_canvas_settings(
        surface_id: impl Into<String>,
        canvas_settings: CanvasSettings,
    ) -> Self {
        Self::SetCanvasSettings {
            surface_id: surface_id.into(),
            canvas_settings,
        }
    }

    pub fn data_update(surface_id: impl Into<String>, path: Option<String>, value: Value) -> Self {
        Self::DataModelUpdate {
            surface_id: surface_id.into(),
            path,
            contents: vec![super::DataEntry::new("value", value)],
        }
    }

    pub fn delete_surface(surface_id: impl Into<String>) -> Self {
        Self::DeleteSurface {
            surface_id: surface_id.into(),
        }
    }

    pub fn request_elements(element_ids: Vec<String>) -> Self {
        Self::RequestElements { element_ids }
    }

    pub fn upsert_element(element_id: impl Into<String>, value: Value) -> Self {
        Self::UpsertElement {
            element_id: element_id.into(),
            value,
        }
    }

    pub fn navigate_to(route: impl Into<String>, replace: bool) -> Self {
        Self::NavigateTo {
            route: route.into(),
            replace,
            query_params: None,
        }
    }

    pub fn navigate_to_with_params(
        route: impl Into<String>,
        replace: bool,
        query_params: std::collections::HashMap<String, String>,
    ) -> Self {
        Self::NavigateTo {
            route: route.into(),
            replace,
            query_params: if query_params.is_empty() {
                None
            } else {
                Some(query_params)
            },
        }
    }

    pub fn create_element(
        surface_id: impl Into<String>,
        parent_id: impl Into<String>,
        component: SurfaceComponent,
        index: Option<usize>,
    ) -> Self {
        Self::CreateElement {
            surface_id: surface_id.into(),
            parent_id: parent_id.into(),
            component,
            index,
        }
    }

    pub fn remove_element(surface_id: impl Into<String>, element_id: impl Into<String>) -> Self {
        Self::RemoveElement {
            surface_id: surface_id.into(),
            element_id: element_id.into(),
        }
    }

    pub fn set_global_state(key: impl Into<String>, value: Value) -> Self {
        Self::SetGlobalState {
            key: key.into(),
            value,
        }
    }

    pub fn set_page_state(
        page_id: impl Into<String>,
        key: impl Into<String>,
        value: Value,
    ) -> Self {
        Self::SetPageState {
            page_id: page_id.into(),
            key: key.into(),
            value,
        }
    }

    pub fn clear_page_state(page_id: impl Into<String>) -> Self {
        Self::ClearPageState {
            page_id: page_id.into(),
        }
    }

    pub fn set_query_param(key: impl Into<String>, value: Option<String>, replace: bool) -> Self {
        Self::SetQueryParam {
            key: key.into(),
            value,
            replace,
        }
    }

    pub fn open_dialog(
        route: impl Into<String>,
        title: Option<String>,
        query_params: Option<std::collections::HashMap<String, String>>,
        dialog_id: Option<String>,
    ) -> Self {
        Self::OpenDialog {
            route: route.into(),
            title,
            query_params,
            dialog_id,
        }
    }

    pub fn close_dialog(dialog_id: Option<String>) -> Self {
        Self::CloseDialog { dialog_id }
    }
}

/// Messages sent from client to server
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum A2UIClientMessage {
    UserAction {
        name: String,
        surface_id: String,
        source_component_id: String,
        timestamp: i64,
        context: HashMap<String, Value>,
    },
    ClientError {
        surface_id: String,
        component_id: Option<String>,
        message: String,
        code: String,
    },
}

impl A2UIClientMessage {
    pub fn user_action(
        name: impl Into<String>,
        surface_id: impl Into<String>,
        component_id: impl Into<String>,
        context: HashMap<String, Value>,
    ) -> Self {
        Self::UserAction {
            name: name.into(),
            surface_id: surface_id.into(),
            source_component_id: component_id.into(),
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as i64,
            context,
        }
    }

    pub fn error(
        surface_id: impl Into<String>,
        component_id: Option<String>,
        message: impl Into<String>,
        code: impl Into<String>,
    ) -> Self {
        Self::ClientError {
            surface_id: surface_id.into(),
            component_id,
            message: message.into(),
            code: code.into(),
        }
    }
}

/// Helper for building surfaces with a fluent API
pub struct SurfaceBuilder {
    surface: Surface,
    data_model: DataModel,
}

impl SurfaceBuilder {
    pub fn new(id: impl Into<String>, root_id: impl Into<String>) -> Self {
        Self {
            surface: Surface::new(id, root_id),
            data_model: DataModel::new(),
        }
    }

    pub fn catalog(mut self, catalog_id: impl Into<String>) -> Self {
        self.surface.catalog_id = Some(catalog_id.into());
        self
    }

    pub fn component(mut self, component: SurfaceComponent) -> Self {
        self.surface.add_component(component);
        self
    }

    pub fn data(mut self, path: &str, value: Value) -> Self {
        self.data_model.set(path, value);
        self
    }

    pub fn build(self) -> (Surface, DataModel) {
        (self.surface, self.data_model)
    }
}

/// Thread-safe surface manager wrapper
pub type SharedSurfaceManager = Arc<SurfaceManager>;

pub fn create_surface_manager() -> SharedSurfaceManager {
    Arc::new(SurfaceManager::new())
}

// ============================================================================
// Proto Conversions
// ============================================================================

impl From<SurfaceComponent> for proto::Component {
    fn from(value: SurfaceComponent) -> Self {
        let component_json = to_vec(&value.component).ok();
        proto::Component {
            id: value.id,
            style: value.style.map(Into::into),
            component_json,
            component: None,
        }
    }
}

impl From<proto::Component> for SurfaceComponent {
    fn from(proto: proto::Component) -> Self {
        let component = proto
            .component_json
            .and_then(|json| from_slice(&json).ok())
            .unwrap_or(Value::Null);

        SurfaceComponent {
            id: proto.id,
            style: proto.style.map(Into::into),
            component,
            event_relevant: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_types::tokio;

    #[test]
    fn test_surface_new() {
        let surface = Surface::new("surface-1", "root-component");
        assert_eq!(surface.id, "surface-1");
        assert_eq!(surface.root_component_id, "root-component");
        assert!(surface.components.is_empty());
        assert!(surface.catalog_id.is_none());
    }

    #[test]
    fn test_surface_component_new() {
        let component = SurfaceComponent::new("comp-1", Value::String("test".to_string()));
        assert_eq!(component.id, "comp-1");
        assert!(component.style.is_none());
    }

    #[test]
    fn test_surface_component_with_style() {
        let component = SurfaceComponent::new("comp-1", Value::Null).with_style(Style::default());
        assert!(component.style.is_some());
    }

    #[test]
    fn test_surface_add_component() {
        let mut surface = Surface::new("s1", "c1");
        let component = SurfaceComponent::new("c1", Value::String("test".to_string()));
        surface.add_component(component);
        assert_eq!(surface.components.len(), 1);
        assert!(surface.get_component("c1").is_some());
    }

    #[test]
    fn test_surface_get_root() {
        let mut surface = Surface::new("s1", "root");
        let component = SurfaceComponent::new("root", Value::String("root-comp".to_string()));
        surface.add_component(component);
        assert!(surface.get_root().is_some());
        assert_eq!(surface.get_root().unwrap().id, "root");
    }

    #[test]
    fn test_surface_remove_component() {
        let mut surface = Surface::new("s1", "c1");
        surface.add_component(SurfaceComponent::new("c1", Value::Null));
        surface.add_component(SurfaceComponent::new("c2", Value::Null));
        assert_eq!(surface.components.len(), 2);

        let removed = surface.remove_component("c2");
        assert!(removed.is_some());
        assert_eq!(surface.components.len(), 1);
    }

    #[test]
    fn test_surface_with_catalog() {
        let surface = Surface::new("s1", "r1").with_catalog("catalog-123");
        assert_eq!(surface.catalog_id, Some("catalog-123".to_string()));
    }

    #[test]
    fn test_a2ui_server_message_begin_rendering() {
        let mut surface = Surface::new("s1", "r1");
        surface.add_component(SurfaceComponent::new("r1", Value::Null));
        let data_model = DataModel::new();
        let msg = A2UIServerMessage::begin_rendering(&surface, &data_model);
        match msg {
            A2UIServerMessage::BeginRendering {
                surface_id,
                root_component_id,
                ..
            } => {
                assert_eq!(surface_id, "s1");
                assert_eq!(root_component_id, "r1");
            }
            _ => panic!("Expected BeginRendering"),
        }
    }

    #[test]
    fn test_a2ui_server_message_surface_update() {
        let components = vec![SurfaceComponent::new("c1", Value::Null)];
        let msg = A2UIServerMessage::surface_update("s1", components);
        match msg {
            A2UIServerMessage::SurfaceUpdate {
                surface_id,
                components,
                parent_id,
            } => {
                assert_eq!(surface_id, "s1");
                assert_eq!(components.len(), 1);
                assert!(parent_id.is_none());
            }
            _ => panic!("Expected SurfaceUpdate"),
        }
    }

    #[test]
    fn test_a2ui_server_message_data_update() {
        let msg = A2UIServerMessage::data_update("s1", None, Value::Bool(true));
        match msg {
            A2UIServerMessage::DataModelUpdate {
                surface_id, path, ..
            } => {
                assert_eq!(surface_id, "s1");
                assert!(path.is_none());
            }
            _ => panic!("Expected DataModelUpdate"),
        }
    }

    #[test]
    fn test_a2ui_client_message_user_action() {
        let msg = A2UIClientMessage::user_action("click", "surface-1", "button-1", HashMap::new());
        match msg {
            A2UIClientMessage::UserAction {
                name,
                surface_id,
                source_component_id,
                timestamp,
                ..
            } => {
                assert_eq!(name, "click");
                assert_eq!(surface_id, "surface-1");
                assert_eq!(source_component_id, "button-1");
                assert!(timestamp > 0);
            }
            _ => panic!("Expected UserAction"),
        }
    }

    #[test]
    fn test_a2ui_client_message_error() {
        let msg = A2UIClientMessage::error(
            "surface-1",
            Some("comp-1".to_string()),
            "Render failed",
            "RENDER_ERR",
        );
        match msg {
            A2UIClientMessage::ClientError {
                surface_id,
                component_id,
                message,
                code,
            } => {
                assert_eq!(surface_id, "surface-1");
                assert_eq!(component_id, Some("comp-1".to_string()));
                assert_eq!(message, "Render failed");
                assert_eq!(code, "RENDER_ERR");
            }
            _ => panic!("Expected ClientError"),
        }
    }

    #[test]
    fn test_surface_builder_basic() {
        let (surface, _) = SurfaceBuilder::new("s1", "root")
            .catalog("catalog-1")
            .build();
        assert_eq!(surface.id, "s1");
        assert_eq!(surface.root_component_id, "root");
        assert_eq!(surface.catalog_id, Some("catalog-1".to_string()));
    }

    #[test]
    fn test_surface_builder_with_component() {
        let component = SurfaceComponent::new("c1", Value::String("test".to_string()));
        let (surface, _) = SurfaceBuilder::new("s1", "c1").component(component).build();
        assert_eq!(surface.components.len(), 1);
        assert!(surface.components.contains_key("c1"));
    }

    #[test]
    fn test_surface_builder_with_data() {
        let (_, data_model) = SurfaceBuilder::new("s1", "root")
            .data("test/key", Value::Bool(true))
            .build();
        assert!(data_model.get("test/key").is_some());
    }

    #[test]
    fn test_surface_multiple_components() {
        let c1 = SurfaceComponent::new("c1", Value::String("comp1".to_string()));
        let c2 = SurfaceComponent::new("c2", Value::String("comp2".to_string()));
        let c3 = SurfaceComponent::new("c3", Value::String("comp3".to_string()));

        let (surface, _) = SurfaceBuilder::new("s1", "c1")
            .component(c1)
            .component(c2)
            .component(c3)
            .build();

        assert_eq!(surface.components.len(), 3);
    }

    #[test]
    fn test_data_model_new() {
        let model = DataModel::new();
        assert!(model.is_empty());
    }

    #[test]
    fn test_data_entry_creation() {
        let entry = super::super::DataEntry::new("user.name", Value::String("Alice".to_string()));
        assert_eq!(entry.key, "user.name");
    }

    #[tokio::test]
    async fn test_surface_manager_create_and_get() {
        let manager = SurfaceManager::new();
        let surface = Surface::new("s1", "root");

        manager.create_surface(surface, None).await;

        let retrieved = manager.get_surface("s1").await;
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().id, "s1");
    }

    #[tokio::test]
    async fn test_surface_manager_delete() {
        let manager = SurfaceManager::new();
        let surface = Surface::new("s1", "root");

        manager.create_surface(surface, None).await;
        manager.delete_surface("s1").await;

        let retrieved = manager.get_surface("s1").await;
        assert!(retrieved.is_none());
    }

    #[tokio::test]
    async fn test_surface_manager_update_data() {
        let manager = SurfaceManager::new();
        let surface = Surface::new("s1", "root");

        manager
            .create_surface(surface, Some(DataModel::new()))
            .await;
        manager
            .update_data("s1", "test/value", Value::Number(42.into()))
            .await;

        let data_model = manager.get_data_model("s1").await;
        assert!(data_model.is_some());
        assert!(data_model.unwrap().get("test/value").is_some());
    }

    #[test]
    fn test_open_dialog_serialization() {
        let msg = A2UIServerMessage::open_dialog("/new", Some("Test".to_string()), None, None);
        let json = serde_json::to_string(&msg).unwrap();
        // Verify the type field is "openDialog" (camelCase)
        assert!(
            json.contains(r#""type":"openDialog""#),
            "Expected 'openDialog' but got: {}",
            json
        );
        assert!(
            json.contains(r#""route":"/new""#),
            "Expected route '/new' but got: {}",
            json
        );
    }
}
