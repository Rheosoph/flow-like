// Email — FlowScript node declarations (generated, do not edit).
// One declare-function per catalog node. Names are camelCase node types.

// === Email/Access ===

/**
 * Access filename, content_type and data
 * @param attachment — Attachment struct
 * @returns filename — Attachment filename
 * @returns contentType — MIME content type
 * @returns data — Raw bytes (Vec<u8>)
 */
declare function attachmentFields({ attachment: Struct }): { filename: string, contentType: string, data: bytes[] };

/**
 * Access attachments array
 * @param email — Email struct
 * @returns attachments — Array of attachments
 */
declare function emailGetAttachments({ email: Struct }): Struct[];

/**
 * Access subject, date, plain and HTML bodies
 * @param email — Email struct
 * @returns subject — Email subject
 * @returns date — Email date
 * @returns plain — Plaintext body
 * @returns html — HTML body
 */
declare function emailGetContent({ email: Struct }): { subject: string, date: string, plain: string, html: string };

/**
 * Access address header fields of an Email
 * @param email — Email struct
 * @returns from — Primary from address
 * @returns sender — Sender addresses
 * @returns to — To addresses
 * @returns cc — Carbon copy addresses
 * @returns bcc — Blind carbon copy addresses
 */
declare function emailGetHeaders({ email: Struct }): { from: Struct, sender: Struct[], to: Struct[], cc: Struct[], bcc: Struct[] };

/**
 * Access name and email on a MailAddress
 * @param address — MailAddress struct
 * @returns name — Display name (optional)
 * @returns email — Email address
 */
declare function mailAddressFields({ address: Struct }): { name: string, email: string };

/**
 * Transforms a Mail struct into a reference
 * @param mail — Mail struct
 * @returns reference — Mail reference
 */
declare function mailImapInboxMailToReference({ mail: Struct }): Struct;


// === Email/IMAP ===

/**
 * Connects to an IMAP server and caches the session. For Gmail: use host 'imap.gmail.com', port 993, encryption 'Tls', your Gmail address as username, and an App Password (not your regular password). Generate an App Password at: https://support.google.com/mail/answer/185833
 * @param host (optional) — IMAP server hostname
 * @param port (optional) — IMAP server port
 * @param username — Email account username
 * @param password — Email account password
 * @param encryption (optional) — Connection encryption: Tls, StartTls, or Plain
 * @returns connection — Cached IMAP connection reference
 * @impure has side effects / drives control flow
 */
declare function emailImapConnect({ host?: string, port?: int, username: string, password: string, encryption?: string }): Struct;

/**
 * Copies a mail (by UID) to another IMAP mailbox
 * @param email — EmailRef containing connection, inbox, uid
 * @param destination (optional) — Target mailbox (e.g. Archive)
 * @param createIfMissing (optional) — Create the destination mailbox if it doesn't exist
 * @returns newMessageRef — Reference to the copied message
 * @impure has side effects / drives control flow
 */
declare function emailImapCopyMessage({ email: Struct, destination?: string, createIfMissing?: bool }): Struct;

/**
 * Appends a new draft message to a mailbox (defaults to 'Drafts')
 * @param connection — IMAP connection details
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
declare function emailImapCreateDraft({ connection: Struct, mailbox?: string, createIfMissing?: bool, from: string, to: string, cc?: string, bcc?: string, subject?: string, bodyText?: string, bodyHtml?: string, attachments?: Struct[], markSeen?: bool }): string;

/**
 * Deletes a mail (by UID) from its current mailbox
 * @param email — EmailRef containing connection, inbox, uid
 * @param expungeMode (optional) — How to remove after marking \Deleted
 * @impure has side effects / drives control flow
 */
declare function emailImapDeleteMessage({ email: Struct, expungeMode?: string }): void;

/**
 * Fetches the full email content
 * @param emailRef — Reference to the email (connection+uid+inbox)
 * @returns email — Parsed email metadata
 * @impure has side effects / drives control flow
 */
declare function emailImapInboxFetchMail({ emailRef: Struct }): Struct;

/**
 * Marks a mail (by UID) as seen/read in IMAP mailbox
 * @param email — EmailRef containing connection, inbox, uid
 * @param markAsSeen (optional) — True to mark as seen, false to mark as unseen
 * @returns emailRef — Reference to the marked message
 * @impure has side effects / drives control flow
 */
declare function emailImapMarkSeen({ email: Struct, markAsSeen?: bool }): Struct;

/**
 * Moves a mail (by UID) to another IMAP mailbox
 * @param email — EmailRef containing connection, inbox, uid
 * @param destination (optional) — Target mailbox (e.g. Archive)
 * @param createIfMissing (optional) — Create the destination mailbox if it doesn't exist
 * @param expungeMode (optional) — How to remove from source when MOVE is unavailable
 * @returns newMessageRef — Reference to the new message
 * @impure has side effects / drives control flow
 */
declare function emailImapMoveMessage({ email: Struct, destination?: string, createIfMissing?: bool, expungeMode?: string }): Struct;

