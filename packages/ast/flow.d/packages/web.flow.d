// web — FlowScript node declarations (generated, do not edit).
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


// === Web ===

/**
 * Downloads a file from a url
 * @param request — The HTTP request to perform
 * @param flowPath — The path to save the file to
 * @impure has side effects / drives control flow
 */
declare function httpDownload({ request: Struct, flowPath: Struct }): void;


// === Web/API ===

/**
 * Performs an HTTP request
 * @param request — The HTTP request to perform
 * @returns response — The HTTP response
 * @impure has side effects / drives control flow
 */
declare function httpFetch({ request: Struct }): Struct;

/**
 * Performs an HTTP request
 * @param request — The HTTP request to perform
 * @returns streamingResponse — The HTTP response
 * @returns response — The HTTP response
 * @impure has side effects / drives control flow
 */
declare function streamingHttpFetch({ request: Struct }): { streamingResponse: bytes[], response: Struct };


// === Web/API/Request ===

/**
 * Gets a header from a http request
 * @param request — The http request
 * @param header — The header to get
 * @returns found — True if the header was found
 * @returns value — The value of the header
 */
declare function httpGetHeader({ request: Struct, header: string }): { found: bool, value: string };

/**
 * Gets all headers from a http request
 * @param request — The http request
 * @returns headers — The headers of the request
 */
declare function httpGetHeaders({ request: Struct }): Map<string, string>;

/**
 * Gets the method from a http request
 * @param request — The http request
 * @returns method — The method of the request
 */
declare function httpGetMethod({ request: Struct }): string;

/**
 * Gets the url from a http request
 * @param request — The http request
 * @returns url — The url of the request
 */
declare function httpGetUrl({ request: Struct }): string;

/**
 * Creates a http request
 * @param method (optional) — Http Method GET,POST etc.
 * @param url — The request URL
 * @returns request — The http request
 */
declare function httpMakeRequest({ method?: string, url: string }): Struct;

/**
 * Sets the Accept header of a http request
 * @param request — The http request
 * @param accept (optional) — The accept header value
 * @returns requestOut — The http request
 */
declare function httpSetAccept({ request: Struct, accept?: string }): Struct;

/**
 * Sets the Authorization header using a Bearer token
 * @param request — The http request
 * @param token — Bearer token
 * @returns requestOut — The http request
 */
declare function httpSetBearerAuth({ request: Struct, token: string }): Struct;

/**
 * Sets the body of a http request
 * @param request — The http request
 * @param body — The body of the request
 * @returns requestOut — The http request
 */
declare function httpSetBytesBody({ request: Struct, body: bytes[] }): Struct;

/**
 * Sets the Content-Type header of a http request
 * @param request — The http request
 * @param contentType (optional) — The content type value
 * @returns requestOut — The http request
 */
declare function httpSetContentType({ request: Struct, contentType?: string }): Struct;

/**
 * Sets the body of a http request to form-encoded data
 * @param request — The http request
 * @param fields (optional) — Form fields to encode
 * @param setContentType (optional) — Adds application/x-www-form-urlencoded when missing
 * @returns requestOut — The http request
 */
declare function httpSetFormBody({ request: Struct, fields?: Struct, setContentType?: bool }): Struct;

/**
 * Sets a header of a http request
 * @param request — The http request
 * @param name — The name of the header
 * @param value — The value of the header
 * @returns requestOut — The http request
 */
declare function httpSetHeader({ request: Struct, name: string, value: string }): Struct;

/**
 * Sets the headers of a http request
 * @param request — The http request
 * @param headers — The headers of the request
 * @param merge (optional) — Merge with existing headers instead of replacing
 * @returns requestOut — The http request
 */
declare function httpSetHeaders({ request: Struct, headers: Map<string, string>, merge?: bool }): Struct;

/**
 * Sets the method of a http request
 * @param request — The http request
 * @param method (optional) — The method of the request
 * @returns requestOut — The http request
 */
declare function httpSetMethod({ request: Struct, method?: string }): Struct;

/**
 * Sets the body of a http request
 * @param request — The http request
 * @param body — The body of the request
 * @returns requestOut — The http request
 */
declare function httpSetStringBody({ request: Struct, body: string }): Struct;

/**
 * Sets the body of a http request
 * @param request — The http request
 * @param body — The body of the request
 * @returns requestOut — The http request
 */
declare function httpSetStructBody({ request: Struct, body: Struct }): Struct;

/**
 * Sets the url of a http request
 * @param request — The http request
 * @param url — The url of the request
 * @returns requestOut — The http request
 */
