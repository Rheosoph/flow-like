use std::sync::Mutex;

use async_trait::async_trait;
use chrono::{DateTime, TimeZone, Utc};
use chrono_tz::Europe::Berlin;
use chrono_tz::Tz;
use flow_like_types::reqwest::Url;

use super::client::{DavAccount, DavCalendar, DavClient, Scope, basic_credential, http_status};
use super::events::{self, EventId};
use super::ical::{self, When};
use super::recur;
use super::transport::{
    DavBody, DavMethod, DavRequest, DavResponse, DavTransport, Depth, HttpDavTransport,
    Precondition,
};
use super::xml::{self, CALDAV, DAV};
use super::zones::{self, Zone, Zones};
use crate::calendar::provider::{self, EventDraft, ExternalEvent, Provider};
use crate::env::Environment;
use crate::error::code;
use crate::services::rest::RestResponse;
use crate::services::{ConnectorEntry, Credential, StaticSource};
use flow_like_types::bot_connector::{
    CalDavCalendarInfo, ConnectorKind, ConnectorPolicy, ConnectorRule,
};

const ICLOUD_ROOT: &str = include_str!("fixtures/icloud_root.xml");
const ICLOUD_PRINCIPAL: &str = include_str!("fixtures/icloud_principal.xml");
const ICLOUD_CALENDARS: &str = include_str!("fixtures/icloud_calendars.xml");
const ICLOUD_EVENT: &str = include_str!("fixtures/icloud_event.ics");
const NEXTCLOUD_DAV: &str = include_str!("fixtures/nextcloud_dav.xml");
const NEXTCLOUD_PRINCIPAL: &str = include_str!("fixtures/nextcloud_principal.xml");
const NEXTCLOUD_CALENDARS: &str = include_str!("fixtures/nextcloud_calendars.xml");
const NEXTCLOUD_REPORT: &str = include_str!("fixtures/nextcloud_report_expanded.xml");
const RADICALE_ROOT: &str = include_str!("fixtures/radicale_root.xml");
const RADICALE_PRINCIPAL: &str = include_str!("fixtures/radicale_principal.xml");
const RADICALE_CALENDARS: &str = include_str!("fixtures/radicale_calendars.xml");
const RADICALE_REPORT: &str = include_str!("fixtures/radicale_report.xml");
const WEEKLY_SERIES: &str = include_str!("fixtures/weekly_series.ics");

const RADICALE_CALENDAR: &str = "http://localhost:5232/felix/a1b2c3d4-e5f6-7a8b-9c0d-1e2f3a4b5c6d/";
const NEXTCLOUD_PERSONAL: &str =
    "https://cloud.example.com/remote.php/dav/calendars/felix/personal/";
const ICLOUD_HOME: &str = "https://p52-caldav.icloud.com/1234567890/calendars/home/";

fn utc(y: i32, m: u32, d: u32, h: u32, min: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(y, m, d, h, min, 0).unwrap()
}

#[derive(Debug, Clone)]
struct Seen {
    method: DavMethod,
    url: String,
    depth: Option<Depth>,
    precondition: Precondition,
    body: Option<String>,
    authorization: Option<String>,
}

struct Route {
    method: DavMethod,
    url: String,
    response: DavResponse,
}

/// Answers each request with the first unused route for its method and URL (a URL ending in
/// `*` matches by prefix) and records what was sent.
#[derive(Default)]
struct FakeDav {
    routes: Mutex<Vec<Route>>,
    seen: Mutex<Vec<Seen>>,
}

impl FakeDav {
    fn on(self, method: DavMethod, url: &str, status: u16, text: &str) -> Self {
        self.answer(
            method,
            url,
            DavResponse {
                status,
                text: text.to_string(),
                ..DavResponse::default()
            },
        )
    }

    fn answer(self, method: DavMethod, url: &str, response: DavResponse) -> Self {
        self.routes.lock().unwrap().push(Route {
            method,
            url: url.to_string(),
            response,
        });
        self
    }

    fn seen(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }

    fn left(&self) -> usize {
        self.routes.lock().unwrap().len()
    }
}

fn etagged(status: u16, etag: &str, text: &str) -> DavResponse {
    DavResponse {
        status,
        etag: Some(etag.to_string()),
        text: text.to_string(),
        ..DavResponse::default()
    }
}

fn redirect(location: &str) -> DavResponse {
    DavResponse {
        status: 301,
        location: Some(location.to_string()),
        ..DavResponse::default()
    }
}

#[async_trait]
impl DavTransport for FakeDav {
    async fn send(
        &self,
        request: &DavRequest,
        credential: &Credential,
    ) -> Result<DavResponse, String> {
        let url = request.url.to_string();
        self.seen.lock().unwrap().push(Seen {
            method: request.method,
            url: url.clone(),
            depth: request.depth,
            precondition: request.precondition.clone(),
            body: request.body.as_ref().map(|body| match body {
                DavBody::Xml(text) | DavBody::Calendar(text) => text.clone(),
            }),
            authorization: credential.header().map(str::to_string),
        });
        let mut routes = self.routes.lock().unwrap();
        let index = routes
            .iter()
            .position(|route| {
                route.method == request.method
                    && match route.url.strip_suffix('*') {
                        Some(prefix) => url.starts_with(prefix),
                        None => route.url == url,
                    }
            })
            .ok_or_else(|| format!("no fake answer for {} {url}", request.method.name()))?;
        Ok(routes.remove(index).response)
    }
}

fn account(url: &str) -> DavAccount {
    DavAccount::new("Calendar", url, &[]).unwrap()
}

fn credential() -> Credential {
    basic_credential("felix@example.com", "abcd-efgh-ijkl-mnop")
}

fn calendar(url: &str, zone: Option<Tz>) -> DavCalendar {
    DavCalendar {
        url: Url::parse(url).unwrap(),
        name: "Personal".into(),
        color: None,
        ctag: None,
        sync_token: None,
        writable: true,
        zone,
    }
}

fn find<'a>(events: &'a [ExternalEvent], id_end: &str) -> &'a ExternalEvent {
    events
        .iter()
        .find(|event| event.event_id.ends_with(id_end))
        .unwrap_or_else(|| panic!("no event ending in {id_end}: {events:#?}"))
}

