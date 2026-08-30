// Events — FlowScript node declarations (generated, do not edit).
// One `function` per catalog node, grouped by FlowScript namespace. Call a node as
// `ns::alias({ pin: value })`, or write `use ns::*` once at the top of a .flow file and
// call `alias({ pin: value })`. A `this: T` parameter marks the receiver pin: such a node
// is also a method on that value (`x.alias(...)`, remaining inputs positional or named).
// JSDoc tags carry the node type (`@node`), the receiver pin (`@receiver`) and the legacy
// camelCase spelling (`@alias`), which is still accepted.

declare namespace chat {
    // === Events/Chat ===

    /**
     * Pulls down image, audio, video, and document attachments referenced in the latest chat message
     * @node ai_gen_llm_history_extract_attachments @alias aiGenLlmHistoryExtractAttachments
     * @param history — Chat history whose final message may contain media parts
     * @param attachments (optional) — Existing attachments to merge with new downloads
     * @returns paths — Virtual file paths pointing to cached attachments
     * @impure has side effects / drives control flow
     */
    function extractAttachments({ history: Struct, attachments?: Struct[] }): Struct[];

    /**
     * Pushes a response chunk to the chat
     * @node events_chat_push_attachment @alias eventsChatPushAttachment
     * @param attachment — Attachment to the Chat
     * @impure has side effects / drives control flow
     */
    function pushAttachment({ attachment: Struct }): void;

    /**
     * Pushes a response chunk to the chat
     * @node events_chat_push_attachments @alias eventsChatPushAttachments
     * @param attachments — Attachment to the Chat
     * @impure has side effects / drives control flow
     */
    function pushAttachments({ attachments: Struct[] }): void;

    /**
     * Pushes a response chunk to the chat
     * @node events_chat_push_response_chunk @alias eventsChatPushResponseChunk
     * @param chunk — Generated Chat Chunk
     * @impure has side effects / drives control flow
     */
    function pushChunk({ chunk: Struct }): void;

    /**
     * Pushes a new global session to the chat. The session persists for all chat sessions.
     * @node events_chat_push_global_session @alias eventsChatPushGlobalSession
     * @param globalSession (optional) — Generic Struct Type
     * @impure has side effects / drives control flow
     */
    function pushGlobalSession({ globalSession?: Struct }): void;

    /**
     * Pushes a new local session to the chat. The session persists for one chat session.
     * @node events_chat_push_local_session @alias eventsChatPushLocalSession
     * @param localSession (optional) — Generic Struct Type
     * @impure has side effects / drives control flow
     */
    function pushLocalSession({ localSession?: Struct }): void;

    /**
     * Pushes reasoning tokens to the current step
     * @node events_chat_push_reasoning @alias eventsChatPushReasoning
     * @param reasoning — Reasoning text to append to current step
     * @impure has side effects / drives control flow
     */
    function pushReasoning({ reasoning: string }): void;

    /**
     * Pushes a response to the chat
     * @node events_chat_push_response @alias eventsChatPushResponse
     * @param response — Chat Response
     * @impure has side effects / drives control flow
     */
    function pushResponse({ response: Struct }): void;

    /**
     * Pushes a single LLM usage stat to the chat for transparent model usage display
     * @node events_chat_push_stat @alias eventsChatPushStat
     * @param stepName — Label for this step (e.g. 'Summarization', 'Tool Selection')
     * @param stat — LLM usage statistics
     * @impure has side effects / drives control flow
     */
    function pushStat({ stepName: string, stat: Struct }): void;

    /**
     * Pushes multiple LLM usage stats to the chat at once
     * @node events_chat_push_stats @alias eventsChatPushStats
     * @param stepName — Label for this batch of stats (e.g. 'Agent Execution', 'Pipeline')
     * @param inputStats — Array of LLM usage statistics
     * @impure has side effects / drives control flow
     */
    function pushStats({ stepName: string, inputStats: Struct[] }): void;

    /**
     * Starts a new plan step with title and description
     * @node events_chat_push_step @alias eventsChatPushStep
     * @param title — Step title
     * @param description — Step description (optional)
     * @returns stepId — The ID of the created step
     * @impure has side effects / drives control flow
     */
    function pushStep({ title: string, description: string }): int;

    /**
     * Appends text to the current step's reasoning
     * @node events_chat_push_text_to_step @alias eventsChatPushTextToStep
     * @param text — Text to append to current step
     * @impure has side effects / drives control flow
     */
    function pushTextToStep({ text: string }): void;

    /**
     * Embeds an a2ui widget instance into the chat message. Connect the Element Ref of an Instantiate Widget node.
     * @node events_chat_push_widget @alias eventsChatPushWidget
     * @param elementRef — Widget instance to embed (from Instantiate Widget)
     * @impure has side effects / drives control flow
     */
    function pushWidget({ elementRef: Struct }): void;

