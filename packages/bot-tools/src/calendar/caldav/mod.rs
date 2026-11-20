//! CalDAV calendars (RFC 4791): iCloud with an app-specific password, Nextcloud and other
//! SabreDAV servers, Fastmail, mailbox.org, Posteo, Synology, Radicale. Discovery from the
//! address an owner gives, listings of a window (expanded by the server, else here), and writes
//! guarded by ETags, mapped to the calendar module's [`ExternalEvent`](super::provider::ExternalEvent)
//! and [`EventDraft`](super::provider::EventDraft).
//!
//! Requests go through `transport::DavTransport`; `HttpDavTransport` applies the egress rules of
//! `connector_request`, and `client::DavClient` sends the sign-in only to the account's hosts.
// Phase 1 of the CalDAV connector: the provider seam that calls this comes in phase 2
// (todo/bot-calendar/lane-g.md).
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