#[test]
fn content_lines_unfold_keep_quoted_params_and_refold() {
    let text = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nATTENDEE;CN=\"Doe, Jane; PhD\";PARTSTAT=\r\n ACCEPTED:mailto:jane@example.com\r\nSUMMARY:One\\, two\\; three\\nfour\\\\\n\tfive\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
    let calendar = ical::parse(text).unwrap();
    let event = calendar.children_named("VEVENT").next().unwrap();
    let attendee = event.property("ATTENDEE").unwrap();
    assert_eq!(attendee.param("CN"), Some("Doe, Jane; PhD"));
    assert_eq!(attendee.param("partstat"), Some("ACCEPTED"));
    assert_eq!(attendee.value, "mailto:jane@example.com");
    assert_eq!(
        event.text("SUMMARY").unwrap(),
        "One, two; three\nfour\\five"
    );
    let rendered = calendar.render();
    assert!(
        rendered.split("\r\n").all(|line| line.len() <= 75),
        "{rendered}"
    );
    assert_eq!(ical::parse(&rendered).unwrap(), calendar);

    let long = "x".repeat(70) + "äöü" + &"y".repeat(100);
    let mut event = ical::Component::new("VEVENT");
    event.push(ical::Line::text("DESCRIPTION", &long));
    let rendered = event.render();
    assert!(rendered.split("\r\n").all(|line| line.len() <= 75));
    let mut wrapped = ical::Component::new("VCALENDAR");
    wrapped.push_child(event);
    let back = ical::parse(&wrapped.render()).unwrap();
    assert_eq!(
        back.children().next().unwrap().text("DESCRIPTION").unwrap(),
        long
    );
}

#[test]
fn text_escaping_round_trips() {
    let text = "Room 4, floor 2; bring \\ and\nnotes";
    assert_eq!(
        ical::escape(text),
        "Room 4\\, floor 2\\; bring \\\\ and\\nnotes"
    );
    assert_eq!(ical::unescape(&ical::escape(text)), text);
    assert_eq!(ical::unescape("a\\Nb"), "a\nb");
}

#[test]
fn durations_read_weeks_days_and_times() {
    let minutes = |text: &str| ical::duration(text).map(|duration| duration.num_minutes());
    assert_eq!(minutes("PT1H30M"), Some(90));
    assert_eq!(minutes("P1D"), Some(1440));
    assert_eq!(minutes("P2W"), Some(20160));
    assert_eq!(minutes("P1DT2H"), Some(1560));
    assert_eq!(minutes("-PT15M"), Some(-15));
    assert_eq!(minutes("PT45S"), Some(0));
    for bad in ["", "P", "PT", "1H", "PT1D", "P1H", "PT1H2"] {
        assert_eq!(minutes(bad), None, "{bad}");
    }
}

#[test]
fn every_windows_zone_maps_to_a_known_zone() {
    for (windows, iana) in zones::windows_table() {
        assert!(iana.parse::<Tz>().is_ok(), "{windows} -> {iana}");
    }
}

#[test]
fn zones_resolve_iana_vendor_windows_and_vtimezone_names() {
    let text = "BEGIN:VCALENDAR\nBEGIN:VTIMEZONE\nTZID:Custom Berlin\nX-LIC-LOCATION:Europe/Berlin\nEND:VTIMEZONE\nBEGIN:VTIMEZONE\nTZID:India\nBEGIN:STANDARD\nDTSTART:19700101T000000\nTZOFFSETFROM:+0530\nTZOFFSETTO:+0530\nEND:STANDARD\nEND:VTIMEZONE\nBEGIN:VTIMEZONE\nTZID:Shifting\nBEGIN:STANDARD\nTZOFFSETTO:+0100\nEND:STANDARD\nBEGIN:DAYLIGHT\nTZOFFSETTO:+0200\nEND:DAYLIGHT\nEND:VTIMEZONE\nEND:VCALENDAR\n";
    let calendar = ical::parse(text).unwrap();
    let zones = Zones::of(&calendar);
    let berlin = Some(Zone::Iana(Berlin));
    assert_eq!(zones.resolve("Europe/Berlin"), berlin);
    assert_eq!(zones.resolve("europe/berlin"), berlin);
    assert_eq!(zones.resolve("\"Europe/Berlin\""), berlin);
    assert_eq!(
        zones.resolve("/mozilla.org/20050126_1/Europe/Berlin"),
        berlin
    );
    assert_eq!(zones.resolve("W. Europe Standard Time"), berlin);
    assert_eq!(zones.resolve("Custom Berlin"), berlin);
    assert_eq!(
        zones.resolve("India"),
        Some(Zone::Fixed(chrono::FixedOffset::east_opt(19_800).unwrap()))
    );
    assert_eq!(zones.resolve("Shifting"), None);
    assert_eq!(zones.resolve("Mars/Olympus_Mons"), None);
    assert_eq!(
        zones.resolve("Argentina Standard Time"),
        Some(Zone::Iana(chrono_tz::America::Argentina::Buenos_Aires))
    );
}

#[test]
fn multistatus_keeps_only_found_properties() {
    let entries = xml::multistatus(ICLOUD_ROOT).unwrap();
    assert_eq!(entries.len(), 1);
    let root = &entries[0];
    assert_eq!(root.href, "/");
    assert_eq!(
        root.prop(DAV, "current-user-principal").unwrap().hrefs,
        ["/1234567890/principal/"]
    );
    assert!(root.prop(CALDAV, "calendar-home-set").is_none());
    assert!(xml::multistatus("<html><body>Login</body></html>").is_err());
    assert!(
        xml::multistatus("<!DOCTYPE x [<!ENTITY a \"aaaa\">]><multistatus xmlns=\"DAV:\"/>")
            .is_err()
    );
}

#[test]
fn dav_error_bodies_name_the_condition() {
    let sabre = r#"<?xml version="1.0" encoding="utf-8"?>
<d:error xmlns:d="DAV:" xmlns:s="http://sabredav.org/ns">
  <s:sabredav-version>4.5.0</s:sabredav-version>
  <s:exception>Sabre\DAV\Exception\PreconditionFailed</s:exception>
  <s:message>An If-Match header was specified, but none of the specified ETags matched.</s:message>
</d:error>"#;
    assert_eq!(
        xml::error_message(sabre).as_deref(),
        Some("An If-Match header was specified, but none of the specified ETags matched.")
    );
    let condition = r#"<D:error xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav"><C:no-uid-conflict><D:href>/cal/other.ics</D:href></C:no-uid-conflict></D:error>"#;
    assert_eq!(
        xml::error_message(condition).as_deref(),
        Some("no-uid-conflict")
    );
    assert_eq!(xml::error_message("Precondition Failed"), None);
}

