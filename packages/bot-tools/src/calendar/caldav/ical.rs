//! iCalendar (RFC 5545) as content lines in components. A stored event can be changed property
//! by property while every other line keeps its exact text: attendees, alarms, vendor properties
//! and time zones survive an edit.

use chrono::{DateTime, Duration, NaiveDate, NaiveDateTime, TimeZone, Utc};
use chrono_tz::Tz;

use super::zones::{Zone, Zones};
use crate::calendar::provider::{local_midnight, local_time};

const FOLD_OCTETS: usize = 75;
const UTC_FORMAT: &str = "%Y%m%dT%H%M%SZ";
const LOCAL_FORMAT: &str = "%Y%m%dT%H%M%S";
const DATE_FORMAT: &str = "%Y%m%d";

/// One unfolded content line: `NAME;PARAM=value:value`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    pub name: String,
    pub params: Vec<(String, String)>,
    pub value: String,
    /// The line as read; a parsed line is written back exactly so.
    raw: Option<String>,
}

fn unquote(value: &str) -> String {
    let value = value.trim();
    match value
        .strip_prefix('"')
        .and_then(|inner| inner.strip_suffix('"'))
    {
        Some(inner) if !inner.contains('"') => inner.to_string(),
        _ => value.to_string(),
    }
}

impl Line {
    pub fn new(name: &str, value: impl Into<String>) -> Self {
        Self {
            name: name.to_ascii_uppercase(),
            params: Vec::new(),
            value: value.into(),
            raw: None,
        }
    }

    /// A TEXT property, escaped.
    pub fn text(name: &str, text: &str) -> Self {
        Self::new(name, escape(text))
    }

    pub fn with_param(mut self, name: &str, value: &str) -> Self {
        self.params
            .push((name.to_ascii_uppercase(), value.to_string()));
        self
    }

    pub fn param(&self, name: &str) -> Option<&str> {
        self.params
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    fn parse(text: &str) -> Option<Self> {
        let mut quoted = false;
        let mut cuts = Vec::new();
        let mut colon = None;
        for (index, ch) in text.char_indices() {
            match ch {
                '"' => quoted = !quoted,
                ';' if !quoted => cuts.push(index),
                ':' if !quoted => {
                    colon = Some(index);
                    break;
                }
                _ => {}
            }
        }
        let colon = colon?;
        let head = &text[..colon];
        let mut segments = Vec::with_capacity(cuts.len() + 1);
        let mut start = 0;
        for cut in cuts {
            segments.push(&head[start..cut]);
            start = cut + 1;
        }
        segments.push(&head[start..]);
        let name = segments[0].trim();
        if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
            return None;
        }
        let params = segments[1..]
            .iter()
            .filter_map(|segment| {
                let (key, value) = segment.split_once('=')?;
                Some((key.trim().to_ascii_uppercase(), unquote(value)))
            })
            .collect();
        Some(Self {
            name: name.to_ascii_uppercase(),
            params,
            value: text[colon + 1..].to_string(),
            raw: Some(text.to_string()),
        })
    }

