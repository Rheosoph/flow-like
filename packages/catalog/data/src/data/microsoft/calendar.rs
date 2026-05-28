use super::{
    graph::{graph_error_message, graph_get_paginated_values},
    provider::{MICROSOFT_PROVIDER_ID, MicrosoftGraphProvider},
};
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
pub struct Calendar {
    pub id: String,
    pub name: String,
    pub color: Option<String>,
    pub is_default: bool,
    pub can_edit: Option<bool>,
    pub owner_name: Option<String>,
    pub owner_email: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CalendarEvent {
    pub id: String,
    pub subject: String,
    pub body_preview: Option<String>,
    pub start_date_time: String,
    pub start_time_zone: String,
    pub end_date_time: String,
    pub end_time_zone: String,
    pub location: Option<String>,
    pub is_all_day: bool,
    pub is_cancelled: bool,
    pub organizer_name: Option<String>,
    pub organizer_email: Option<String>,
    pub web_link: Option<String>,
    pub response_status: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MeetingTimeSuggestion {
    pub start_date_time: String,
    pub end_date_time: String,
    pub confidence: f64,
    pub organizer_availability: String,
    pub attendee_availability: Vec<String>,
}

fn parse_calendar(value: &Value) -> Option<Calendar> {
    Some(Calendar {
        id: value["id"].as_str()?.to_string(),
        name: value["name"].as_str()?.to_string(),
        color: value["color"].as_str().map(String::from),
        is_default: value["isDefaultCalendar"].as_bool().unwrap_or(false),
        can_edit: value["canEdit"].as_bool(),
        owner_name: value["owner"]["name"].as_str().map(String::from),
        owner_email: value["owner"]["address"].as_str().map(String::from),
    })
}

fn parse_event(value: &Value) -> Option<CalendarEvent> {
    Some(CalendarEvent {
        id: value["id"].as_str()?.to_string(),
        subject: value["subject"]
            .as_str()
            .unwrap_or("(No subject)")
            .to_string(),
        body_preview: value["bodyPreview"].as_str().map(String::from),
        start_date_time: value["start"]["dateTime"].as_str()?.to_string(),
        start_time_zone: value["start"]["timeZone"]
            .as_str()
            .unwrap_or("UTC")
            .to_string(),
        end_date_time: value["end"]["dateTime"].as_str()?.to_string(),
        end_time_zone: value["end"]["timeZone"]
            .as_str()
            .unwrap_or("UTC")
            .to_string(),
        location: value["location"]["displayName"].as_str().map(String::from),
        is_all_day: value["isAllDay"].as_bool().unwrap_or(false),
        is_cancelled: value["isCancelled"].as_bool().unwrap_or(false),
        organizer_name: value["organizer"]["emailAddress"]["name"]
            .as_str()
            .map(String::from),
        organizer_email: value["organizer"]["emailAddress"]["address"]
            .as_str()
            .map(String::from),
        web_link: value["webLink"].as_str().map(String::from),
        response_status: value["responseStatus"]["response"]
            .as_str()
            .map(String::from),
    })
}

fn parse_meeting_time(value: &Value) -> Option<MeetingTimeSuggestion> {
    Some(MeetingTimeSuggestion {
        start_date_time: value["meetingTimeSlot"]["start"]["dateTime"]
            .as_str()?
            .to_string(),
        end_date_time: value["meetingTimeSlot"]["end"]["dateTime"]
            .as_str()?
            .to_string(),
        confidence: value["confidence"].as_f64().unwrap_or(0.0),
        organizer_availability: value["organizerAvailability"]
            .as_str()
            .unwrap_or("unknown")
            .to_string(),
        attendee_availability: value["attendeeAvailability"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|a| a["availability"].as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default(),
    })
}

// =============================================================================
// List Calendars Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct ListCalendarsNode {}

impl ListCalendarsNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for ListCalendarsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_microsoft_calendar_list_calendars",
            "List Calendars",
            "List all calendars for the user",
            "Data/Microsoft/Calendar",
        );
        node.set_version(1);
        node.add_icon("/flow/icons/outlook.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_input_pin(
            "provider",
            "Provider",
            "Microsoft Graph provider",
            VariableType::Struct,
        )
        .set_schema::<MicrosoftGraphProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin("calendars", "Calendars", "", VariableType::Struct)
            .set_value_type(ValueType::Array)
            .set_schema::<Calendar>();
        node.add_output_pin("count", "Count", "", VariableType::Integer);
        node.add_output_pin("error_message", "Error Message", "", VariableType::String);

        node.add_required_oauth_scopes(MICROSOFT_PROVIDER_ID, vec!["Calendars.Read"]);
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: MicrosoftGraphProvider = context.evaluate_pin("provider").await?;

        let client = reqwest::Client::new();
        match graph_get_paginated_values(&client, &provider, provider.api_url("/me/calendars"))
            .await
        {
            Ok(values) => {
                let calendars: Vec<Calendar> = values.iter().filter_map(parse_calendar).collect();
                let count = calendars.len() as i64;
                context.set_pin_value("calendars", json!(calendars)).await?;
                context.set_pin_value("count", json!(count)).await?;
                context.activate_exec_pin("exec_out").await?;
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
// Create Calendar Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct CreateCalendarNode {}

impl CreateCalendarNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for CreateCalendarNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_microsoft_calendar_create_calendar",
            "Create Calendar",
            "Create a new calendar",
            "Data/Microsoft/Calendar",
        );
        node.set_version(1);
        node.add_icon("/flow/icons/outlook.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_input_pin(
            "provider",
            "Provider",
            "Microsoft Graph provider",
            VariableType::Struct,
        )
        .set_schema::<MicrosoftGraphProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin("name", "Name", "Calendar name", VariableType::String);
        node.add_input_pin("color", "Color", "Calendar color", VariableType::String)
            .set_default_value(Some(json!("auto")))
            .set_options(
                PinOptions::new()
                    .set_valid_values(vec![
                        "auto".to_string(),
                        "lightBlue".to_string(),
                        "lightGreen".to_string(),
                        "lightOrange".to_string(),
                        "lightGray".to_string(),
                        "lightYellow".to_string(),
                        "lightTeal".to_string(),
                        "lightPink".to_string(),
                        "lightBrown".to_string(),
                        "lightRed".to_string(),
                    ])
                    .build(),
            );

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin("calendar", "Calendar", "", VariableType::Struct)
            .set_schema::<Calendar>();
        node.add_output_pin("error_message", "Error Message", "", VariableType::String);

        node.add_required_oauth_scopes(MICROSOFT_PROVIDER_ID, vec!["Calendars.ReadWrite"]);
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: MicrosoftGraphProvider = context.evaluate_pin("provider").await?;
        let name: String = context.evaluate_pin("name").await?;
        let color: String = context
            .evaluate_pin("color")
            .await
            .unwrap_or_else(|_| "auto".to_string());

        let body = json!({
            "name": name,
            "color": color
        });

        let client = reqwest::Client::new();
        let response = client
            .post(provider.api_url("/me/calendars"))
            .header("Authorization", format!("Bearer {}", provider.access_token))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await;

        match response {
            Ok(resp) if resp.status().is_success() => {
                let body: Value = resp.json().await?;
                if let Some(calendar) = parse_calendar(&body) {
                    context.set_pin_value("calendar", json!(calendar)).await?;
                    context.activate_exec_pin("exec_out").await?;
                } else {
                    context
                        .set_pin_value("error_message", json!("Failed to parse response"))
                        .await?;
                    context.activate_exec_pin("error").await?;
                }
            }
            Ok(resp) => {
                let error = graph_error_message(resp).await;
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
pub struct ListEventsNode {}

impl ListEventsNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for ListEventsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_microsoft_calendar_list_events",
            "List Events",
            "List calendar events within a time range",
            "Data/Microsoft/Calendar",
        );
        node.set_version(1);
        node.add_icon("/flow/icons/outlook.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_input_pin(
            "provider",
            "Provider",
            "Microsoft Graph provider",
            VariableType::Struct,
        )
        .set_schema::<MicrosoftGraphProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin(
            "calendar_id",
            "Calendar ID",
            "ID of the calendar (optional, uses default)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        node.add_input_pin("start_date", "Start Date", "Start date", VariableType::Date);
        node.add_input_pin("end_date", "End Date", "End date", VariableType::Date);
        node.add_input_pin(
            "top",
            "Top",
            "Maximum number of events to return",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(50)));

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin("events", "Events", "", VariableType::Struct)
            .set_value_type(ValueType::Array)
            .set_schema::<CalendarEvent>();
        node.add_output_pin("count", "Count", "", VariableType::Integer);
        node.add_output_pin("error_message", "Error Message", "", VariableType::String);

        node.add_required_oauth_scopes(MICROSOFT_PROVIDER_ID, vec!["Calendars.Read"]);
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: MicrosoftGraphProvider = context.evaluate_pin("provider").await?;
        let calendar_id: String = context
            .evaluate_pin("calendar_id")
            .await
            .unwrap_or_default();
        let start_date: DateTime<Utc> = context.evaluate_pin("start_date").await?;
        let end_date: DateTime<Utc> = context.evaluate_pin("end_date").await?;
        let top: i64 = context.evaluate_pin("top").await.unwrap_or(50);

        let base_url = if calendar_id.is_empty() {
            provider.api_url("/me/calendar/calendarView")
        } else {
            provider.api_url(&format!("/me/calendars/{}/calendarView", calendar_id))
        };
        let url = reqwest::Url::parse_with_params(
            &base_url,
            [
                ("startDateTime", start_date.to_rfc3339()),
                ("endDateTime", end_date.to_rfc3339()),
                ("$top", top.to_string()),
            ],
        )?;

        let client = reqwest::Client::new();
        match graph_get_paginated_values(&client, &provider, url.to_string()).await {
            Ok(values) => {
                let events: Vec<CalendarEvent> = values.iter().filter_map(parse_event).collect();
                let count = events.len() as i64;
                context.set_pin_value("events", json!(events)).await?;
                context.set_pin_value("count", json!(count)).await?;
                context.activate_exec_pin("exec_out").await?;
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
pub struct CreateEventNode {}

impl CreateEventNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for CreateEventNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_microsoft_calendar_create_event",
            "Create Event",
            "Create a new calendar event",
            "Data/Microsoft/Calendar",
        );
        node.set_version(1);
        node.add_icon("/flow/icons/outlook.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_input_pin(
            "provider",
            "Provider",
            "Microsoft Graph provider",
            VariableType::Struct,
        )
        .set_schema::<MicrosoftGraphProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin("subject", "Subject", "Event subject", VariableType::String);
        node.add_input_pin(
            "body",
            "Body",
            "Event description (HTML)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        node.add_input_pin(
            "start_date_time",
            "Start DateTime",
            "Start date/time",
            VariableType::Date,
        );
        node.add_input_pin(
            "end_date_time",
            "End DateTime",
            "End date/time",
            VariableType::Date,
        );
        node.add_input_pin("time_zone", "Time Zone", "Time zone", VariableType::String)
            .set_default_value(Some(json!("UTC")));
        node.add_input_pin(
            "location",
            "Location",
            "Event location",
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
            "is_online_meeting",
            "Is Online Meeting",
            "Create Teams meeting",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin("event", "Event", "", VariableType::Struct)
            .set_schema::<CalendarEvent>();
        node.add_output_pin("error_message", "Error Message", "", VariableType::String);

        node.add_required_oauth_scopes(MICROSOFT_PROVIDER_ID, vec!["Calendars.ReadWrite"]);
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: MicrosoftGraphProvider = context.evaluate_pin("provider").await?;
        let subject: String = context.evaluate_pin("subject").await?;
        let body: String = context.evaluate_pin("body").await.unwrap_or_default();
        let start_date_time: DateTime<Utc> = context.evaluate_pin("start_date_time").await?;
        let end_date_time: DateTime<Utc> = context.evaluate_pin("end_date_time").await?;
        let time_zone: String = context
            .evaluate_pin("time_zone")
            .await
            .unwrap_or_else(|_| "UTC".to_string());
        let location: String = context.evaluate_pin("location").await.unwrap_or_default();
        let attendees: String = context.evaluate_pin("attendees").await.unwrap_or_default();
        let is_online_meeting: bool = context
            .evaluate_pin("is_online_meeting")
            .await
            .unwrap_or(false);

        let mut request_body = json!({
            "subject": subject,
            "start": {
                "dateTime": start_date_time,
                "timeZone": time_zone
            },
            "end": {
                "dateTime": end_date_time,
                "timeZone": time_zone
            }
        });

        if !body.is_empty() {
            request_body["body"] = json!({
                "contentType": "HTML",
                "content": body
            });
        }

        if !location.is_empty() {
            request_body["location"] = json!({
                "displayName": location
            });
        }

        if !attendees.is_empty() {
            let attendee_list: Vec<Value> = attendees
                .split(',')
                .map(|email| email.trim())
                .filter(|email| !email.is_empty())
                .map(|email| {
                    json!({
                        "emailAddress": {
                            "address": email
                        },
                        "type": "required"
                    })
                })
                .collect();
            request_body["attendees"] = json!(attendee_list);
        }

        if is_online_meeting {
            request_body["isOnlineMeeting"] = json!(true);
            request_body["onlineMeetingProvider"] = json!("teamsForBusiness");
        }

        let client = reqwest::Client::new();
        let response = client
            .post(provider.api_url("/me/calendar/events"))
            .header("Authorization", format!("Bearer {}", provider.access_token))
            .header("Content-Type", "application/json")
            .json(&request_body)
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
                        .set_pin_value("error_message", json!("Failed to parse response"))
                        .await?;
                    context.activate_exec_pin("error").await?;
                }
            }
            Ok(resp) => {
                let error = graph_error_message(resp).await;
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
pub struct DeleteEventNode {}

impl DeleteEventNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for DeleteEventNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_microsoft_calendar_delete_event",
            "Delete Event",
            "Delete a calendar event",
            "Data/Microsoft/Calendar",
        );
        node.set_version(1);
        node.add_icon("/flow/icons/outlook.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_input_pin(
            "provider",
            "Provider",
            "Microsoft Graph provider",
            VariableType::Struct,
        )
        .set_schema::<MicrosoftGraphProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin(
            "event_id",
            "Event ID",
            "ID of the event",
            VariableType::String,
        );

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin("error_message", "Error Message", "", VariableType::String);

        node.add_required_oauth_scopes(MICROSOFT_PROVIDER_ID, vec!["Calendars.ReadWrite"]);
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: MicrosoftGraphProvider = context.evaluate_pin("provider").await?;
        let event_id: String = context.evaluate_pin("event_id").await?;

        let client = reqwest::Client::new();
        let response = client
            .delete(provider.api_url(&format!("/me/events/{}", event_id)))
            .header("Authorization", format!("Bearer {}", provider.access_token))
            .send()
            .await;

        match response {
            Ok(resp) if resp.status().is_success() || resp.status().as_u16() == 204 => {
                context.activate_exec_pin("exec_out").await?;
            }
            Ok(resp) => {
                let error = graph_error_message(resp).await;
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
// Find Meeting Times Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct FindMeetingTimesNode {}

impl FindMeetingTimesNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for FindMeetingTimesNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_microsoft_calendar_find_meeting_times",
            "Find Meeting Times",
            "Find available meeting times for attendees",
            "Data/Microsoft/Calendar",
        );
        node.set_version(1);
        node.add_icon("/flow/icons/outlook.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_input_pin(
            "provider",
            "Provider",
            "Microsoft Graph provider",
            VariableType::Struct,
        )
        .set_schema::<MicrosoftGraphProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin(
            "attendees",
            "Attendees",
            "Comma-separated email addresses",
            VariableType::String,
        );
        node.add_input_pin(
            "duration_minutes",
            "Duration (Minutes)",
            "Meeting duration in minutes",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(30)));
        node.add_input_pin(
            "start_date",
            "Start Date",
            "Start of search window",
            VariableType::Date,
        );
        node.add_input_pin(
            "end_date",
            "End Date",
            "End of search window",
            VariableType::Date,
        );

        node.add_input_pin("time_zone", "Time Zone", "Time zone", VariableType::String)
            .set_default_value(Some(json!("UTC")));

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin("suggestions", "Suggestions", "", VariableType::Struct)
            .set_value_type(ValueType::Array)
            .set_schema::<MeetingTimeSuggestion>();
        node.add_output_pin("count", "Count", "", VariableType::Integer);
        node.add_output_pin("error_message", "Error Message", "", VariableType::String);

        node.add_required_oauth_scopes(MICROSOFT_PROVIDER_ID, vec!["Calendars.Read"]);
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: MicrosoftGraphProvider = context.evaluate_pin("provider").await?;
        let attendees: String = context.evaluate_pin("attendees").await?;
        let duration_minutes: i64 = context.evaluate_pin("duration_minutes").await.unwrap_or(30);
        let start_date: DateTime<Utc> = context.evaluate_pin("start_date").await?;
        let end_date: DateTime<Utc> = context.evaluate_pin("end_date").await?;
        let time_zone: String = context
            .evaluate_pin("time_zone")
            .await
            .unwrap_or_else(|_| "UTC".to_string());

        let attendee_list: Vec<Value> = attendees
            .split(',')
            .map(|email| email.trim())
            .filter(|email| !email.is_empty())
            .map(|email| {
                json!({
                    "type": "required",
                    "emailAddress": {
                        "address": email
                    }
                })
            })
            .collect();

        let request_body = json!({
            "attendees": attendee_list,
            "timeConstraint": {
                "activityDomain": "work",
                "timeSlots": [{
                    "start": {
                        "dateTime": start_date.to_rfc3339(),
                        "timeZone": time_zone
                    },
                    "end": {
                        "dateTime": end_date.to_rfc3339(),
                        "timeZone": time_zone
                    }
                }]
            },
            "meetingDuration": format!("PT{}M", duration_minutes)
        });

        let client = reqwest::Client::new();
        let response = client
            .post(provider.api_url("/me/findMeetingTimes"))
            .header("Authorization", format!("Bearer {}", provider.access_token))
            .header("Content-Type", "application/json")
            .json(&request_body)
            .send()
            .await;

        match response {
            Ok(resp) if resp.status().is_success() => {
                let body: Value = resp.json().await?;
                let suggestions: Vec<MeetingTimeSuggestion> = body["meetingTimeSuggestions"]
                    .as_array()
                    .map(|arr| arr.iter().filter_map(parse_meeting_time).collect())
                    .unwrap_or_default();
                let count = suggestions.len() as i64;
                context
                    .set_pin_value("suggestions", json!(suggestions))
                    .await?;
                context.set_pin_value("count", json!(count)).await?;
                context.activate_exec_pin("exec_out").await?;
            }
            Ok(resp) => {
                let error = graph_error_message(resp).await;
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
// Get Schedule (Free/Busy) Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct GetScheduleNode {}

impl GetScheduleNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for GetScheduleNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_microsoft_calendar_get_schedule",
            "Get Schedule",
            "Get free/busy schedule for users",
            "Data/Microsoft/Calendar",
        );
        node.set_version(1);
        node.add_icon("/flow/icons/outlook.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_input_pin(
            "provider",
            "Provider",
            "Microsoft Graph provider",
            VariableType::Struct,
        )
        .set_schema::<MicrosoftGraphProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin(
            "schedules",
            "Schedules",
            "Comma-separated email addresses",
            VariableType::String,
        );
        node.add_input_pin(
            "start_date_time",
            "Start DateTime",
            "Start date/time",
            VariableType::Date,
        );
        node.add_input_pin(
            "end_date_time",
            "End DateTime",
            "End date/time",
            VariableType::Date,
        );
        node.add_input_pin("time_zone", "Time Zone", "Time zone", VariableType::String)
            .set_default_value(Some(json!("UTC")));
        node.add_input_pin(
            "interval_minutes",
            "Interval (Minutes)",
            "Availability interval in minutes",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(30)));

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin(
            "schedule_data",
            "Schedule Data",
            "Raw schedule response",
            VariableType::Struct,
        );
        node.add_output_pin("error_message", "Error Message", "", VariableType::String);

        node.add_required_oauth_scopes(MICROSOFT_PROVIDER_ID, vec!["Calendars.Read"]);
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: MicrosoftGraphProvider = context.evaluate_pin("provider").await?;
        let schedules: String = context.evaluate_pin("schedules").await?;
        let start_date_time: DateTime<Utc> = context.evaluate_pin("start_date_time").await?;
        let end_date_time: DateTime<Utc> = context.evaluate_pin("end_date_time").await?;
        let time_zone: String = context
            .evaluate_pin("time_zone")
            .await
            .unwrap_or_else(|_| "UTC".to_string());
        let interval_minutes: i64 = context.evaluate_pin("interval_minutes").await.unwrap_or(30);

        let schedule_list: Vec<String> = schedules
            .split(',')
            .map(|email| email.trim().to_string())
            .filter(|email| !email.is_empty())
            .collect();

        let request_body = json!({
            "schedules": schedule_list,
            "startTime": {
                "dateTime": start_date_time.to_rfc3339(),
                "timeZone": time_zone
            },
            "endTime": {
                "dateTime": end_date_time.to_rfc3339(),
                "timeZone": time_zone
            },
            "availabilityViewInterval": interval_minutes
        });

        let client = reqwest::Client::new();
        let response = client
            .post(provider.api_url("/me/calendar/getSchedule"))
            .header("Authorization", format!("Bearer {}", provider.access_token))
            .header("Content-Type", "application/json")
            .json(&request_body)
            .send()
            .await;

        match response {
            Ok(resp) if resp.status().is_success() => {
                let body: Value = resp.json().await?;
                context.set_pin_value("schedule_data", body).await?;
                context.activate_exec_pin("exec_out").await?;
            }
            Ok(resp) => {
                let error = graph_error_message(resp).await;
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
pub struct UpdateEventNode {}

impl UpdateEventNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for UpdateEventNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_microsoft_calendar_update_event",
            "Update Event",
            "Update an existing calendar event",
            "Data/Microsoft/Calendar",
        );
        node.set_version(1);
        node.add_icon("/flow/icons/outlook.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_input_pin(
            "provider",
            "Provider",
            "Microsoft Graph provider",
            VariableType::Struct,
        )
        .set_schema::<MicrosoftGraphProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin(
            "event_id",
            "Event ID",
            "ID of the event",
            VariableType::String,
        );
        node.add_input_pin(
            "subject",
            "Subject",
            "New subject (leave empty to keep)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        node.add_input_pin(
            "start_date_time",
            "Start DateTime",
            "New start (leave empty to keep)",
            VariableType::Date,
        );
        node.add_input_pin(
            "end_date_time",
            "End DateTime",
            "New end (leave empty to keep)",
            VariableType::Date,
        );
        node.add_input_pin(
            "time_zone",
            "Time Zone",
            "Time zone for dates",
            VariableType::String,
        )
        .set_default_value(Some(json!("UTC")));
        node.add_input_pin(
            "location",
            "Location",
            "New location (leave empty to keep)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin("event", "Event", "", VariableType::Struct)
            .set_schema::<CalendarEvent>();
        node.add_output_pin("error_message", "Error Message", "", VariableType::String);

        node.add_required_oauth_scopes(MICROSOFT_PROVIDER_ID, vec!["Calendars.ReadWrite"]);
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: MicrosoftGraphProvider = context.evaluate_pin("provider").await?;
        let event_id: String = context.evaluate_pin("event_id").await?;
        let subject: String = context.evaluate_pin("subject").await.unwrap_or_default();
        let start_date_time: Option<DateTime<Utc>> =
            context.evaluate_pin("start_date_time").await.ok();
        let end_date_time: Option<DateTime<Utc>> = context.evaluate_pin("end_date_time").await.ok();
        let time_zone: String = context
            .evaluate_pin("time_zone")
            .await
            .unwrap_or_else(|_| "UTC".to_string());
        let location: String = context.evaluate_pin("location").await.unwrap_or_default();

        let mut request_body = json!({});

        if !subject.is_empty() {
            request_body["subject"] = json!(subject);
        }
        if let Some(start_date_time) = start_date_time {
            request_body["start"] = json!({
                "dateTime": start_date_time.to_rfc3339(),
                "timeZone": time_zone
            });
        }
        if let Some(end_date_time) = end_date_time {
            request_body["end"] = json!({
                "dateTime": end_date_time.to_rfc3339(),
                "timeZone": time_zone
            });
        }
        if !location.is_empty() {
            request_body["location"] = json!({
                "displayName": location
            });
        }

        let client = reqwest::Client::new();
        let response = client
            .patch(provider.api_url(&format!("/me/events/{}", event_id)))
            .header("Authorization", format!("Bearer {}", provider.access_token))
            .header("Content-Type", "application/json")
            .json(&request_body)
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
                        .set_pin_value("error_message", json!("Failed to parse response"))
                        .await?;
                    context.activate_exec_pin("error").await?;
                }
            }
            Ok(resp) => {
                let error = graph_error_message(resp).await;
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