#[test]
fn expanded_occurrences_get_stable_ids_and_utc_times() {
    let from = utc(2026, 11, 23, 0, 0);
    let to = utc(2026, 11, 30, 0, 0);
    let mut all = Vec::new();
    for entry in xml::multistatus(NEXTCLOUD_REPORT).unwrap() {
        let data = entry.prop(CALDAV, "calendar-data").unwrap().text.clone();
        all.extend(
            events::read(&entry.href, &data, Berlin, from, to)
                .unwrap()
                .events,
        );
    }
    assert_eq!(all.len(), 4, "{all:#?}");
    let standup = find(&all, "weekly-standup.ics#20261123T080000Z");
    assert!(standup.recurring);
    assert_eq!(standup.starts_at, utc(2026, 11, 23, 8, 0));
    assert_eq!(standup.ends_at, Some(utc(2026, 11, 23, 8, 15)));
    find(&all, "weekly-standup.ics#20261125T080000Z");

    let dentist = find(&all, "2e3f4a5b6c7d.ics");
    assert_eq!(dentist.title, "Dentist");
    assert_eq!(dentist.starts_at, utc(2026, 11, 26, 13, 30));
    assert_eq!(dentist.ends_at, Some(utc(2026, 11, 26, 14, 15)));
    assert_eq!(
        dentist.location.as_deref(),
        Some("Praxis Dr. Weber, Hauptstraße 5, 10115 Berlin")
    );
    assert_eq!(
        dentist.notes.as_deref(),
        Some("Bring the insurance card.\nAsk about the night guard.")
    );
    assert_eq!(
        dentist.link.as_deref(),
        Some("https://praxis-weber.example/termine/4711")
    );
    assert!(!dentist.recurring && dentist.busy && !dentist.all_day);

    let holiday = find(&all, "holiday.ics");
    assert!(holiday.all_day && !holiday.busy);
    assert_eq!(holiday.starts_at, utc(2026, 11, 26, 23, 0));
    assert_eq!(holiday.ends_at, Some(utc(2026, 11, 28, 23, 0)));
    assert!(all.iter().all(|event| event.title != "Quarterly review"));
}

fn radicale_events(from: DateTime<Utc>, to: DateTime<Utc>) -> (Vec<ExternalEvent>, usize) {
    let mut all = Vec::new();
    let mut unreadable = 0;
    for entry in xml::multistatus(RADICALE_REPORT).unwrap() {
        let data = entry.prop(CALDAV, "calendar-data").unwrap().text.clone();
        match events::read(&entry.href, &data, Berlin, from, to) {
            Ok(read) => all.extend(read.events),
            Err(_) => unreadable += 1,
        }
    }
    all.sort_by_key(|event| event.starts_at);
    (all, unreadable)
}

#[test]
fn series_are_expanded_here_with_exdates_moves_and_dst() {
    let (all, unreadable) = radicale_events(utc(2026, 10, 15, 0, 0), utc(2026, 11, 20, 0, 0));
    assert_eq!(unreadable, 1, "the Mars zone is unknown");
    let sync: Vec<(&str, DateTime<Utc>)> = all
        .iter()
        .filter(|event| event.event_id.contains("design-sync.ics"))
        .map(|event| (event.event_id.rsplit('#').next().unwrap(), event.starts_at))
        .collect();
    assert_eq!(
        sync,
        [
            ("20261019T070000Z", utc(2026, 10, 19, 7, 0)),
            ("20261026T080000Z", utc(2026, 10, 26, 8, 0)),
            ("20261102T080000Z", utc(2026, 11, 2, 8, 0)),
            ("20261116T080000Z", utc(2026, 11, 16, 9, 0)),
        ]
    );
    let first = find(&all, "#20261019T070000Z");
    assert_eq!(first.title, "Design sync");
    assert_eq!(first.ends_at, Some(utc(2026, 10, 19, 7, 30)));
    assert_eq!(first.location.as_deref(), Some("Room 4.12"));
    assert_eq!(
        first.notes.as_deref(),
        Some(
            "Weekly sync with the design team. Agenda: review open tickets, plan the next sprint and check the release notes before Friday."
        )
    );
    assert_eq!(find(&all, "#20261116T080000Z").title, "Design sync (moved)");

    let lunch = find(&all, "lunch.ics");
    assert_eq!(
        lunch.starts_at,
        utc(2026, 10, 22, 10, 30),
        "floating time read in Berlin"
    );
    let birthday = find(&all, "birthday-mia.ics#20261102");
    assert!(birthday.all_day && birthday.recurring);
    assert_eq!(birthday.starts_at, utc(2026, 11, 1, 23, 0));
    let concert = find(&all, "concert.ics");
    assert_eq!(concert.starts_at, utc(2026, 11, 3, 17, 0));
    assert_eq!(concert.ends_at, Some(utc(2026, 11, 3, 19, 30)));
}

#[test]
fn until_count_and_cancelled_occurrences_end_a_series() {
    let december = events::read(
        "/c/yoga.ics",
        WEEKLY_SERIES,
        Berlin,
        utc(2026, 12, 1, 0, 0),
        utc(2027, 1, 15, 0, 0),
    )
    .unwrap()
    .events;
    let days: Vec<u32> = december
        .iter()
        .map(|event| chrono::Datelike::day(&event.starts_at))
        .collect();
    assert_eq!(days, [2, 9, 16, 23, 30]);
    assert_eq!(december[4].starts_at, utc(2026, 12, 30, 17, 0));

    let october = events::read(
        "/c/yoga.ics",
        WEEKLY_SERIES,
        Berlin,
        utc(2026, 10, 1, 0, 0),
        utc(2026, 10, 15, 0, 0),
    )
    .unwrap()
    .events;
    let ids: Vec<&str> = october
        .iter()
        .map(|event| event.event_id.as_str())
        .collect();
    assert_eq!(
        ids,
        ["/c/yoga.ics#20261014T160000Z"],
        "the cancelled 7 October is left out"
    );

    let starts = recur::rule_starts(
        "FREQ=DAILY;COUNT=3",
        When::Utc(utc(2026, 11, 1, 9, 0)),
        Berlin,
        utc(2026, 10, 1, 0, 0),
        utc(2026, 12, 1, 0, 0),
        100,
    )
    .unwrap();
    assert_eq!(starts.starts.len(), 3);
    let dated = recur::rule_starts(
        "FREQ=WEEKLY;UNTIL=20261115",
        When::Date(chrono::NaiveDate::from_ymd_opt(2026, 11, 1).unwrap()),
        Berlin,
        utc(2026, 10, 1, 0, 0),
        utc(2026, 12, 1, 0, 0),
        100,
    )
    .unwrap();
    assert_eq!(dated.starts.len(), 3, "{dated:?}");
    let before = recur::rule_starts(
        "FREQ=DAILY;UNTIL=20200101T000000Z",
        When::Utc(utc(2026, 11, 1, 9, 0)),
        Berlin,
        utc(2026, 10, 1, 0, 0),
        utc(2026, 12, 1, 0, 0),
        100,
    )
    .unwrap();
    assert_eq!(before.starts, [When::Utc(utc(2026, 11, 1, 9, 0))]);
}

