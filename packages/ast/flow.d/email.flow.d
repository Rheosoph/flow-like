// Email — FlowScript node declarations (generated, do not edit).
// One `function` per catalog node, grouped by FlowScript namespace. Call a node as
// `ns::alias({ pin: value })`, or write `use ns::*` once at the top of a .flow file and
// call `alias({ pin: value })`. A `this: T` parameter marks the receiver pin: such a node
// is also a method on that value (`x.alias(...)`, remaining inputs positional or named).
// JSDoc tags carry the node type (`@node`), the receiver pin (`@receiver`) and the legacy
// camelCase spelling (`@alias`), which is still accepted.

declare namespace email {
    // === Email/Access ===

    /**
     * Access name and email on a MailAddress
     * @node mail_address_fields @alias mailAddressFields
     * @param address — MailAddress struct
     * @returns name — Display name (optional)
     * @returns email — Email address
     */
    function addressToFields({ address: Struct }): { name: string, email: string };

    /**
     * Access filename, content_type and data
     * @node attachment_fields @alias attachmentFields
     * @param attachment — Attachment struct
     * @returns filename — Attachment filename
     * @returns contentType — MIME content type
     * @returns data — Raw bytes (Vec<u8>)
     */
    function attachmentToFields({ attachment: Struct }): { filename: string, contentType: string, data: bytes[] };

    /**
     * Access attachments array
     * @node email_get_attachments @receiver email @alias emailGetAttachments
     * @param email — Email struct (receiver: `this` in `x.getAttachments(...)`)
     * @returns attachments — Array of attachments
     */
    function getAttachments(this: Email, { email: Struct }): Struct[];

    /**
     * Access subject, date, plain and HTML bodies
     * @node email_get_content @receiver email @alias emailGetContent
     * @param email — Email struct (receiver: `this` in `x.getContent(...)`)
     * @returns subject — Email subject
     * @returns date — Email date
     * @returns plain — Plaintext body
     * @returns html — HTML body
     */
    function getContent(this: Email, { email: Struct }): { subject: string, date: string, plain: string, html: string };

    /**
     * Access address header fields of an Email
     * @node email_get_headers @receiver email @alias emailGetHeaders
     * @param email — Email struct (receiver: `this` in `x.getHeaders(...)`)
     * @returns from — Primary from address
     * @returns sender — Sender addresses
     * @returns to — To addresses
     * @returns cc — Carbon copy addresses
     * @returns bcc — Blind carbon copy addresses
     */
    function getHeaders(this: Email, { email: Struct }): { from: Struct, sender: Struct[], to: Struct[], cc: Struct[], bcc: Struct[] };

    /**
     * Transforms a Mail struct into a reference
     * @node mail_imap_inbox_mail_to_reference @receiver mail @alias mailImapInboxMailToReference
     * @param mail — Mail struct (receiver: `this` in `x.toReference(...)`)
     * @returns reference — Mail reference
     */
    function toReference(this: Email, { mail: Struct }): Struct;
}

declare namespace imap {
    // === Email/IMAP ===

    /**
     * Connects to an IMAP server and caches the session. For Gmail: use host 'imap.gmail.com', port 993, encryption 'Tls', your Gmail address as username, and an App Password (not your regular password). Generate an App Password at: https://support.google.com/mail/answer/185833
     * @node email_imap_connect @alias emailImapConnect
     * @param host (optional) — IMAP server hostname
     * @param port (optional) — IMAP server port
     * @param username — Email account username
     * @param password — Email account password
     * @param encryption (optional) — Connection encryption: Tls, StartTls, or Plain
     * @returns connection — Cached IMAP connection reference
     * @impure has side effects / drives control flow
     */
    function connect({ host?: string, port?: int, username: string, password: string, encryption?: string }): Struct;

