// Events — FlowScript node declarations (generated, do not edit).
// One declare-function per catalog node. Names are camelCase node types.

// === Events ===

/**
 * A simple Chat event
 * @returns history — Chat History
 * @returns localSession — Local to the Chat
 * @returns globalSession — Global to the User
 * @returns tools — Tools requested by the user
 * @returns actions — User Actions
 * @returns attachments — User Attachments or References
 * @returns user — User Information
 * @impure has side effects / drives control flow
 */
declare function eventsChat(): { history: Struct, localSession: Struct, globalSession: Struct, tools: string[], actions: Struct[], attachments: Struct[], user: Struct };

/**
 * A generic event without input or output
 * @returns payload — The payload of the event
 * @impure has side effects / drives control flow
 */
declare function eventsGeneric(): Struct;

/**
 * A simple event without input or output
 * @impure has side effects / drives control flow
 */
declare function eventsSimple(): void;

/**
 * Entry point triggered when a widget action is invoked. Provides action context data.
 * @param actionId (optional) — The action identifier that triggers this event (e.g., 'clicked_delete', 'clicked_open')
 * @returns widgetInstanceId — The unique ID of the widget instance that triggered the action
 * @returns eventName — The action ID / event name that was triggered
 * @returns actionContext — The context data passed from the widget action (JSON object with field values)
 * @returns inputValues — Map of component ID to current value for components marked as event-relevant
 * @impure has side effects / drives control flow
 */
declare function eventsWidgetAction({ actionId?: string }): { widgetInstanceId: string, eventName: string, actionContext: Struct, inputValues: Struct };


// === Events/Chat ===

/**
 * Pulls down image, audio, video, and document attachments referenced in the latest chat message
 * @param history — Chat history whose final message may contain media parts
 * @param attachments (optional) — Existing attachments to merge with new downloads
 * @returns paths — Virtual file paths pointing to cached attachments
 * @impure has side effects / drives control flow
 */
declare function aiGenLlmHistoryExtractAttachments({ history: Struct, attachments?: Struct[] }): Struct[];

/**
 * Pushes a response chunk to the chat
 * @param attachment — Attachment to the Chat
 * @impure has side effects / drives control flow
 */
declare function eventsChatPushAttachment({ attachment: Struct }): void;

/**
 * Pushes a response chunk to the chat
 * @param attachments — Attachment to the Chat
 * @impure has side effects / drives control flow
 */
declare function eventsChatPushAttachments({ attachments: Struct[] }): void;

/**
 * Pushes a new global session to the chat. The session persists for all chat sessions.
 * @param globalSession (optional) — Generic Struct Type
 * @impure has side effects / drives control flow
 */
declare function eventsChatPushGlobalSession({ globalSession?: Struct }): void;

/**
 * Pushes a new local session to the chat. The session persists for one chat session.
 * @param localSession (optional) — Generic Struct Type
 * @impure has side effects / drives control flow
 */
declare function eventsChatPushLocalSession({ localSession?: Struct }): void;

/**
 * Pushes reasoning tokens to the current step
 * @param reasoning — Reasoning text to append to current step
 * @impure has side effects / drives control flow
 */
declare function eventsChatPushReasoning({ reasoning: string }): void;

/**
 * Pushes a response to the chat
 * @param response — Chat Response
 * @impure has side effects / drives control flow
 */
declare function eventsChatPushResponse({ response: Struct }): void;

/**
 * Pushes a response chunk to the chat
 * @param chunk — Generated Chat Chunk
 * @impure has side effects / drives control flow
 */
declare function eventsChatPushResponseChunk({ chunk: Struct }): void;

/**
 * Pushes a single LLM usage stat to the chat for transparent model usage display
 * @param stepName — Label for this step (e.g. 'Summarization', 'Tool Selection')
 * @param stat — LLM usage statistics
 * @impure has side effects / drives control flow
 */
declare function eventsChatPushStat({ stepName: string, stat: Struct }): void;

/**
 * Pushes multiple LLM usage stats to the chat at once
 * @param stepName — Label for this batch of stats (e.g. 'Agent Execution', 'Pipeline')
 * @param inputStats — Array of LLM usage statistics
 * @impure has side effects / drives control flow
 */
declare function eventsChatPushStats({ stepName: string, inputStats: Struct[] }): void;

/**
 * Starts a new plan step with title and description
 * @param title — Step title
 * @param description — Step description (optional)
 * @returns stepId — The ID of the created step
 * @impure has side effects / drives control flow
 */
declare function eventsChatPushStep({ title: string, description: string }): int;

/**
 * Appends text to the current step's reasoning
 * @param text — Text to append to current step
 * @impure has side effects / drives control flow
 */
declare function eventsChatPushTextToStep({ text: string }): void;

