//! The bot's calendar over a span: occurrences of its own events and the events of its connected
//! calendars, with the wake-up each one has or would have.

use std::collections::HashMap;

use chrono::{DateTime, Duration, Utc};
use flow_like_types::Result;
use flow_like_types::bot_calendar::{
    CalendarEntry, CalendarEntrySource, CalendarRange, CalendarSource, CalendarWake, EntryWake,
    EventSchedule, ExternalEventRef, TaskEvent,
};
use flow_like_types::bot_task::{BotTask, TaskKind};

use super::provider::{Client, calendars, rfc3339};
use crate::schedule::Scheduler;

/// Occurrences of one repeating event a listing shows at most.
const MAX_OCCURRENCES: usize = 200;

/// One listed occurrence and the time it blocks.
#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub entry: CalendarEntry,
    pub start: DateTime<Utc>,
    pub end: Option<DateTime<Utc>>,
    /// Blocks time: not a whole-day event, not one its provider marks free.
    pub busy: bool,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Listing {
    pub items: Vec<Item>,
    pub sources: Vec<CalendarSource>,
}

impl Listing {
    pub fn range(self, from: DateTime<Utc>, to: DateTime<Utc>) -> CalendarRange {
        CalendarRange {
            from: rfc3339(from),
            to: rfc3339(to),
            entries: self.items.into_iter().map(|item| item.entry).collect(),
            sources: self.sources,
        }
    }
}

pub fn parse(text: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(text.trim())
        .ok()
        .map(|time| time.with_timezone(&Utc))
}

fn entry_wake(event: &TaskEvent, next_at: Option<String>) -> Option<EntryWake> {
    event.wakes().then(|| EntryWake {
        offsets_minutes: event.wake_offsets.clone(),
        own: event.wake.is_some(),
        next_at,
    })
}

/// An event of the bot's own calendar: an event task without an external reference.
pub fn own_event(task: &BotTask) -> Option<&TaskEvent> {
    task.event
        .as_ref()
        .filter(|event| task.kind == TaskKind::Event && event.external.is_none())
}

/// The external event a wake-up row keeps wake-ups for.
pub fn external_row(task: &BotTask) -> Option<(&TaskEvent, &ExternalEventRef)> {
    if task.kind != TaskKind::Event {
        return None;
    }
    let event = task.event.as_ref()?;
    Some((event, event.external.as_ref()?))
}

/// One occurrence of an own event as a listing shows it; `next_at` is the task's next wake-up
/// when it belongs to this occurrence.
pub fn own_entry(
    task: &BotTask,
    event: &TaskEvent,
    start: DateTime<Utc>,
    end: Option<DateTime<Utc>>,
    next_at: Option<String>,
) -> CalendarEntry {
    CalendarEntry {
        id: task.id.clone(),
        source: CalendarEntrySource::Bot,
        title: task.title.clone(),
        starts_at: rfc3339(start),
        ends_at: end.map(rfc3339),
        all_day: event.all_day,
        location: event.location.clone(),
        notes: Some(task.instruction.trim().to_string()).filter(|notes| !notes.is_empty()),
        connector_id: None,
        calendar: None,
        link: event.link.clone(),
        recurring: task
            .cron
            .as_deref()
            .is_some_and(|cron| !cron.trim().is_empty()),
        task_id: Some(task.id.clone()),
        wake: entry_wake(event, next_at),
    }
}

/// The entry of an own event task at its first start, for a tool's answer after a write.
pub fn task_entry(task: &BotTask) -> Option<CalendarEntry> {
    let event = own_event(task)?;
    let start = parse(&event.starts_at)?;
    let end = event.ends_at.as_deref().and_then(parse);
    Some(own_entry(task, event, start, end, task.next_run_at.clone()))
}

fn own_items(task: &BotTask, from: DateTime<Utc>, to: DateTime<Utc>) -> Vec<Item> {
    let Some(event) = own_event(task) else {
        return Vec::new();
    };
    let Ok(schedule) = EventSchedule::of(event, task.cron.as_deref(), task.tz.as_deref()) else {
        return Vec::new();
    };
    let next_occurrence = task
        .next_run_at
        .as_deref()
        .and_then(parse)
        .and_then(|due| schedule.occurrence_for(due));
    schedule
        .occurrences(from, to, MAX_OCCURRENCES)
        .into_iter()
        .map(|start| {
            let end = schedule.duration.map(|length| start + length);
            let next_at = (next_occurrence == Some(start))
                .then(|| task.next_run_at.clone())
                .flatten();
            Item {
                entry: own_entry(task, event, start, end, next_at),
                start,
                end,
                busy: !event.all_day,
            }
        })
        .collect()
}

/// Everything on the bot's calendar between `from` and `to`, earliest first. Without a client
/// only the bot's own events are listed; a connected calendar that cannot be read is named in
/// `sources` with its error.
pub async fn collect(
    client: Option<&Client<'_>>,
    scheduler: &dyn Scheduler,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    now: DateTime<Utc>,
) -> Result<Listing> {
    let tasks = scheduler.list().await?;
    let mut listing = Listing::default();
    for task in &tasks {
        listing.items.extend(own_items(task, from, to));
    }
    if let Some(client) = client {
        let rows: HashMap<&ExternalEventRef, (&BotTask, &TaskEvent)> = tasks
            .iter()
            .filter_map(|task| {
                let (event, reference) = external_row(task)?;
                Some((reference, (task, event)))
            })
            .collect();
        let calendars = calendars(client.source()).await?;
        let settings = if calendars.is_empty() {
            CalendarWake::default()
        } else {
            scheduler.calendar_wake().await.unwrap_or_default()
        };
        let results = client.list_all(&calendars, from, to).await;
        for (calendar, result) in calendars.iter().zip(results) {
            let listed = match result {
                Ok(listed) => listed,
                Err(error) => {
                    listing.sources.push(calendar.source(Some(error.message)));
                    continue;
                }
            };
            listing.sources.push(calendar.source(listed.note()));
            for event in &listed.events {
                let mut entry = event.entry(calendar);
                let reference = calendar.reference(&event.event_id);
                match rows.get(&reference) {
                    Some((task, row)) => {
                        entry.task_id = Some(task.id.clone());
                        entry.wake = entry_wake(row, task.next_run_at.clone());
                    }
                    None if event.starts_at > now => {
                        let offsets = settings.offsets_for(event.subject(), None);
                        entry.wake = (!offsets.is_empty()).then_some(EntryWake {
                            offsets_minutes: offsets,
                            own: false,
                            next_at: None,
                        });
                    }
                    None => {}
                }
                listing.items.push(Item {
                    entry,
                    start: event.starts_at,
                    end: event.ends_at,
                    busy: event.busy && !event.all_day,
                });
            }
        }
    }
    listing.items.sort_by(|a, b| {
        a.start
            .cmp(&b.start)
            .then_with(|| a.entry.title.cmp(&b.entry.title))
    });
    Ok(listing)
}

/// The end of an occurrence for blocking time: its end, or a short block for a moment.
pub fn blocked_until(item: &Item, untimed: Duration) -> DateTime<Utc> {
    item.end
        .filter(|end| *end > item.start)
        .unwrap_or(item.start + untimed)
}