    fn render(&self) -> String {
        if let Some(raw) = &self.raw {
            return raw.clone();
        }
        let mut out = self.name.clone();
        for (key, value) in &self.params {
            out.push(';');
            out.push_str(key);
            out.push('=');
            if value.contains([':', ';', ',']) {
                out.push('"');
                out.push_str(value);
                out.push('"');
            } else {
                out.push_str(value);
            }
        }
        out.push(':');
        out.push_str(&self.value);
        out
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Item {
    Line(Line),
    Child(Component),
}

/// `BEGIN:NAME` … `END:NAME` with its lines and nested components in their order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Component {
    pub name: String,
    pub items: Vec<Item>,
}

impl Component {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.trim().to_ascii_uppercase(),
            items: Vec::new(),
        }
    }

    pub fn lines(&self) -> impl Iterator<Item = &Line> {
        self.items.iter().filter_map(|item| match item {
            Item::Line(line) => Some(line),
            Item::Child(_) => None,
        })
    }

    pub fn property(&self, name: &str) -> Option<&Line> {
        self.lines().find(|line| line.name == name)
    }

    pub fn properties<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Line> {
        self.lines().filter(move |line| line.name == name)
    }

    /// A TEXT property unescaped, `None` when missing or blank.
    pub fn text(&self, name: &str) -> Option<String> {
        self.property(name)
            .map(|line| unescape(&line.value))
            .filter(|text| !text.trim().is_empty())
    }

    pub fn children(&self) -> impl Iterator<Item = &Component> {
        self.items.iter().filter_map(|item| match item {
            Item::Child(child) => Some(child),
            Item::Line(_) => None,
        })
    }

    pub fn children_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Component> {
        self.children().filter(move |child| child.name == name)
    }

    pub fn children_mut(&mut self) -> impl Iterator<Item = &mut Component> {
        self.items.iter_mut().filter_map(|item| match item {
            Item::Child(child) => Some(child),
            Item::Line(_) => None,
        })
    }

    pub fn push(&mut self, line: Line) {
        self.items.push(Item::Line(line));
    }

    pub fn push_child(&mut self, child: Component) {
        self.items.push(Item::Child(child));
    }

    /// Puts `line` where the first line of its name is (dropping the others), else before the
    /// first nested component.
    pub fn set(&mut self, line: Line) {
        let at = self
            .items
            .iter()
            .position(|item| matches!(item, Item::Line(old) if old.name == line.name));
        self.remove(&line.name);
        let at = at.unwrap_or_else(|| {
            self.items
                .iter()
                .position(|item| matches!(item, Item::Child(_)))
                .unwrap_or(self.items.len())
        });
        self.items
            .insert(at.min(self.items.len()), Item::Line(line));
    }

    pub fn remove(&mut self, name: &str) {
        self.items
            .retain(|item| !matches!(item, Item::Line(line) if line.name == name));
    }

    fn write(&self, out: &mut String) {
        fold(&format!("BEGIN:{}", self.name), out);
        for item in &self.items {
            match item {
                Item::Line(line) => fold(&line.render(), out),
                Item::Child(child) => child.write(out),
            }
        }
        fold(&format!("END:{}", self.name), out);
    }

    /// The component as iCalendar text: CRLF lines folded at 75 octets.
    pub fn render(&self) -> String {
        let mut out = String::new();
        self.write(&mut out);
        out
    }
}

fn unfold(text: &str) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for physical in text.split('\n') {
        let physical = physical.strip_suffix('\r').unwrap_or(physical);
        if let Some(rest) = physical.strip_prefix([' ', '\t'])
            && let Some(last) = lines.last_mut()
        {
            last.push_str(rest);
            continue;
        }
        if !physical.trim().is_empty() {
            lines.push(physical.to_string());
        }
    }
    lines
}

fn fold(line: &str, out: &mut String) {
    let mut width = 0;
    for ch in line.chars() {
        let len = ch.len_utf8();
        if width + len > FOLD_OCTETS {
            out.push_str("\r\n ");
            width = 1;
        }
        out.push(ch);
        width += len;
    }
    out.push_str("\r\n");
}

fn close(stack: &mut [Component], done: Component, root: &mut Option<Component>) {
    match stack.last_mut() {
        Some(parent) => parent.push_child(done),
        None => {
            root.get_or_insert(done);
        }
    }
}

/// The VCALENDAR of an iCalendar text. Lines that are not content lines are skipped; components
/// left open at the end are closed.
pub fn parse(text: &str) -> Result<Component, String> {
    let mut stack: Vec<Component> = Vec::new();
    let mut root = None;
    for raw in unfold(text.trim_start_matches('\u{feff}')) {
        let Some(line) = Line::parse(&raw) else {
            continue;
        };
        match line.name.as_str() {
            "BEGIN" => stack.push(Component::new(&line.value)),
            "END" => {
                let done = stack
                    .pop()
                    .ok_or_else(|| format!("END:{} has no BEGIN", line.value.trim()))?;
                if !done.name.eq_ignore_ascii_case(line.value.trim()) {
                    return Err(format!(
                        "END:{} closes BEGIN:{}",
                        line.value.trim(),
                        done.name
                    ));
                }
                close(&mut stack, done, &mut root);
            }
            _ => {
                if let Some(current) = stack.last_mut() {
                    current.push(line);
                }
            }
        }
    }
    while let Some(done) = stack.pop() {
        close(&mut stack, done, &mut root);
    }
    let root = root.ok_or("the calendar data is empty")?;
    if root.name != "VCALENDAR" {
        return Err(format!(
            "the calendar data is a {}, not a VCALENDAR",
            root.name
        ));
    }
    Ok(root)
}

pub fn unescape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        match chars.next() {
            Some('n' | 'N') => out.push('\n'),
            Some(other) => out.push(other),
            None => out.push('\\'),
        }
    }
    out
}

pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            ';' => out.push_str("\\;"),
            ',' => out.push_str("\\,"),
            '\n' => out.push_str("\\n"),
            c if c.is_control() && c != '\t' => {}
            c => out.push(c),
        }
    }
    out
}