declare function httpSetUrl({ request: Struct, url: string }): Struct;


// === Web/API/Response ===

/**
 * Gets a header from a http request
 * @param response — The http response
 * @param header — The header to get
 * @returns found — True if the header was found
 * @returns value — The value of the header
 */
declare function httpResponseGetHeader({ response: Struct, header: string }): { found: bool, value: string };

/**
 * Gets all headers from a http request
 * @param response — The http response
 * @returns headers — The headers of the response
 */
declare function httpResponseGetHeaders({ response: Struct }): Map<string, string>;

/**
 * Gets the status code from a http response
 * @param response — The http response
 * @returns statusCode — The status code of the response
 */
declare function httpResponseGetStatus({ response: Struct }): int;

/**
 * Checks if the status code of a http response is a success
 * @param response — The http response
 * @returns isSuccess — True if the status code is a success
 */
declare function httpResponseIsSuccess({ response: Struct }): bool;

/**
 * Gets the body of a http response as bytes
 * @param response — The http response
 * @returns bytes — The body of the response as bytes
 * @impure has side effects / drives control flow
 */
declare function httpResponseToBytes({ response: Struct }): bytes[];

/**
 * Gets the body of a http response as json
 * @param response — The http response
 * @returns struct — The body of the response as json
 * @impure has side effects / drives control flow
 */
declare function httpResponseToJson({ response: Struct }): Struct;

/**
 * Gets the body of a http response as text
 * @param response — The http response
 * @returns text — The body of the response as text
 * @impure has side effects / drives control flow
 */
declare function httpResponseToText({ response: Struct }): string;


// === Web/Auth ===

/**
 * Creates REST auth that requires a configured API key header.
 * @param header (optional) — Header that carries the API key
 * @param key — Expected API key
 * @returns auth — API key auth config
 */
declare function apiKeyAuth({ header?: string, key: string }): Struct;

/**
 * Creates REST auth that requires HTTP Basic credentials.
 * @param username — Expected username
 * @param password — Expected password
 * @returns auth — Basic auth config
 */
declare function basicAuth({ username: string, password: string }): Struct;

/**
 * Creates REST auth that requires a static Authorization bearer token.
 * @param token — Expected bearer token
 * @returns auth — Bearer token auth config
 */
declare function bearerTokenAuth({ token: string }): Struct;

/**
 * Creates REST auth that verifies an HMAC-SHA256 request signature.
 * @param secret — Shared HMAC secret
 * @param signatureHeader (optional) — Header that carries the lowercase hex HMAC signature
 * @param timestampHeader (optional) — Header that carries the Unix timestamp in seconds
 * @param maxSkewSeconds (optional) — Allowed timestamp skew in seconds; zero disables timestamp freshness checks
 * @returns auth — HMAC auth config
 */
declare function hmacSha256Auth({ secret: string, signatureHeader?: string, timestampHeader?: string, maxSkewSeconds?: int }): Struct;

/**
 * Creates OAuth bearer auth from a JWKS JSON FlowPath loaded when the server starts.
 * @param jwksFlowPath — JWKS JSON file FlowPath
 * @param issuer — Required token issuer. Empty disables issuer validation.
 * @param audience — Required token audience. Empty disables audience validation.
 * @param requiredScopes — Scopes that must be present in the token scope/scp claims.
 * @returns auth — OAuth auth config
 */
declare function oauthJwksFileAuth({ jwksFlowPath: Struct, issuer: string, audience: string, requiredScopes: string[] }): Struct;

/**
 * Creates OAuth bearer auth that fetches a JWKS endpoint once when the server starts.
 * @param jwksUrl — JWKS endpoint URL
 * @param issuer — Required token issuer. Empty disables issuer validation.
 * @param audience — Required token audience. Empty disables audience validation.
 * @param requiredScopes — Scopes that must be present in the token scope/scp claims.
 * @returns auth — OAuth auth config
 */
declare function oauthJwksUrlAuth({ jwksUrl: string, issuer: string, audience: string, requiredScopes: string[] }): Struct;

/**
 * Creates OAuth bearer auth by discovering the JWKS URI from an OpenID Connect issuer.
 * @param issuer — OIDC issuer URL. The server fetches /.well-known/openid-configuration.
 * @param audience — Required token audience. Empty disables audience validation.
 * @param requiredScopes — Scopes that must be present in the token scope/scp claims.
 * @returns auth — OIDC auth config
 */
declare function oidcDiscoveryAuth({ issuer: string, audience: string, requiredScopes: string[] }): Struct;


