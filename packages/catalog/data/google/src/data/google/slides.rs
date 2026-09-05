use super::provider::{GOOGLE_PROVIDER_ID, GoogleProvider};
use crate::data::path::FlowPath;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_types::{JsonSchema, Value, async_trait, create_id, json::json, reqwest};
use serde::{Deserialize, Serialize};

// =============================================================================
// Google Slides Types
// =============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GooglePresentation {
    pub presentation_id: String,
    pub title: String,
    pub locale: Option<String>,
    pub revision_id: Option<String>,
    pub slides: Vec<GoogleSlide>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GoogleSlide {
    pub object_id: String,
    pub page_type: Option<String>,
}

fn parse_presentation(value: &Value) -> Option<GooglePresentation> {
    let slides = value["slides"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|s| {
                    Some(GoogleSlide {
                        object_id: s["objectId"].as_str()?.to_string(),
                        page_type: s["pageType"].as_str().map(String::from),
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    Some(GooglePresentation {
        presentation_id: value["presentationId"].as_str()?.to_string(),
        title: value["title"].as_str()?.to_string(),
        locale: value["locale"].as_str().map(String::from),
        revision_id: value["revisionId"].as_str().map(String::from),
        slides,
    })
}

// =============================================================================
// Create Presentation Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct CreateGoogleSlidesNode {}

impl CreateGoogleSlidesNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for CreateGoogleSlidesNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_google_slides_create",
            "Create Presentation",
            "Create a new Google Slides presentation",
            "Data/Google/Slides",
        );
        node.set_flowscript_name("google.slides", "create");
        node.add_icon("/flow/icons/google.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_input_pin(
            "provider",
            "Provider",
            "Google Drive provider",
            VariableType::Struct,
        )
        .set_schema::<GoogleProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin("title", "Title", "Presentation title", VariableType::String);

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin(
            "presentation_id",
            "Presentation ID",
            "",
            VariableType::String,
        );
        node.add_output_pin("presentation", "Presentation", "", VariableType::Struct)
            .set_schema::<GooglePresentation>();
        node.add_output_pin("error_message", "Error Message", "", VariableType::String);

        node.add_required_oauth_scopes(
            GOOGLE_PROVIDER_ID,
            vec!["https://www.googleapis.com/auth/presentations"],
        );
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: GoogleProvider = context.evaluate_pin("provider").await?;
        let title: String = context.evaluate_pin("title").await?;

        let body = json!({ "title": title });

        let client = reqwest::Client::new();
        let response = client
            .post("https://slides.googleapis.com/v1/presentations")
            .header("Authorization", format!("Bearer {}", provider.access_token))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await;

        match response {
            Ok(resp) if resp.status().is_success() => {
                let body: Value = resp.json().await?;
                if let Some(presentation) = parse_presentation(&body) {
                    let id = presentation.presentation_id.clone();
                    context.set_pin_value("presentation_id", json!(id)).await?;
                    context
                        .set_pin_value("presentation", json!(presentation))
                        .await?;
                    context.activate_exec_pin("exec_out").await?;
                } else {
                    context
                        .set_pin_value("error_message", json!("Failed to parse response"))
                        .await?;
                    context.activate_exec_pin("error").await?;
                }
            }
            Ok(resp) => {
                let error = resp.text().await.unwrap_or_default();
                context.set_pin_value("error_message", json!(error)).await?;
                context.activate_exec_pin("error").await?;
            }
            Err(e) => {
                context
                    .set_pin_value("error_message", json!(e.to_string()))
                    .await?;
                context.activate_exec_pin("error").await?;
            }
        }

        Ok(())
    }
}

// =============================================================================
// Get Presentation Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct GetGoogleSlidesNode {}

impl GetGoogleSlidesNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for GetGoogleSlidesNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_google_slides_get",
            "Get Presentation",
            "Get a Google Slides presentation's metadata and slides",
            "Data/Google/Slides",
        );
        node.set_flowscript_name("google.slides", "get");
        node.add_icon("/flow/icons/google.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_input_pin(
            "provider",
            "Provider",
            "Google Drive provider",
            VariableType::Struct,
        )
        .set_schema::<GoogleProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin(
            "presentation_id",
            "Presentation ID",
            "",
            VariableType::String,
        );

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin("presentation", "Presentation", "", VariableType::Struct)
            .set_schema::<GooglePresentation>();
        node.add_output_pin("slides", "Slides", "List of slides", VariableType::Struct)
            .set_value_type(ValueType::Array)
            .set_schema::<GoogleSlide>();
        node.add_output_pin("slide_count", "Slide Count", "", VariableType::Integer);
        node.add_output_pin("error_message", "Error Message", "", VariableType::String);

        node.add_required_oauth_scopes(
            GOOGLE_PROVIDER_ID,
            vec!["https://www.googleapis.com/auth/presentations.readonly"],
        );
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: GoogleProvider = context.evaluate_pin("provider").await?;
        let presentation_id: String = context.evaluate_pin("presentation_id").await?;

        let client = reqwest::Client::new();
        let response = client
            .get(format!(
                "https://slides.googleapis.com/v1/presentations/{}",
                presentation_id
            ))
            .header("Authorization", format!("Bearer {}", provider.access_token))
            .send()
            .await;

        match response {
            Ok(resp) if resp.status().is_success() => {
                let body: Value = resp.json().await?;
                if let Some(presentation) = parse_presentation(&body) {
                    let slides = presentation.slides.clone();
                    let slide_count = slides.len() as i64;
                    context
                        .set_pin_value("presentation", json!(presentation))
                        .await?;
                    context.set_pin_value("slides", json!(slides)).await?;
                    context
                        .set_pin_value("slide_count", json!(slide_count))
                        .await?;
                    context.activate_exec_pin("exec_out").await?;
                } else {
                    context
                        .set_pin_value("error_message", json!("Failed to parse response"))
                        .await?;
                    context.activate_exec_pin("error").await?;
                }
            }
            Ok(resp) => {
                let error = resp.text().await.unwrap_or_default();
                context.set_pin_value("error_message", json!(error)).await?;
                context.activate_exec_pin("error").await?;
            }
            Err(e) => {
                context
                    .set_pin_value("error_message", json!(e.to_string()))
                    .await?;
                context.activate_exec_pin("error").await?;
            }
        }

        Ok(())
    }
}

