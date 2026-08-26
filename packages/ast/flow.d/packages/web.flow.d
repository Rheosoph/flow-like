// web — FlowScript node declarations (generated, do not edit).
// One `function` per catalog node, grouped by FlowScript namespace. Call a node as
// `ns::alias({ pin: value })`, or write `use ns::*` once at the top of a .flow file and
// call `alias({ pin: value })`. A `this: T` parameter marks the receiver pin: such a node
// is also a method on that value (`x.alias(...)`, remaining inputs positional or named).
// JSDoc tags carry the node type (`@node`), the receiver pin (`@receiver`) and the legacy
// camelCase spelling (`@alias`), which is still accepted.

declare namespace auth {
    // === Web/Auth ===

    /**
     * Creates REST auth that requires a configured API key header.
     * @node api_key_auth @alias apiKeyAuth
     * @param header (optional) — Header that carries the API key
     * @param key — Expected API key
     * @returns auth — API key auth config
     */
    function apiKey({ header?: string, key: string }): Struct;

    /**
     * Creates REST auth that requires HTTP Basic credentials.
     * @node basic_auth @alias basicAuth
     * @param username — Expected username
     * @param password — Expected password
     * @returns auth — Basic auth config
     */
    function basic({ username: string, password: string }): Struct;

    /**
     * Creates REST auth that requires a static Authorization bearer token.
     * @node bearer_token_auth @alias bearerTokenAuth
     * @param token — Expected bearer token
     * @returns auth — Bearer token auth config
     */
    function bearer({ token: string }): Struct;

    /**
     * Creates REST auth that verifies an HMAC-SHA256 request signature.
     * @node hmac_sha256_auth @alias hmacSha256Auth
     * @param secret — Shared HMAC secret
     * @param signatureHeader (optional) — Header that carries the lowercase hex HMAC signature
     * @param timestampHeader (optional) — Header that carries the Unix timestamp in seconds
     * @param maxSkewSeconds (optional) — Allowed timestamp skew in seconds; zero disables timestamp freshness checks
     * @returns auth — HMAC auth config
     */
    function hmacSha256({ secret: string, signatureHeader?: string, timestampHeader?: string, maxSkewSeconds?: int }): Struct;

    /**
     * Creates OAuth bearer auth from a JWKS JSON FlowPath loaded when the server starts.
     * @node oauth_jwks_file_auth @alias oauthJwksFileAuth
     * @param jwksFlowPath — JWKS JSON file FlowPath
     * @param issuer — Required token issuer. Empty disables issuer validation.
     * @param audience — Required token audience. Empty disables audience validation.
     * @param requiredScopes — Scopes that must be present in the token scope/scp claims.
     * @returns auth — OAuth auth config
     */
    function oauthJwksFile({ jwksFlowPath: Struct, issuer: string, audience: string, requiredScopes: string[] }): Struct;

    /**
     * Creates OAuth bearer auth that fetches a JWKS endpoint once when the server starts.
     * @node oauth_jwks_url_auth @alias oauthJwksUrlAuth
     * @param jwksUrl — JWKS endpoint URL
     * @param issuer — Required token issuer. Empty disables issuer validation.
     * @param audience — Required token audience. Empty disables audience validation.
     * @param requiredScopes — Scopes that must be present in the token scope/scp claims.
     * @returns auth — OAuth auth config
     */
    function oauthJwksUrl({ jwksUrl: string, issuer: string, audience: string, requiredScopes: string[] }): Struct;

    /**
     * Creates OAuth bearer auth by discovering the JWKS URI from an OpenID Connect issuer.
     * @node oidc_discovery_auth @alias oidcDiscoveryAuth
     * @param issuer — OIDC issuer URL. The server fetches /.well-known/openid-configuration.
     * @param audience — Required token audience. Empty disables audience validation.
     * @param requiredScopes — Scopes that must be present in the token scope/scp claims.
     * @returns auth — OIDC auth config
     */
    function oidcDiscovery({ issuer: string, audience: string, requiredScopes: string[] }): Struct;
}

