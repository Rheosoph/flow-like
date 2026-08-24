use super::provider::{GOOGLE_PROVIDER_ID, GoogleProvider};
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{JsonSchema, Value, async_trait, json::json, reqwest};
use serde::{Deserialize, Serialize};

// =============================================================================
// Meet Types
// =============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GoogleMeetInfo {
    pub meet_link: String,
    pub conference_id: String,
    pub event_id: String,
    pub event_link: Option<String>,
    pub dial_in: Option<String>,
}

// =============================================================================
// Create Meeting Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct CreateGoogleMeetNode {}

impl CreateGoogleMeetNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for CreateGoogleMeetNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_google_meet_create",
            "Create Meeting",
            "Create a new Google Meet meeting",
            "Data/Google/Meet",
        );
        node.set_flowscript_name("google.meet", "create");
        node.add_icon("/flow/icons/google.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_input_pin(
            "provider",
            "Provider",
            "Google provider",
            VariableType::Struct,
        )
        .set_schema::<GoogleProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin("summary", "Summary", "Meeting title", VariableType::String);
        node.add_input_pin(
            "description",
            "Description",
            "Meeting description",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        node.add_input_pin(
            "start_time",
            "Start Time",
            "Start time (RFC3339, e.g., 2024-01-01T10:00:00-05:00)",
            VariableType::String,
        );
        node.add_input_pin(
            "duration_minutes",
            "Duration (Minutes)",
            "Meeting duration in minutes",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(60)));
        node.add_input_pin(
            "time_zone",
            "Time Zone",
            "Time zone (e.g., America/New_York)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        node.add_input_pin(
            "attendees",
            "Attendees",
            "Comma-separated email addresses",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        node.add_input_pin(
            "send_invites",
            "Send Invites",
            "Send calendar invitations",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin(
            "meet_info",
            "Meet Info",
            "Meeting information",
            VariableType::Struct,
        )
        .set_schema::<GoogleMeetInfo>();
        node.add_output_pin(
            "meet_link",
            "Meet Link",
            "Google Meet URL",
            VariableType::String,
        );
        node.add_output_pin(
            "event_id",
            "Event ID",
            "Calendar event ID",
            VariableType::String,
        );
        node.add_output_pin("error_message", "Error Message", "", VariableType::String);

        node.add_required_oauth_scopes(
            GOOGLE_PROVIDER_ID,
            vec!["https://www.googleapis.com/auth/calendar.events"],
        );
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: GoogleProvider = context.evaluate_pin("provider").await?;
        let summary: String = context.evaluate_pin("summary").await?;
        let description: String = context
            .evaluate_pin("description")
            .await
            .unwrap_or_default();
        let start_time: String = context.evaluate_pin("start_time").await?;
        let duration_minutes: i64 = context.evaluate_pin("duration_minutes").await.unwrap_or(60);
        let time_zone: String = context.evaluate_pin("time_zone").await.unwrap_or_default();
        let attendees_str: String = context.evaluate_pin("attendees").await.unwrap_or_default();
        let send_invites: bool = context.evaluate_pin("send_invites").await.unwrap_or(true);

        let end_time = calculate_end_time(&start_time, duration_minutes);

        let mut event_body = json!({
            "summary": summary,
            "start": {
                "dateTime": start_time
            },
            "end": {
                "dateTime": end_time
            },
            "conferenceData": {
                "createRequest": {
                    "requestId": format!("meet-{}", chrono::Utc::now().timestamp_millis()),
                    "conferenceSolutionKey": {
                        "type": "hangoutsMeet"
                    }
                }
            }
        });

        if !description.is_empty() {
            event_body["description"] = json!(description);
        }
        if !time_zone.is_empty() {
            event_body["start"]["timeZone"] = json!(time_zone.clone());
            event_body["end"]["timeZone"] = json!(time_zone);
        }

        if !attendees_str.is_empty() {
            let attendees: Vec<Value> = attendees_str
                .split(',')
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .map(|email| json!({"email": email}))
                .collect();
            event_body["attendees"] = json!(attendees);
        }

        let send_updates = if send_invites { "all" } else { "none" };

        let client = reqwest::Client::new();
        let response = client
            .post("https://www.googleapis.com/calendar/v3/calendars/primary/events")
            .header("Authorization", format!("Bearer {}", provider.access_token))
            .header("Content-Type", "application/json")
            .query(&[
                ("conferenceDataVersion", "1"),
                ("sendUpdates", send_updates),
            ])
            .json(&event_body)
            .send()
            .await;

        match response {
            Ok(resp) if resp.status().is_success() => {
                let body: Value = resp.json().await?;
                let event_id = body["id"].as_str().unwrap_or("").to_string();
                let event_link = body["htmlLink"].as_str().map(String::from);

                let meet_link = body["hangoutLink"]
                    .as_str()
                    .or_else(|| {
                        body["conferenceData"]["entryPoints"]
                            .as_array()
                            .and_then(|arr| {
                                arr.iter()
                                    .find(|e| e["entryPointType"].as_str() == Some("video"))
                            })
                            .and_then(|e| e["uri"].as_str())
                    })
                    .unwrap_or("")
                    .to_string();

                let conference_id = body["conferenceData"]["conferenceId"]
                    .as_str()
                    .unwrap_or("")
                    .to_string();

                let dial_in = body["conferenceData"]["entryPoints"]
                    .as_array()
                    .and_then(|arr| {
                        arr.iter()
                            .find(|e| e["entryPointType"].as_str() == Some("phone"))
                    })
                    .and_then(|e| {
                        let number = e["uri"].as_str()?;
                        let pin = e["pin"].as_str().unwrap_or("");
                        Some(format!("{} PIN: {}", number, pin))
                    });

                let meet_info = GoogleMeetInfo {
                    meet_link: meet_link.clone(),
                    conference_id,
                    event_id: event_id.clone(),
                    event_link,
                    dial_in,
                };

                context.set_pin_value("meet_info", json!(meet_info)).await?;
                context.set_pin_value("meet_link", json!(meet_link)).await?;
                context.set_pin_value("event_id", json!(event_id)).await?;
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

fn calculate_end_time(start_time: &str, duration_minutes: i64) -> String {
    use chrono::{DateTime, Duration};

    if let Ok(dt) = DateTime::parse_from_rfc3339(start_time) {
        let end = dt + Duration::minutes(duration_minutes);
        return end.to_rfc3339();
    }

    if let Ok(dt) = DateTime::parse_from_str(start_time, "%Y-%m-%dT%H:%M:%S%:z") {
        let end = dt + Duration::minutes(duration_minutes);
        return end.to_rfc3339();
    }

    start_time.to_string()
}

// =============================================================================
// Create Instant Meeting Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct CreateInstantMeetNode {}

impl CreateInstantMeetNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for CreateInstantMeetNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_google_meet_instant",
            "Create Instant Meeting",
            "Create an instant Google Meet meeting starting now",
            "Data/Google/Meet",
        );
        node.set_flowscript_name("google.meet", "instant");
        node.add_icon("/flow/icons/google.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_input_pin(
            "provider",
            "Provider",
            "Google provider",
            VariableType::Struct,
        )
        .set_schema::<GoogleProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin("summary", "Summary", "Meeting title", VariableType::String)
            .set_default_value(Some(json!("Quick Meeting")));
        node.add_input_pin(
            "duration_minutes",
            "Duration (Minutes)",
            "Meeting duration",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(30)));

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin(
            "meet_link",
            "Meet Link",
            "Google Meet URL",
            VariableType::String,
        );
        node.add_output_pin(
            "event_id",
            "Event ID",
            "Calendar event ID",
            VariableType::String,
        );
        node.add_output_pin("error_message", "Error Message", "", VariableType::String);

        node.add_required_oauth_scopes(
            GOOGLE_PROVIDER_ID,
            vec!["https://www.googleapis.com/auth/calendar.events"],
        );
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: GoogleProvider = context.evaluate_pin("provider").await?;
        let summary: String = context
            .evaluate_pin("summary")
            .await
            .unwrap_or_else(|_| "Quick Meeting".to_string());
        let duration_minutes: i64 = context.evaluate_pin("duration_minutes").await.unwrap_or(30);

        let now = chrono::Utc::now();
        let start_time = now.to_rfc3339();
        let end_time = (now + chrono::Duration::minutes(duration_minutes)).to_rfc3339();

        let event_body = json!({
            "summary": summary,
            "start": {
                "dateTime": start_time
            },
            "end": {
                "dateTime": end_time
            },
            "conferenceData": {
                "createRequest": {
                    "requestId": format!("instant-{}", now.timestamp_millis()),
                    "conferenceSolutionKey": {
                        "type": "hangoutsMeet"
                    }
                }
            }
        });

        let client = reqwest::Client::new();
        let response = client
            .post("https://www.googleapis.com/calendar/v3/calendars/primary/events")
            .header("Authorization", format!("Bearer {}", provider.access_token))
            .header("Content-Type", "application/json")
            .query(&[("conferenceDataVersion", "1")])
            .json(&event_body)
            .send()
            .await;

        match response {
            Ok(resp) if resp.status().is_success() => {
                let body: Value = resp.json().await?;
                let event_id = body["id"].as_str().unwrap_or("").to_string();
                let meet_link = body["hangoutLink"]
                    .as_str()
                    .or_else(|| {
                        body["conferenceData"]["entryPoints"]
                            .as_array()
                            .and_then(|arr| {
                                arr.iter()
                                    .find(|e| e["entryPointType"].as_str() == Some("video"))
                            })
                            .and_then(|e| e["uri"].as_str())
                    })
                    .unwrap_or("")
                    .to_string();

                context.set_pin_value("meet_link", json!(meet_link)).await?;
                context.set_pin_value("event_id", json!(event_id)).await?;
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
// Get Meeting Details Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct GetGoogleMeetDetailsNode {}

impl GetGoogleMeetDetailsNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for GetGoogleMeetDetailsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_google_meet_get",
            "Get Meeting Details",
            "Get details of a Google Meet meeting from its calendar event",
            "Data/Google/Meet",
        );
        node.set_flowscript_name("google.meet", "get");
        node.add_icon("/flow/icons/google.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_input_pin(
            "provider",
            "Provider",
            "Google provider",
            VariableType::Struct,
        )
        .set_schema::<GoogleProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin(
            "event_id",
            "Event ID",
            "Calendar event ID",
            VariableType::String,
        );
        node.add_input_pin(
            "calendar_id",
            "Calendar ID",
            "Calendar ID",
            VariableType::String,
        )
        .set_default_value(Some(json!("primary")));

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin(
            "meet_info",
            "Meet Info",
            "Meeting information",
            VariableType::Struct,
        )
        .set_schema::<GoogleMeetInfo>();
        node.add_output_pin(
            "meet_link",
            "Meet Link",
            "Google Meet URL",
            VariableType::String,
        );
        node.add_output_pin(
            "has_meet",
            "Has Meet",
            "Whether this event has a Google Meet",
            VariableType::Boolean,
        );
        node.add_output_pin("error_message", "Error Message", "", VariableType::String);

        node.add_required_oauth_scopes(
            GOOGLE_PROVIDER_ID,
            vec!["https://www.googleapis.com/auth/calendar.readonly"],
        );
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: GoogleProvider = context.evaluate_pin("provider").await?;
        let event_id: String = context.evaluate_pin("event_id").await?;
        let calendar_id: String = context
            .evaluate_pin("calendar_id")
            .await
            .unwrap_or_else(|_| "primary".to_string());

        let client = reqwest::Client::new();
        let response = client
            .get(format!(
                "https://www.googleapis.com/calendar/v3/calendars/{}/events/{}",
                urlencoding::encode(&calendar_id),
                urlencoding::encode(&event_id)
            ))
            .header("Authorization", format!("Bearer {}", provider.access_token))
            .send()
            .await;

        match response {
            Ok(resp) if resp.status().is_success() => {
                let body: Value = resp.json().await?;

                let meet_link = body["hangoutLink"]
                    .as_str()
                    .or_else(|| {
                        body["conferenceData"]["entryPoints"]
                            .as_array()
                            .and_then(|arr| {
                                arr.iter()
                                    .find(|e| e["entryPointType"].as_str() == Some("video"))
                            })
                            .and_then(|e| e["uri"].as_str())
                    })
                    .unwrap_or("")
                    .to_string();

                let has_meet = !meet_link.is_empty();

                if has_meet {
                    let conference_id = body["conferenceData"]["conferenceId"]
                        .as_str()
                        .unwrap_or("")
                        .to_string();

                    let event_link = body["htmlLink"].as_str().map(String::from);

                    let dial_in = body["conferenceData"]["entryPoints"]
                        .as_array()
                        .and_then(|arr| {
                            arr.iter()
                                .find(|e| e["entryPointType"].as_str() == Some("phone"))
                        })
                        .and_then(|e| {
                            let number = e["uri"].as_str()?;
                            let pin = e["pin"].as_str().unwrap_or("");
                            Some(format!("{} PIN: {}", number, pin))
                        });

                    let meet_info = GoogleMeetInfo {
                        meet_link: meet_link.clone(),
                        conference_id,
                        event_id,
                        event_link,
                        dial_in,
                    };

                    context.set_pin_value("meet_info", json!(meet_info)).await?;
                }

                context.set_pin_value("meet_link", json!(meet_link)).await?;
                context.set_pin_value("has_meet", json!(has_meet)).await?;
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
// Add Meet to Event Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct AddMeetToEventNode {}

impl AddMeetToEventNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for AddMeetToEventNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_google_meet_add_to_event",
            "Add Meet to Event",
            "Add Google Meet to an existing calendar event",
            "Data/Google/Meet",
        );
        node.set_flowscript_name("google.meet", "addToEvent");
        node.add_icon("/flow/icons/google.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_input_pin(
            "provider",
            "Provider",
            "Google provider",
            VariableType::Struct,
        )
        .set_schema::<GoogleProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin(
            "event_id",
            "Event ID",
            "Calendar event ID",
            VariableType::String,
        );
        node.add_input_pin(
            "calendar_id",
            "Calendar ID",
            "Calendar ID",
            VariableType::String,
        )
        .set_default_value(Some(json!("primary")));

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin(
            "meet_link",
            "Meet Link",
            "Google Meet URL",
            VariableType::String,
        );
        node.add_output_pin("error_message", "Error Message", "", VariableType::String);

        node.add_required_oauth_scopes(
            GOOGLE_PROVIDER_ID,
            vec!["https://www.googleapis.com/auth/calendar.events"],
        );
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: GoogleProvider = context.evaluate_pin("provider").await?;
        let event_id: String = context.evaluate_pin("event_id").await?;
        let calendar_id: String = context
            .evaluate_pin("calendar_id")
            .await
            .unwrap_or_else(|_| "primary".to_string());

        let update_body = json!({
            "conferenceData": {
                "createRequest": {
                    "requestId": format!("add-meet-{}", chrono::Utc::now().timestamp_millis()),
                    "conferenceSolutionKey": {
                        "type": "hangoutsMeet"
                    }
                }
            }
        });

        let client = reqwest::Client::new();
        let response = client
            .patch(format!(
                "https://www.googleapis.com/calendar/v3/calendars/{}/events/{}",
                urlencoding::encode(&calendar_id),
                urlencoding::encode(&event_id)
            ))
            .header("Authorization", format!("Bearer {}", provider.access_token))
            .header("Content-Type", "application/json")
            .query(&[("conferenceDataVersion", "1")])
            .json(&update_body)
            .send()
            .await;

        match response {
            Ok(resp) if resp.status().is_success() => {
                let body: Value = resp.json().await?;
                let meet_link = body["hangoutLink"]
                    .as_str()
                    .or_else(|| {
                        body["conferenceData"]["entryPoints"]
                            .as_array()
                            .and_then(|arr| {
                                arr.iter()
                                    .find(|e| e["entryPointType"].as_str() == Some("video"))
                            })
                            .and_then(|e| e["uri"].as_str())
                    })
                    .unwrap_or("")
                    .to_string();

                context.set_pin_value("meet_link", json!(meet_link)).await?;
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