    /**
     * Copies a mail (by UID) to another IMAP mailbox
     * @node email_imap_copy_message @receiver email @alias emailImapCopyMessage
     * @param email — EmailRef containing connection, inbox, uid (receiver: `this` in `x.copyMessage(...)`)
     * @param destination (optional) — Target mailbox (e.g. Archive)
     * @param createIfMissing (optional) — Create the destination mailbox if it doesn't exist
     * @returns newMessageRef — Reference to the copied message
     * @impure has side effects / drives control flow
     */
    function copyMessage(this: EmailRef, { email: Struct, destination?: string, createIfMissing?: bool }): Struct;

    /**
     * Appends a new draft message to a mailbox (defaults to 'Drafts')
     * @node email_imap_create_draft @receiver connection @alias emailImapCreateDraft
     * @param connection — IMAP connection details (receiver: `this` in `x.createDraft(...)`)
     * @param mailbox (optional) — Where to store the draft
     * @param createIfMissing (optional) — Create the destination mailbox if it doesn't exist
     * @param from — From header
     * @param to — Comma-separated list
     * @param cc (optional) — Comma-separated list
     * @param bcc (optional) — Comma-separated list
     * @param subject (optional) — Subject line
     * @param bodyText (optional) — Plaintext body
     * @param bodyHtml (optional) — Optional HTML body
     * @param attachments (optional) — Files to attach
     * @param markSeen (optional) — Save draft with \Seen flag in addition to \Draft
     * @returns messageId — The generated Message-ID
     * @impure has side effects / drives control flow
     */
    function createDraft(this: ImapConnection, { connection: Struct, mailbox?: string, createIfMissing?: bool, from: string, to: string, cc?: string, bcc?: string, subject?: string, bodyText?: string, bodyHtml?: string, attachments?: Struct[], markSeen?: bool }): string;

    /**
     * Creates a mailbox if it doesn't exist; no-op if it already exists
     * @node mail_imap_create_mailbox @receiver connection @alias mailImapCreateMailbox
     * @param connection — Reference to an existing IMAP connection (receiver: `this` in `x.createMailbox(...)`)
     * @param name (optional) — Mailbox to create if missing
     * @returns created — True if created, false if it already existed
     * @returns inboxStruct — The resulting mailbox wrapped as ImapInbox
     * @impure has side effects / drives control flow
     */
    function createMailbox(this: ImapConnection, { connection: Struct, name?: string }): { created: bool, inboxStruct: Struct };

    /**
     * Deletes a mail (by UID) from its current mailbox
     * @node email_imap_delete_message @receiver email @alias emailImapDeleteMessage
     * @param email — EmailRef containing connection, inbox, uid (receiver: `this` in `x.deleteMessage(...)`)
     * @param expungeMode (optional) — How to remove after marking \Deleted
     * @impure has side effects / drives control flow
     */
    function deleteMessage(this: EmailRef, { email: Struct, expungeMode?: string }): void;

    /**
     * Fetches the full email content
     * @node email_imap_inbox_fetch_mail @receiver email_ref @alias emailImapInboxFetchMail
     * @param emailRef — Reference to the email (connection+uid+inbox) (receiver: `this` in `x.fetchMail(...)`)
     * @returns email — Parsed email metadata
     * @impure has side effects / drives control flow
     */
    function fetchMail(this: EmailRef, { emailRef: Struct }): Struct;

    /**
     * Wraps an IMAP mailbox for paginated fetching
     * @node mail_imap_inbox @receiver connection @alias mailImapInbox
     * @param connection — Reference to an existing IMAP connection (receiver: `this` in `x.inbox(...)`)
     * @param inbox (optional) — Mailbox name to wrap
     * @returns inboxStruct — Wrapped IMAP inbox for pagination
     * @impure has side effects / drives control flow
     */
    function inbox(this: ImapConnection, { connection: Struct, inbox?: string }): Struct;

