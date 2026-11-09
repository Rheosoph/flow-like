use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const STORAGE_KEY: &str = "_flow_like_availability";
pub const DEFAULT_DURATION_MINUTES: u32 = 60;
pub const MAX_DURATION_MINUTES: u32 = 240;
pub const CALENDAR_CACHE_MS: i64 = 5 * 60_000;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AvailabilityStatus {
    #[default]
    Available,
    Busy,
    Focus,
    Away,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AvailabilitySource {
    Manual,
    Bot,
    Calendar,
    #[default]
    Default,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AvailabilityPreferences {
    pub allow_mentions: bool,
    pub allow_calls: bool,
}

impl Default for AvailabilityPreferences {
    fn default() -> Self {
        Self {
            allow_mentions: true,
            allow_calls: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AvailabilityOverride {
    pub status: AvailabilityStatus,
    pub source: AvailabilitySource,
    pub until: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BusyInterval {
    pub start: i64,
    pub end: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FreeBusy {
    pub window_start: i64,
    pub window_end: i64,
    pub busy: Vec<BusyInterval>,
}

impl FreeBusy {
    pub fn normalized(window_start: i64, window_end: i64, intervals: Vec<BusyInterval>) -> Self {
        let mut intervals: Vec<_> = intervals
            .into_iter()
            .filter_map(|interval| {
                let start = interval.start.max(window_start);
                let end = interval.end.min(window_end);
                (start < end).then_some(BusyInterval { start, end })
            })
            .collect();
        intervals.sort_unstable_by_key(|interval| (interval.start, interval.end));
        let mut busy: Vec<BusyInterval> = Vec::new();
        for interval in intervals {
            if let Some(last) = busy.last_mut()
                && interval.start <= last.end
            {
                last.end = last.end.max(interval.end);
            } else {
                busy.push(interval);
            }
        }
        Self {
            window_start,
            window_end,
            busy,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CalendarLink {
    pub id: String,
    pub provider_id: String,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredCalendarLink {
    #[serde(flatten)]
    pub display: CalendarLink,
    pub connection_handle: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AvailabilityState {
    pub status_override: Option<AvailabilityOverride>,
    pub preferences: AvailabilityPreferences,
    pub calendar_links: Vec<StoredCalendarLink>,
    pub free_busy: Option<FreeBusy>,
    pub calendar_valid_until: i64,
    pub calendar_checked_at: i64,
    pub calendar_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublicAvailability {
    pub status: AvailabilityStatus,
    pub source: AvailabilitySource,
    pub until: Option<i64>,
}

impl PublicAvailability {
    pub fn without_calendar_schedule(self) -> Self {
        if self.source == AvailabilitySource::Calendar {
            return Self {
                status: self.status,
                source: AvailabilitySource::Default,
                until: None,
            };
        }
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserAvailability {
    #[serde(flatten)]
    pub current: PublicAvailability,
    pub preferences: AvailabilityPreferences,
    pub calendar_links: Vec<CalendarLink>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub free_busy: Option<FreeBusy>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub calendar_error: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetAvailability {
    pub status: Option<AvailabilityStatus>,
    pub duration_minutes: Option<u32>,
    pub preferences: Option<AvailabilityPreferences>,
}

impl SetAvailability {
    pub fn validate(&self) -> Result<u32, &'static str> {
        let duration = self.duration_minutes.unwrap_or(DEFAULT_DURATION_MINUTES);
        if !(1..=MAX_DURATION_MINUTES).contains(&duration) {
            return Err("Choose a duration between 1 and 240 minutes");
        }
        if self.status.is_none() && self.preferences.is_none() {
            return Err("Provide a status or notification preferences");
        }
        Ok(duration)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum NotificationPriority {
    #[default]
    Routine,
    Mention,
    Call,
}

impl AvailabilityState {
    pub fn read(information: Option<&Value>) -> Self {
        information
            .and_then(|value| value.get(STORAGE_KEY))
            .and_then(|value| serde_json::from_value(value.clone()).ok())
            .unwrap_or_default()
    }

    pub fn current(&self, now: i64) -> PublicAvailability {
        if let Some(value) = &self.status_override
            && value.until > now
        {
            return PublicAvailability {
                status: value.status,
                source: value.source,
                until: Some(value.until),
            };
        }
        if self.calendar_valid_until > now
            && let Some(calendar) = &self.free_busy
            && calendar.window_start <= now
            && calendar.window_end > now
        {
            if let Some(interval) = calendar
                .busy
                .iter()
                .find(|interval| interval.start <= now && interval.end > now)
            {
                return PublicAvailability {
                    status: AvailabilityStatus::Busy,
                    source: AvailabilitySource::Calendar,
                    until: Some(
                        interval
                            .end
                            .min(self.calendar_valid_until)
                            .min(calendar.window_end),
                    ),
                };
            }
            let next_change = calendar
                .busy
                .iter()
                .filter(|interval| interval.start > now)
                .map(|interval| interval.start)
                .min()
                .unwrap_or(calendar.window_end);
            return PublicAvailability {
                status: AvailabilityStatus::Available,
                source: AvailabilitySource::Calendar,
                until: Some(
                    next_change
                        .min(self.calendar_valid_until)
                        .min(calendar.window_end),
                ),
            };
        }
        PublicAvailability {
            status: AvailabilityStatus::Available,
            source: AvailabilitySource::Default,
            until: None,
        }
    }

    pub fn view(&self, now: i64) -> UserAvailability {
        UserAvailability {
            current: self.current(now),
            preferences: self.preferences.clone(),
            calendar_links: self
                .calendar_links
                .iter()
                .map(|link| link.display.clone())
                .collect(),
            free_busy: (self.calendar_valid_until > now)
                .then(|| self.free_busy.clone())
                .flatten(),
            calendar_error: self.calendar_error.clone(),
        }
    }

    pub fn allows_notification(&self, priority: NotificationPriority, now: i64) -> bool {
        self.current(now).status != AvailabilityStatus::Focus
            || match priority {
                NotificationPriority::Routine => false,
                NotificationPriority::Mention => self.preferences.allow_mentions,
                NotificationPriority::Call => self.preferences.allow_calls,
            }
    }
}

pub fn public_information(information: Option<Value>) -> Option<Value> {
    information.map(|mut value| {
        if let Some(object) = value.as_object_mut() {
            object.remove(STORAGE_KEY);
        }
        value
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn override_takes_precedence_and_expires_without_a_timer() {
        let mut state = AvailabilityState {
            status_override: Some(AvailabilityOverride {
                status: AvailabilityStatus::Focus,
                source: AvailabilitySource::Bot,
                until: 200,
            }),
            free_busy: Some(FreeBusy::normalized(
                0,
                1_000,
                vec![BusyInterval { start: 0, end: 800 }],
            )),
            calendar_valid_until: 500,
            ..Default::default()
        };
        assert_eq!(state.current(199).status, AvailabilityStatus::Focus);
        assert_eq!(state.current(200).source, AvailabilitySource::Calendar);
        assert_eq!(state.current(200).until, Some(500));
        assert_eq!(state.current(500).status, AvailabilityStatus::Available);
        state.status_override.as_mut().unwrap().status = AvailabilityStatus::Available;
        assert_eq!(state.current(100).status, AvailabilityStatus::Available);
    }

    #[test]
    fn focus_respects_exceptions_while_busy_keeps_notifications() {
        let mut state = AvailabilityState {
            status_override: Some(AvailabilityOverride {
                status: AvailabilityStatus::Focus,
                source: AvailabilitySource::Manual,
                until: 100,
            }),
            ..Default::default()
        };
        assert!(!state.allows_notification(NotificationPriority::Routine, 0));
        assert!(state.allows_notification(NotificationPriority::Mention, 0));
        assert!(state.allows_notification(NotificationPriority::Call, 0));
        state.preferences.allow_mentions = false;
        assert!(!state.allows_notification(NotificationPriority::Mention, 0));
        assert!(state.allows_notification(NotificationPriority::Routine, 100));
        state.status_override.as_mut().unwrap().status = AvailabilityStatus::Busy;
        assert!(state.allows_notification(NotificationPriority::Routine, 0));
    }

    #[test]
    fn duration_and_private_projection_are_bounded() {
        let mut update = SetAvailability {
            status: Some(AvailabilityStatus::Focus),
            ..Default::default()
        };
        assert_eq!(update.validate(), Ok(60));
        update.duration_minutes = Some(240);
        assert_eq!(update.validate(), Ok(240));
        for duration in [0, 241, u32::MAX] {
            update.duration_minutes = Some(duration);
            assert!(update.validate().is_err());
        }
        let value = serde_json::json!({"team":"ops", STORAGE_KEY:{"connection_handle":"private"}});
        assert_eq!(
            public_information(Some(value)),
            Some(serde_json::json!({"team":"ops"}))
        );
    }

    #[test]
    fn public_projection_hides_calendar_schedule_but_keeps_status() {
        let state = AvailabilityState {
            free_busy: Some(FreeBusy::normalized(
                0,
                1_000,
                vec![BusyInterval { start: 0, end: 800 }],
            )),
            calendar_valid_until: 900,
            ..Default::default()
        };
        let calendar = state.current(100).without_calendar_schedule();
        assert_eq!(calendar.status, AvailabilityStatus::Busy);
        assert_eq!(calendar.source, AvailabilitySource::Default);
        assert_eq!(calendar.until, None);
        let manual = AvailabilityState {
            status_override: Some(AvailabilityOverride {
                status: AvailabilityStatus::Focus,
                source: AvailabilitySource::Manual,
                until: 500,
            }),
            ..Default::default()
        };
        assert_eq!(
            manual.current(100).without_calendar_schedule().until,
            Some(500)
        );
    }

    #[test]
    fn calendar_ranges_are_clipped_sorted_and_merged() {
        assert_eq!(
            FreeBusy::normalized(
                10,
                60,
                vec![
                    BusyInterval { start: 40, end: 90 },
                    BusyInterval { start: 0, end: 20 },
                    BusyInterval { start: 20, end: 30 },
                    BusyInterval { start: 80, end: 90 }
                ]
            )
            .busy,
            vec![
                BusyInterval { start: 10, end: 30 },
                BusyInterval { start: 40, end: 60 }
            ]
        );
    }

    #[test]
    fn calendar_free_status_expires_at_the_next_change() {
        let state = AvailabilityState {
            free_busy: Some(FreeBusy::normalized(
                100,
                1_000,
                vec![BusyInterval {
                    start: 200,
                    end: 300,
                }],
            )),
            calendar_valid_until: 800,
            ..Default::default()
        };
        assert_eq!(
            state.current(150),
            PublicAvailability {
                status: AvailabilityStatus::Available,
                source: AvailabilitySource::Calendar,
                until: Some(200)
            }
        );
        assert_eq!(state.current(300).until, Some(800));
        assert_eq!(state.current(99).source, AvailabilitySource::Default);
        assert_eq!(state.current(800).source, AvailabilitySource::Default);
    }
}