declare namespace camera {
    // === Web/Camera ===

    /**
     * Captures a frame from an IP camera
     * @node web_camera_grab_frame @alias webCameraGrabFrame
     * @param request — The HTTP request to perform
     * @returns image — The captured image frame
     * @impure has side effects / drives control flow
     */
    function grabFrame({ request: Struct }): Struct;

    /**
     * Captures one frame from an RTSP camera stream
     * @node web_camera_grab_rtsp_frame @alias webCameraGrabRtspFrame
     * @param rtspUrl — RTSP or RTSPS stream URL
     * @param transport (optional) — RTSP RTP transport protocol
     * @param timeoutMs (optional) — Maximum time in milliseconds to connect and decode a frame
     * @param maxFrames (optional) — Maximum video frames to inspect before failing
     * @returns image — The captured RTSP frame
     * @returns errorMessage — Readable capture error
     * @impure has side effects / drives control flow
     */
    function grabRtspFrame({ rtspUrl: string, transport?: string, timeoutMs?: int, maxFrames?: int }): { image: Struct, errorMessage: string };
}

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

declare namespace http {
    // === Web ===

    /**
     * Downloads a file from a url
     * @node http_download @receiver request @alias httpDownload
     * @param request — The HTTP request to perform (receiver: `this` in `x.download(...)`)
     * @param flowPath — The path to save the file to
     * @impure has side effects / drives control flow
     */
    function download(this: HttpRequest, { request: Struct, flowPath: Struct }): void;

    // === Web/API ===

    /**
     * Performs an HTTP request
     * @node http_fetch @receiver request @alias httpFetch
     * @param request — The HTTP request to perform (receiver: `this` in `x.fetch(...)`)
     * @returns response — The HTTP response
     * @impure has side effects / drives control flow
     */
    function fetch(this: HttpRequest, { request: Struct }): Struct;

    /**
     * Performs an HTTP request
     * @node streaming_http_fetch @receiver request @alias streamingHttpFetch
     * @param request — The HTTP request to perform (receiver: `this` in `x.fetchStreaming(...)`)
     * @returns streamingResponse — The HTTP response
     * @returns response — The HTTP response
     * @impure has side effects / drives control flow
     */
    function fetchStreaming(this: HttpRequest, { request: Struct }): { streamingResponse: bytes[], response: Struct };

    // === Web/API/Request ===

    /**
     * Gets a header from a http request
     * @node http_get_header @receiver request @alias httpGetHeader
     * @param request — The http request (receiver: `this` in `x.getHeader(...)`)
     * @param header — The header to get
     * @returns found — True if the header was found
     * @returns value — The value of the header
     */
    function getHeader(this: HttpRequest, { request: Struct, header: string }): { found: bool, value: string };

    /**
     * Gets all headers from a http request
     * @node http_get_headers @receiver request @alias httpGetHeaders
     * @param request — The http request (receiver: `this` in `x.getHeaders(...)`)
     * @returns headers — The headers of the request
     */
    function getHeaders(this: HttpRequest, { request: Struct }): Map<string, string>;

    /**
     * Gets the method from a http request
     * @node http_get_method @receiver request @alias httpGetMethod
     * @param request — The http request (receiver: `this` in `x.getMethod(...)`)
     * @returns method — The method of the request
     */
    function getMethod(this: HttpRequest, { request: Struct }): string;

    /**
     * Gets the url from a http request
     * @node http_get_url @receiver request @alias httpGetUrl
     * @param request — The http request (receiver: `this` in `x.getUrl(...)`)
     * @returns url — The url of the request
     */
    function getUrl(this: HttpRequest, { request: Struct }): string;

    /**
     * Creates a http request
     * @node http_make_request @alias httpMakeRequest
     * @param method (optional) — Http Method GET,POST etc.
     * @param url — The request URL
     * @returns request — The http request
     */
    function request({ method?: string, url: string }): Struct;