// =============================================================================
// Add Slide Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct AddGoogleSlideNode {}

impl AddGoogleSlideNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for AddGoogleSlideNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_google_slides_add_slide",
            "Add Slide",
            "Add a new slide to a Google Slides presentation",
            "Data/Google/Slides",
        );
        node.set_flowscript_name("google.slides", "addSlide");
        node.add_icon("/flow/icons/google.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_input_pin(
            "provider",
            "Provider",
            "Google Drive provider",
            VariableType::Struct,
        )
        .set_schema::<GoogleProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin(
            "presentation_id",
            "Presentation ID",
            "",
            VariableType::String,
        );
        node.add_input_pin(
            "layout",
            "Layout",
            "Predefined layout for the slide",
            VariableType::String,
        )
        .set_default_value(Some(json!("BLANK")))
        .set_options(
            PinOptions::new()
                .set_valid_values(vec![
                    "BLANK".to_string(),
                    "CAPTION_ONLY".to_string(),
                    "TITLE".to_string(),
                    "TITLE_AND_BODY".to_string(),
                    "TITLE_AND_TWO_COLUMNS".to_string(),
                    "TITLE_ONLY".to_string(),
                    "ONE_COLUMN_TEXT".to_string(),
                    "MAIN_POINT".to_string(),
                    "BIG_NUMBER".to_string(),
                ])
                .build(),
        );
        node.add_input_pin(
            "insert_index",
            "Insert Index",
            "Index where to insert slide (optional)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(-1)));

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin(
            "slide_id",
            "Slide ID",
            "ID of the created slide",
            VariableType::String,
        );
        node.add_output_pin("error_message", "Error Message", "", VariableType::String);

        node.add_required_oauth_scopes(
            GOOGLE_PROVIDER_ID,
            vec!["https://www.googleapis.com/auth/presentations"],
        );
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: GoogleProvider = context.evaluate_pin("provider").await?;
        let presentation_id: String = context.evaluate_pin("presentation_id").await?;
        let layout: String = context
            .evaluate_pin("layout")
            .await
            .unwrap_or_else(|_| "BLANK".to_string());
        let insert_index: i64 = context.evaluate_pin("insert_index").await.unwrap_or(-1);

        let slide_id = format!("slide_{}", &create_id()[..12]);

        let mut create_request = json!({
            "objectId": slide_id,
            "slideLayoutReference": {
                "predefinedLayout": layout
            }
        });

        if insert_index >= 0 {
            create_request["insertionIndex"] = json!(insert_index);
        }

        let body = json!({
            "requests": [{
                "createSlide": create_request
            }]
        });

        let client = reqwest::Client::new();
        let response = client
            .post(format!(
                "https://slides.googleapis.com/v1/presentations/{}:batchUpdate",
                presentation_id
            ))
            .header("Authorization", format!("Bearer {}", provider.access_token))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await;

        match response {
            Ok(resp) if resp.status().is_success() => {
                let body: Value = resp.json().await?;
                let created_id = body["replies"][0]["createSlide"]["objectId"]
                    .as_str()
                    .unwrap_or(&slide_id)
                    .to_string();
                context.set_pin_value("slide_id", json!(created_id)).await?;
                context.activate_exec_pin("exec_out").await?;
            }
            Ok(resp) => {
                let error = resp.text().await.unwrap_or_default();
                context.set_pin_value("error_message", json!(error)).await?;
                context.activate_exec_pin("error").await?;
            }
            Err(e) => {
                context
                    .set_pin_value("error_message", json!(e.to_string()))
                    .await?;
                context.activate_exec_pin("error").await?;
            }
        }

        Ok(())
    }
}

