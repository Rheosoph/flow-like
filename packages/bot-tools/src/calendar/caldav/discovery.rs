//! From the address an owner gives to the calendars behind it (RFC 6764, RFC 4791 §6): the
//! address itself or `/.well-known/caldav` (redirects followed only to the account's hosts),
//! `current-user-principal`, `calendar-home-set`, then the calendars of each home that hold
//! events. iCloud answers its home on a partition host (`p52-caldav.icloud.com`).

use flow_like_types::reqwest::Url;

use super::client::{DISCOVER, DavCalendar, DavClient, collection, decoded, http_status};
use super::ical;
use super::transport::{DavBody, DavMethod, DavRequest, Depth};
use super::xml::{
    self, APPLE, CALDAV, CALENDARS_QUERY, CALENDARSERVER, DAV, Entry, PRINCIPAL_QUERY,
};
use super::zones::{Zone, Zones};
use crate::error::{ToolError, ToolResult, code};

const MAX_REDIRECTS: usize = 5;
const MAX_HOMES: usize = 3;
const WELL_KNOWN: &str = "/.well-known/caldav";

/// What one Depth 0 PROPFIND on an address reported.
#[derive(Debug, Default)]
struct Found {
    url: Option<Url>,
    principal: Option<Url>,
    homes: Vec<Url>,
    calendar: bool,
}

impl Found {
    fn leads_somewhere(&self) -> bool {
        self.calendar || self.principal.is_some() || !self.homes.is_empty()
    }
}

fn same_path(left: &Url, right: &Url) -> bool {
    decoded(left.path()).trim_end_matches('/') == decoded(right.path()).trim_end_matches('/')
}

fn hrefs(base: &Url, entry: &Entry, ns: &str, name: &str) -> Vec<Url> {
    entry
        .prop(ns, name)
        .map(|prop| {
            prop.hrefs
                .iter()
                .filter_map(|href| base.join(href).ok())
                .collect()
        })
        .unwrap_or_default()
}

/// The zone a `calendar-timezone` VTIMEZONE names, when it is an IANA zone.
fn calendar_zone(text: &str) -> Option<chrono_tz::Tz> {
    let calendar = ical::parse(text).ok()?;
    let zones = Zones::of(&calendar);
    let tzid = calendar
        .children_named("VTIMEZONE")
        .find_map(|timezone| timezone.property("TZID"))?;
    match zones.resolve(&tzid.value)? {
        Zone::Iana(zone) => Some(zone),
        Zone::Fixed(_) => None,
    }
}

