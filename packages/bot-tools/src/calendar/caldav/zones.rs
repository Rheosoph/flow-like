//! TZID parameters as real zones. Calendars name zones by IANA name, by IANA name behind a
//! vendor prefix (`/mozilla.org/20050126_1/Europe/Berlin`), by Windows name (Outlook, Exchange),
//! or only through their own VTIMEZONE (`X-LIC-LOCATION`, or one fixed offset).

use chrono::FixedOffset;
use chrono_tz::Tz;

use super::ical::Component;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Zone {
    Iana(Tz),
    Fixed(FixedOffset),
}

/// Windows zone names and the IANA zone CLDR maps them to (territory 001).
const WINDOWS_ZONES: &[(&str, &str)] = &[
    ("Dateline Standard Time", "Etc/GMT+12"),
    ("UTC-11", "Etc/GMT+11"),
    ("Hawaiian Standard Time", "Pacific/Honolulu"),
    ("Alaskan Standard Time", "America/Anchorage"),
    ("Pacific Standard Time (Mexico)", "America/Tijuana"),
    ("Pacific Standard Time", "America/Los_Angeles"),
    ("US Mountain Standard Time", "America/Phoenix"),
    ("Mountain Standard Time (Mexico)", "America/Mazatlan"),
    ("Mountain Standard Time", "America/Denver"),
    ("Central America Standard Time", "America/Guatemala"),
    ("Central Standard Time (Mexico)", "America/Mexico_City"),
    ("Central Standard Time", "America/Chicago"),
    ("Canada Central Standard Time", "America/Regina"),
    ("SA Pacific Standard Time", "America/Bogota"),
    ("Eastern Standard Time (Mexico)", "America/Cancun"),
    ("Eastern Standard Time", "America/New_York"),
    ("US Eastern Standard Time", "America/Indiana/Indianapolis"),
    ("Venezuela Standard Time", "America/Caracas"),
    ("Atlantic Standard Time", "America/Halifax"),
    ("SA Western Standard Time", "America/La_Paz"),
    ("Pacific SA Standard Time", "America/Santiago"),
    ("Newfoundland Standard Time", "America/St_Johns"),
    ("E. South America Standard Time", "America/Sao_Paulo"),
    ("Argentina Standard Time", "America/Argentina/Buenos_Aires"),
    ("SA Eastern Standard Time", "America/Cayenne"),
    ("Greenland Standard Time", "America/Nuuk"),
    ("UTC-02", "Etc/GMT+2"),
    ("Azores Standard Time", "Atlantic/Azores"),
    ("Cape Verde Standard Time", "Atlantic/Cape_Verde"),
    ("UTC", "Etc/UTC"),
    ("Coordinated Universal Time", "Etc/UTC"),
    ("GMT Standard Time", "Europe/London"),
    ("Greenwich Standard Time", "Atlantic/Reykjavik"),
    ("Morocco Standard Time", "Africa/Casablanca"),
    ("W. Europe Standard Time", "Europe/Berlin"),
    ("Central Europe Standard Time", "Europe/Budapest"),
    ("Romance Standard Time", "Europe/Paris"),
    ("Central European Standard Time", "Europe/Warsaw"),
    ("W. Central Africa Standard Time", "Africa/Lagos"),
    ("GTB Standard Time", "Europe/Bucharest"),
    ("E. Europe Standard Time", "Europe/Chisinau"),
    ("Egypt Standard Time", "Africa/Cairo"),
    ("FLE Standard Time", "Europe/Kyiv"),
    ("Israel Standard Time", "Asia/Jerusalem"),
    ("South Africa Standard Time", "Africa/Johannesburg"),
    ("Turkey Standard Time", "Europe/Istanbul"),
    ("Arab Standard Time", "Asia/Riyadh"),
    ("Arabic Standard Time", "Asia/Baghdad"),
    ("E. Africa Standard Time", "Africa/Nairobi"),
    ("Russian Standard Time", "Europe/Moscow"),
    ("Iran Standard Time", "Asia/Tehran"),
    ("Arabian Standard Time", "Asia/Dubai"),
    ("Afghanistan Standard Time", "Asia/Kabul"),
    ("Pakistan Standard Time", "Asia/Karachi"),
    ("West Asia Standard Time", "Asia/Tashkent"),
    ("India Standard Time", "Asia/Kolkata"),
    ("Nepal Standard Time", "Asia/Kathmandu"),
    ("Bangladesh Standard Time", "Asia/Dhaka"),
    ("SE Asia Standard Time", "Asia/Bangkok"),
    ("China Standard Time", "Asia/Shanghai"),
    ("Singapore Standard Time", "Asia/Singapore"),
    ("Taipei Standard Time", "Asia/Taipei"),
    ("W. Australia Standard Time", "Australia/Perth"),
    ("Tokyo Standard Time", "Asia/Tokyo"),
    ("Korea Standard Time", "Asia/Seoul"),
    ("Cen. Australia Standard Time", "Australia/Adelaide"),
    ("AUS Central Standard Time", "Australia/Darwin"),
    ("E. Australia Standard Time", "Australia/Brisbane"),
    ("AUS Eastern Standard Time", "Australia/Sydney"),
    ("Tasmania Standard Time", "Australia/Hobart"),
    ("New Zealand Standard Time", "Pacific/Auckland"),
    ("UTC+12", "Etc/GMT-12"),
    ("Tonga Standard Time", "Pacific/Tongatapu"),
];