/**
 * Embeds an a2ui widget instance into the chat message. Connect the Element Ref of an Instantiate Widget node.
 * @param elementRef — Widget instance to embed (from Instantiate Widget)
 * @impure has side effects / drives control flow
 */
declare function eventsChatPushWidget({ elementRef: Struct }): void;

/**
 * Embeds multiple a2ui widget instances into the chat message. Add an Element Ref pin for each Instantiate Widget node.
 * @param elementRef — Widget instance to embed (from Instantiate Widget). Add more pins for multiple widgets.
 * @impure has side effects / drives control flow
 */
declare function eventsChatPushWidgets({ elementRef: Struct }): void;

/**
 * Removes a step from the plan by its ID
 * @param stepId — ID of the step to remove
 * @impure has side effects / drives control flow
 */
declare function eventsChatRemoveStep({ stepId: int }): void;


// === Events/Chat/Attachments ===

/**
 * Creates an attachment from a FlowPath with optional metadata
 * @param path — FlowPath to create attachment from
 * @param name (optional) — Display name for the attachment (optional, defaults to filename)
 * @param previewText (optional) — Preview text/description for the attachment (optional)
 * @param page (optional) — Page number reference (optional, for documents)
 * @param anchor (optional) — Anchor/section reference within the document (optional)
 * @param expiration (optional) — Expiration time for the signed URL
 * @returns attachment — The created attachment
 * @impure has side effects / drives control flow
 */
declare function eventsChatAttachmentFromPath({ path: Struct, name?: string, previewText?: string, page?: int, anchor?: string, expiration?: int }): Struct;

/**
 * Get the URL from an attachment
 * @param signedUrl
 * @returns attachment — Attachment to the Chat
 */
declare function eventsChatAttachmentFromSignedUrl({ signedUrl: string }): Struct;

/**
 * Get the URL from an attachment
 * @param attachment — Attachment to the Chat
 * @returns signedUrl
 * @returns success
 */
declare function eventsChatAttachmentToSignedUrl({ attachment: Struct }): { signedUrl: string, success: bool };


// === Events/Chat/Interaction ===

/**
 * Builds a JSON Schema form from a referenced callback function's pins and executes it with typed submitted values.
 * @param name (optional) — Display name for this interaction
 * @param description (optional) — Prompt shown to the user
 * @param ttlSeconds (optional) — How long to wait for response
 * @returns response — JSON object with pin name -> typed value mappings
 * @returns responded — Whether the user responded (vs timeout)
 * @impure has side effects / drives control flow
 */
declare function interactionForm({ name?: string, description?: string, ttlSeconds?: int }): { response: string, responded: bool };

/**
 * Request the user to pick one or more options. Pauses execution until a response or timeout.
 * @param name (optional) — Display name for this interaction
 * @param description (optional) — Prompt shown to the user
 * @param options (optional) — Choice option labels
 * @param options (optional) — Choice option labels
 * @param minSelections (optional) — Minimum number of options the user must select
 * @param maxSelections (optional) — Maximum number of options the user can select (0 = unlimited)
 * @param ttlSeconds (optional) — How long to wait for response
 * @returns response — JSON array of selected option labels
 * @returns responded — Whether the user responded (vs timeout)
 * @impure has side effects / drives control flow
 */
declare function interactionMultipleChoice({ name?: string, description?: string, options?: string, options?: string, minSelections?: int, maxSelections?: int, ttlSeconds?: int }): { response: string[], responded: bool };

/**
 * Request the user to pick one option. Pauses execution until a response or timeout.
 * @param name (optional) — Display name for this interaction
 * @param description (optional) — Prompt shown to the user
 * @param options (optional) — Choice option labels
 * @param options (optional) — Choice option labels
 * @param allowFreeform (optional) — Let user type a custom answer
 * @param ttlSeconds (optional) — How long to wait for response
 * @returns response — The selected option label or freeform text
 * @returns responded — Whether the user responded (vs timeout)
 * @impure has side effects / drives control flow
 */
declare function interactionSingleChoice({ name?: string, description?: string, options?: string, options?: string, allowFreeform?: bool, ttlSeconds?: int }): { response: string, responded: bool };


// === Events/Generic ===

/**
 * Return a result
 * @param response — Chat Response
 * @impure has side effects / drives control flow
 */
declare function eventsGenericReturnResult({ response: any }): void;


// === Events/Remote ===

/**
 * Call an internal REST API exposed by a connected project and return its status, headers and response body.
 * @param flowRemoteAppId (optional) — Connected project to invoke the event in
 * @param flowRemoteEvent (optional) — REST API event of the selected project
 * @param flowRemoteEventMeta (optional) — Auto-filled by the editor when an event is selected. Drives the typed pins.
 * @param route — Route of the remote API to call
 * @param query (optional) — Query parameters as an object
 * @param body (optional) — Request body (JSON)
 * @param headers (optional) — Additional request headers as an object
 * @param timeoutSeconds (optional) — Maximum time to wait for the remote request to finish
 * @returns status — HTTP status code of the response
 * @returns responseHeaders — Response headers as an object
 * @returns response — Response body (JSON when parseable, else text)
 * @returns file — Response body as a downloaded file when it is binary
 * @impure has side effects / drives control flow
 */