    /**
     * Sets the Accept header of a http request
     * @node http_set_accept @receiver request @alias httpSetAccept
     * @param request — The http request (receiver: `this` in `x.setAccept(...)`)
     * @param accept (optional) — The accept header value
     * @returns requestOut — The http request
     */
    function setAccept(this: HttpRequest, { request: Struct, accept?: string }): Struct;

    /**
     * Sets the Authorization header using a Bearer token
     * @node http_set_bearer_auth @receiver request @alias httpSetBearerAuth
     * @param request — The http request (receiver: `this` in `x.setBearerAuth(...)`)
     * @param token — Bearer token
     * @returns requestOut — The http request
     */
    function setBearerAuth(this: HttpRequest, { request: Struct, token: string }): Struct;

    /**
     * Sets the body of a http request
     * @node http_set_bytes_body @receiver request @alias httpSetBytesBody
     * @param request — The http request (receiver: `this` in `x.setBytesBody(...)`)
     * @param body — The body of the request
     * @returns requestOut — The http request
     */
    function setBytesBody(this: HttpRequest, { request: Struct, body: bytes[] }): Struct;

    /**
     * Sets the Content-Type header of a http request
     * @node http_set_content_type @receiver request @alias httpSetContentType
     * @param request — The http request (receiver: `this` in `x.setContentType(...)`)
     * @param contentType (optional) — The content type value
     * @returns requestOut — The http request
     */
    function setContentType(this: HttpRequest, { request: Struct, contentType?: string }): Struct;

    /**
     * Sets the body of a http request to form-encoded data
     * @node http_set_form_body @receiver request @alias httpSetFormBody
     * @param request — The http request (receiver: `this` in `x.setFormBody(...)`)
     * @param fields (optional) — Form fields to encode
     * @param setContentType (optional) — Adds application/x-www-form-urlencoded when missing
     * @returns requestOut — The http request
     */
    function setFormBody(this: HttpRequest, { request: Struct, fields?: Struct, setContentType?: bool }): Struct;

    /**
     * Sets a header of a http request
     * @node http_set_header @receiver request @alias httpSetHeader
     * @param request — The http request (receiver: `this` in `x.setHeader(...)`)
     * @param name — The name of the header
     * @param value — The value of the header
     * @returns requestOut — The http request
     */
    function setHeader(this: HttpRequest, { request: Struct, name: string, value: string }): Struct;

    /**
     * Sets the headers of a http request
     * @node http_set_headers @receiver request @alias httpSetHeaders
     * @param request — The http request (receiver: `this` in `x.setHeaders(...)`)
     * @param headers — The headers of the request
     * @param merge (optional) — Merge with existing headers instead of replacing
     * @returns requestOut — The http request
     */
    function setHeaders(this: HttpRequest, { request: Struct, headers: Map<string, string>, merge?: bool }): Struct;

    /**
     * Sets the method of a http request
     * @node http_set_method @receiver request @alias httpSetMethod
     * @param request — The http request (receiver: `this` in `x.setMethod(...)`)
     * @param method (optional) — The method of the request
     * @returns requestOut — The http request
     */
    function setMethod(this: HttpRequest, { request: Struct, method?: string }): Struct;

    /**
     * Sets the body of a http request
     * @node http_set_string_body @receiver request @alias httpSetStringBody
     * @param request — The http request (receiver: `this` in `x.setStringBody(...)`)
     * @param body — The body of the request
     * @returns requestOut — The http request
     */
    function setStringBody(this: HttpRequest, { request: Struct, body: string }): Struct;

    /**
     * Sets the body of a http request
     * @node http_set_struct_body @receiver request @alias httpSetStructBody
     * @param request — The http request (receiver: `this` in `x.setStructBody(...)`)
     * @param body — The body of the request
     * @returns requestOut — The http request
     */
    function setStructBody(this: HttpRequest, { request: Struct, body: Struct }): Struct;