// === Web/Camera ===

/**
 * Captures a frame from an IP camera
 * @param request — The HTTP request to perform
 * @returns image — The captured image frame
 * @impure has side effects / drives control flow
 */
declare function webCameraGrabFrame({ request: Struct }): Struct;


// === Web/MCP ===

/**
 * Registers MCP server authentication settings.
 * @param configIn — MCP server config
 * @param auth — Auth config
 * @returns configOut — Updated config
 */
declare function mcpRegisterAuth({ configIn: Struct, auth: Struct }): Struct;

/**
 * Registers referenced Flow functions as MCP tools.
 * @param configIn — MCP server config
 * @returns configOut — Updated config
 */
declare function mcpRegisterFunctions({ configIn: Struct }): Struct;

/**
 * Registers a static MCP prompt template.
 * @param configIn — MCP server config
 * @param name — Prompt name
 * @param description — Optional description
 * @param template — Prompt template
 * @returns configOut — Updated config
 */
declare function mcpRegisterPrompt({ configIn: Struct, name: string, description: string, template: string }): Struct;

/**
 * Registers a FlowPath as an MCP resource.
 * @param configIn — MCP server config
 * @param flowPath — Resource FlowPath
 * @param uri — MCP resource URI exposed to clients. Defaults to file://<flow path> when empty.
 * @param name — Resource display name. Defaults to the FlowPath filename when empty.
 * @param description — Optional description
 * @returns configOut — Updated config
 */
declare function mcpRegisterResource({ configIn: Struct, flowPath: Struct, uri: string, name: string, description: string }): Struct;

/**
 * Starts an MCP server from a composed config.
 * @param config — MCP server config
 * @returns localAddr — Bound address
 * @impure has side effects / drives control flow
 */
declare function mcpServer({ config: Struct }): string;

/**
 * Creates an MCP server config that function, resource, prompt, auth, and server nodes can compose.
 * @param host (optional) — Bind host
 * @param port (optional) — Bind port
 * @param path (optional) — MCP HTTP path
 * @param timeoutSeconds (optional) — Server lifetime timeout; zero means run until cancelled
 * @param maxConnections (optional) — Maximum concurrent requests
 * @param maxBodyBytes (optional) — Maximum HTTP request body size
 * @param tls — TLS security config
 * @returns config — MCP server config
 */
declare function mcpServerConfig({ host?: string, port?: int, path?: string, timeoutSeconds?: int, maxConnections?: int, maxBodyBytes?: int, tls: Struct }): Struct;


// === Web/MQTT ===

/**
 * Binds a lightweight MQTT broker for daemon workflows. Typed lifecycle events are exposed as pins; published messages are delivered to the referenced on-message handler.
 * @param config — MQTT broker configuration
 * @returns localAddr — Bound broker socket address
 * @returns clientId — Connected MQTT client id
 * @returns remoteAddr — Remote client socket address
 * @impure has side effects / drives control flow
 */
declare function mqttBroker({ config: Struct }): { localAddr: string, clientId: string, remoteAddr: string };

/**
 * Connects to an MQTT broker and returns a session reference for use with Publish, Subscribe, and Disconnect nodes.
 * @param config — MQTT connection configuration (host, port, client_id, optional credentials, TLS)
 * @returns session — MQTT session reference for use with Publish/Subscribe/Disconnect nodes
 * @impure has side effects / drives control flow
 */
declare function mqttConnect({ config: Struct }): Struct;

/**
 * Disconnects from an MQTT broker and cleans up the session
 * @param session — MQTT session to disconnect
 * @impure has side effects / drives control flow
 */
declare function mqttDisconnect({ session: Struct }): void;

/**
 * Publishes a message to an MQTT topic
 * @param session — MQTT session reference
 * @param topic — The MQTT topic to publish to
 * @param payload — The message content to publish
 * @param qos (optional) — Quality of Service level
 * @param retain (optional) — Whether the broker should retain this message
 * @impure has side effects / drives control flow
 */
declare function mqttPublish({ session: Struct, topic: string, payload: string, qos?: string, retain?: bool }): void;

/**
 * Subscribes to an MQTT topic and invokes a handler for each incoming message. Holds execution until the connection closes or timeout, then triggers on_close.
 * @param session — MQTT session reference
 * @param topic — The MQTT topic filter to subscribe to
 * @param qos (optional) — Quality of Service level for the subscription
 * @param timeoutSeconds (optional) — How long to listen before auto-closing (0 = indefinite)
 * @impure has side effects / drives control flow
 */
