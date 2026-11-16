import type {
	IChatWidget,
	IMessage,
} from "../components/interfaces/chat-default/chat-db";
import type {
	IInboxFileHandle,
	IInboxUiPart,
} from "../state/backend-state/inbox-state";
import { isRecord } from "./response-shape";

const isFilled = (value: unknown): value is string =>
	typeof value === "string" && value.length > 0;

/**
 * A widget with the app run that pushed it (`{origin, widget}`, snake_case on the wire), so its
 * actions run against that board. Anything else is null.
 */
export function appWidgetOf(value: unknown): IChatWidget | null {
	if (!isRecord(value)) return null;
	const { origin, widget } = value;
	if (!isRecord(origin) || !isFilled(origin.app_id)) return null;
	if (
		!isRecord(widget) ||
		!isFilled(widget.instance_id) ||
		typeof widget.widget_id !== "string" ||
		typeof widget.surface_id !== "string" ||
		!isRecord(widget.component) ||
		(widget.updates !== undefined && !Array.isArray(widget.updates))
	)
		return null;
	return {
		instance_id: widget.instance_id,
		widget_id: widget.widget_id,
		surface_id: widget.surface_id,
		component: widget.component,
		...(Array.isArray(widget.updates) ? { updates: widget.updates } : {}),
		origin: {
			appId: origin.app_id,
			...(isFilled(origin.board_id) ? { boardId: origin.board_id } : {}),
			...(isFilled(origin.event_id) ? { eventId: origin.event_id } : {}),
		},
	};
}

/** An `{"type":"app_widget",…}` document, inline in a ui part or in the file its handle names. */
export function appWidgetDocumentOf(value: unknown): IChatWidget | null {
	return isRecord(value) && value.type === "app_widget"
		? appWidgetOf(value)
		: null;
}

const isStoredHandle = (value: unknown): value is IInboxFileHandle =>
	isRecord(value) && isFilled(value.app_id) && isFilled(value.path);

/** A ui part the chat can show: an inline app widget, or a handle to its document. */
export function isInboxUiPart(part: unknown): part is IInboxUiPart {
	if (!isRecord(part) || part.kind !== "ui" || typeof part.id !== "string")
		return false;
	return part.a2ui === undefined
		? isStoredHandle(part.handle)
		: appWidgetDocumentOf(part.a2ui) !== null;
}

/** The widget without the app run it came from, so none of its actions can run. */
export function readOnlyWidget(widget: IChatWidget): IChatWidget {
	const { origin: _origin, ...rest } = widget;
	return rest;
}

/**
 * A message's ui parts as the chat shows them: inline widgets, and stored documents to load.
 * `actions: false` shows them read-only, for widgets someone else's bot sent: their actions would
 * run an app as the reader, with whatever the widget's author put in them.
 */
export function messageWidgetsOf(
	parts: readonly IInboxUiPart[] | undefined,
	{ actions = true }: { actions?: boolean } = {},
): Pick<IMessage, "widgets" | "widget_documents" | "widget_actions"> {
	const widgets: IChatWidget[] = [];
	const documents: IInboxFileHandle[] = [];
	for (const part of parts ?? []) {
		const widget = part.a2ui ? appWidgetDocumentOf(part.a2ui) : null;
		if (widget) widgets.push(actions ? widget : readOnlyWidget(widget));
		else if (part.handle) documents.push(part.handle);
	}
	return {
		...(widgets.length > 0 ? { widgets } : {}),
		...(documents.length > 0 ? { widget_documents: documents } : {}),
		...(!actions && documents.length > 0 ? { widget_actions: false } : {}),
	};
}