    /**
     * Embeds multiple a2ui widget instances into the chat message. Add an Element Ref pin for each Instantiate Widget node.
     * @node events_chat_push_widgets @alias eventsChatPushWidgets
     * @param elementRef — Widget instance to embed (from Instantiate Widget). Add more pins for multiple widgets.
     * @impure has side effects / drives control flow
     */
    function pushWidgets({ elementRef: Struct }): void;

    /**
     * Removes a step from the plan by its ID
     * @node events_chat_remove_step @alias eventsChatRemoveStep
     * @param stepId — ID of the step to remove
     * @impure has side effects / drives control flow
     */
    function removeStep({ stepId: int }): void;

    // === Events/Chat/Attachments ===

    /**
     * Creates an attachment from a FlowPath with optional metadata
     * @node events_chat_attachment_from_path @alias eventsChatAttachmentFromPath
     * @param path — FlowPath to create attachment from
     * @param name (optional) — Display name for the attachment (optional, defaults to filename)
     * @param previewText (optional) — Preview text/description for the attachment (optional)
     * @param page (optional) — Page number reference (optional, for documents)
     * @param anchor (optional) — Anchor/section reference within the document (optional)
     * @param expiration (optional) — Expiration time for the signed URL
     * @returns attachment — The created attachment
     * @impure has side effects / drives control flow
     */
    function attachmentFromPath({ path: Struct, name?: string, previewText?: string, page?: int, anchor?: string, expiration?: int }): Struct;

    /**
     * Get the URL from an attachment
     * @node events_chat_attachment_from_signed_url @alias eventsChatAttachmentFromSignedUrl
     * @param signedUrl
     * @returns attachment — Attachment to the Chat
     */
    function attachmentFromSignedUrl({ signedUrl: string }): Struct;

    /**
     * Get the URL from an attachment
     * @node events_chat_attachment_to_signed_url @receiver attachment @alias eventsChatAttachmentToSignedUrl
     * @param attachment — Attachment to the Chat (receiver: `this` in `x.toSignedUrl(...)`)
     * @returns signedUrl
     * @returns success
     */
    function toSignedUrl(this: Attachment, { attachment: Struct }): { signedUrl: string, success: bool };

    // === Events/Chat/Interaction ===

    /**
     * Builds a JSON Schema form from a referenced callback function's pins and executes it with typed submitted values.
     * @node interaction_form @alias interactionForm
     * @param name (optional) — Display name for this interaction
     * @param description (optional) — Prompt shown to the user
     * @param ttlSeconds (optional) — How long to wait for response
     * @returns response — JSON object with pin name -> typed value mappings
     * @returns responded — Whether the user responded (vs timeout)
     * @impure has side effects / drives control flow
     */
    function askForm({ name?: string, description?: string, ttlSeconds?: int }): { response: string, responded: bool };

    /**
     * Request the user to pick one or more options. Pauses execution until a response or timeout.
     * @node interaction_multiple_choice @alias interactionMultipleChoice
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
    function askMultipleChoice({ name?: string, description?: string, options?: string, options?: string, minSelections?: int, maxSelections?: int, ttlSeconds?: int }): { response: string[], responded: bool };

    /**
     * Request the user to pick one option. Pauses execution until a response or timeout.
     * @node interaction_single_choice @alias interactionSingleChoice
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
    function askSingleChoice({ name?: string, description?: string, options?: string, options?: string, allowFreeform?: bool, ttlSeconds?: int }): { response: string, responded: bool };
}

declare namespace events {
    // === Events ===

    /**
     * A simple Chat event
     * @node events_chat @alias eventsChat
     * @returns history — Chat History
     * @returns localSession — Local to the Chat
     * @returns globalSession — Global to the User
     * @returns tools — Tools requested by the user
     * @returns actions — User Actions
     * @returns attachments — User Attachments or References
     * @returns user — User Information
     * @impure has side effects / drives control flow
     */
    function chat(): { history: Struct, localSession: Struct, globalSession: Struct, tools: string[], actions: Struct[], attachments: Struct[], user: Struct };

    /**
     * A generic event without input or output
     * @node events_generic @alias eventsGeneric
     * @returns payload — The payload of the event
     * @impure has side effects / drives control flow
     */
    function generic(): Struct;

    /**
     * A simple event without input or output
     * @node events_simple @alias eventsSimple
     * @impure has side effects / drives control flow
     */
    function simple(): void;

