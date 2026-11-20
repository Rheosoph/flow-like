//! A host's calendar sync: keeps one wake-up row per connected-calendar event that wakes the bot,
//! in step with the provider.

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Duration, Utc};
use flow_like_types::Result;
use flow_like_types::bot_calendar::{CALENDAR_LOOKAHEAD_SECS, ExternalEventRef, TaskEvent};
use flow_like_types::bot_task::{BotTask, CreateBotTask, TaskKind, UpdateBotTask};

use super::SyncReport;
use super::listing::{external_row, parse};
use super::provider::{Client, ExternalEvent, calendars};
use crate::schedule::Scheduler;

/// A row whose event ended longer ago than this is removed, whatever its calendar says.
const ENDED_GRACE_SECS: i64 = 3600;
const MAX_ERRORS: usize = 20;

struct Row<'a> {
    task: &'a BotTask,
    event: &'a TaskEvent,
    reference: &'a ExternalEventRef,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
}

fn rows(tasks: &[BotTask], now: DateTime<Utc>) -> Vec<Row<'_>> {
    tasks
        .iter()
        .filter_map(|task| {
            let (event, reference) = external_row(task)?;
            let start = parse(&event.starts_at).unwrap_or(now);
            let end = event.ends_at.as_deref().and_then(parse).unwrap_or(start);
            Some(Row {
                task,
                event,
                reference,
                start,
                end,
            })
        })
        .collect()
}

fn note(report: &mut SyncReport, line: String) {
    if report.errors.len() < MAX_ERRORS {
        report.errors.push(line);
    }
}

/// Whether the provider's event differs from what its row keeps.
fn moved(row: &Row<'_>, wanted: &TaskEvent, title: &str) -> bool {
    row.task.title != title
        || row.event.starts_at != wanted.starts_at
        || row.event.ends_at != wanted.ends_at
        || row.event.all_day != wanted.all_day
        || row.event.location != wanted.location
        || row.event.link != wanted.link
}

/// How far a sync reads: the settings' lookahead, or further when an event's own wake-up comes
/// earlier.
fn lookahead(settings_lookahead: Duration, rows: &[Row<'_>]) -> Duration {
    let largest_own = rows
        .iter()
        .filter_map(|row| row.event.wake.as_ref())
        .filter(|wake| wake.enabled)
        .flat_map(|wake| wake.offsets_minutes.iter().copied())
        .max()
        .unwrap_or_default();
    settings_lookahead
        .max(Duration::minutes(i64::from(largest_own)) + Duration::seconds(CALENDAR_LOOKAHEAD_SECS))
}

pub async fn run(
    client: &Client<'_>,
    scheduler: &dyn Scheduler,
    now: DateTime<Utc>,
) -> Result<SyncReport> {
    let settings = scheduler.calendar_wake().await?;
    let tasks = scheduler.list().await?;
    let rows = rows(&tasks, now);
    let mut report = SyncReport::default();
    let keeps_own = rows.iter().any(|row| row.event.wake.is_some());
    if !settings.syncs_external() && !keeps_own {
        for row in &rows {
            match scheduler.cancel(&row.task.id).await {
                Ok(true) => report.removed += 1,
                Ok(false) => {}
                Err(error) => note(
                    &mut report,
                    format!(
                        "removing the wake-up of '{}' failed: {error}",
                        row.task.title
                    ),
                ),
            }
        }
        return Ok(report);
    }
    let until = now + lookahead(settings.lookahead(), &rows);
    let calendars = calendars(client.source()).await?;
    let attached: HashSet<&str> = calendars.iter().map(|calendar| calendar.id()).collect();
    let mut seen_until: HashMap<String, DateTime<Utc>> = HashMap::new();
    let mut listed: Vec<(ExternalEventRef, ExternalEvent)> = Vec::new();
    let results = client.list_all(&calendars, now, until).await;
    for (calendar, result) in calendars.iter().zip(results) {
        match result {
            Ok(result) => {
                let read_to = if result.complete {
                    until
                } else {
                    result.events.last().map_or(now, |event| event.starts_at)
                };
                seen_until.insert(calendar.id().to_string(), read_to);
                listed.extend(
                    result
                        .events
                        .into_iter()
                        .map(|event| (calendar.reference(&event.event_id), event)),
                );
            }
            Err(error) => note(
                &mut report,
                format!("{}: {}", calendar.label(), error.message),
            ),
        }
    }
    let listed_refs: HashSet<&ExternalEventRef> =
        listed.iter().map(|(reference, _)| reference).collect();
    let mut removed: HashSet<&str> = HashSet::new();
    for row in &rows {
        let connector = row.reference.connector_id.as_str();
        let gone = match seen_until.get(connector) {
            Some(read_to) => !listed_refs.contains(row.reference) && row.start < *read_to,
            None => !attached.contains(connector),
        };
        let ended = row.end + Duration::seconds(ENDED_GRACE_SECS) < now;
        let unwanted = !settings.syncs_external() && row.event.wake.is_none();
        if !(gone || ended || unwanted) {
            continue;
        }
        match scheduler.cancel(&row.task.id).await {
            Ok(cancelled) => {
                removed.insert(row.task.id.as_str());
                if cancelled {
                    report.removed += 1;
                }
            }
            Err(error) => note(
                &mut report,
                format!(
                    "removing the wake-up of '{}' failed: {error}",
                    row.task.title
                ),
            ),
        }
    }
    let by_ref: HashMap<&ExternalEventRef, &Row<'_>> =
        rows.iter().map(|row| (row.reference, row)).collect();
    for (reference, event) in &listed {
        match by_ref.get(reference) {
            Some(row) if removed.contains(row.task.id.as_str()) => {}
            Some(row)
                if row.event.wake.is_none()
                    && settings.offsets_for(event.subject(), None).is_empty() =>
            {
                match scheduler.cancel(&row.task.id).await {
                    Ok(true) => report.removed += 1,
                    Ok(false) => {}
                    Err(error) => note(
                        &mut report,
                        format!("removing the wake-up of '{}' failed: {error}", event.title),
                    ),
                }
            }
            Some(row) => {
                let wanted = event.task_event(reference.clone(), row.event.wake.clone());
                if !moved(row, &wanted, &event.title) {
                    continue;
                }
                let request = UpdateBotTask {
                    title: Some(event.title.clone()),
                    event: Some(wanted),
                    ..UpdateBotTask::default()
                };
                match scheduler.update(&row.task.id, request).await {
                    Ok(Some(_)) => report.moved += 1,
                    Ok(None) => {}
                    Err(error) => note(
                        &mut report,
                        format!("moving the wake-up of '{}' failed: {error}", event.title),
                    ),
                }
            }
            None => {
                if event.starts_at <= now || settings.offsets_for(event.subject(), None).is_empty()
                {
                    continue;
                }
                let request = CreateBotTask {
                    kind: Some(TaskKind::Event),
                    title: Some(event.title.clone()),
                    event: Some(event.task_event(reference.clone(), None)),
                    ..CreateBotTask::default()
                };
                match scheduler.create(request).await {
                    Ok(_) => report.created += 1,
                    Err(error) => note(
                        &mut report,
                        format!("planning a wake-up for '{}' failed: {error}", event.title),
                    ),
                }
            }
        }
    }
    Ok(report)
}
