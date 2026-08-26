use super::provider::{GOOGLE_PROVIDER_ID, GoogleProvider};
use chrono::{DateTime, Utc};
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_types::{JsonSchema, Value, async_trait, json::json, reqwest};
use serde::{Deserialize, Serialize};

// =============================================================================
// Calendar Types
// =============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GoogleCalendar {
    pub id: String,
    pub summary: String,
    pub description: Option<String>,
    pub time_zone: Option<String>,
    pub primary: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GoogleCalendarEvent {
    pub id: String,
    pub summary: Option<String>,
    pub description: Option<String>,
    pub location: Option<String>,
    pub start: Option<String>,
    pub end: Option<String>,
    pub status: Option<String>,
    pub html_link: Option<String>,
    pub hangout_link: Option<String>,
    pub attendees: Vec<GoogleEventAttendee>,
    pub organizer: Option<GoogleEventOrganizer>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GoogleEventAttendee {
    pub email: String,
    pub display_name: Option<String>,
    pub response_status: Option<String>,
    pub organizer: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GoogleEventOrganizer {
    pub email: String,
    pub display_name: Option<String>,
}

fn parse_event(value: &Value) -> Option<GoogleCalendarEvent> {
    let start = value["start"]["dateTime"]
        .as_str()
        .or_else(|| value["start"]["date"].as_str())
        .map(String::from);
    let end = value["end"]["dateTime"]
        .as_str()
        .or_else(|| value["end"]["date"].as_str())
        .map(String::from);

    let attendees = value["attendees"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|a| {
                    Some(GoogleEventAttendee {
                        email: a["email"].as_str()?.to_string(),
                        display_name: a["displayName"].as_str().map(String::from),
                        response_status: a["responseStatus"].as_str().map(String::from),
                        organizer: a["organizer"].as_bool().unwrap_or(false),
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    let organizer = value["organizer"]["email"]
        .as_str()
        .map(|email| GoogleEventOrganizer {
            email: email.to_string(),
            display_name: value["organizer"]["displayName"].as_str().map(String::from),
        });

    Some(GoogleCalendarEvent {
        id: value["id"].as_str()?.to_string(),
        summary: value["summary"].as_str().map(String::from),
        description: value["description"].as_str().map(String::from),
        location: value["location"].as_str().map(String::from),
        start,
        end,
        status: value["status"].as_str().map(String::from),
        html_link: value["htmlLink"].as_str().map(String::from),
        hangout_link: value["hangoutLink"].as_str().map(String::from),
        attendees,
        organizer,
    })
}

// =============================================================================
// List Calendars Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct ListGoogleCalendarsNode {}

impl ListGoogleCalendarsNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for ListGoogleCalendarsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_google_calendar_list",
            "List Calendars",
            "List all Google Calendars",
            "Data/Google/Calendar",
        );
        node.set_flowscript_name("google.calendar", "list");
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

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin(
            "calendars",
            "Calendars",
            "List of calendars",
            VariableType::Struct,
        )
        .set_value_type(ValueType::Array)
        .set_schema::<GoogleCalendar>();
        node.add_output_pin(
            "primary_calendar_id",
            "Primary Calendar ID",
            "",
            VariableType::String,
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

        let client = reqwest::Client::new();
        let response = client
            .get("https://www.googleapis.com/calendar/v3/users/me/calendarList")
            .header("Authorization", format!("Bearer {}", provider.access_token))
            .send()
            .await;

        match response {
            Ok(resp) if resp.status().is_success() => {
                let body: Value = resp.json().await?;
                let mut primary_id = String::new();
                let calendars: Vec<GoogleCalendar> = body["items"]
                    .as_array()
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|c| {
                                let is_primary = c["primary"].as_bool().unwrap_or(false);
                                let calendar = GoogleCalendar {
                                    id: c["id"].as_str()?.to_string(),
                                    summary: c["summary"].as_str().unwrap_or("").to_string(),
                                    description: c["description"].as_str().map(String::from),
                                    time_zone: c["timeZone"].as_str().map(String::from),
                                    primary: is_primary,
                                };
                                if is_primary {
                                    primary_id = calendar.id.clone();
                                }
                                Some(calendar)
                            })
                            .collect()
                    })
                    .unwrap_or_default();

                context.set_pin_value("calendars", json!(calendars)).await?;
                context
                    .set_pin_value("primary_calendar_id", json!(primary_id))
                    .await?;
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
// List Events Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct ListGoogleCalendarEventsNode {}

impl ListGoogleCalendarEventsNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for ListGoogleCalendarEventsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_google_calendar_list_events",
            "List Events",
            "List events from a Google Calendar",
            "Data/Google/Calendar",
        );
        node.set_flowscript_name("google.calendar", "listEvents");
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
            "calendar_id",
            "Calendar ID",
            "Calendar ID (default: primary)",
            VariableType::String,
        )
        .set_default_value(Some(json!("primary")));
        node.add_input_pin(
            "time_min",
            "Time Min",
            "Start time (RFC3339, e.g., 2024-01-01T00:00:00Z)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        node.add_input_pin(
            "time_max",
            "Time Max",
            "End time (RFC3339)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        node.add_input_pin(
            "max_results",
            "Max Results",
            "Maximum results (1-2500)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(100)));
        node.add_input_pin(
            "page_token",
            "Page Token",
            "Token for pagination",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin("events", "Events", "List of events", VariableType::Struct)
            .set_value_type(ValueType::Array)
            .set_schema::<GoogleCalendarEvent>();
        node.add_output_pin(
            "next_page_token",
            "Next Page Token",
            "",
            VariableType::String,
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
        let calendar_id: String = context
            .evaluate_pin("calendar_id")
            .await
            .unwrap_or_else(|_| "primary".to_string());
        let time_min: String = context.evaluate_pin("time_min").await.unwrap_or_default();
        let time_max: String = context.evaluate_pin("time_max").await.unwrap_or_default();
        let max_results: i64 = context
            .evaluate_pin("max_results")
            .await
            .unwrap_or(100)
            .min(2500);
        let page_token: String = context.evaluate_pin("page_token").await.unwrap_or_default();

        let mut query_params: Vec<(&str, String)> = vec![
            ("maxResults", max_results.to_string()),
            ("singleEvents", "true".to_string()),
            ("orderBy", "startTime".to_string()),
        ];

        if !time_min.is_empty() {
            query_params.push(("timeMin", time_min));
        }
        if !time_max.is_empty() {
            query_params.push(("timeMax", time_max));
        }
        if !page_token.is_empty() {
            query_params.push(("pageToken", page_token));
        }

        let client = reqwest::Client::new();
        let response = client
            .get(format!(
                "https://www.googleapis.com/calendar/v3/calendars/{}/events",
                urlencoding::encode(&calendar_id)
            ))
            .header("Authorization", format!("Bearer {}", provider.access_token))
            .query(&query_params)
            .send()
            .await;

        match response {
            Ok(resp) if resp.status().is_success() => {
                let body: Value = resp.json().await?;
                let events: Vec<GoogleCalendarEvent> = body["items"]
                    .as_array()
                    .map(|arr| arr.iter().filter_map(parse_event).collect())
                    .unwrap_or_default();
                let next_page_token = body["nextPageToken"].as_str().unwrap_or("").to_string();

                context.set_pin_value("events", json!(events)).await?;
                context
                    .set_pin_value("next_page_token", json!(next_page_token))
                    .await?;
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
// Get Event Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct GetGoogleCalendarEventNode {}

impl GetGoogleCalendarEventNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for GetGoogleCalendarEventNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_google_calendar_get_event",
            "Get Event",
            "Get a specific calendar event",
            "Data/Google/Calendar",
        );
        node.set_flowscript_name("google.calendar", "getEvent");
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
            "calendar_id",
            "Calendar ID",
            "Calendar ID",
            VariableType::String,
        )
        .set_default_value(Some(json!("primary")));
        node.add_input_pin("event_id", "Event ID", "Event ID", VariableType::String);

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin("event", "Event", "Event details", VariableType::Struct)
            .set_schema::<GoogleCalendarEvent>();
        node.add_output_pin("raw", "Raw", "Raw API response", VariableType::Generic);
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
        let calendar_id: String = context
            .evaluate_pin("calendar_id")
            .await
            .unwrap_or_else(|_| "primary".to_string());
        let event_id: String = context.evaluate_pin("event_id").await?;

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
                if let Some(event) = parse_event(&body) {
                    context.set_pin_value("event", json!(event)).await?;
                    context.set_pin_value("raw", body).await?;
                    context.activate_exec_pin("exec_out").await?;
                } else {
                    context
                        .set_pin_value("error_message", json!("Failed to parse event"))
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
// Create Event Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct CreateGoogleCalendarEventNode {}

impl CreateGoogleCalendarEventNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for CreateGoogleCalendarEventNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_google_calendar_create_event",
            "Create Event",
            "Create a new calendar event",
            "Data/Google/Calendar",
        );
        node.set_flowscript_name("google.calendar", "createEvent");
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
            "calendar_id",
            "Calendar ID",
            "Calendar ID",
            VariableType::String,
        )
        .set_default_value(Some(json!("primary")));
        node.add_input_pin("summary", "Summary", "Event title", VariableType::String);
        node.add_input_pin(
            "description",
            "Description",
            "Event description",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        node.add_input_pin(
            "location",
            "Location",
            "Event location",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        node.add_input_pin("start_time", "Start Time", "Start time", VariableType::Date);
        node.add_input_pin("end_time", "End Time", "End time", VariableType::Date);
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
            "add_meet",
            "Add Google Meet",
            "Add Google Meet conference",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin("event", "Event", "Created event", VariableType::Struct)
            .set_schema::<GoogleCalendarEvent>();
        node.add_output_pin("event_id", "Event ID", "", VariableType::String);
        node.add_output_pin(
            "meet_link",
            "Meet Link",
            "Google Meet link (if created)",
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
        let calendar_id: String = context
            .evaluate_pin("calendar_id")
            .await
            .unwrap_or_else(|_| "primary".to_string());
        let summary: String = context.evaluate_pin("summary").await?;
        let description: String = context
            .evaluate_pin("description")
            .await
            .unwrap_or_default();
        let location: String = context.evaluate_pin("location").await.unwrap_or_default();
        let start_time: DateTime<Utc> = context.evaluate_pin("start_time").await?;
        let end_time: DateTime<Utc> = context.evaluate_pin("end_time").await?;
        let time_zone: String = context.evaluate_pin("time_zone").await.unwrap_or_default();
        let attendees_str: String = context.evaluate_pin("attendees").await.unwrap_or_default();
        let add_meet: bool = context.evaluate_pin("add_meet").await.unwrap_or(false);

        let mut event_body = json!({
            "summary": summary,
            "start": {
                "dateTime": start_time.to_rfc3339()
            },
            "end": {
                "dateTime": end_time.to_rfc3339()
            }
        });

        if !description.is_empty() {
            event_body["description"] = json!(description);
        }
        if !location.is_empty() {
            event_body["location"] = json!(location);
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

        if add_meet {
            event_body["conferenceData"] = json!({
                "createRequest": {
                    "requestId": format!("meet-{}", chrono::Utc::now().timestamp_millis()),
                    "conferenceSolutionKey": {
                        "type": "hangoutsMeet"
                    }
                }
            });
        }

        let mut query_params = vec![];
        if add_meet {
            query_params.push(("conferenceDataVersion", "1"));
        }

        let client = reqwest::Client::new();
        let response = client
            .post(format!(
                "https://www.googleapis.com/calendar/v3/calendars/{}/events",
                urlencoding::encode(&calendar_id)
            ))
            .header("Authorization", format!("Bearer {}", provider.access_token))
            .header("Content-Type", "application/json")
            .query(&query_params)
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

                if let Some(event) = parse_event(&body) {
                    context.set_pin_value("event", json!(event)).await?;
                }
                context.set_pin_value("event_id", json!(event_id)).await?;
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

// =============================================================================
// Update Event Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct UpdateGoogleCalendarEventNode {}

impl UpdateGoogleCalendarEventNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for UpdateGoogleCalendarEventNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_google_calendar_update_event",
            "Update Event",
            "Update an existing calendar event",
            "Data/Google/Calendar",
        );
        node.set_flowscript_name("google.calendar", "updateEvent");
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
            "calendar_id",
            "Calendar ID",
            "Calendar ID",
            VariableType::String,
        )
        .set_default_value(Some(json!("primary")));
        node.add_input_pin(
            "event_id",
            "Event ID",
            "Event ID to update",
            VariableType::String,
        );
        node.add_input_pin(
            "summary",
            "Summary",
            "Event title (empty to keep)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        node.add_input_pin(
            "description",
            "Description",
            "Event description (empty to keep)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        node.add_input_pin(
            "location",
            "Location",
            "Event location (empty to keep)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "start_time",
            "Start Time",
            "Start time (leave empty to keep)",
            VariableType::Date,
        );

        node.add_input_pin(
            "end_time",
            "End Time",
            "End time (leave empty to keep)",
            VariableType::Date,
        );

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin("event", "Event", "Updated event", VariableType::Struct)
            .set_schema::<GoogleCalendarEvent>();
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
        let calendar_id: String = context
            .evaluate_pin("calendar_id")
            .await
            .unwrap_or_else(|_| "primary".to_string());
        let event_id: String = context.evaluate_pin("event_id").await?;
        let summary: String = context.evaluate_pin("summary").await.unwrap_or_default();
        let description: String = context
            .evaluate_pin("description")
            .await
            .unwrap_or_default();
        let location: String = context.evaluate_pin("location").await.unwrap_or_default();

        let mut event_body = json!({});

        if !summary.is_empty() {
            event_body["summary"] = json!(summary);
        }
        if !description.is_empty() {
            event_body["description"] = json!(description);
        }
        if !location.is_empty() {
            event_body["location"] = json!(location);
        }

        // Handle optional start_time
        if let Ok(start_time) = context.evaluate_pin::<DateTime<Utc>>("start_time").await {
            event_body["start"] = json!({"dateTime": start_time.to_rfc3339()});
        }

        // Handle optional end_time
        if let Ok(end_time) = context.evaluate_pin::<DateTime<Utc>>("end_time").await {
            event_body["end"] = json!({"dateTime": end_time.to_rfc3339()});
        }

        let client = reqwest::Client::new();
        let response = client
            .patch(format!(
                "https://www.googleapis.com/calendar/v3/calendars/{}/events/{}",
                urlencoding::encode(&calendar_id),
                urlencoding::encode(&event_id)
            ))
            .header("Authorization", format!("Bearer {}", provider.access_token))
            .header("Content-Type", "application/json")
            .json(&event_body)
            .send()
            .await;

        match response {
            Ok(resp) if resp.status().is_success() => {
                let body: Value = resp.json().await?;
                if let Some(event) = parse_event(&body) {
                    context.set_pin_value("event", json!(event)).await?;
                    context.activate_exec_pin("exec_out").await?;
                } else {
                    context
                        .set_pin_value("error_message", json!("Failed to parse event"))
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
// Delete Event Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct DeleteGoogleCalendarEventNode {}

impl DeleteGoogleCalendarEventNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for DeleteGoogleCalendarEventNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_google_calendar_delete_event",
            "Delete Event",
            "Delete a calendar event",
            "Data/Google/Calendar",
        );
        node.set_flowscript_name("google.calendar", "deleteEvent");
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
            "calendar_id",
            "Calendar ID",
            "Calendar ID",
            VariableType::String,
        )
        .set_default_value(Some(json!("primary")));
        node.add_input_pin(
            "event_id",
            "Event ID",
            "Event ID to delete",
            VariableType::String,
        );
        node.add_input_pin(
            "send_notifications",
            "Send Notifications",
            "Notify attendees",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
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
        let calendar_id: String = context
            .evaluate_pin("calendar_id")
            .await
            .unwrap_or_else(|_| "primary".to_string());
        let event_id: String = context.evaluate_pin("event_id").await?;
        let send_notifications: bool = context
            .evaluate_pin("send_notifications")
            .await
            .unwrap_or(false);

        let client = reqwest::Client::new();
        let response = client
            .delete(format!(
                "https://www.googleapis.com/calendar/v3/calendars/{}/events/{}",
                urlencoding::encode(&calendar_id),
                urlencoding::encode(&event_id)
            ))
            .header("Authorization", format!("Bearer {}", provider.access_token))
            .query(&[(
                "sendUpdates",
                if send_notifications { "all" } else { "none" },
            )])
            .send()
            .await;

        match response {
            Ok(resp) if resp.status().is_success() || resp.status().as_u16() == 204 => {
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
// Quick Add Event Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct QuickAddGoogleCalendarEventNode {}

impl QuickAddGoogleCalendarEventNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for QuickAddGoogleCalendarEventNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_google_calendar_quick_add",
            "Quick Add Event",
            "Create an event from natural language text",
            "Data/Google/Calendar",
        );
        node.set_flowscript_name("google.calendar", "quickAdd");
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
            "calendar_id",
            "Calendar ID",
            "Calendar ID",
            VariableType::String,
        )
        .set_default_value(Some(json!("primary")));
        node.add_input_pin(
            "text",
            "Text",
            "Natural language event description (e.g., 'Lunch with John tomorrow at noon')",
            VariableType::String,
        );

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin("event", "Event", "Created event", VariableType::Struct)
            .set_schema::<GoogleCalendarEvent>();
        node.add_output_pin("event_id", "Event ID", "", VariableType::String);
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
        let calendar_id: String = context
            .evaluate_pin("calendar_id")
            .await
            .unwrap_or_else(|_| "primary".to_string());
        let text: String = context.evaluate_pin("text").await?;

        let client = reqwest::Client::new();
        let response = client
            .post(format!(
                "https://www.googleapis.com/calendar/v3/calendars/{}/events/quickAdd",
                urlencoding::encode(&calendar_id)
            ))
            .header("Authorization", format!("Bearer {}", provider.access_token))
            .query(&[("text", &text)])
            .send()
            .await;

        match response {
            Ok(resp) if resp.status().is_success() => {
                let body: Value = resp.json().await?;
                let event_id = body["id"].as_str().unwrap_or("").to_string();

                if let Some(event) = parse_event(&body) {
                    context.set_pin_value("event", json!(event)).await?;
                }
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
// Free/Busy Query Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct QueryFreeBusyNode {}

impl QueryFreeBusyNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for QueryFreeBusyNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_google_calendar_free_busy",
            "Query Free/Busy",
            "Query free/busy information for calendars",
            "Data/Google/Calendar",
        );
        node.set_flowscript_name("google.calendar", "freeBusy");
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
            "time_min",
            "Time Min",
            "Start time (RFC3339)",
            VariableType::String,
        );
        node.add_input_pin(
            "time_max",
            "Time Max",
            "End time (RFC3339)",
            VariableType::String,
        );
        node.add_input_pin(
            "calendar_ids",
            "Calendar IDs",
            "Comma-separated calendar IDs (default: primary)",
            VariableType::String,
        )
        .set_default_value(Some(json!("primary")));

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin(
            "busy_times",
            "Busy Times",
            "Busy time slots per calendar",
            VariableType::Generic,
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
        let time_min: String = context.evaluate_pin("time_min").await?;
        let time_max: String = context.evaluate_pin("time_max").await?;
        let calendar_ids: String = context
            .evaluate_pin("calendar_ids")
            .await
            .unwrap_or_else(|_| "primary".to_string());

        let calendars: Vec<Value> = calendar_ids
            .split(',')
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(|id| json!({"id": id}))
            .collect();

        let request_body = json!({
            "timeMin": time_min,
            "timeMax": time_max,
            "items": calendars
        });

        let client = reqwest::Client::new();
        let response = client
            .post("https://www.googleapis.com/calendar/v3/freeBusy")
            .header("Authorization", format!("Bearer {}", provider.access_token))
            .header("Content-Type", "application/json")
            .json(&request_body)
            .send()
            .await;

        match response {
            Ok(resp) if resp.status().is_success() => {
                let body: Value = resp.json().await?;
                context
                    .set_pin_value("busy_times", body["calendars"].clone())
                    .await?;
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
