#[cfg(feature = "execute")]
use chrono::{DateTime, NaiveDateTime, Utc};
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
#[cfg(feature = "execute")]
use flow_like_types::{anyhow, async_trait, json::json, reqwest};
#[cfg(not(feature = "execute"))]
use flow_like_types::{async_trait, json::json};
#[cfg(feature = "execute")]
use futures::TryStreamExt;
#[cfg(feature = "execute")]
use icalendar::{Calendar, CalendarDateTime, Component, DatePerhapsTime, Event};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::mail::imap::ImapConnection;
use crate::mail::imap::inbox::ImapInbox;

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct CalendarEvent {
    pub uid: String,
    pub summary: Option<String>,
    pub description: Option<String>,
    pub location: Option<String>,
    pub start: Option<String>,
    pub end: Option<String>,
    pub organizer: Option<String>,
    pub attendees: Vec<String>,
    pub status: Option<String>,
}

#[crate::register_node]
#[derive(Default)]
pub struct ImapListCalendarEventsNode;

impl ImapListCalendarEventsNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for ImapListCalendarEventsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "mail_imap_calendar_list_events",
            "List Calendar Events",
            "Lists calendar events from an IMAP calendar folder",
            "Email/IMAP/Calendar",
        );
        node.set_flowscript_name("imap", "listCalendarEvents");
        node.set_receiver("connection");
        node.add_icon("/flow/icons/calendar.svg");

        node.add_input_pin("exec_in", "In", "Execution input", VariableType::Execution);
        node.add_output_pin(
            "exec_out",
            "Out",
            "Execution output",
            VariableType::Execution,
        );

        node.add_input_pin(
            "connection",
            "Connection",
            "IMAP connection",
            VariableType::Struct,
        )
        .set_schema::<ImapConnection>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "calendar_folder",
            "Calendar Folder",
            "Calendar folder name",
            VariableType::String,
        )
        .set_default_value(Some(json!("Calendar")));

        node.add_input_pin(
            "start_date",
            "Start Date",
            "Filter events starting from this date",
            VariableType::Date,
        );

        node.add_input_pin(
            "end_date",
            "End Date",
            "Filter events ending before this date",
            VariableType::Date,
        );

        node.add_output_pin(
            "events",
            "Events",
            "List of calendar events",
            VariableType::Struct,
        )
        .set_schema::<CalendarEvent>()
        .set_value_type(ValueType::Array);

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let connection: ImapConnection = context.evaluate_pin("connection").await?;
        let calendar_folder: String = context.evaluate_pin("calendar_folder").await?;
        let start_date: Option<DateTime<Utc>> = context.evaluate_pin("start_date").await.ok();
        let end_date: Option<DateTime<Utc>> = context.evaluate_pin("end_date").await.ok();

        let session_arc = connection.to_session(context).await?;
        let mut session = session_arc.lock().await;

        session
            .select(&calendar_folder)
            .await
            .map_err(|e| anyhow!("Failed to select calendar folder: {}", e))?;

        let uids = session
            .uid_search("ALL")
            .await
            .map_err(|e| anyhow!("Failed to search calendar: {}", e))?;

        let mut events = Vec::new();

        for uid in uids.iter() {
            let fetch = session
                .uid_fetch(uid.to_string(), "BODY.PEEK[]")
                .await
                .map_err(|e| anyhow!("Failed to fetch event: {}", e))?
                .try_collect::<Vec<_>>()
                .await?;

            if let Some(msg) = fetch.first()
                && let Some(body) = msg.body()
                && let Ok(body_str) = std::str::from_utf8(body)
                && let Ok(calendar) = body_str.parse::<Calendar>()
            {
                for component in calendar.components {
                    if let icalendar::CalendarComponent::Event(event) = component {
                        let event_data = parse_calendar_event(&event);

                        let should_include = if let (Some(start), Some(end)) =
                            (start_date, end_date)
                        {
                            if let Some(event_start_str) = &event_data.start {
                                if let Some(event_start) = parse_event_datetime(event_start_str) {
                                    event_start >= start && event_start <= end
                                } else {
                                    false
                                }
                            } else {
                                false
                            }
                        } else {
                            true
                        };

                        if should_include {
                            events.push(event_data);
                        }
                    }
                }
            }
        }

        context.set_pin_value("events", json!(events)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Web functionality requires the 'execute' feature"
        ))
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct ImapListCalendarsNode;