// =============================================================================
// Delete Slide Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct DeleteGoogleSlideNode {}

impl DeleteGoogleSlideNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for DeleteGoogleSlideNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_google_slides_delete_slide",
            "Delete Slide",
            "Delete a slide from a Google Slides presentation",
            "Data/Google/Slides",
        );
        node.set_flowscript_name("google.slides", "deleteSlide");
        node.add_icon("/flow/icons/google.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_input_pin(
            "provider",
            "Provider",
            "Google Drive provider",
            VariableType::Struct,
        )
        .set_schema::<GoogleProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin(
            "presentation_id",
            "Presentation ID",
            "",
            VariableType::String,
        );
        node.add_input_pin(
            "slide_id",
            "Slide ID",
            "ID of the slide to delete",
            VariableType::String,
        );

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin("error_message", "Error Message", "", VariableType::String);

        node.add_required_oauth_scopes(
            GOOGLE_PROVIDER_ID,
            vec!["https://www.googleapis.com/auth/presentations"],
        );
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: GoogleProvider = context.evaluate_pin("provider").await?;
        let presentation_id: String = context.evaluate_pin("presentation_id").await?;
        let slide_id: String = context.evaluate_pin("slide_id").await?;

        let body = json!({
            "requests": [{
                "deleteObject": {
                    "objectId": slide_id
                }
            }]
        });

        let client = reqwest::Client::new();
        let response = client
            .post(format!(
                "https://slides.googleapis.com/v1/presentations/{}:batchUpdate",
                presentation_id
            ))
            .header("Authorization", format!("Bearer {}", provider.access_token))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await;

        match response {
            Ok(resp) if resp.status().is_success() => {
                context.activate_exec_pin("exec_out").await?;
            }
            Ok(resp) => {
                let error = resp.text().await.unwrap_or_default();
                context.set_pin_value("error_message", json!(error)).await?;
                context.activate_exec_pin("error").await?;
            }
            Err(e) => {
                context
                    .set_pin_value("error_message", json!(e.to_string()))
                    .await?;
                context.activate_exec_pin("error").await?;
            }
        }

        Ok(())
    }
}

// =============================================================================
// Add Text to Slide Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct AddTextToSlideNode {}

impl AddTextToSlideNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for AddTextToSlideNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_google_slides_add_text",
            "Add Text to Slide",
            "Add a text box with text to a Google Slide",
            "Data/Google/Slides",
        );
        node.set_flowscript_name("google.slides", "addText");
        node.add_icon("/flow/icons/google.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_input_pin(
            "provider",
            "Provider",
            "Google Drive provider",
            VariableType::Struct,
        )
        .set_schema::<GoogleProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin(
            "presentation_id",
            "Presentation ID",
            "",
            VariableType::String,
        );
        node.add_input_pin(
            "slide_id",
            "Slide ID",
            "ID of the slide",
            VariableType::String,
        );
        node.add_input_pin("text", "Text", "Text content to add", VariableType::String);
        node.add_input_pin(
            "x",
            "X Position",
            "X position in EMU (914400 EMU = 1 inch)",
            VariableType::Float,
        )
        .set_default_value(Some(json!(914400.0)));
        node.add_input_pin("y", "Y Position", "Y position in EMU", VariableType::Float)
            .set_default_value(Some(json!(914400.0)));
        node.add_input_pin("width", "Width", "Width in EMU", VariableType::Float)
            .set_default_value(Some(json!(7315200.0)));
        node.add_input_pin("height", "Height", "Height in EMU", VariableType::Float)
            .set_default_value(Some(json!(914400.0)));

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin(
            "shape_id",
            "Shape ID",
            "ID of the created text box",
            VariableType::String,
        );
        node.add_output_pin("error_message", "Error Message", "", VariableType::String);

        node.add_required_oauth_scopes(
            GOOGLE_PROVIDER_ID,
            vec!["https://www.googleapis.com/auth/presentations"],
        );
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: GoogleProvider = context.evaluate_pin("provider").await?;
        let presentation_id: String = context.evaluate_pin("presentation_id").await?;
        let slide_id: String = context.evaluate_pin("slide_id").await?;
        let text: String = context.evaluate_pin("text").await?;
        let x: f64 = context.evaluate_pin("x").await.unwrap_or(914400.0);
        let y: f64 = context.evaluate_pin("y").await.unwrap_or(914400.0);
        let width: f64 = context.evaluate_pin("width").await.unwrap_or(7315200.0);
        let height: f64 = context.evaluate_pin("height").await.unwrap_or(914400.0);

        let shape_id = format!("textbox_{}", &create_id()[..12]);

        let body = json!({
            "requests": [
                {
                    "createShape": {
                        "objectId": shape_id,
                        "shapeType": "TEXT_BOX",
                        "elementProperties": {
                            "pageObjectId": slide_id,
                            "size": {
                                "width": { "magnitude": width, "unit": "EMU" },
                                "height": { "magnitude": height, "unit": "EMU" }
                            },
                            "transform": {
                                "scaleX": 1,
                                "scaleY": 1,
                                "translateX": x,
                                "translateY": y,
                                "unit": "EMU"
                            }
                        }
                    }
                },
                {
                    "insertText": {
                        "objectId": shape_id,
                        "text": text,
                        "insertionIndex": 0
                    }
                }
            ]
        });

        let client = reqwest::Client::new();
        let response = client
            .post(format!(
                "https://slides.googleapis.com/v1/presentations/{}:batchUpdate",
                presentation_id
            ))
            .header("Authorization", format!("Bearer {}", provider.access_token))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await;

        match response {
            Ok(resp) if resp.status().is_success() => {
                context.set_pin_value("shape_id", json!(shape_id)).await?;
                context.activate_exec_pin("exec_out").await?;
            }
            Ok(resp) => {
                let error = resp.text().await.unwrap_or_default();
                context.set_pin_value("error_message", json!(error)).await?;
                context.activate_exec_pin("error").await?;
            }
            Err(e) => {
                context
                    .set_pin_value("error_message", json!(e.to_string()))
                    .await?;
                context.activate_exec_pin("error").await?;
            }
        }

        Ok(())
    }
}