    /**
     * Lists all available IMAP mailboxes
     * @node mail_imap_list_inboxes @receiver connection @alias mailImapListInboxes
     * @param connection — Reference to an existing IMAP connection (receiver: `this` in `x.listInboxes(...)`)
     * @returns names — All mailbox names returned by the server
     * @returns inboxes — All mailboxes wrapped as ImapInbox
     * @impure has side effects / drives control flow
     */
    function listInboxes(this: ImapConnection, { connection: Struct }): { names: string[], inboxes: Struct[] };

    /**
     * Lists email UIDs for a mailbox page with selectable filters
     * @node mail_imap_list @receiver inbox @alias mailImapList
     * @param inbox — Mailbox name (receiver: `this` in `x.listMails(...)`)
     * @param filter (optional) — IMAP search filter
     * @returns emails — List of email references
     * @impure has side effects / drives control flow
     */
    function listMails(this: ImapInbox, { inbox: Struct, filter?: string }): Struct[];

    /**
     * Marks a mail (by UID) as seen/read in IMAP mailbox
     * @node email_imap_mark_seen @receiver email @alias emailImapMarkSeen
     * @param email — EmailRef containing connection, inbox, uid (receiver: `this` in `x.markSeen(...)`)
     * @param markAsSeen (optional) — True to mark as seen, false to mark as unseen
     * @returns emailRef — Reference to the marked message
     * @impure has side effects / drives control flow
     */
    function markSeen(this: EmailRef, { email: Struct, markAsSeen?: bool }): Struct;

    /**
     * Moves a mail (by UID) to another IMAP mailbox
     * @node email_imap_move_message @receiver email @alias emailImapMoveMessage
     * @param email — EmailRef containing connection, inbox, uid (receiver: `this` in `x.moveMessage(...)`)
     * @param destination (optional) — Target mailbox (e.g. Archive)
     * @param createIfMissing (optional) — Create the destination mailbox if it doesn't exist
     * @param expungeMode (optional) — How to remove from source when MOVE is unavailable
     * @returns newMessageRef — Reference to the new message
     * @impure has side effects / drives control flow
     */
    function moveMessage(this: EmailRef, { email: Struct, destination?: string, createIfMissing?: bool, expungeMode?: string }): Struct;

    // === Email/IMAP/Calendar ===

    /**
     * Creates a new calendar event in an IMAP calendar folder
     * @node mail_imap_calendar_create_event @receiver connection @alias mailImapCalendarCreateEvent
     * @param connection — IMAP connection (receiver: `this` in `x.createCalendarEvent(...)`)
     * @param calendarFolder (optional) — Calendar folder name
     * @param summary — Event title
     * @param description (optional) — Event description
     * @param location (optional) — Event location
     * @param start — Start date/time
     * @param end — End date/time
     * @param attendees (optional) — Comma-separated email addresses
     * @returns eventUid — Created event UID
     * @returns errorMessage — Error details
     * @impure has side effects / drives control flow
     */
    function createCalendarEvent(this: ImapConnection, { connection: Struct, calendarFolder?: string, summary: string, description?: string, location?: string, start: Date, end: Date, attendees?: string }): { eventUid: string, errorMessage: string };

    /**
     * Deletes a calendar event from an IMAP calendar folder
     * @node mail_imap_calendar_delete_event @receiver connection @alias mailImapCalendarDeleteEvent
     * @param connection — IMAP connection (receiver: `this` in `x.deleteCalendarEvent(...)`)
     * @param calendarFolder (optional) — Calendar folder name
     * @param eventUid — Event unique ID
     * @returns errorMessage — Error details
     * @impure has side effects / drives control flow
     */
    function deleteCalendarEvent(this: ImapConnection, { connection: Struct, calendarFolder?: string, eventUid: string }): string;

    /**
     * Gets a specific calendar event by UID
     * @node mail_imap_calendar_get_event @receiver connection @alias mailImapCalendarGetEvent
     * @param connection — IMAP connection (receiver: `this` in `x.getCalendarEvent(...)`)
     * @param calendarFolder (optional) — Calendar folder name
     * @param eventUid — Event unique ID
     * @returns event — Calendar event
     * @returns errorMessage — Error details
     * @impure has side effects / drives control flow
     */
    function getCalendarEvent(this: ImapConnection, { connection: Struct, calendarFolder?: string, eventUid: string }): { event: Struct, errorMessage: string };