#[test]
fn a_dense_rule_from_long_ago_is_cut_short() {
    let started = std::time::Instant::now();
    let starts = recur::rule_starts(
        "FREQ=SECONDLY",
        When::Utc(utc(2000, 1, 1, 0, 0)),
        Berlin,
        utc(2026, 11, 1, 0, 0),
        utc(2026, 11, 2, 0, 0),
        events::MAX_OCCURRENCES,
    )
    .unwrap();
    assert!(starts.limited);
    assert!(starts.starts.is_empty());
    assert!(started.elapsed() < std::time::Duration::from_secs(5));
}

#[test]
fn an_unreadable_rule_keeps_the_first_occurrence() {
    let text = "BEGIN:VCALENDAR\nBEGIN:VEVENT\nUID:x\nDTSTART:20261110T090000Z\nDTEND:20261110T100000Z\nRRULE:FREQ=SOMETIMES\nSUMMARY:Odd\nEND:VEVENT\nEND:VCALENDAR\n";
    let read = events::read(
        "/c/odd.ics",
        text,
        Berlin,
        utc(2026, 11, 1, 0, 0),
        utc(2026, 11, 30, 0, 0),
    )
    .unwrap();
    assert_eq!(read.events.len(), 1);
    assert!(read.events[0].recurring);
    assert_eq!(read.events[0].event_id, "/c/odd.ics#20261110T090000Z");
}

#[test]
fn an_occurrence_is_found_from_the_series() {
    let calendar = ical::parse(WEEKLY_SERIES).unwrap();
    let found = events::find("/c/yoga.ics", &calendar, Berlin, Some("20261014T160000Z"))
        .unwrap()
        .unwrap();
    assert_eq!(found.starts_at, utc(2026, 10, 14, 16, 0));
    assert_eq!(found.ends_at, Some(utc(2026, 10, 14, 17, 30)));
    assert!(
        events::find("/c/yoga.ics", &calendar, Berlin, Some("20261007T160000Z"))
            .unwrap()
            .is_none()
    );
    assert!(
        events::find("/c/yoga.ics", &calendar, Berlin, Some("20261015T160000Z"))
            .unwrap()
            .is_none()
    );
    assert_eq!(
        EventId::parse("/c/yoga.ics#20261014T160000Z").unwrap(),
        EventId {
            href: "/c/yoga.ics".into(),
            occurrence: Some("20261014T160000Z".into())
        }
    );
}

#[test]
fn accounts_send_the_sign_in_only_to_their_hosts() {
    assert_eq!(
        basic_credential("felix", "pw").header(),
        Some("Basic ZmVsaXg6cHc=")
    );
    let icloud = account("https://caldav.icloud.com/");
    assert!(
        icloud
            .permits(&Url::parse("https://p52-caldav.icloud.com/1/calendars/").unwrap())
            .is_ok()
    );
    assert!(
        icloud
            .permits(&Url::parse("http://p52-caldav.icloud.com/").unwrap())
            .is_err()
    );
    assert!(
        icloud
            .permits(&Url::parse("https://icloud.com.evil.example/").unwrap())
            .is_err()
    );
    let nextcloud = account("https://cloud.example.com/");
    assert!(
        nextcloud
            .permits(&Url::parse("https://other.example.com/").unwrap())
            .is_err()
    );
    assert!(DavAccount::new("x", "https://user:pw@cloud.example.com/", &[]).is_err());
    assert!(DavAccount::new("x", "ftp://cloud.example.com/", &[]).is_err());
    let lan = account("http://nas.local:5232/");
    assert!(
        lan.permits(&Url::parse("https://nas.local:5232/felix/").unwrap())
            .is_ok()
    );
}

#[tokio::test]
async fn the_http_transport_keeps_the_egress_rules() {
    let request = |url: &str| DavRequest::new(DavMethod::Propfind, Url::parse(url).unwrap());
    let server = HttpDavTransport(Environment::Server);
    let none = Credential::none();
    for url in [
        "http://93.184.216.34/dav/",
        "https://127.0.0.1/dav/",
        "https://192.168.1.20/dav/",
        "https://10.0.0.5/",
    ] {
        assert!(server.send(&request(url), &none).await.is_err(), "{url}");
    }
    let desktop = HttpDavTransport(Environment::Local);
    let refused = desktop
        .send(&request("http://169.254.169.254/latest/"), &none)
        .await
        .unwrap_err();
    assert!(refused.contains("metadata"), "{refused}");
}

#[tokio::test]
async fn icloud_discovery_follows_the_partition_host() {
    let fake = FakeDav::default()
        .on(
            DavMethod::Propfind,
            "https://caldav.icloud.com/",
            207,
            ICLOUD_ROOT,
        )
        .on(
            DavMethod::Propfind,
            "https://caldav.icloud.com/1234567890/principal/",
            207,
            ICLOUD_PRINCIPAL,
        )
        .on(
            DavMethod::Propfind,
            "https://p52-caldav.icloud.com/1234567890/calendars/",
            207,
            ICLOUD_CALENDARS,
        );
    let account = account("https://caldav.icloud.com/");
    let credential = credential();
    let client = DavClient::new(&account, &credential, &fake, chrono_tz::UTC);
    let calendars = client.discover().await.unwrap();
    let names: Vec<&str> = calendars
        .iter()
        .map(|calendar| calendar.name.as_str())
        .collect();
    assert_eq!(names, ["Home", "Work", "Family"]);
    let home = &calendars[0];
    assert_eq!(home.url.as_str(), ICLOUD_HOME);
    assert_eq!(home.color.as_deref(), Some("#1BADF8FF"));
    assert_eq!(home.zone, Some(Berlin));
    assert!(home.ctag.is_some() && home.sync_token.is_some());
    assert!(home.writable && calendars[1].writable && !calendars[2].writable);
    let seen = fake.seen();
    assert_eq!(seen.len(), 3);
    assert!(
        seen.iter()
            .all(|request| request.authorization == credential.header().map(str::to_string))
    );
    assert_eq!(seen[0].depth, Some(Depth::Zero));
    assert_eq!(seen[2].depth, Some(Depth::One));
    assert!(
        seen[2]
            .body
            .as_deref()
            .unwrap()
            .contains("current-user-privilege-set")
    );
}