// =============================================================================
// Export Presentation Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct ExportGoogleSlidesNode {}

impl ExportGoogleSlidesNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for ExportGoogleSlidesNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_google_slides_export",
            "Export Presentation",
            "Export a Google Slides presentation into a FlowPath",
            "Data/Google/Slides",
        );
        node.set_flowscript_name("google.slides", "export");
        node.add_icon("/flow/icons/google.svg");
        node.set_version(2);

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_input_pin(
            "provider",
            "Provider",
            "Google Drive provider",
            VariableType::Struct,
        )
        .set_schema::<GoogleProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin(
            "presentation_id",
            "Presentation ID",
            "",
            VariableType::String,
        );
        node.add_input_pin("format", "Format", "Export format", VariableType::String)
            .set_default_value(Some(json!("application/pdf")))
            .set_options(
                PinOptions::new()
                    .set_valid_values(vec![
                        "application/pdf".to_string(),
                        "application/vnd.openxmlformats-officedocument.presentationml.presentation"
                            .to_string(),
                        "text/plain".to_string(),
                    ])
                    .build(),
            );
        node.add_input_pin(
            "output_path",
            "Output Path",
            "FlowPath to write the exported presentation into",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin("path", "Path", "Written file path", VariableType::Struct)
            .set_schema::<FlowPath>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_output_pin(
            "size",
            "Size",
            "Exported size in bytes",
            VariableType::Integer,
        );
        node.add_output_pin("error_message", "Error Message", "", VariableType::String);

        node.add_required_oauth_scopes(
            GOOGLE_PROVIDER_ID,
            vec!["https://www.googleapis.com/auth/drive.file"],
        );
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: GoogleProvider = context.evaluate_pin("provider").await?;
        let presentation_id: String = context.evaluate_pin("presentation_id").await?;
        let format: String = context
            .evaluate_pin("format")
            .await
            .unwrap_or_else(|_| "application/pdf".to_string());
        let output_path: FlowPath = context.evaluate_pin("output_path").await?;

        let client = reqwest::Client::new();
        let response = client
            .get(format!(
                "https://www.googleapis.com/drive/v3/files/{}/export",
                presentation_id
            ))
            .header("Authorization", format!("Bearer {}", provider.access_token))
            .query(&[("mimeType", format.as_str())])
            .send()
            .await;

        match response {
            Ok(resp) if resp.status().is_success() => {
                let bytes = resp.bytes().await?;
                let size = bytes.len() as i64;
                output_path.put(context, bytes.to_vec(), false).await?;
                context.set_pin_value("path", json!(output_path)).await?;
                context.set_pin_value("size", json!(size)).await?;
                context.activate_exec_pin("exec_out").await?;
            }
            Ok(resp) => {
                let error = resp.text().await.unwrap_or_default();
                context.set_pin_value("error_message", json!(error)).await?;
                context.activate_exec_pin("error").await?;
            }
            Err(e) => {
                context
                    .set_pin_value("error_message", json!(e.to_string()))
                    .await?;
                context.activate_exec_pin("error").await?;
            }
        }

        Ok(())
    }
}