declare function callRemoteApi({ flowRemoteAppId?: string, flowRemoteEvent?: string, flowRemoteEventMeta?: string, route: string, query?: any, body?: any, headers?: any, timeoutSeconds?: int }): { status: int, responseHeaders: any, response: any, file: Struct };

/**
 * Call a chat event in a connected project. Chunks, complete responses, widgets, attachments and session state are exposed while the remote chat streams.
 * @param flowRemoteAppId (optional) — Connected project to invoke the event in
 * @param flowRemoteEvent (optional) — Chat event of the selected project
 * @param flowRemoteEventMeta (optional) — Auto-filled by the editor when an event is selected. Drives the typed pins.
 * @param message (optional) — User message appended to the conversation
 * @param history (optional) — Prior conversation history
 * @param localSession (optional) — State local to this chat session
 * @param globalSession (optional) — State shared for the remote chat user
 * @param tools (optional) — Tool ids the remote assistant may use
 * @param actions (optional) — User actions included with the chat request
 * @param attachments (optional) — Attachments included with the chat request
 * @param user (optional) — User information forwarded to the remote chat
 * @param timeoutSeconds (optional) — Maximum time to wait for the remote request to finish
 * @returns chunk — Latest streamed response chunk
 * @returns response — Latest complete model response
 * @returns responseText — Text of the latest complete response
 * @returns widgets — Widgets emitted by the remote chat update
 * @returns attachmentsOut — Attachments emitted by the remote chat update
 * @returns actionsOut — Actions emitted by the remote chat update
 * @returns plan — Latest streamed reasoning plan
 * @returns localSessionOut — Latest remote local session state
 * @returns globalSessionOut — Latest remote global session state
 * @returns usageStat — Latest model usage update
 * @returns modelId — Model reported by the remote chat
 * @returns eventType — Type of the latest streamed remote event
 * @returns eventPayload — Raw payload of the latest streamed remote event
 * @returns runId — Remote run id
 * @returns status — Final run status
 * @impure has side effects / drives control flow
 */
declare function callRemoteChat({ flowRemoteAppId?: string, flowRemoteEvent?: string, flowRemoteEventMeta?: string, message?: string, history?: Struct, localSession?: Struct, globalSession?: Struct, tools?: string[], actions?: Struct[], attachments?: Struct[], user?: Struct, timeoutSeconds?: int }): { chunk: Struct, response: Struct, responseText: string, widgets: Struct[], attachmentsOut: Struct[], actionsOut: Struct[], plan: Struct, localSessionOut: Struct, globalSessionOut: Struct, usageStat: Struct, modelId: string, eventType: string, eventPayload: any, runId: string, status: string };

/**
 * Invoke a chat, API or MCP event of a connected project. Pins adapt to the selected event. The project must have granted this app a role that allows executing events.
 * @param flowRemoteAppId (optional) — Connected project to invoke the event in
 * @param flowRemoteEvent (optional) — Event of the selected project to invoke
 * @param flowRemoteEventMeta (optional) — Auto-filled by the editor when an event is selected. Drives the typed pins.
 * @param payload — Input payload passed to the remote event
 * @param waitForResult (optional) — Wait for the remote run to finish and return its result
 * @param timeoutSeconds (optional) — Maximum time to wait for the remote request to finish
 * @returns runId — Remote run id
 * @returns status — Final run status
 * @returns result — Result payload of the remote run
 * @impure has side effects / drives control flow
 */
declare function callRemoteEvent({ flowRemoteAppId?: string, flowRemoteEvent?: string, flowRemoteEventMeta?: string, payload: any, waitForResult?: bool, timeoutSeconds?: int }): { runId: string, status: string, result: any };


// === Events/Widget ===

/**
 * Extracts a field value from an action context payload by field name
 * @param actionContext — The action context payload from a Widget Action Event
 * @param fieldName (optional) — The name of the field to extract
 * @returns value — The extracted field value (null if field does not exist)
 */
declare function eventsExtractActionContext({ actionContext: Struct, fieldName?: string }): any;

/**
 * Extracts a component's current value from the input values payload by component ID
 * @param inputValues — The input values payload from a Widget Action Event
 * @param componentId (optional) — The ID of the component whose value to extract
 * @returns value — The current value of the component (null if not found)
 */
declare function eventsExtractInputValue({ inputValues: Struct, componentId?: string }): any;