#[tokio::test]
async fn nextcloud_discovery_uses_the_well_known_redirect() {
    let fake = FakeDav::default()
        .on(DavMethod::Propfind, "https://cloud.example.com/", 405, "")
        .answer(
            DavMethod::Propfind,
            "https://cloud.example.com/.well-known/caldav",
            redirect("/remote.php/dav/"),
        )
        .on(
            DavMethod::Propfind,
            "https://cloud.example.com/remote.php/dav/",
            207,
            NEXTCLOUD_DAV,
        )
        .on(
            DavMethod::Propfind,
            "https://cloud.example.com/remote.php/dav/principals/users/felix/",
            207,
            NEXTCLOUD_PRINCIPAL,
        )
        .on(
            DavMethod::Propfind,
            "https://cloud.example.com/remote.php/dav/calendars/felix/",
            207,
            NEXTCLOUD_CALENDARS,
        );
    let account = account("https://cloud.example.com/");
    let credential = credential();
    let client = DavClient::new(&account, &credential, &fake, chrono_tz::UTC);
    let calendars = client.discover().await.unwrap();
    let names: Vec<&str> = calendars
        .iter()
        .map(|calendar| calendar.name.as_str())
        .collect();
    assert_eq!(names, ["Personal", "Contact birthdays", "work stuff"]);
    assert_eq!(calendars[0].url.as_str(), NEXTCLOUD_PERSONAL);
    assert!(calendars[0].writable && !calendars[1].writable);
    assert_eq!(
        calendars[0].sync_token.as_deref(),
        Some("http://sabre.io/ns/sync/4711")
    );
    assert_eq!(fake.left(), 0);
}

#[tokio::test]
async fn radicale_on_localhost_is_found_from_its_root() {
    let fake = FakeDav::default()
        .on(
            DavMethod::Propfind,
            "http://localhost:5232/",
            207,
            RADICALE_ROOT,
        )
        .on(
            DavMethod::Propfind,
            "http://localhost:5232/felix/",
            207,
            RADICALE_PRINCIPAL,
        )
        .on(
            DavMethod::Propfind,
            "http://localhost:5232/felix/",
            207,
            RADICALE_CALENDARS,
        );
    let account = account("http://localhost:5232/");
    let credential = credential();
    let client = DavClient::new(&account, &credential, &fake, chrono_tz::UTC);
    let calendars = client.discover().await.unwrap();
    assert_eq!(calendars.len(), 1);
    assert_eq!(calendars[0].url.as_str(), RADICALE_CALENDAR);
    assert_eq!(calendars[0].color.as_deref(), Some("#ff0000ff"));
    assert!(calendars[0].writable);
}

#[tokio::test]
async fn a_redirect_off_the_account_is_refused_before_the_password_goes_out() {
    for location in [
        "https://evil.example.net/dav/",
        "http://cloud.example.com/remote.php/dav/",
    ] {
        let fake = FakeDav::default()
            .on(DavMethod::Propfind, "https://cloud.example.com/", 404, "")
            .answer(
                DavMethod::Propfind,
                "https://cloud.example.com/.well-known/caldav",
                redirect(location),
            );
        let account = account("https://cloud.example.com/");
        let credential = credential();
        let client = DavClient::new(&account, &credential, &fake, chrono_tz::UTC);
        assert!(client.discover().await.is_err(), "{location}");
        assert!(
            fake.seen()
                .iter()
                .all(|request| request.url.starts_with("https://cloud.example.com/")),
            "{location}"
        );
    }
}

#[tokio::test]
async fn a_wrong_password_stops_discovery() {
    let fake = FakeDav::default().on(
        DavMethod::Propfind,
        "https://caldav.icloud.com/",
        401,
        "<html>Unauthorized abcd-efgh-ijkl-mnop</html>",
    );
    let account = account("https://caldav.icloud.com/");
    let credential = credential();
    let client = DavClient::new(&account, &credential, &fake, chrono_tz::UTC);
    let error = client.discover().await.unwrap_err();
    assert_eq!(http_status(&error), Some(401));
    assert!(error.message.contains("sign-in"), "{}", error.message);
    assert!(
        error.message.contains("caldav.icloud.com"),
        "{}",
        error.message
    );
    assert_eq!(fake.seen().len(), 1, "no well-known retry after a 401");
}