    /**
     * Sets the url of a http request
     * @node http_set_url @receiver request @alias httpSetUrl
     * @param request — The http request (receiver: `this` in `x.setUrl(...)`)
     * @param url — The url of the request
     * @returns requestOut — The http request
     */
    function setUrl(this: HttpRequest, { request: Struct, url: string }): Struct;

    // === Web/API/Response ===

    /**
     * Gets a header from a http request
     * @node http_response_get_header @receiver response @alias httpResponseGetHeader
     * @param response — The http response (receiver: `this` in `x.header(...)`)
     * @param header — The header to get
     * @returns found — True if the header was found
     * @returns value — The value of the header
     */
    function header(this: HttpResponse, { response: Struct, header: string }): { found: bool, value: string };

    /**
     * Gets all headers from a http request
     * @node http_response_get_headers @receiver response @alias httpResponseGetHeaders
     * @param response — The http response (receiver: `this` in `x.headers(...)`)
     * @returns headers — The headers of the response
     */
    function headers(this: HttpResponse, { response: Struct }): Map<string, string>;

    /**
     * Checks if the status code of a http response is a success
     * @node http_response_is_success @receiver response @alias httpResponseIsSuccess
     * @param response — The http response (receiver: `this` in `x.isSuccess(...)`)
     * @returns isSuccess — True if the status code is a success
     */
    function isSuccess(this: HttpResponse, { response: Struct }): bool;

    /**
     * Gets the status code from a http response
     * @node http_response_get_status @receiver response @alias httpResponseGetStatus
     * @param response — The http response (receiver: `this` in `x.status(...)`)
     * @returns statusCode — The status code of the response
     */
    function status(this: HttpResponse, { response: Struct }): int;

    /**
     * Gets the body of a http response as bytes
     * @node http_response_to_bytes @receiver response @alias httpResponseToBytes
     * @param response — The http response (receiver: `this` in `x.toBytes(...)`)
     * @returns bytes — The body of the response as bytes
     * @impure has side effects / drives control flow
     */
    function toBytes(this: HttpResponse, { response: Struct }): bytes[];

    /**
     * Gets the body of a http response as json
     * @node http_response_to_json @receiver response @alias httpResponseToJson
     * @param response — The http response (receiver: `this` in `x.toJson(...)`)
     * @returns struct — The body of the response as json
     * @impure has side effects / drives control flow
     */
    function toJson(this: HttpResponse, { response: Struct }): Struct;

    /**
     * Gets the body of a http response as text
     * @node http_response_to_text @receiver response @alias httpResponseToText
     * @param response — The http response (receiver: `this` in `x.toText(...)`)
     * @returns text — The body of the response as text
     * @impure has side effects / drives control flow
     */
    function toText(this: HttpResponse, { response: Struct }): string;
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

declare namespace mcp {
    // === Web/MCP ===

    /**
     * Registers MCP server authentication settings.
     * @node mcp_register_auth @receiver config_in @alias mcpRegisterAuth
     * @param configIn — MCP server config (receiver: `this` in `x.registerAuth(...)`)
     * @param auth — Auth config
     * @returns configOut — Updated config
     */
    function registerAuth(this: McpServerConfig, { configIn: Struct, auth: Struct }): Struct;

    /**
     * Registers referenced Flow functions as MCP tools.
     * @node mcp_register_functions @receiver config_in @alias mcpRegisterFunctions
     * @param configIn — MCP server config (receiver: `this` in `x.registerFunctions(...)`)
     * @returns configOut — Updated config
     */
    function registerFunctions(this: McpServerConfig, { configIn: Struct }): Struct;

    /**
     * Registers a static MCP prompt template.
     * @node mcp_register_prompt @receiver config_in @alias mcpRegisterPrompt
     * @param configIn — MCP server config (receiver: `this` in `x.registerPrompt(...)`)
     * @param name — Prompt name
     * @param description — Optional description
     * @param template — Prompt template
     * @returns configOut — Updated config
     */
    function registerPrompt(this: McpServerConfig, { configIn: Struct, name: string, description: string, template: string }): Struct;