    /**
     * Lists calendar events from an IMAP calendar folder
     * @node mail_imap_calendar_list_events @receiver connection @alias mailImapCalendarListEvents
     * @param connection — IMAP connection (receiver: `this` in `x.listCalendarEvents(...)`)
     * @param calendarFolder (optional) — Calendar folder name
     * @param startDate — Filter events starting from this date
     * @param endDate — Filter events ending before this date
     * @returns events — List of calendar events
     * @impure has side effects / drives control flow
     */
    function listCalendarEvents(this: ImapConnection, { connection: Struct, calendarFolder?: string, startDate: Date, endDate: Date }): Struct[];

    /**
     * Lists mailbox names and heuristically-detected calendar folders
     * @node mail_imap_calendar_list @receiver connection @alias mailImapCalendarList
     * @param connection — IMAP connection (receiver: `this` in `x.listCalendars(...)`)
     * @returns names — All mailbox names returned by the server
     * @returns calendarNames — Mailbox names that look like calendar folders
     * @returns calendars — Detected calendar mailboxes wrapped as ImapInbox
     * @impure has side effects / drives control flow
     */
    function listCalendars(this: ImapConnection, { connection: Struct }): { names: string[], calendarNames: string[], calendars: Struct[] };

    /**
     * Fetches and parses calendar events from an iCalendar subscription URL
     * @node mail_imap_calendar_subscribe @alias mailImapCalendarSubscribe
     * @param url — iCalendar subscription URL (.ics)
     * @param startDate — Filter events starting from this date
     * @param endDate — Filter events ending before this date
     * @returns events — List of calendar events
     * @returns calendarName — Name from X-WR-CALNAME if present
     * @returns errorMessage — Error details
     * @impure has side effects / drives control flow
     */
    function subscribeCalendar({ url: string, startDate: Date, endDate: Date }): { events: Struct[], calendarName: string, errorMessage: string };
}

declare namespace smtp {
    // === Email/SMTP ===

    /**
     * Connects to an SMTP server and caches the session. For Gmail: use host 'smtp.gmail.com', port 587, encryption 'StartTls', your Gmail address as username, and an App Password (not your regular password). Generate an App Password at: https://support.google.com/mail/answer/185833
     * @node email_smtp_connect @alias emailSmtpConnect
     * @param host (optional) — SMTP server hostname
     * @param port (optional) — SMTP server port
     * @param username — Email account username
     * @param password — Email account password
     * @param encryption (optional) — Connection encryption: Tls, StartTls, or Plain
     * @returns connection — Cached SMTP connection reference
     * @impure has side effects / drives control flow
     */
    function connect({ host?: string, port?: int, username: string, password: string, encryption?: string }): Struct;

    /**
     * Sends an email via SMTP using a cached connection
     * @node email_smtp_send @receiver connection @alias emailSmtpSend
     * @param connection — SMTP connection handle (receiver: `this` in `x.send(...)`)
     * @param from — From header (single address)
     * @param to — Comma-separated list
     * @param cc (optional) — Comma-separated list
     * @param bcc (optional) — Comma-separated list
     * @param subject (optional) — Subject line
     * @param bodyText (optional) — Plaintext body
     * @param bodyHtml (optional) — Optional HTML body
     * @param attachments (optional) — Files to attach
     * @param includeBccHeader (optional) — If true, the Bcc header is included in the message; otherwise it's omitted (recipients still receive).
     * @returns messageId — The generated Message-ID
     * @impure has side effects / drives control flow
     */
    function send(this: SmtpConnection, { connection: Struct, from: string, to: string, cc?: string, bcc?: string, subject?: string, bodyText?: string, bodyHtml?: string, attachments?: Struct[], includeBccHeader?: bool }): string;
}