impl ImapListCalendarsNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for ImapListCalendarsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "mail_imap_calendar_list",
            "IMAP List Calendars",
            "Lists mailbox names and heuristically-detected calendar folders",
            "Email/IMAP/Calendar",
        );
        node.set_flowscript_name("imap", "listCalendars");
        node.set_receiver("connection");
        node.add_icon("/flow/icons/calendar.svg");

        node.add_input_pin("exec_in", "In", "Execution input", VariableType::Execution);
        node.add_output_pin(
            "exec_out",
            "Out",
            "Execution output",
            VariableType::Execution,
        );

        node.add_input_pin(
            "connection",
            "Connection",
            "IMAP connection",
            VariableType::Struct,
        )
        .set_schema::<ImapConnection>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "names",
            "Mailbox Names",
            "All mailbox names returned by the server",
            VariableType::String,
        )
        .set_value_type(ValueType::Array);

        node.add_output_pin(
            "calendar_names",
            "Calendar Names",
            "Mailbox names that look like calendar folders",
            VariableType::String,
        )
        .set_value_type(ValueType::Array);

        node.add_output_pin(
            "calendars",
            "Calendar Inboxes",
            "Detected calendar mailboxes wrapped as ImapInbox",
            VariableType::Struct,
        )
        .set_schema::<ImapInbox>()
        .set_value_type(ValueType::Array);

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let connection: ImapConnection = context.evaluate_pin("connection").await?;
        let session_arc = connection.to_session(context).await?;
        let mut session = session_arc.lock().await;

        let name_stream = session
            .list(None, Some("*"))
            .await
            .map_err(|e| anyhow!("IMAP LIST failed: {}", e))?;

        let names: Vec<String> = name_stream
            .map_ok(|n| n.name().to_string())
            .try_collect()
            .await
            .map_err(|e| anyhow!("IMAP LIST stream failed: {}", e))?;

        // Heuristic: consider mailboxes that contain "calendar" or "cal" in the name
        let calendar_names: Vec<String> = names
            .iter()
            .filter(|&n| {
                let s = n.to_lowercase();
                s.contains("calendar") || s.contains("cal") || s.contains("events")
            })
            .cloned()
            .collect();

        let calendars: Vec<ImapInbox> = calendar_names
            .iter()
            .cloned()
            .map(|n| ImapInbox::new(connection.clone(), n))
            .collect();

        context.set_pin_value("names", json!(names)).await?;
        context
            .set_pin_value("calendar_names", json!(calendar_names))
            .await?;
        context.set_pin_value("calendars", json!(calendars)).await?;

        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Web functionality requires the 'execute' feature"
        ))
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct ImapSubscribeCalendarNode;

impl ImapSubscribeCalendarNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for ImapSubscribeCalendarNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "mail_imap_calendar_subscribe",
            "Subscribe to Calendar URL",
            "Fetches and parses calendar events from an iCalendar subscription URL",
            "Email/IMAP/Calendar",
        );
        node.set_flowscript_name("imap", "subscribeCalendar");
        node.add_icon("/flow/icons/calendar.svg");

        node.add_input_pin("exec_in", "In", "Execution input", VariableType::Execution);
        node.add_output_pin(
            "exec_out",
            "Success",
            "Calendar fetched and parsed",
            VariableType::Execution,
        );
        node.add_output_pin(
            "error",
            "Error",
            "Failed to fetch or parse calendar",
            VariableType::Execution,
        );

        node.add_input_pin(
            "url",
            "Calendar URL",
            "iCalendar subscription URL (.ics)",
            VariableType::String,
        );

        node.add_input_pin(
            "start_date",
            "Start Date",
            "Filter events starting from this date",
            VariableType::Date,
        );

        node.add_input_pin(
            "end_date",
            "End Date",
            "Filter events ending before this date",
            VariableType::Date,
        );

        node.add_output_pin(
            "events",
            "Events",
            "List of calendar events",
            VariableType::Struct,
        )
        .set_schema::<CalendarEvent>()
        .set_value_type(ValueType::Array);

        node.add_output_pin(
            "calendar_name",
            "Calendar Name",
            "Name from X-WR-CALNAME if present",
            VariableType::String,
        );

        node.add_output_pin(
            "error_message",
            "Error Message",
            "Error details",
            VariableType::String,
        );

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let url: String = context.evaluate_pin("url").await?;
        let start_date: Option<DateTime<Utc>> = context.evaluate_pin("start_date").await.ok();
        let end_date: Option<DateTime<Utc>> = context.evaluate_pin("end_date").await.ok();

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| anyhow!("Failed to create HTTP client: {}", e))?;

        let response = match client.get(&url).send().await {
            Ok(resp) => resp,
            Err(e) => {
                context
                    .set_pin_value(
                        "error_message",
                        json!(format!("Failed to fetch URL: {}", e)),
                    )
                    .await?;
                context.activate_exec_pin("error").await?;
                return Ok(());
            }
        };

        if !response.status().is_success() {
            context
                .set_pin_value(
                    "error_message",
                    json!(format!("HTTP error: {}", response.status())),
                )
                .await?;
            context.activate_exec_pin("error").await?;
            return Ok(());
        }

        let ical_text = match response.text().await {
            Ok(text) => text,
            Err(e) => {
                context
                    .set_pin_value(
                        "error_message",
                        json!(format!("Failed to read response: {}", e)),
                    )
                    .await?;
                context.activate_exec_pin("error").await?;
                return Ok(());
            }
        };

        let calendar = match ical_text.parse::<Calendar>() {
            Ok(cal) => cal,
            Err(e) => {
                context
                    .set_pin_value(
                        "error_message",
                        json!(format!("Failed to parse iCalendar: {}", e)),
                    )
                    .await?;
                context.activate_exec_pin("error").await?;
                return Ok(());
            }
        };

        let calendar_name = "Calendar".to_string();

        let mut events = Vec::new();

        for component in calendar.components {
            if let icalendar::CalendarComponent::Event(event) = component {
                let event_data = parse_calendar_event(&event);

                let should_include = if let (Some(start), Some(end)) = (start_date, end_date) {
                    if let Some(event_start_str) = &event_data.start {
                        if let Some(event_start) = parse_event_datetime(event_start_str) {
                            event_start >= start && event_start <= end
                        } else {
                            false
                        }
                    } else {
                        false
                    }
                } else {
                    true
                };

                if should_include {
                    events.push(event_data);
                }
            }
        }

        context.set_pin_value("events", json!(events)).await?;
        context
            .set_pin_value("calendar_name", json!(calendar_name))
            .await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Web functionality requires the 'execute' feature"
        ))
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct ImapGetCalendarEventNode;