impl DavClient<'_> {
    /// A PROPFIND that follows redirects within the account's hosts. Answers the final URL and
    /// the multistatus responses.
    async fn propfind(
        &self,
        mut url: Url,
        depth: Depth,
        body: &str,
    ) -> ToolResult<(Url, Vec<Entry>)> {
        for _ in 0..=MAX_REDIRECTS {
            let request = DavRequest::new(DavMethod::Propfind, url.clone())
                .depth(depth)
                .body(DavBody::Xml(body.to_string()));
            let response = self.send(DISCOVER, request).await?;
            match response.status {
                301 | 302 | 303 | 307 | 308 => {
                    let location = response.location.as_deref().unwrap_or_default();
                    url = url.join(location).map_err(|error| {
                        ToolError::new(
                            code::FAILED,
                            format!(
                                "'{}' redirected to an invalid address '{location}': {error}",
                                self.label()
                            ),
                        )
                    })?;
                }
                200 | 207 => {
                    let entries = xml::multistatus(&response.text).map_err(|error| {
                        ToolError::new(
                            code::FAILED,
                            format!(
                                "'{}' is not a CalDAV server at {}: {error}",
                                self.label(),
                                url.host_str().unwrap_or_default()
                            ),
                        )
                    })?;
                    return Ok((url, entries));
                }
                _ => return Err(self.refusal(DISCOVER, &url, &response)),
            }
        }
        Err(ToolError::new(
            code::FAILED,
            format!(
                "'{}' redirected more than {MAX_REDIRECTS} times while looking for calendars",
                self.label()
            ),
        ))
    }

    async fn probe(&self, url: Url) -> ToolResult<Found> {
        let (url, entries) = self.propfind(url, Depth::Zero, PRINCIPAL_QUERY).await?;
        let Some(entry) = entries
            .iter()
            .find(|entry| {
                url.join(&entry.href)
                    .is_ok_and(|href| same_path(&href, &url))
            })
            .or(entries.first())
        else {
            return Ok(Found::default());
        };
        Ok(Found {
            principal: hrefs(&url, entry, DAV, "current-user-principal")
                .into_iter()
                .next(),
            homes: hrefs(&url, entry, CALDAV, "calendar-home-set"),
            calendar: entry
                .prop(DAV, "resourcetype")
                .is_some_and(|prop| prop.has(CALDAV, "calendar")),
            url: Some(url),
        })
    }

    /// The first address of the account that leads to calendars: the given one, else
    /// `/.well-known/caldav` on its host.
    async fn locate(&self) -> ToolResult<Found> {
        let start = self.account.url.clone();
        let first = self.probe(start.clone()).await;
        let settled = match &first {
            Ok(found) => found.leads_somewhere(),
            Err(error) => matches!(http_status(error), Some(401 | 403 | 429 | 503)),
        };
        if settled {
            return first;
        }
        if start.path() != WELL_KNOWN
            && let Ok(well_known) = start.join(WELL_KNOWN)
            && let Ok(found) = self.probe(well_known).await
            && found.leads_somewhere()
        {
            return Ok(found);
        }
        first?;
        Err(ToolError::not_found(format!(
            "No CalDAV calendars found at {}; use the CalDAV address from your provider's settings",
            start.host_str().unwrap_or_default()
        )))
    }

    fn calendar_of(&self, base: &Url, entry: &Entry) -> Option<DavCalendar> {
        let is_calendar = entry
            .prop(DAV, "resourcetype")
            .is_some_and(|prop| prop.has(CALDAV, "calendar"));
        let holds_events = entry
            .prop(CALDAV, "supported-calendar-component-set")
            .is_none_or(|prop| {
                prop.elements.iter().any(|element| {
                    element.ns == CALDAV
                        && element.name == "comp"
                        && element
                            .name_attr
                            .as_deref()
                            .is_some_and(|name| name.eq_ignore_ascii_case("VEVENT"))
                })
            });
        if !is_calendar || !holds_events {
            return None;
        }
        let url = collection(base.join(&entry.href).ok()?);
        self.account.permits(&url).ok()?;
        let fallback = || {
            decoded(url.path())
                .trim_end_matches('/')
                .rsplit('/')
                .next()
                .unwrap_or_default()
                .to_string()
        };
        let text = |ns: &str, name: &str| {
            entry
                .prop(ns, name)
                .and_then(|prop| prop.trimmed())
                .map(str::to_string)
        };
        let writable = entry
            .prop(DAV, "current-user-privilege-set")
            .is_none_or(|prop| {
                ["write", "write-content", "bind", "all"]
                    .iter()
                    .any(|privilege| prop.has(DAV, privilege))
            });
        Some(DavCalendar {
            name: text(DAV, "displayname").unwrap_or_else(fallback),
            color: text(APPLE, "calendar-color"),
            ctag: text(CALENDARSERVER, "getctag"),
            sync_token: text(DAV, "sync-token"),
            writable,
            zone: text(CALDAV, "calendar-timezone").and_then(|zone| calendar_zone(&zone)),
            url,
        })
    }

    async fn calendars_at(&self, url: Url, depth: Depth) -> ToolResult<Vec<DavCalendar>> {
        let (url, entries) = self.propfind(url, depth, CALENDARS_QUERY).await?;
        Ok(entries
            .iter()
            .filter_map(|entry| self.calendar_of(&url, entry))
            .collect())
    }

    /// The calendars of the account that hold events, in the server's order. An address of one
    /// calendar gives that calendar.
    pub async fn discover(&self) -> ToolResult<Vec<DavCalendar>> {
        let found = self.locate().await?;
        let at = found
            .url
            .clone()
            .unwrap_or_else(|| self.account.url.clone());
        if found.calendar {
            return self.calendars_at(at, Depth::Zero).await;
        }
        let mut homes = found.homes;
        if homes.is_empty()
            && let Some(principal) = found.principal
        {
            homes = self.probe(principal).await?.homes;
        }
        if homes.is_empty() {
            return Err(ToolError::not_found(format!(
                "'{}' did not say where the calendars are (no calendar-home-set at {})",
                self.label(),
                at.host_str().unwrap_or_default()
            )));
        }
        let mut calendars: Vec<DavCalendar> = Vec::new();
        for home in homes.into_iter().take(MAX_HOMES) {
            for calendar in self.calendars_at(home, Depth::One).await? {
                if !calendars.iter().any(|known| known.url == calendar.url) {
                    calendars.push(calendar);
                }
            }
        }
        Ok(calendars)
    }
}