    /**
     * Registers a FlowPath as an MCP resource.
     * @node mcp_register_resource @receiver config_in @alias mcpRegisterResource
     * @param configIn — MCP server config (receiver: `this` in `x.registerResource(...)`)
     * @param flowPath — Resource FlowPath
     * @param uri — MCP resource URI exposed to clients. Defaults to file://<flow path> when empty.
     * @param name — Resource display name. Defaults to the FlowPath filename when empty.
     * @param description — Optional description
     * @returns configOut — Updated config
     */
    function registerResource(this: McpServerConfig, { configIn: Struct, flowPath: Struct, uri: string, name: string, description: string }): Struct;

    /**
     * Starts an MCP server from a composed config.
     * @node mcp_server @alias mcpServer
     * @param config — MCP server config
     * @returns localAddr — Bound address
     * @impure has side effects / drives control flow
     */
    function server({ config: Struct }): string;

    /**
     * Creates an MCP server config that function, resource, prompt, auth, and server nodes can compose.
     * @node mcp_server_config @alias mcpServerConfig
     * @param host (optional) — Bind host
     * @param port (optional) — Bind port
     * @param path (optional) — MCP HTTP path
     * @param timeoutSeconds (optional) — Server lifetime timeout; zero means run until cancelled
     * @param maxConnections (optional) — Maximum concurrent requests
     * @param maxBodyBytes (optional) — Maximum HTTP request body size
     * @param tls — TLS security config
     * @returns config — MCP server config
     */
    function serverConfig({ host?: string, port?: int, path?: string, timeoutSeconds?: int, maxConnections?: int, maxBodyBytes?: int, tls: Struct }): Struct;
}

declare namespace mqtt {
    // === Web/MQTT ===

    /**
     * Binds a lightweight MQTT broker for daemon workflows. Typed lifecycle events are exposed as pins; published messages are delivered to the referenced on-message handler.
     * @node mqtt_broker @alias mqttBroker
     * @param config — MQTT broker configuration
     * @returns localAddr — Bound broker socket address
     * @returns clientId — Connected MQTT client id
     * @returns remoteAddr — Remote client socket address
     * @impure has side effects / drives control flow
     */
    function broker({ config: Struct }): { localAddr: string, clientId: string, remoteAddr: string };

    /**
     * Connects to an MQTT broker and returns a session reference for use with Publish, Subscribe, and Disconnect nodes.
     * @node mqtt_connect @alias mqttConnect
     * @param config — MQTT connection configuration (host, port, client_id, optional credentials, TLS)
     * @returns session — MQTT session reference for use with Publish/Subscribe/Disconnect nodes
     * @impure has side effects / drives control flow
     */
    function connect({ config: Struct }): Struct;

    /**
     * Disconnects from an MQTT broker and cleans up the session
     * @node mqtt_disconnect @receiver session @alias mqttDisconnect
     * @param session — MQTT session to disconnect (receiver: `this` in `x.disconnect(...)`)
     * @impure has side effects / drives control flow
     */
    function disconnect(this: MqttSession, { session: Struct }): void;

    /**
     * Publishes a message to an MQTT topic
     * @node mqtt_publish @receiver session @alias mqttPublish
     * @param session — MQTT session reference (receiver: `this` in `x.publish(...)`)
     * @param topic — The MQTT topic to publish to
     * @param payload — The message content to publish
     * @param qos (optional) — Quality of Service level
     * @param retain (optional) — Whether the broker should retain this message
     * @impure has side effects / drives control flow
     */
    function publish(this: MqttSession, { session: Struct, topic: string, payload: string, qos?: string, retain?: bool }): void;