impl ImapGetCalendarEventNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for ImapGetCalendarEventNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "mail_imap_calendar_get_event",
            "Get Calendar Event",
            "Gets a specific calendar event by UID",
            "Email/IMAP/Calendar",
        );
        node.set_flowscript_name("imap", "getCalendarEvent");
        node.set_receiver("connection");
        node.add_icon("/flow/icons/calendar.svg");

        node.add_input_pin("exec_in", "In", "Execution input", VariableType::Execution);
        node.add_output_pin(
            "exec_out",
            "Success",
            "Event found",
            VariableType::Execution,
        );
        node.add_output_pin(
            "error",
            "Error",
            "Event not found or error",
            VariableType::Execution,
        );

        node.add_input_pin(
            "connection",
            "Connection",
            "IMAP connection",
            VariableType::Struct,
        )
        .set_schema::<ImapConnection>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "calendar_folder",
            "Calendar Folder",
            "Calendar folder name",
            VariableType::String,
        )
        .set_default_value(Some(json!("Calendar")));

        node.add_input_pin(
            "event_uid",
            "Event UID",
            "Event unique ID",
            VariableType::String,
        );

        node.add_output_pin("event", "Event", "Calendar event", VariableType::Struct)
            .set_schema::<CalendarEvent>();

        node.add_output_pin(
            "error_message",
            "Error Message",
            "Error details",
            VariableType::String,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let connection: ImapConnection = context.evaluate_pin("connection").await?;
        let calendar_folder: String = context.evaluate_pin("calendar_folder").await?;
        let event_uid: String = context.evaluate_pin("event_uid").await?;

        let session_arc = connection.to_session(context).await?;
        let mut session = session_arc.lock().await;

        if let Err(e) = session.select(&calendar_folder).await {
            context
                .set_pin_value(
                    "error_message",
                    json!(format!("Failed to select folder: {}", e)),
                )
                .await?;
            context.activate_exec_pin("error").await?;
            return Ok(());
        }

        let search_query = format!("HEADER X-UID {}", event_uid);
        let uids = match session.uid_search(&search_query).await {
            Ok(uids) => uids,
            Err(e) => {
                context
                    .set_pin_value("error_message", json!(format!("Search failed: {}", e)))
                    .await?;
                context.activate_exec_pin("error").await?;
                return Ok(());
            }
        };

        if uids.is_empty() {
            context
                .set_pin_value("error_message", json!("Event not found"))
                .await?;
            context.activate_exec_pin("error").await?;
            return Ok(());
        }

        let uid = *uids.iter().next().unwrap();
        let fetch = session
            .uid_fetch(uid.to_string(), "BODY.PEEK[]")
            .await
            .map_err(|e| anyhow!("Failed to fetch event: {}", e))?
            .try_collect::<Vec<_>>()
            .await?;

        if let Some(msg) = fetch.first()
            && let Some(body) = msg.body()
            && let Ok(body_str) = std::str::from_utf8(body)
            && let Ok(calendar) = body_str.parse::<Calendar>()
        {
            for component in calendar.components {
                if let icalendar::CalendarComponent::Event(event) = component {
                    let event_data = parse_calendar_event(&event);
                    if event_data.uid == event_uid {
                        context.set_pin_value("event", json!(event_data)).await?;
                        context.activate_exec_pin("exec_out").await?;
                        return Ok(());
                    }
                }
            }
        }

        context
            .set_pin_value("error_message", json!("Event not found"))
            .await?;
        context.activate_exec_pin("error").await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Web functionality requires the 'execute' feature"
        ))
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct ImapCreateCalendarEventNode;

impl ImapCreateCalendarEventNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for ImapCreateCalendarEventNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "mail_imap_calendar_create_event",
            "Create Calendar Event",
            "Creates a new calendar event in an IMAP calendar folder",
            "Email/IMAP/Calendar",
        );
        node.set_flowscript_name("imap", "createCalendarEvent");
        node.set_receiver("connection");
        node.add_icon("/flow/icons/calendar.svg");

        node.add_input_pin("exec_in", "In", "Execution input", VariableType::Execution);
        node.add_output_pin(
            "exec_out",
            "Success",
            "Event created",
            VariableType::Execution,
        );
        node.add_output_pin(
            "error",
            "Error",
            "Failed to create event",
            VariableType::Execution,
        );

        node.add_input_pin(
            "connection",
            "Connection",
            "IMAP connection",
            VariableType::Struct,
        )
        .set_schema::<ImapConnection>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "calendar_folder",
            "Calendar Folder",
            "Calendar folder name",
            VariableType::String,
        )
        .set_default_value(Some(json!("Calendar")));

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
        node.add_input_pin("start", "Start", "Start date/time", VariableType::Date);
        node.add_input_pin("end", "End", "End date/time", VariableType::Date);
        node.add_input_pin(
            "attendees",
            "Attendees",
            "Comma-separated email addresses",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_output_pin(
            "event_uid",
            "Event UID",
            "Created event UID",
            VariableType::String,
        );
        node.add_output_pin(
            "error_message",
            "Error Message",
            "Error details",
            VariableType::String,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let connection: ImapConnection = context.evaluate_pin("connection").await?;
        let calendar_folder: String = context.evaluate_pin("calendar_folder").await?;
        let summary: String = context.evaluate_pin("summary").await?;
        let description: String = context.evaluate_pin("description").await?;
        let location: String = context.evaluate_pin("location").await?;
        let start: DateTime<Utc> = context.evaluate_pin("start").await?;
        let end: DateTime<Utc> = context.evaluate_pin("end").await?;
        let attendees_str: String = context.evaluate_pin("attendees").await?;

        let event_uid = uuid::Uuid::new_v4().to_string();

        let mut event = Event::new();
        event.uid(&event_uid);
        event.summary(&summary);
        event.add_property("DTSTART", start.to_rfc3339());
        event.add_property("DTEND", end.to_rfc3339());

        if !description.is_empty() {
            event.description(&description);
        }
        if !location.is_empty() {
            event.add_property("LOCATION", &location);
        }

        if !attendees_str.is_empty() {
            for email in attendees_str.split(',') {
                let email = email.trim();
                if !email.is_empty() {
                    event.add_property("ATTENDEE", format!("mailto:{}", email));
                }
            }
        }

        let mut calendar = Calendar::new();
        calendar.push(event);

        let ical_data = calendar.to_string();

        let session_arc = connection.to_session(context).await?;
        let mut session = session_arc.lock().await;

        if let Err(e) = session.select(&calendar_folder).await {
            context
                .set_pin_value(
                    "error_message",
                    json!(format!("Failed to select folder: {}", e)),
                )
                .await?;
            context.activate_exec_pin("error").await?;
            return Ok(());
        }

        let result = session
            .append(&calendar_folder, None, None, ical_data.as_bytes())
            .await;

        match result {
            Ok(_) => {
                context.set_pin_value("event_uid", json!(event_uid)).await?;
                context.activate_exec_pin("exec_out").await?;
            }
            Err(e) => {
                context
                    .set_pin_value(
                        "error_message",
                        json!(format!("Failed to create event: {}", e)),
                    )
                    .await?;
                context.activate_exec_pin("error").await?;
            }
        }

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Web functionality requires the 'execute' feature"
        ))
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct ImapDeleteCalendarEventNode;

impl ImapDeleteCalendarEventNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for ImapDeleteCalendarEventNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "mail_imap_calendar_delete_event",
            "Delete Calendar Event",
            "Deletes a calendar event from an IMAP calendar folder",
            "Email/IMAP/Calendar",
        );
        node.set_flowscript_name("imap", "deleteCalendarEvent");
        node.set_receiver("connection");
        node.add_icon("/flow/icons/calendar.svg");

        node.add_input_pin("exec_in", "In", "Execution input", VariableType::Execution);
        node.add_output_pin(
            "exec_out",
            "Success",
            "Event deleted",
            VariableType::Execution,
        );
        node.add_output_pin(
            "error",
            "Error",
            "Failed to delete event",
            VariableType::Execution,
        );

        node.add_input_pin(
            "connection",
            "Connection",
            "IMAP connection",
            VariableType::Struct,
        )
        .set_schema::<ImapConnection>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "calendar_folder",
            "Calendar Folder",
            "Calendar folder name",
            VariableType::String,
        )
        .set_default_value(Some(json!("Calendar")));

        node.add_input_pin(
            "event_uid",
            "Event UID",
            "Event unique ID",
            VariableType::String,
        );

        node.add_output_pin(
            "error_message",
            "Error Message",
            "Error details",
            VariableType::String,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let connection: ImapConnection = context.evaluate_pin("connection").await?;
        let calendar_folder: String = context.evaluate_pin("calendar_folder").await?;
        let event_uid: String = context.evaluate_pin("event_uid").await?;

        let session_arc = connection.to_session(context).await?;
        let mut session = session_arc.lock().await;

        if let Err(e) = session.select(&calendar_folder).await {
            context
                .set_pin_value(
                    "error_message",
                    json!(format!("Failed to select folder: {}", e)),
                )
                .await?;
            context.activate_exec_pin("error").await?;
            return Ok(());
        }

        let search_query = format!("HEADER X-UID {}", event_uid);
        let uids = match session.uid_search(&search_query).await {
            Ok(uids) => uids,
            Err(e) => {
                context
                    .set_pin_value("error_message", json!(format!("Search failed: {}", e)))
                    .await?;
                context.activate_exec_pin("error").await?;
                return Ok(());
            }
        };

        if uids.is_empty() {
            context
                .set_pin_value("error_message", json!("Event not found"))
                .await?;
            context.activate_exec_pin("error").await?;
            return Ok(());
        }

        let uid = *uids.iter().next().unwrap();
        let uid_set = format!("{}", uid);

        if let Err(e) = session.uid_store(&uid_set, "+FLAGS (\\Deleted)").await {
            context
                .set_pin_value(
                    "error_message",
                    json!(format!("Failed to delete event: {}", e)),
                )
                .await?;
            context.activate_exec_pin("error").await?;
            return Ok(());
        }

        if let Err(e) = session.expunge().await {
            context
                .set_pin_value("error_message", json!(format!("Failed to expunge: {}", e)))
                .await?;
            context.activate_exec_pin("error").await?;
            return Ok(());
        }

        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Web functionality requires the 'execute' feature"
        ))
    }
}