declare function mqttSubscribe({ session: Struct, topic: string, qos?: string, timeoutSeconds?: int }): void;


// === Web/REST ===

/**
 * Registers REST server authentication settings.
 * @param configIn — REST server config
 * @param auth (optional) — Auth config
 * @returns configOut — Updated config
 */
declare function restRegisterAuth({ configIn: Struct, auth?: Struct }): Struct;

/**
 * Registers a FlowPath file or directory as static REST responses.
 * @param configIn — REST server config
 * @param path — HTTP route path
 * @param flowPath — File or directory FlowPath
 * @param directory (optional) — Serve the FlowPath as a directory mount
 * @param contentType (optional) — Optional response content type override
 * @returns configOut — Updated config
 */
declare function restRegisterFiles({ configIn: Struct, path: string, flowPath: Struct, directory?: bool, contentType?: string }): Struct;

/**
 * Registers referenced Flow functions as handlers for a REST path.
 * @param configIn — REST server config
 * @param path — HTTP route path
 * @param method (optional) — Allowed HTTP method. ANY accepts all methods.
 * @returns configOut — Updated config
 */
declare function restRegisterFunction({ configIn: Struct, path: string, method?: string }): Struct;

/**
 * Registers OpenAPI JSON and browser UI endpoints generated from the REST server config.
 * @param configIn — REST server config
 * @param path (optional) — OpenAPI JSON route path
 * @param uiPath (optional) — OpenAPI browser UI route path; empty disables the UI
 * @returns configOut — Updated config
 */
declare function restRegisterOpenApi({ configIn: Struct, path?: string, uiPath?: string }): Struct;

/**
 * Starts a REST server from a composed config. Function routes and files are registered on the config before this node runs.
 * @param config — REST server config
 * @returns localAddr — Bound address
 * @impure has side effects / drives control flow
 */
declare function restServer({ config: Struct }): string;

/**
 * Creates a REST server config that route, file, auth, and server nodes can compose.
 * @param host (optional) — Bind host
 * @param port (optional) — Bind port
 * @param timeoutSeconds (optional) — Server lifetime timeout; zero means run until cancelled
 * @param maxConnections (optional) — Maximum concurrent requests
 * @param maxBodyBytes (optional) — Maximum HTTP request body size
 * @param tls — TLS security config
 * @returns config — REST server config
 */
declare function restServerConfig({ host?: string, port?: int, timeoutSeconds?: int, maxConnections?: int, maxBodyBytes?: int, tls: Struct }): Struct;


// === Web/Scraping ===

/**
 * Extracts links from the input text
 * @param startingPage — The page to start extracting links from
 * @param sameDomain (optional) — Stay on the same domain or subdomains
 * @param offsetMs (optional) — Delay between requests
 * @param depth (optional) — The depth to extract links from
 * @returns links — The extracted links
 * @impure has side effects / drives control flow
 */
declare function webScrapeExtractLinks({ startingPage: string, sameDomain?: bool, offsetMs?: int, depth?: int }): Set<string>;


// === Web/TCP ===

/**
 * Closes an open TCP connection gracefully
 * @param session — TCP session to close
 * @impure has side effects / drives control flow
 */
declare function tcpClose({ session: Struct }): void;

/**
 * Opens a TCP connection to a remote host. Triggers on_connect with the session, then invokes the on-message handler for each incoming data chunk. Holds execution until the connection closes, then triggers on_close.
 * @param config — TCP connection configuration (host, port, optional timeout)
 * @returns session — TCP session reference for use with Send/Close nodes
 * @impure has side effects / drives control flow
 */
declare function tcpConnect({ config: Struct }): Struct;

/**
 * Binds a TCP listener on a port. Fires on_listening, then accepts incoming connections and invokes the handler for each. Holds execution until closed or timed out, then triggers on_close.
 * @param config — TCP listener configuration (host, port, optional timeout, max connections)
 * @impure has side effects / drives control flow
 */
declare function tcpListen({ config: Struct }): void;

/**
 * Sends data through an open TCP connection
 * @param session — TCP session reference
 * @param messageType (optional) — Whether to send as text (UTF-8) or binary
 * @param payload — The data to send (string for Text, byte array for Binary)
 * @impure has side effects / drives control flow
 */
declare function tcpSend({ session: Struct, messageType?: string, payload: string }): void;

/**
 * Binds a TCP server. Typed lifecycle events are exposed as pins; incoming data chunks are delivered to the referenced on-message handler.
 * @param config — TCP server configuration
 * @returns localAddr — Bound local socket address
 * @returns session — Accepted TCP client session
 * @returns remoteAddr — Remote client socket address
 * @impure has side effects / drives control flow
 */