    /**
     * Subscribes to an MQTT topic and invokes a handler for each incoming message. Holds execution until the connection closes or timeout, then triggers on_close.
     * @node mqtt_subscribe @receiver session @alias mqttSubscribe
     * @param session — MQTT session reference (receiver: `this` in `x.subscribe(...)`)
     * @param topic — The MQTT topic filter to subscribe to
     * @param qos (optional) — Quality of Service level for the subscription
     * @param timeoutSeconds (optional) — How long to listen before auto-closing (0 = indefinite)
     * @impure has side effects / drives control flow
     */
    function subscribe(this: MqttSession, { session: Struct, topic: string, qos?: string, timeoutSeconds?: int }): void;
}

declare namespace rest {
    // === Web/REST ===

    /**
     * Registers REST server authentication settings.
     * @node rest_register_auth @receiver config_in @alias restRegisterAuth
     * @param configIn — REST server config (receiver: `this` in `x.registerAuth(...)`)
     * @param auth (optional) — Auth config
     * @returns configOut — Updated config
     */
    function registerAuth(this: RestServerConfig, { configIn: Struct, auth?: Struct }): Struct;

    /**
     * Registers a FlowPath file or directory as static REST responses.
     * @node rest_register_files @receiver config_in @alias restRegisterFiles
     * @param configIn — REST server config (receiver: `this` in `x.registerFiles(...)`)
     * @param path — HTTP route path
     * @param flowPath — File or directory FlowPath
     * @param directory (optional) — Serve the FlowPath as a directory mount
     * @param contentType (optional) — Optional response content type override
     * @returns configOut — Updated config
     */
    function registerFiles(this: RestServerConfig, { configIn: Struct, path: string, flowPath: Struct, directory?: bool, contentType?: string }): Struct;

    /**
     * Registers referenced Flow functions as handlers for a REST path.
     * @node rest_register_function @receiver config_in @alias restRegisterFunction
     * @param configIn — REST server config (receiver: `this` in `x.registerFunction(...)`)
     * @param path — HTTP route path
     * @param method (optional) — Allowed HTTP method. ANY accepts all methods.
     * @returns configOut — Updated config
     */
    function registerFunction(this: RestServerConfig, { configIn: Struct, path: string, method?: string }): Struct;

    /**
     * Registers OpenAPI JSON and browser UI endpoints generated from the REST server config.
     * @node rest_register_open_api @receiver config_in @alias restRegisterOpenApi
     * @param configIn — REST server config (receiver: `this` in `x.registerOpenApi(...)`)
     * @param path (optional) — OpenAPI JSON route path
     * @param uiPath (optional) — OpenAPI browser UI route path; empty disables the UI
     * @returns configOut — Updated config
     */
    function registerOpenApi(this: RestServerConfig, { configIn: Struct, path?: string, uiPath?: string }): Struct;

    /**
     * Starts a REST server from a composed config. Function routes and files are registered on the config before this node runs.
     * @node rest_server @alias restServer
     * @param config — REST server config
     * @returns localAddr — Bound address
     * @impure has side effects / drives control flow
     */
    function server({ config: Struct }): string;

    /**
     * Creates a REST server config that route, file, auth, and server nodes can compose.
     * @node rest_server_config @alias restServerConfig
     * @param host (optional) — Bind host
     * @param port (optional) — Bind port
     * @param timeoutSeconds (optional) — Server lifetime timeout; zero means run until cancelled
     * @param maxConnections (optional) — Maximum concurrent requests
     * @param maxBodyBytes (optional) — Maximum HTTP request body size
     * @param tls — TLS security config
     * @returns config — REST server config
     */
    function serverConfig({ host?: string, port?: int, timeoutSeconds?: int, maxConnections?: int, maxBodyBytes?: int, tls: Struct }): Struct;
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

declare namespace tcp {
    // === Web/TCP ===

    /**
     * Closes an open TCP connection gracefully
     * @node tcp_close @receiver session @alias tcpClose
     * @param session — TCP session to close (receiver: `this` in `x.close(...)`)
     * @impure has side effects / drives control flow
     */
    function close(this: TcpSession, { session: Struct }): void;