#[cfg(feature = "execute")]
fn parse_event_datetime(datetime_str: &str) -> Option<DateTime<Utc>> {
    // Try parsing RFC3339 (for UTC dates like "2023-12-19T13:00:00+00:00")
    if let Ok(dt) = DateTime::parse_from_rfc3339(datetime_str) {
        return Some(dt.with_timezone(&Utc));
    }

    // Try parsing timezone format like "2023-12-19T13:00:00[Europe/Berlin]"
    if let Some(bracket_pos) = datetime_str.find('[') {
        let dt_part = &datetime_str[..bracket_pos];
        // Parse as naive datetime and assume UTC for filtering purposes
        if let Ok(naive_dt) = NaiveDateTime::parse_from_str(dt_part, "%Y-%m-%dT%H:%M:%S") {
            return Some(naive_dt.and_utc());
        }
    }

    // Try parsing floating datetime (no timezone)
    if let Ok(naive_dt) = NaiveDateTime::parse_from_str(datetime_str, "%Y-%m-%dT%H:%M:%S") {
        return Some(naive_dt.and_utc());
    }

    // Try parsing date-only format
    if let Ok(naive_date) = chrono::NaiveDate::parse_from_str(datetime_str, "%Y-%m-%d")
        && let Some(naive_dt) = naive_date.and_hms_opt(0, 0, 0)
    {
        return Some(naive_dt.and_utc());
    }

    None
}

#[cfg(feature = "execute")]
fn format_date_perhaps_time(dt: DatePerhapsTime) -> String {
    match dt {
        DatePerhapsTime::DateTime(calendar_dt) => match calendar_dt {
            CalendarDateTime::Floating(naive_dt) => {
                naive_dt.format("%Y-%m-%dT%H:%M:%S").to_string()
            }
            CalendarDateTime::Utc(utc_dt) => utc_dt.to_rfc3339(),
            CalendarDateTime::WithTimezone { date_time, tzid } => {
                format!("{}[{}]", date_time.format("%Y-%m-%dT%H:%M:%S"), tzid)
            }
        },
        DatePerhapsTime::Date(naive_date) => naive_date.format("%Y-%m-%d").to_string(),
    }
}

#[cfg(feature = "execute")]
fn parse_calendar_event(event: &Event) -> CalendarEvent {
    let uid = event
        .get_uid()
        .map(|s| s.to_string())
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    let summary = event.get_summary().map(|s| s.to_string());
    let description = event.get_description().map(|s| s.to_string());

    let location = event
        .properties()
        .iter()
        .find(|(name, _)| name.as_str() == "LOCATION")
        .map(|(_, prop)| prop.value().to_string());

    let start = event.get_start().map(format_date_perhaps_time);
    let end = event.get_end().map(format_date_perhaps_time);

    let organizer = event
        .properties()
        .iter()
        .find(|(name, _)| name.as_str() == "ORGANIZER")
        .map(|(_, prop)| prop.value().to_string());

    let attendees = event
        .properties()
        .iter()
        .filter(|(name, _)| name.as_str() == "ATTENDEE")
        .map(|(_, prop)| prop.value().to_string())
        .collect();

    let status = event.get_status().map(|s| match s {
        icalendar::EventStatus::Tentative => "TENTATIVE".to_string(),
        icalendar::EventStatus::Confirmed => "CONFIRMED".to_string(),
        icalendar::EventStatus::Cancelled => "CANCELLED".to_string(),
    });

    CalendarEvent {
        uid,
        summary,
        description,
        location,
        start,
        end,
        organizer,
        attendees,
        status,
    }
}