/**
 * Creates a mailbox if it doesn't exist; no-op if it already exists
 * @param connection — Reference to an existing IMAP connection
 * @param name (optional) — Mailbox to create if missing
 * @returns created — True if created, false if it already existed
 * @returns inboxStruct — The resulting mailbox wrapped as ImapInbox
 * @impure has side effects / drives control flow
 */
declare function mailImapCreateMailbox({ connection: Struct, name?: string }): { created: bool, inboxStruct: Struct };

/**
 * Wraps an IMAP mailbox for paginated fetching
 * @param connection — Reference to an existing IMAP connection
 * @param inbox (optional) — Mailbox name to wrap
 * @returns inboxStruct — Wrapped IMAP inbox for pagination
 * @impure has side effects / drives control flow
 */
declare function mailImapInbox({ connection: Struct, inbox?: string }): Struct;

/**
 * Lists email UIDs for a mailbox page with selectable filters
 * @param inbox — Mailbox name
 * @param filter (optional) — IMAP search filter
 * @returns emails — List of email references
 * @impure has side effects / drives control flow
 */
declare function mailImapList({ inbox: Struct, filter?: string }): Struct[];

/**
 * Lists all available IMAP mailboxes
 * @param connection — Reference to an existing IMAP connection
 * @returns names — All mailbox names returned by the server
 * @returns inboxes — All mailboxes wrapped as ImapInbox
 * @impure has side effects / drives control flow
 */
declare function mailImapListInboxes({ connection: Struct }): { names: string[], inboxes: Struct[] };


// === Email/IMAP/Calendar ===

/**
 * Creates a new calendar event in an IMAP calendar folder
 * @param connection — IMAP connection
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
declare function mailImapCalendarCreateEvent({ connection: Struct, calendarFolder?: string, summary: string, description?: string, location?: string, start: Date, end: Date, attendees?: string }): { eventUid: string, errorMessage: string };

/**
 * Deletes a calendar event from an IMAP calendar folder
 * @param connection — IMAP connection
 * @param calendarFolder (optional) — Calendar folder name
 * @param eventUid — Event unique ID
 * @returns errorMessage — Error details
 * @impure has side effects / drives control flow
 */
declare function mailImapCalendarDeleteEvent({ connection: Struct, calendarFolder?: string, eventUid: string }): string;

/**
 * Gets a specific calendar event by UID
 * @param connection — IMAP connection
 * @param calendarFolder (optional) — Calendar folder name
 * @param eventUid — Event unique ID
 * @returns event — Calendar event
 * @returns errorMessage — Error details
 * @impure has side effects / drives control flow
 */
declare function mailImapCalendarGetEvent({ connection: Struct, calendarFolder?: string, eventUid: string }): { event: Struct, errorMessage: string };

/**
 * Lists mailbox names and heuristically-detected calendar folders
 * @param connection — IMAP connection
 * @returns names — All mailbox names returned by the server
 * @returns calendarNames — Mailbox names that look like calendar folders
 * @returns calendars — Detected calendar mailboxes wrapped as ImapInbox
 * @impure has side effects / drives control flow
 */
declare function mailImapCalendarList({ connection: Struct }): { names: string[], calendarNames: string[], calendars: Struct[] };

/**
 * Lists calendar events from an IMAP calendar folder
 * @param connection — IMAP connection
 * @param calendarFolder (optional) — Calendar folder name
 * @param startDate — Filter events starting from this date
 * @param endDate — Filter events ending before this date
 * @returns events — List of calendar events
 * @impure has side effects / drives control flow
 */
declare function mailImapCalendarListEvents({ connection: Struct, calendarFolder?: string, startDate: Date, endDate: Date }): Struct[];

/**
 * Fetches and parses calendar events from an iCalendar subscription URL
 * @param url — iCalendar subscription URL (.ics)
 * @param startDate — Filter events starting from this date
 * @param endDate — Filter events ending before this date
 * @returns events — List of calendar events
 * @returns calendarName — Name from X-WR-CALNAME if present
 * @returns errorMessage — Error details
 * @impure has side effects / drives control flow
 */
declare function mailImapCalendarSubscribe({ url: string, startDate: Date, endDate: Date }): { events: Struct[], calendarName: string, errorMessage: string };


// === Email/SMTP ===

/**
 * Connects to an SMTP server and caches the session. For Gmail: use host 'smtp.gmail.com', port 587, encryption 'StartTls', your Gmail address as username, and an App Password (not your regular password). Generate an App Password at: https://support.google.com/mail/answer/185833
 * @param host (optional) — SMTP server hostname
 * @param port (optional) — SMTP server port
 * @param username — Email account username
 * @param password — Email account password
 * @param encryption (optional) — Connection encryption: Tls, StartTls, or Plain
 * @returns connection — Cached SMTP connection reference
 * @impure has side effects / drives control flow
 */
declare function emailSmtpConnect({ host?: string, port?: int, username: string, password: string, encryption?: string }): Struct;

/**
 * Sends an email via SMTP using a cached connection
 * @param connection — SMTP connection handle
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
declare function emailSmtpSend({ connection: Struct, from: string, to: string, cc?: string, bcc?: string, subject?: string, bodyText?: string, bodyHtml?: string, attachments?: Struct[], includeBccHeader?: bool }): string;