    /**
     * Entry point triggered when a widget action is invoked. Provides action context data.
     * @node events_widget_action @alias eventsWidgetAction
     * @param actionId (optional) — The action identifier that triggers this event (e.g., 'clicked_delete', 'clicked_open')
     * @returns widgetInstanceId — The unique ID of the widget instance that triggered the action
     * @returns eventName — The action ID / event name that was triggered
     * @returns actionContext — The context data passed from the widget action (JSON object with field values)
     * @returns inputValues — Map of component ID to current value for components marked as event-relevant
     * @impure has side effects / drives control flow
     */
    function widgetAction({ actionId?: string }): { widgetInstanceId: string, eventName: string, actionContext: Struct, inputValues: Struct };

    // === Events/Generic ===

    /**
     * Return a result
     * @node events_generic_return_result @alias eventsGenericReturnResult
     * @param response — Chat Response
     * @impure has side effects / drives control flow
     */
    function returnResult({ response: any }): void;

    // === Events/Widget ===

    /**
     * Extracts a field value from an action context payload by field name
     * @node events_extract_action_context @alias eventsExtractActionContext
     * @param actionContext — The action context payload from a Widget Action Event
     * @param fieldName (optional) — The name of the field to extract
     * @returns value — The extracted field value (null if field does not exist)
     */
    function extractActionContext({ actionContext: Struct, fieldName?: string }): any;

    /**
     * Extracts a component's current value from the input values payload by component ID
     * @node events_extract_input_value @alias eventsExtractInputValue
     * @param inputValues — The input values payload from a Widget Action Event
     * @param componentId (optional) — The ID of the component whose value to extract
     * @returns value — The current value of the component (null if not found)
     */
    function extractInputValue({ inputValues: Struct, componentId?: string }): any;
}

declare namespace remote {
    // === Events/Remote ===

    /**
     * Call an internal REST API exposed by a connected project and return its status, headers and response body.
     * @node call_remote_api @alias callRemoteApi
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
    function callApi({ flowRemoteAppId?: string, flowRemoteEvent?: string, flowRemoteEventMeta?: string, route: string, query?: any, body?: any, headers?: any, timeoutSeconds?: int }): { status: int, responseHeaders: any, response: any, file: Struct };

    /**
     * Call a chat event in a connected project. Chunks, complete responses, widgets, attachments and session state are exposed while the remote chat streams.
     * @node call_remote_chat @alias callRemoteChat
     * @param flowRemoteAppId (optional) — Connected project to invoke the event in
     * @param flowRemoteEvent (optional) — Chat event of the selected project
     * @param flowRemoteEventMeta (optional) — Auto-filled by the editor when an event is selected. Drives the typed pins.
     * @param history (optional) — Conversation to send, including the new user message
     * @param localSession (optional) — State local to this chat session
     * @param globalSession (optional) — State shared for the remote chat user
     * @param tools (optional) — Tool ids the remote assistant may use
     * @param actions (optional) — User actions included with the chat request
     * @param attachments (optional) — Attachments included with the chat request
     * @param user (optional) — User information forwarded to the remote chat
     * @param timeoutSeconds (optional) — Maximum time to wait for the remote request to finish
     * @returns chunk — Latest streamed response chunk
     * @returns response — Complete model response
     * @returns responseText — Text of the complete response
     * @returns widgets — Widgets emitted by the remote chat
     * @returns attachmentsOut — Attachments emitted by the remote chat
     * @returns actionsOut — Actions emitted by the remote chat
     * @returns localSessionOut — Latest remote local session state
     * @returns globalSessionOut — Latest remote global session state
     * @returns modelId — Model reported by the remote chat
     * @returns runId — Remote run id
     * @returns status — Final run status
     * @returns plan — Latest streamed reasoning plan
     * @returns usageStat — Latest model usage update
     * @returns eventType — Type of the latest streamed remote event
     * @returns eventPayload — Raw payload of the latest streamed remote event
     * @impure has side effects / drives control flow
     */
    function callChat({ flowRemoteAppId?: string, flowRemoteEvent?: string, flowRemoteEventMeta?: string, history?: Struct, localSession?: Struct, globalSession?: Struct, tools?: string[], actions?: Struct[], attachments?: Struct[], user?: Struct, timeoutSeconds?: int }): { chunk: Struct, response: Struct, responseText: string, widgets: Struct[], attachmentsOut: Struct[], actionsOut: Struct[], localSessionOut: Struct, globalSessionOut: Struct, modelId: string, runId: string, status: string, plan: Struct, usageStat: Struct, eventType: string, eventPayload: any };

    /**
     * Invoke a chat, API or MCP event of a connected project. Pins adapt to the selected event. The project must have granted this app a role that allows executing events.
     * @node call_remote_event @alias callRemoteEvent
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
    function callEvent({ flowRemoteAppId?: string, flowRemoteEvent?: string, flowRemoteEventMeta?: string, payload: any, waitForResult?: bool, timeoutSeconds?: int }): { runId: string, status: string, result: any };
}