declare function tcpServer({ config: Struct }): { localAddr: string, session: Struct, remoteAddr: string };


// === Web/TLS ===

/**
 * Creates a local certificate authority certificate and private key.
 * @param commonName (optional) — Certificate authority common name
 * @returns certificate — Certificate authority PEM bundle
 * @impure has side effects / drives control flow
 */
declare function createCaCertificate({ commonName?: string }): Struct;

/**
 * Creates a server or client certificate signed by a local certificate authority.
 * @param ca — Certificate authority PEM bundle
 * @param commonName (optional) — Certificate common name
 * @param subjectAltNames (optional) — DNS names and IP addresses covered by this certificate
 * @param usage (optional) — Certificate usage
 * @returns certificate — Signed certificate PEM bundle
 * @impure has side effects / drives control flow
 */
declare function createCaSignedCertificate({ ca: Struct, commonName?: string, subjectAltNames?: string[], usage?: string }): Struct;

/**
 * Creates a self-signed certificate and private key.
 * @param subjectAltNames (optional) — DNS names and IP addresses covered by this certificate
 * @returns certificate — Self-signed certificate PEM bundle
 * @impure has side effects / drives control flow
 */
declare function createSelfSignedCertificate({ subjectAltNames?: string[] }): Struct;


// === Web/UDP ===

/**
 * Binds a UDP socket to a local address and port
 * @param config — UDP bind configuration (host and port)
 * @returns session — UDP session reference for use with SendTo/Receive/Close nodes
 * @impure has side effects / drives control flow
 */
declare function udpBind({ config: Struct }): Struct;

/**
 * Closes a bound UDP socket and releases resources
 * @param session — UDP session to close
 * @impure has side effects / drives control flow
 */
declare function udpClose({ session: Struct }): void;

/**
 * Listens for incoming datagrams on a bound UDP socket. Invokes the on-message handler for each received datagram. Holds execution until the socket is closed or the timeout expires, then fires on_close.
 * @param session — UDP session reference from a Bind node
 * @param timeoutSeconds (optional) — How long to listen before auto-closing (0 = indefinite)
 * @impure has side effects / drives control flow
 */
declare function udpReceive({ session: Struct, timeoutSeconds?: int }): void;

/**
 * Sends a datagram to a target address through a bound UDP socket
 * @param session — UDP session reference
 * @param targetHost — Destination host address
 * @param targetPort — Destination port number
 * @param payload — The message content to send
 * @returns bytesSent — Number of bytes sent
 * @impure has side effects / drives control flow
 */
declare function udpSendTo({ session: Struct, targetHost: string, targetPort: int, payload: string }): int;

/**
 * Binds a UDP socket. Typed lifecycle pins describe the socket; incoming datagrams are delivered to the referenced on-message handler.
 * @param config — UDP server configuration
 * @returns session — UDP server socket session
 * @returns localAddr — Bound local socket address
 * @impure has side effects / drives control flow
 */
declare function udpServer({ config: Struct }): { session: Struct, localAddr: string };


// === Web/WebSocket ===

/**
 * Closes an open WebSocket connection gracefully
 * @param session — WebSocket session to close
 * @impure has side effects / drives control flow
 */
declare function websocketClose({ session: Struct }): void;

/**
 * Opens a WebSocket connection. Immediately triggers on_connect with the session, then invokes on_message for each incoming message. Holds execution until the connection closes, then triggers on_close.
 * @param config — WebSocket connection configuration (URL, optional headers, optional timeout)
 * @returns session — WebSocket session reference for use with Send/Close nodes
 * @impure has side effects / drives control flow
 */
declare function websocketConnect({ config: Struct }): Struct;

/**
 * Sends a message through an open WebSocket connection
 * @param session — WebSocket session reference
 * @param messageType (optional) — Whether to send as text or binary
 * @param payload — The message content to send (string for Text, byte array for Binary)
 * @impure has side effects / drives control flow
 */
declare function websocketSend({ session: Struct, messageType?: string, payload: string }): void;

/**
 * Binds a WebSocket server. Typed lifecycle events are exposed as pins; incoming messages are delivered to the referenced on-message handler.
 * @param config — WebSocket server configuration
 * @returns localAddr — Bound local socket address
 * @returns session — Accepted WebSocket client session
 * @returns remoteAddr — Remote client socket address
 * @impure has side effects / drives control flow
 */
declare function websocketServer({ config: Struct }): { localAddr: string, session: Struct, remoteAddr: string };