    /**
     * Opens a TCP connection to a remote host. Triggers on_connect with the session, then invokes the on-message handler for each incoming data chunk. Holds execution until the connection closes, then triggers on_close.
     * @node tcp_connect @alias tcpConnect
     * @param config — TCP connection configuration (host, port, optional timeout)
     * @returns session — TCP session reference for use with Send/Close nodes
     * @impure has side effects / drives control flow
     */
    function connect({ config: Struct }): Struct;

    /**
     * Binds a TCP listener on a port. Fires on_listening, then accepts incoming connections and invokes the handler for each. Holds execution until closed or timed out, then triggers on_close.
     * @node tcp_listen @alias tcpListen
     * @param config — TCP listener configuration (host, port, optional timeout, max connections)
     * @impure has side effects / drives control flow
     */
    function listen({ config: Struct }): void;

    /**
     * Sends data through an open TCP connection
     * @node tcp_send @receiver session @alias tcpSend
     * @param session — TCP session reference (receiver: `this` in `x.send(...)`)
     * @param messageType (optional) — Whether to send as text (UTF-8) or binary
     * @param payload — The data to send (string for Text, byte array for Binary)
     * @impure has side effects / drives control flow
     */
    function send(this: TcpSession, { session: Struct, messageType?: string, payload: string }): void;

    /**
     * Binds a TCP server. Typed lifecycle events are exposed as pins; incoming data chunks are delivered to the referenced on-message handler.
     * @node tcp_server @alias tcpServer
     * @param config — TCP server configuration
     * @returns localAddr — Bound local socket address
     * @returns session — Accepted TCP client session
     * @returns remoteAddr — Remote client socket address
     * @impure has side effects / drives control flow
     */
    function server({ config: Struct }): { localAddr: string, session: Struct, remoteAddr: string };
}

declare namespace tls {
    // === Web/TLS ===

    /**
     * Creates a local certificate authority certificate and private key.
     * @node create_ca_certificate @alias createCaCertificate
     * @param commonName (optional) — Certificate authority common name
     * @returns certificate — Certificate authority PEM bundle
     * @impure has side effects / drives control flow
     */
    function createCaCertificate({ commonName?: string }): Struct;

    /**
     * Creates a server or client certificate signed by a local certificate authority.
     * @node create_ca_signed_certificate @alias createCaSignedCertificate
     * @param ca — Certificate authority PEM bundle
     * @param commonName (optional) — Certificate common name
     * @param subjectAltNames (optional) — DNS names and IP addresses covered by this certificate
     * @param usage (optional) — Certificate usage
     * @returns certificate — Signed certificate PEM bundle
     * @impure has side effects / drives control flow
     */
    function createCaSignedCertificate({ ca: Struct, commonName?: string, subjectAltNames?: string[], usage?: string }): Struct;

    /**
     * Creates a self-signed certificate and private key.
     * @node create_self_signed_certificate @alias createSelfSignedCertificate
     * @param subjectAltNames (optional) — DNS names and IP addresses covered by this certificate
     * @returns certificate — Self-signed certificate PEM bundle
     * @impure has side effects / drives control flow
     */
    function createSelfSignedCertificate({ subjectAltNames?: string[] }): Struct;
}

declare namespace udp {
    // === Web/UDP ===

    /**
     * Binds a UDP socket to a local address and port
     * @node udp_bind @alias udpBind
     * @param config — UDP bind configuration (host and port)
     * @returns session — UDP session reference for use with SendTo/Receive/Close nodes
     * @impure has side effects / drives control flow
     */
    function bind({ config: Struct }): Struct;

    /**
     * Closes a bound UDP socket and releases resources
     * @node udp_close @receiver session @alias udpClose
     * @param session — UDP session to close (receiver: `this` in `x.close(...)`)
     * @impure has side effects / drives control flow
     */
    function close(this: UdpSession, { session: Struct }): void;

