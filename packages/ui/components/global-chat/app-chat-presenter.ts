import { createId } from "@paralleldrive/cuid2";
import { Response } from "../../lib/llm/response";
import type { IIntercomEvent } from "../../lib/schema/events/intercom-event";
import { IRole } from "../../lib/schema/llm/history";
import { useGlobalChatStore } from "../../state/global-chat/global-chat-store";
import { foldA2UIServerMessage } from "../a2ui/fold-surfaces";
import { getFrontendStateStore } from "../a2ui/frontend-state";
import type { A2UIServerMessage, Surface } from "../a2ui/types";
import type { IAttachment, IMessage } from "../interfaces/chat-default/chat-db";
import {
	type ProcessChatEventsResult,
	processChatEvents,
} from "../interfaces/chat-default/event-processor";
import { joinContentText } from "../interfaces/chat-default/inline-segments";
import {
	refreshMainChatWidget,
	resolveMainChatWidget,
} from "../interfaces/chat-default/main-widget-state";
import type { RunScope } from "./global-tool-bridge";

export type AppChatScope = Pick<
	RunScope,
	| "runId"
	| "addSubWidgets"
	| "setSubMainWidget"
	| "addSubAttachments"
	| "addSubUsageStats"
>;

export interface AppChatPresenterOptions {
	scope: AppChatScope;
	appId: string;
	event: { id: string; name?: string; board_id: string };
	chatId?: string;
	resolveAppName?: () => Promise<string | undefined>;
	onProcessed?: (result: ProcessChatEventsResult) => void;
}

export interface AppChatAttachmentSummary {
	url: string;
	name?: string;
	type?: string;
}

export interface AppChatOutcome {
	files: (IAttachment | string)[];
	attachments: AppChatAttachmentSummary[];
	text: string;
	widgetCount: number;
	surfaceCount: number;
}

export function summarizeAppChatFiles(
	files: readonly IAttachment[],
): AppChatAttachmentSummary[] {
	return files.map((file) =>
		typeof file === "string"
			? { url: file }
			: { url: file.url, name: file.name, type: file.type },
	);
}

/** Outcome fields reported to the model; omitted when the app pushed nothing. */
export function appChatOutcomeFields(outcome: AppChatOutcome) {
	return {
		attachments: outcome.attachments.length ? outcome.attachments : undefined,
		pushed_widget_count: outcome.widgetCount || undefined,
		pushed_surface_count: outcome.surfaceCount || undefined,
	};
}

export function createAppChatPresenter(options: AppChatPresenterOptions) {
	const { scope, appId, event, onProcessed } = options;
	const chatId = options.chatId ?? createId();
	let responseMessage: IMessage = {
		id: createId(),
		appId,
		sessionId: chatId,
		inner: { role: IRole.Assistant, content: "" },
		files: [],
		tools: [],
		actions: [],
		timestamp: Date.now(),
	};
	const responseMessages = [responseMessage];
	let intermediate = Response.default();
	let attachments = new Map<string, IAttachment>();
	let done = false;
	let surfaces = new Map<string, Surface>();

	const widgetOrigin = {
		appId,
		boardId: event.board_id,
		eventId: event.id,
	};
	const currentMainWidget = () => {
		const state = useGlobalChatStore.getState();
		const ownerRun = scope.runId ? state.runs[scope.runId] : undefined;
		return ownerRun
			? resolveMainChatWidget(
					[
						...state.messages,
						...Object.values(state.runs).flatMap((run) =>
							run.message ? [run.message] : [],
						),
					],
					ownerRun.conversationId,
				)
			: undefined;
	};
	const inheritedMainWidget = currentMainWidget();
	if (
		inheritedMainWidget?.widget?.origin?.appId === appId &&
		inheritedMainWidget.widget.origin.boardId === event.board_id
	) {
		responseMessage.main_widget = inheritedMainWidget;
	}
	let lastPublishedMainWidget = responseMessage.main_widget;

	const publishWidgets = () => {
		const widgets = responseMessages.flatMap(
			(message) => message.widgets ?? [],
		);
		if (widgets.length) {
			scope.addSubWidgets(
				widgets.map((widget) => ({ ...widget, origin: widgetOrigin })),
			);
		}
		const mainWidget = responseMessage.main_widget;
		if (mainWidget && mainWidget !== lastPublishedMainWidget) {
			lastPublishedMainWidget = mainWidget;
			scope.setSubMainWidget({
				...mainWidget,
				widget: mainWidget.widget
					? { ...mainWidget.widget, origin: widgetOrigin }
					: null,
			});
		}
	};

	const push = (batch: Pick<IIntercomEvent, "event_type" | "payload">[]) => {
		refreshMainChatWidget(responseMessage, currentMainWidget());
		const result = processChatEvents(batch, {
			intermediateResponse: intermediate,
			responseMessage,
			responseMessages,
			attachments,
			tmpLocalState: null,
			tmpGlobalState: null,
			done,
			appId,
			eventId: event.id,
			sessionId: chatId,
		});
		intermediate = result.intermediateResponse;
		responseMessage = result.responseMessage;
		attachments = result.attachments;
		done = result.done;
		onProcessed?.(result);
		for (const item of batch) {
			if (item?.event_type !== "a2ui" || !item.payload) continue;
			if (getFrontendStateStore(appId).handleMessage(item.payload)) continue;
			surfaces = foldA2UIServerMessage(
				surfaces,
				item.payload as A2UIServerMessage,
			);
		}
		publishWidgets();
		if (result.interactions?.length) {
			useGlobalChatStore.getState().addInteractions(result.interactions);
		}
	};

	const finish = async (): Promise<AppChatOutcome> => {
		if (surfaces.size > 0) {
			const appName = await options.resolveAppName?.().catch(() => undefined);
			useGlobalChatStore.getState().addInlineAppSurface(
				{
					appId,
					name: appName || event.name || appId,
					surfaces: Array.from(surfaces.values()),
				},
				scope.runId,
			);
		}
		const text = responseMessages
			.map((message) => joinContentText(message.inner.content))
			.filter(Boolean)
			.join("\n\n");
		const files = responseMessages.flatMap((message) => message.files ?? []);
		if (files.length > 0) scope.addSubAttachments(files);
		const usageStats = responseMessages.flatMap(
			(message) => message.usage_stats ?? [],
		);
		if (usageStats.length) scope.addSubUsageStats(usageStats);
		return {
			files,
			attachments: summarizeAppChatFiles(files),
			text,
			widgetCount: responseMessages.reduce(
				(count, message) => count + (message.widgets?.length ?? 0),
				0,
			),
			surfaceCount: surfaces.size,
		};
	};

	return { chatId, push, finish, responseMessages };
}
