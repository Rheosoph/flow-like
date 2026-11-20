//! CalDAV calendars (RFC 4791): iCloud with an app-specific password, Nextcloud and other
//! SabreDAV servers, Fastmail, mailbox.org, Posteo, Synology, Radicale. Discovery from the
//! address an owner gives, listings of a window (expanded by the server, else here), and writes
//! guarded by ETags, mapped to the calendar module's `ExternalEvent` and `EventDraft`.
//!
//! Requests go through [`DavTransport`]; [`HttpDavTransport`] applies the egress rules of
//! `connector_request`, and the client sends the sign-in only to the account's hosts. Hosts call
//! [`discover`] when the owner connects or refreshes a CalDAV connector.
// The ctag/sync-token, the series scope and the request builders are kept for incremental sync
// and whole-series edits (todo/bot-calendar/lane-g.md).
#![allow(dead_code)]

mod client;
mod discovery;
mod events;
mod ical;
mod recur;
mod transport;
mod xml;
mod zones;

#[cfg(test)]
mod tests;

use flow_like_types::bot_connector::{CalDavCalendarInfo, MAX_CALDAV_CALENDARS};

use crate::services::Credential;

pub use client::basic_credential;
pub(crate) use client::{DavAccount, DavCalendar, DavClient, Scope};
pub use transport::{
    DavBody, DavMethod, DavRequest, DavResponse, DavTransport, Depth, HttpDavTransport,
    Precondition,
};

/// The calendar as a connector's snapshot keeps it.
pub(crate) fn info(calendar: &DavCalendar) -> CalDavCalendarInfo {
    CalDavCalendarInfo {
        url: calendar.url.to_string(),
        name: calendar.name.clone(),
        color: calendar.color.clone(),
        writable: calendar.writable,
        zone: calendar.zone.map(|zone| zone.name().to_string()),
    }
}

/// The calendars with events behind the address `url`, signed in with `credential`: what a
/// CalDAV connector keeps after it is connected or refreshed. The message of an error is for the
/// owner and never carries the password.
pub async fn discover(
    label: &str,
    url: &str,
    credential: &Credential,
    transport: &dyn DavTransport,
) -> Result<Vec<CalDavCalendarInfo>, String> {
    let account = DavAccount::new(label, url, &[])?;
    let client = DavClient::new(&account, credential, transport, chrono_tz::UTC);
    let calendars = client.discover().await.map_err(|error| error.message)?;
    Ok(calendars
        .iter()
        .take(MAX_CALDAV_CALENDARS)
        .map(info)
        .collect())
}

/// The only hosts a CalDAV account's password goes to: the address's host, and iCloud's
/// partition hosts for an iCloud address. Empty for an address that is not a URL.
pub fn account_hosts(url: &str) -> Vec<String> {
    DavAccount::new("", url, &[])
        .map(|account| account.hosts)
        .unwrap_or_default()
}