/// A DATE or DATE-TIME value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum When {
    Date(NaiveDate),
    Utc(DateTime<Utc>),
    /// A wall-clock time; without a zone it floats (read in the calendar's zone).
    Local(NaiveDateTime, Option<Zone>),
}

impl When {
    pub fn is_date(self) -> bool {
        matches!(self, Self::Date(_))
    }

    /// The instant this value names; whole days and floating times are read in `zone`.
    pub fn instant(self, zone: Tz) -> Option<DateTime<Utc>> {
        match self {
            Self::Date(date) => local_midnight(date, zone),
            Self::Utc(time) => Some(time),
            Self::Local(time, Some(Zone::Iana(tz))) => local_time(time, tz),
            Self::Local(time, Some(Zone::Fixed(offset))) => offset
                .from_local_datetime(&time)
                .single()
                .map(|time| time.with_timezone(&Utc)),
            Self::Local(time, None) => local_time(time, zone),
        }
    }

    /// The RECURRENCE-ID text event ids carry: `YYYYMMDD` for a day, else UTC
    /// `YYYYMMDDTHHMMSSZ`.
    pub fn key(self, zone: Tz) -> Option<String> {
        match self {
            Self::Date(date) => Some(date.format(DATE_FORMAT).to_string()),
            _ => self.instant(zone).map(utc_text),
        }
    }
}

pub fn utc_text(time: DateTime<Utc>) -> String {
    time.format(UTC_FORMAT).to_string()
}

pub fn date_text(date: NaiveDate) -> String {
    date.format(DATE_FORMAT).to_string()
}

pub fn local_text(time: NaiveDateTime) -> String {
    time.format(LOCAL_FORMAT).to_string()
}

/// One DATE or DATE-TIME value; `tzid` is its line's TZID parameter.
pub fn when_value(value: &str, tzid: Option<&str>, zones: &Zones) -> Result<When, String> {
    let value = value.trim();
    if value.len() == 8 {
        return NaiveDate::parse_from_str(value, DATE_FORMAT)
            .map(When::Date)
            .map_err(|_| format!("'{value}' is not a date"));
    }
    if let Some(utc) = value.strip_suffix(['Z', 'z']) {
        return NaiveDateTime::parse_from_str(utc, LOCAL_FORMAT)
            .map(|time| When::Utc(time.and_utc()))
            .map_err(|_| format!("'{value}' is not a date-time"));
    }
    let local = NaiveDateTime::parse_from_str(value, LOCAL_FORMAT)
        .map_err(|_| format!("'{value}' is not a date-time"))?;
    match tzid.map(str::trim).filter(|tzid| !tzid.is_empty()) {
        None => Ok(When::Local(local, None)),
        Some(tzid) => zones
            .resolve(tzid)
            .map(|zone| When::Local(local, Some(zone)))
            .ok_or_else(|| format!("time zone '{tzid}' is unknown")),
    }
}

/// The value of a DTSTART, DTEND or RECURRENCE-ID line.
pub fn when_of(line: &Line, zones: &Zones) -> Result<When, String> {
    let first = line.value.split(',').next().unwrap_or_default();
    when_value(first, line.param("TZID"), zones)
}

/// Every value of an EXDATE or RDATE line; a PERIOD gives its start.
pub fn whens_of(line: &Line, zones: &Zones) -> Result<Vec<When>, String> {
    line.value
        .split(',')
        .filter(|value| !value.trim().is_empty())
        .map(|value| {
            let start = value.split('/').next().unwrap_or_default();
            when_value(start, line.param("TZID"), zones)
        })
        .collect()
}

/// A DURATION value such as `PT1H30M`, `P1D` or `-P1W`.
pub fn duration(text: &str) -> Option<Duration> {
    let text = text.trim();
    let (negative, rest) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text.strip_prefix('+').unwrap_or(text)),
    };
    let rest = rest.strip_prefix(['P', 'p'])?;
    let mut total = Duration::zero();
    let mut number = String::new();
    let mut in_time = false;
    let mut any = false;
    for ch in rest.chars() {
        match ch.to_ascii_uppercase() {
            '0'..='9' => number.push(ch),
            'T' if number.is_empty() => in_time = true,
            unit => {
                let count: i64 = number.parse().ok()?;
                number.clear();
                any = true;
                total += match (unit, in_time) {
                    ('W', false) => Duration::try_weeks(count)?,
                    ('D', false) => Duration::try_days(count)?,
                    ('H', true) => Duration::try_hours(count)?,
                    ('M', true) => Duration::try_minutes(count)?,
                    ('S', true) => Duration::try_seconds(count)?,
                    _ => return None,
                };
            }
        }
    }
    (any && number.is_empty()).then_some(if negative { -total } else { total })
}