    /**
     * Listens for incoming datagrams on a bound UDP socket. Invokes the on-message handler for each received datagram. Holds execution until the socket is closed or the timeout expires, then fires on_close.
     * @node udp_receive @receiver session @alias udpReceive
     * @param session — UDP session reference from a Bind node (receiver: `this` in `x.receive(...)`)
     * @param timeoutSeconds (optional) — How long to listen before auto-closing (0 = indefinite)
     * @impure has side effects / drives control flow
     */
    function receive(this: UdpSession, { session: Struct, timeoutSeconds?: int }): void;

    /**
     * Sends a datagram to a target address through a bound UDP socket
     * @node udp_send_to @receiver session @alias udpSendTo
     * @param session — UDP session reference (receiver: `this` in `x.sendTo(...)`)
     * @param targetHost — Destination host address
     * @param targetPort — Destination port number
     * @param payload — The message content to send
     * @returns bytesSent — Number of bytes sent
     * @impure has side effects / drives control flow
     */
    function sendTo(this: UdpSession, { session: Struct, targetHost: string, targetPort: int, payload: string }): int;

    /**
     * Binds a UDP socket. Typed lifecycle pins describe the socket; incoming datagrams are delivered to the referenced on-message handler.
     * @node udp_server @alias udpServer
     * @param config — UDP server configuration
     * @returns session — UDP server socket session
     * @returns localAddr — Bound local socket address
     * @impure has side effects / drives control flow
     */
    function server({ config: Struct }): { session: Struct, localAddr: string };
}

declare namespace web {
    // === Web/Scraping ===

    /**
     * Extracts links from the input text
     * @node web_scrape_extract_links @alias webScrapeExtractLinks
     * @param startingPage — The page to start extracting links from
     * @param sameDomain (optional) — Stay on the same domain or subdomains
     * @param offsetMs (optional) — Delay between requests
     * @param depth (optional) — The depth to extract links from
     * @returns links — The extracted links
     * @impure has side effects / drives control flow
     */
    function extractLinks({ startingPage: string, sameDomain?: bool, offsetMs?: int, depth?: int }): Set<string>;
}

declare namespace websocket {
    // === Web/WebSocket ===

    /**
     * Closes an open WebSocket connection gracefully
     * @node websocket_close @receiver session @alias websocketClose
     * @param session — WebSocket session to close (receiver: `this` in `x.close(...)`)
     * @impure has side effects / drives control flow
     */
    function close(this: WebSocketSession, { session: Struct }): void;

    /**
     * Opens a WebSocket connection. Immediately triggers on_connect with the session, then invokes on_message for each incoming message. Holds execution until the connection closes, then triggers on_close.
     * @node websocket_connect @alias websocketConnect
     * @param config — WebSocket connection configuration (URL, optional headers, optional timeout)
     * @returns session — WebSocket session reference for use with Send/Close nodes
     * @impure has side effects / drives control flow
     */
    function connect({ config: Struct }): Struct;

    /**
     * Sends a message through an open WebSocket connection
     * @node websocket_send @receiver session @alias websocketSend
     * @param session — WebSocket session reference (receiver: `this` in `x.send(...)`)
     * @param messageType (optional) — Whether to send as text or binary
     * @param payload — The message content to send (string for Text, byte array for Binary)
     * @impure has side effects / drives control flow
     */
    function send(this: WebSocketSession, { session: Struct, messageType?: string, payload: string }): void;

    /**
     * Binds a WebSocket server. Typed lifecycle events are exposed as pins; incoming messages are delivered to the referenced on-message handler.
     * @node websocket_server @alias websocketServer
     * @param config — WebSocket server configuration
     * @returns localAddr — Bound local socket address
     * @returns session — Accepted WebSocket client session
     * @returns remoteAddr — Remote client socket address
     * @impure has side effects / drives control flow
     */
    function server({ config: Struct }): { localAddr: string, session: Struct, remoteAddr: string };
}
