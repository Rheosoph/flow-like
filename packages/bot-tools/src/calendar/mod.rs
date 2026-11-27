//! The `calendar` capability: the bot's own calendar (event tasks), the owner's connected Google
//! and Microsoft calendars read through the bot's OAuth connectors and CalDAV calendars through
//! its CalDAV connectors, and wake-ups before events. Hosts call [`list_range`] for their
//! calendar views and [`sync_wakes`] from their background loop; both work with any host's
//! [`Scheduler`] and [`ConnectorSource`]. [`caldav::discover`] fills a CalDAV connector's calendars.
//! A host that keeps provider calls out of its process answers
//! [`ConnectorSource::calendar_jobs`]; the calls then run as [`CALENDAR_JOB`] jobs elsewhere.

pub mod caldav;
mod google;
mod jobs;
mod listing;
mod microsoft;
mod provider;
mod slots;
mod sync;
mod tools;

#[cfg(test)]
mod tests;

use chrono::{DateTime, Utc};
use flow_like_types::Result;
use flow_like_types::bot_calendar::CalendarRange;
use serde::Serialize;

use crate::env::Environment;
use crate::schedule::Scheduler;
use crate::services::ConnectorSource;
use provider::{Client, HttpTransport};

pub use jobs::{
    CALENDAR_JOB, CalendarCall, CalendarJobs, JobAnswer, JobAudit, JobCalendar, JobError,
    JobListing, JobReply, MAX_JOB_EVENTS, MAX_REPLY_BYTES, account_entry, calendar_job,
    discover_with,
};
pub use provider::{EventDraft, ExternalEvent, Listed, calendar_provider};
pub use tools::{
    CALENDAR_CREATE, CALENDAR_DELETE, CALENDAR_FREE_SLOTS, CALENDAR_LIST, CALENDAR_UPDATE,
    CALENDAR_WAKE_SETTINGS, CalendarCreateArgs, CalendarDeleteArgs, CalendarFreeSlotsArgs,
    CalendarListArgs, CalendarUpdateArgs, CalendarWakeSettingsArgs, ask_message, calendar_create,
    calendar_delete, calendar_free_slots, calendar_list, calendar_update, calendar_wake_settings,
    connector_write, grant_scope, specs, write_method,
};

/// Hosts read provider calendars as a server does: public https hosts only.
const HOST_ENVIRONMENT: Environment = Environment::Server;

/// What one calendar sync changed.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize)]
pub struct SyncReport {
    pub created: usize,
    pub moved: usize,
    pub removed: usize,
    /// One line per connected calendar that could not be read; its rows were left alone.
    pub errors: Vec<String>,
}

/// The bot's calendar between `from` and `to`: occurrences of its own events and the events of
/// the connected calendars it can read, earliest first. Without `connectors` only the bot's own
/// events are listed.
pub async fn list_range(
    connectors: Option<&dyn ConnectorSource>,
    scheduler: &dyn Scheduler,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    now: DateTime<Utc>,
) -> Result<CalendarRange> {
    let transport = HttpTransport(HOST_ENVIRONMENT);
    let client = connectors.map(|source| Client::new(source, &transport));
    let listing = listing::collect(client.as_ref(), scheduler, from, to, now).await?;
    Ok(listing.range(from, to))
}

/// Plans the wake-ups of connected-calendar events: reads the calendars of the bot's Google and
/// Microsoft connectors over the sync window, creates an event task for each event that wakes
/// the bot, moves the tasks of events that moved, and removes those of events that were
/// cancelled or have ended. A calendar that cannot be read keeps its rows untouched.
pub async fn sync_wakes(
    connectors: &dyn ConnectorSource,
    scheduler: &dyn Scheduler,
    now: DateTime<Utc>,
) -> Result<SyncReport> {
    let transport = HttpTransport(HOST_ENVIRONMENT);
    sync::run(&Client::new(connectors, &transport), scheduler, now).await
}