#[tokio::test]
async fn listing_asks_the_server_to_expand_and_maps_events() {
    let fake = FakeDav::default().on(DavMethod::Report, NEXTCLOUD_PERSONAL, 207, NEXTCLOUD_REPORT);
    let account = account("https://cloud.example.com/");
    let credential = credential();
    let client = DavClient::new(&account, &credential, &fake, Berlin);
    let calendar = calendar(NEXTCLOUD_PERSONAL, None);
    let listed = client
        .list(&calendar, utc(2026, 11, 23, 0, 0), utc(2026, 11, 30, 0, 0))
        .await
        .unwrap();
    assert!(listed.complete);
    assert_eq!(listed.unreadable, 0);
    assert_eq!(listed.events.len(), 4);
    assert_eq!(
        listed.events[0].event_id,
        "/remote.php/dav/calendars/felix/personal/weekly-standup.ics#20261123T080000Z"
    );
    let seen = fake.seen();
    assert_eq!(seen[0].depth, Some(Depth::One));
    let body = seen[0].body.as_deref().unwrap();
    assert!(
        body.contains(r#"<c:expand start="20261123T000000Z" end="20261130T000000Z"/>"#),
        "{body}"
    );
    assert!(
        body.contains(r#"<c:comp-filter name="VEVENT"><c:time-range start="20261123T000000Z""#),
        "{body}"
    );
}

#[tokio::test]
async fn a_server_that_refuses_expand_is_asked_again_and_expanded_here() {
    let fake = FakeDav::default()
        .on(
            DavMethod::Report,
            RADICALE_CALENDAR,
            501,
            "expand is not supported",
        )
        .on(DavMethod::Report, RADICALE_CALENDAR, 207, RADICALE_REPORT);
    let account = account("http://localhost:5232/");
    let credential = credential();
    let client = DavClient::new(&account, &credential, &fake, chrono_tz::UTC);
    let calendar = calendar(RADICALE_CALENDAR, Some(Berlin));
    let listed = client
        .list(&calendar, utc(2026, 10, 15, 0, 0), utc(2026, 11, 20, 0, 0))
        .await
        .unwrap();
    assert_eq!(listed.unreadable, 1);
    assert_eq!(listed.events.len(), 7, "{:#?}", listed.events);
    assert!(listed.note().unwrap().contains("could not be read"));
    let seen = fake.seen();
    assert!(seen[0].body.as_deref().unwrap().contains("<c:expand"));
    assert!(!seen[1].body.as_deref().unwrap().contains("<c:expand"));
}

#[tokio::test]
async fn hrefs_outside_the_calendar_are_not_read() {
    let report = NEXTCLOUD_REPORT.replace(
        "/remote.php/dav/calendars/felix/personal/holiday.ics",
        "/remote.php/dav/calendars/someone-else/private/holiday.ics",
    );
    let fake = FakeDav::default().on(DavMethod::Report, NEXTCLOUD_PERSONAL, 207, &report);
    let account = account("https://cloud.example.com/");
    let credential = credential();
    let client = DavClient::new(&account, &credential, &fake, Berlin);
    let listed = client
        .list(
            &calendar(NEXTCLOUD_PERSONAL, None),
            utc(2026, 11, 23, 0, 0),
            utc(2026, 11, 30, 0, 0),
        )
        .await
        .unwrap();
    assert_eq!(listed.unreadable, 1);
    assert_eq!(listed.events.len(), 3);

    let home = calendar(ICLOUD_HOME, None);
    let account = account_icloud();
    let client = DavClient::new(&account, &credential, &fake, Berlin);
    for id in [
        "/1234567890/calendars/work/x.ics",
        "../work/x.ics",
        "/1234567890/calendars/home/",
        "https://evil.example.net/1234567890/calendars/home/x.ics",
        "/1234567890/calendars/home/x.ics?export",
    ] {
        let error = client.get(&home, id).await.unwrap_err();
        assert_eq!(error.code, code::INVALID_ARGS, "{id}: {}", error.message);
    }
}

fn account_icloud() -> DavAccount {
    account("https://caldav.icloud.com/")
}

fn timed(title: &str, start: DateTime<Utc>, end: DateTime<Utc>) -> EventDraft {
    EventDraft {
        title: Some(title.into()),
        starts_at: Some(start),
        ends_at: Some(end),
        all_day: Some(false),
        tz: Some(Berlin),
        ..EventDraft::default()
    }
}

#[tokio::test]
async fn create_puts_a_new_resource_only_where_none_exists() {
    let fake = FakeDav::default()
        .on(DavMethod::Put, &format!("{ICLOUD_HOME}*"), 201, "")
        .on(DavMethod::Put, &format!("{ICLOUD_HOME}*"), 201, "");
    let account = account_icloud();
    let credential = credential();
    let client = DavClient::new(&account, &credential, &fake, Berlin);
    let home = calendar(ICLOUD_HOME, Some(Berlin));
    let mut draft = timed(
        "Call, then lunch; maybe",
        utc(2026, 11, 24, 9, 0),
        utc(2026, 11, 24, 10, 0),
    );
    draft.location = Some("Office".into());
    draft.notes = Some("Line one\nLine two".into());
    let created = client.create(&home, &draft).await.unwrap();
    assert_eq!(created.title, "Call, then lunch; maybe");
    assert_eq!(created.starts_at, utc(2026, 11, 24, 9, 0));
    assert!(
        created.event_id.starts_with("/1234567890/calendars/home/")
            && created.event_id.ends_with(".ics")
    );
    let seen = fake.seen();
    assert_eq!(seen[0].precondition, Precondition::IfAbsent);
    assert!(seen[0].url.ends_with(".ics"));
    let body = seen[0].body.clone().unwrap();
    for line in [
        "BEGIN:VCALENDAR\r\n",
        "VERSION:2.0\r\n",
        "PRODID:-//Flow-Like//Bot Calendar//EN\r\n",
        "DTSTART:20261124T090000Z\r\n",
        "DTEND:20261124T100000Z\r\n",
        "SUMMARY:Call\\, then lunch\\; maybe\r\n",
        "LOCATION:Office\r\n",
        "DESCRIPTION:Line one\\nLine two\r\n",
    ] {
        assert!(body.contains(line), "{line} missing in {body}");
    }
    assert!(body.contains("UID:") && body.contains("DTSTAMP:"));

    let all_day = EventDraft {
        title: Some("Off".into()),
        starts_at: Some(utc(2026, 11, 26, 23, 0)),
        ends_at: Some(utc(2026, 11, 28, 23, 0)),
        all_day: Some(true),
        tz: Some(Berlin),
        ..EventDraft::default()
    };
    let created = client.create(&home, &all_day).await.unwrap();
    assert!(created.all_day);
    assert_eq!(created.starts_at, utc(2026, 11, 26, 23, 0));
    let body = fake.seen()[1].body.clone().unwrap();
    assert!(body.contains("DTSTART;VALUE=DATE:20261127\r\n"), "{body}");
    assert!(body.contains("DTEND;VALUE=DATE:20261129\r\n"), "{body}");
}

#[tokio::test]
async fn update_changes_only_what_it_sets() {
    let event_url = format!("{ICLOUD_HOME}4E2B9F3A.ics");
    let fake = FakeDav::default()
        .answer(
            DavMethod::Get,
            &event_url,
            etagged(200, "\"etag-1\"", ICLOUD_EVENT),
        )
        .on(DavMethod::Put, &event_url, 204, "");
    let account = account_icloud();
    let credential = credential();
    let client = DavClient::new(&account, &credential, &fake, chrono_tz::UTC);
    let home = calendar(ICLOUD_HOME, None);
    let draft = timed(
        "Coffee with Anna and Ben",
        utc(2026, 11, 24, 10, 0),
        utc(2026, 11, 24, 11, 0),
    );
    let updated = client
        .update(
            &home,
            "/1234567890/calendars/home/4E2B9F3A.ics",
            &draft,
            Scope::Event,
        )
        .await
        .unwrap();
    assert_eq!(updated.title, "Coffee with Anna and Ben");
    assert_eq!(updated.starts_at, utc(2026, 11, 24, 10, 0));
    let seen = fake.seen();
    assert_eq!(
        seen[1].precondition,
        Precondition::IfMatch("\"etag-1\"".into())
    );
    let body = seen[1].body.clone().unwrap();
    assert!(body.split("\r\n").all(|line| line.len() <= 75));
    let sent = ical::parse(&body).unwrap();
    let before = ical::parse(ICLOUD_EVENT).unwrap();
    let event = sent.children_named("VEVENT").next().unwrap();
    let old = before.children_named("VEVENT").next().unwrap();
    assert_eq!(event.text("SUMMARY").unwrap(), "Coffee with Anna and Ben");
    assert_eq!(
        event.property("DTSTART").unwrap().param("TZID"),
        Some("Europe/Berlin")
    );
    assert_eq!(event.property("DTSTART").unwrap().value, "20261124T110000");
    assert_eq!(event.property("DTEND").unwrap().value, "20261124T120000");
    assert_eq!(event.property("SEQUENCE").unwrap().value, "1");
    for kept in [
        "ATTENDEE",
        "ORGANIZER",
        "LOCATION",
        "X-APPLE-STRUCTURED-LOCATION",
        "X-APPLE-TRAVEL-ADVISORY-BEHAVIOR",
        "UID",
        "CREATED",
    ] {
        let now: Vec<_> = event.properties(kept).collect();
        let then: Vec<_> = old.properties(kept).collect();
        assert_eq!(now, then, "{kept}");
    }
    assert_eq!(
        event.children_named("VALARM").next(),
        old.children_named("VALARM").next()
    );
    assert_eq!(
        sent.children_named("VTIMEZONE").next(),
        before.children_named("VTIMEZONE").next()
    );
}

#[tokio::test]
async fn a_new_place_drops_the_old_map_pin() {
    let event_url = format!("{ICLOUD_HOME}4E2B9F3A.ics");
    let fake = FakeDav::default()
        .answer(
            DavMethod::Get,
            &event_url,
            etagged(200, "\"etag-1\"", ICLOUD_EVENT),
        )
        .on(DavMethod::Put, &event_url, 201, "");
    let account = account_icloud();
    let credential = credential();
    let client = DavClient::new(&account, &credential, &fake, chrono_tz::UTC);
    let draft = EventDraft {
        location: Some("Bar Raval, Berlin".into()),
        notes: Some(String::new()),
        ..EventDraft::default()
    };
    let updated = client
        .update(
            &calendar(ICLOUD_HOME, None),
            "/1234567890/calendars/home/4E2B9F3A.ics",
            &draft,
            Scope::Event,
        )
        .await
        .unwrap();
    assert_eq!(updated.location.as_deref(), Some("Bar Raval, Berlin"));
    let body = fake.seen()[1].body.clone().unwrap();
    assert!(body.contains("LOCATION:Bar Raval\\, Berlin\r\n"));
    assert!(!body.contains("X-APPLE-STRUCTURED-LOCATION"));
    assert!(body.contains("SEQUENCE:0\r\n"), "times did not move");
    assert!(body.contains("DTSTART;TZID=Europe/Berlin:20261124T100000\r\n"));
}

#[tokio::test]
async fn a_412_reads_again_and_retries_once() {
    let event_url = format!("{ICLOUD_HOME}4E2B9F3A.ics");
    let fake = FakeDav::default()
        .answer(
            DavMethod::Get,
            &event_url,
            etagged(200, "\"etag-1\"", ICLOUD_EVENT),
        )
        .on(DavMethod::Put, &event_url, 412, "")
        .answer(
            DavMethod::Get,
            &event_url,
            etagged(200, "\"etag-2\"", ICLOUD_EVENT),
        )
        .on(DavMethod::Put, &event_url, 204, "");
    let account = account_icloud();
    let credential = credential();
    let client = DavClient::new(&account, &credential, &fake, chrono_tz::UTC);
    let draft = EventDraft {
        title: Some("Renamed".into()),
        ..EventDraft::default()
    };
    let id = "/1234567890/calendars/home/4E2B9F3A.ics";
    client
        .update(&calendar(ICLOUD_HOME, None), id, &draft, Scope::Event)
        .await
        .unwrap();
    let seen = fake.seen();
    assert_eq!(seen.len(), 4);
    assert_eq!(
        seen[3].precondition,
        Precondition::IfMatch("\"etag-2\"".into())
    );

    let fake = FakeDav::default()
        .answer(
            DavMethod::Get,
            &event_url,
            etagged(200, "\"etag-1\"", ICLOUD_EVENT),
        )
        .on(DavMethod::Put, &event_url, 412, "")
        .answer(
            DavMethod::Get,
            &event_url,
            etagged(200, "\"etag-2\"", ICLOUD_EVENT),
        )
        .on(DavMethod::Put, &event_url, 412, "");
    let client = DavClient::new(&account, &credential, &fake, chrono_tz::UTC);
    let error = client
        .update(&calendar(ICLOUD_HOME, None), id, &draft, Scope::Event)
        .await
        .unwrap_err();
    assert_eq!(http_status(&error), Some(412));
    assert!(error.message.contains("changed"), "{}", error.message);
}

#[tokio::test]
async fn delete_sends_if_match_and_counts_gone_as_deleted() {
    let event_url = format!("{ICLOUD_HOME}4E2B9F3A.ics");
    let id = "/1234567890/calendars/home/4E2B9F3A.ics";
    let fake = FakeDav::default()
        .answer(
            DavMethod::Get,
            &event_url,
            etagged(200, "\"etag-9\"", ICLOUD_EVENT),
        )
        .on(DavMethod::Delete, &event_url, 204, "")
        .on(DavMethod::Get, &event_url, 404, "");
    let account = account_icloud();
    let credential = credential();
    let client = DavClient::new(&account, &credential, &fake, chrono_tz::UTC);
    let home = calendar(ICLOUD_HOME, None);
    client.delete(&home, id, Scope::Event).await.unwrap();
    assert_eq!(
        fake.seen()[1].precondition,
        Precondition::IfMatch("\"etag-9\"".into())
    );
    client.delete(&home, id, Scope::Event).await.unwrap();
    assert_eq!(fake.seen().len(), 3);
}

#[tokio::test]
async fn occurrences_change_only_as_a_whole_series() {
    let series_url = format!("{NEXTCLOUD_PERSONAL}yoga.ics");
    let id = "/remote.php/dav/calendars/felix/personal/yoga.ics";
    let occurrence = format!("{id}#20261014T160000Z");
    let fake = FakeDav::default()
        .answer(
            DavMethod::Get,
            &series_url,
            etagged(200, "\"s1\"", WEEKLY_SERIES),
        )
        .on(DavMethod::Put, &series_url, 204, "")
        .answer(
            DavMethod::Get,
            &series_url,
            etagged(200, "\"s2\"", WEEKLY_SERIES),
        )
        .answer(
            DavMethod::Get,
            &series_url,
            etagged(200, "\"s2\"", WEEKLY_SERIES),
        )
        .answer(
            DavMethod::Get,
            &series_url,
            etagged(200, "\"s2\"", WEEKLY_SERIES),
        )
        .answer(
            DavMethod::Get,
            &series_url,
            etagged(200, "\"s2\"", WEEKLY_SERIES),
        )
        .on(DavMethod::Delete, &series_url, 204, "");
    let account = account("https://cloud.example.com/");
    let credential = credential();
    let client = DavClient::new(&account, &credential, &fake, Berlin);
    let personal = calendar(NEXTCLOUD_PERSONAL, None);
    let rename = EventDraft {
        title: Some("Yoga (advanced)".into()),
        ..EventDraft::default()
    };

    let error = client
        .update(&personal, &occurrence, &rename, Scope::Event)
        .await
        .unwrap_err();
    assert_eq!(error.code, code::NOT_SUPPORTED);
    assert!(fake.seen().is_empty(), "refused before any request");

    let renamed = client
        .update(&personal, &occurrence, &rename, Scope::Series)
        .await
        .unwrap();
    assert_eq!(renamed.title, "Yoga (advanced)");
    assert_eq!(renamed.event_id, occurrence);
    let body = fake.seen()[1].body.clone().unwrap();
    assert!(body.contains("RRULE:FREQ=WEEKLY;BYDAY=WE;UNTIL=20261230T225959Z\r\n"));
    assert!(body.contains("SUMMARY:Yoga (advanced)\r\n"));
    assert!(
        body.contains("STATUS:CANCELLED\r\n"),
        "the cancelled occurrence stays"
    );

    let moved = timed("Yoga", utc(2026, 10, 14, 17, 0), utc(2026, 10, 14, 18, 30));
    let error = client
        .update(&personal, &occurrence, &moved, Scope::Series)
        .await
        .unwrap_err();
    assert_eq!(error.code, code::NOT_SUPPORTED);
    let error = client
        .update(&personal, id, &rename, Scope::Event)
        .await
        .unwrap_err();
    assert_eq!(error.code, code::NOT_SUPPORTED);
    let error = client
        .delete(&personal, id, Scope::Event)
        .await
        .unwrap_err();
    assert_eq!(error.code, code::NOT_SUPPORTED);
    client
        .delete(&personal, &occurrence, Scope::Series)
        .await
        .unwrap();
    let seen = fake.seen();
    assert_eq!(seen.last().unwrap().method, DavMethod::Delete);
    assert_eq!(
        seen.last().unwrap().precondition,
        Precondition::IfMatch("\"s2\"".into())
    );
    assert_eq!(fake.left(), 0);
}

#[tokio::test]
async fn read_only_calendars_refuse_writes_without_a_request() {
    let fake = FakeDav::default();
    let account = account_icloud();
    let credential = credential();
    let client = DavClient::new(&account, &credential, &fake, Berlin);
    let mut family = calendar(
        "https://p52-caldav.icloud.com/1234567890/calendars/8B1D6F3E-shared/",
        None,
    );
    family.writable = false;
    let draft = timed("Visit", utc(2026, 11, 24, 9, 0), utc(2026, 11, 24, 10, 0));
    assert_eq!(
        client.create(&family, &draft).await.unwrap_err().code,
        code::SCOPE_REFUSED
    );
    assert!(fake.seen().is_empty());
}

/// A calendar transport that reaches only the fake CalDAV server.
struct DavOnly(FakeDav);

#[async_trait]
impl provider::Transport for DavOnly {
    async fn send(
        &self,
        _method: &str,
        url: Url,
        _credential: &Credential,
        _body: Option<serde_json::Value>,
    ) -> Result<RestResponse, String> {
        Err(format!("no REST call expected: {url}"))
    }

    fn dav(&self) -> &dyn DavTransport {
        &self.0
    }
}

fn caldav_entry(id: &str, chosen: bool) -> ConnectorEntry {
    ConnectorEntry {
        rule: ConnectorRule {
            id: id.into(),
            label: format!("Nextcloud {id}"),
            kind: Some(ConnectorKind::Caldav),
            default_policy: Some(ConnectorPolicy::Ask),
            ..ConnectorRule::default()
        },
        kind: ConnectorKind::Caldav,
        url: Some("https://cloud.example.com/".into()),
        provider_id: Some("caldav".into()),
        api_hosts: super::account_hosts("https://cloud.example.com/"),
        calendar: chosen.then(|| CalDavCalendarInfo {
            url: NEXTCLOUD_PERSONAL.into(),
            name: "Personal".into(),
            color: None,
            writable: true,
            zone: Some("Europe/Berlin".into()),
        }),
    }
}

#[tokio::test]
async fn caldav_connectors_are_calendars_of_the_provider_seam() {
    let source = StaticSource::new(vec![
        (caldav_entry("c-dav", true), credential()),
        (caldav_entry("c-unpicked", false), credential()),
    ]);
    let calendars = provider::calendars(&source).await.unwrap();
    assert_eq!(
        calendars.len(),
        1,
        "a connector without a chosen calendar is left out"
    );
    let calendar = &calendars[0];
    assert_eq!(calendar.provider, Provider::Caldav);
    assert_eq!(calendar.calendar_id(), NEXTCLOUD_PERSONAL);
    let transport = DavOnly(FakeDav::default().on(
        DavMethod::Report,
        NEXTCLOUD_PERSONAL,
        207,
        NEXTCLOUD_REPORT,
    ));
    let client = provider::Client::new(&source, &transport);
    let listed = client
        .list(calendar, utc(2026, 11, 23, 0, 0), utc(2026, 11, 30, 0, 0))
        .await
        .unwrap();
    assert_eq!(listed.events.len(), 4);
    let holiday = find(&listed.events, "holiday.ics");
    assert_eq!(
        holiday.starts_at,
        utc(2026, 11, 26, 23, 0),
        "whole days in the calendar's zone"
    );
    let reference = calendar.reference(&holiday.event_id);
    assert_eq!(reference.provider, "caldav");
    assert_eq!(reference.calendar_id, NEXTCLOUD_PERSONAL);
    assert!(reference.clone().check().is_ok());
    let decoded = flow_like_types::bot_calendar::ExternalEventRef::decode(&reference.encode());
    assert_eq!(decoded, Some(reference));
    assert_eq!(
        transport.0.seen()[0].authorization,
        credential().header().map(str::to_string)
    );
    let error = client
        .patch(
            calendar,
            "/remote.php/dav/calendars/felix/personal/weekly-standup.ics#20261123T080000Z",
            &EventDraft {
                title: Some("Renamed".into()),
                ..EventDraft::default()
            },
        )
        .await
        .unwrap_err();
    assert_eq!(error.code, code::NOT_SUPPORTED);
}