/// An IANA zone by exact name, ignoring case, or behind a vendor path prefix.
pub fn iana(name: &str) -> Option<Tz> {
    let name = name.trim().trim_matches('"');
    if let Ok(zone) = name.parse::<Tz>() {
        return Some(zone);
    }
    if let Some(zone) = chrono_tz::TZ_VARIANTS
        .iter()
        .find(|zone| zone.name().eq_ignore_ascii_case(name))
    {
        return Some(*zone);
    }
    let parts: Vec<&str> = name.split('/').filter(|part| !part.is_empty()).collect();
    (2..=3.min(parts.len()))
        .rev()
        .find_map(|take| parts[parts.len() - take..].join("/").parse::<Tz>().ok())
}

/// The IANA zone of a Windows zone name.
pub fn windows(name: &str) -> Option<Tz> {
    let name = name.trim().trim_matches('"');
    WINDOWS_ZONES
        .iter()
        .find(|(windows, _)| windows.eq_ignore_ascii_case(name))
        .and_then(|(_, zone)| zone.parse().ok())
}

#[cfg(test)]
pub fn windows_table() -> &'static [(&'static str, &'static str)] {
    WINDOWS_ZONES
}

/// `+0100`, `-0530`, `+013000`.
pub fn offset(text: &str) -> Option<FixedOffset> {
    let text = text.trim();
    let (sign, digits) = match text.as_bytes().first()? {
        b'+' => (1, &text[1..]),
        b'-' => (-1, &text[1..]),
        _ => return None,
    };
    if !matches!(digits.len(), 4 | 6) || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let hours: i32 = digits[0..2].parse().ok()?;
    let minutes: i32 = digits[2..4].parse().ok()?;
    let seconds: i32 = digits.get(4..6).map_or(Some(0), |s| s.parse().ok())?;
    FixedOffset::east_opt(sign * (hours * 3600 + minutes * 60 + seconds))
}

fn from_vtimezone(timezone: &Component) -> Option<Zone> {
    if let Some(zone) = timezone
        .property("X-LIC-LOCATION")
        .and_then(|line| iana(&line.value))
    {
        return Some(Zone::Iana(zone));
    }
    let offsets: Vec<FixedOffset> = timezone
        .children()
        .filter(|observance| matches!(observance.name.as_str(), "STANDARD" | "DAYLIGHT"))
        .filter_map(|observance| offset(&observance.property("TZOFFSETTO")?.value))
        .collect();
    let first = *offsets.first()?;
    offsets
        .iter()
        .all(|other| *other == first)
        .then_some(Zone::Fixed(first))
}

/// The zones one calendar object names.
pub struct Zones<'a> {
    timezones: Vec<&'a Component>,
}

impl<'a> Zones<'a> {
    pub fn of(calendar: &'a Component) -> Self {
        Self {
            timezones: calendar.children_named("VTIMEZONE").collect(),
        }
    }

    pub fn resolve(&self, tzid: &str) -> Option<Zone> {
        let tzid = tzid.trim().trim_matches('"');
        if let Some(zone) = iana(tzid).or_else(|| windows(tzid)) {
            return Some(Zone::Iana(zone));
        }
        self.timezones
            .iter()
            .find(|timezone| {
                timezone
                    .property("TZID")
                    .is_some_and(|line| line.value.trim() == tzid)
            })
            .and_then(|timezone| from_vtimezone(timezone))
    }
}
