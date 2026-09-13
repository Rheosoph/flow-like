import { buildWorkflowFrontendContext } from "@flow-like/flow-like-ui/components/a2ui/workflow-payload";
import { classifyAppEventInterface } from "@flow-like/flow-like-ui/components/global-chat/app-event-interface";
import type {
	IAttachment,
	IMessage,
} from "@flow-like/flow-like-ui/components/interfaces/chat-default/chat-db";
import { processChatEvents } from "@flow-like/flow-like-ui/components/interfaces/chat-default/event-processor";
import { joinContentText } from "@flow-like/flow-like-ui/components/interfaces/chat-default/inline-segments";
import type { ExecutionEngineProvider } from "@flow-like/flow-like-ui/lib/execution-engine";
import { Response } from "@flow-like/flow-like-ui/lib/llm/response";
import type { IIntercomEvent } from "@flow-like/flow-like-ui/lib/schema/events/intercom-event";
import type { IEvent } from "@flow-like/flow-like-ui/lib/schema/flow/event";
import type { IInteractionRequest } from "@flow-like/flow-like-ui/lib/schema/interaction";
import { IRole } from "@flow-like/flow-like-ui/lib/schema/llm/history";
import type { IBackendState } from "@flow-like/flow-like-ui/state/backend-state";
import {
	type NativePendingAction,
	executeNativeMcpOperation,
	nativeQuickActionPayload,
} from "./native-integration";

export interface NativeEventOutput {
	text: string;
	json?: string;
}

export class NativeEventInteractionRequired extends Error {}

/** Return only explicit workflow outputs and visible chat text, never execution logs. */
export function createNativeEventOutput(
	appId: string,
	event: IEvent,
	sessionId: string,
) {
	const message: IMessage = {
		id: `${sessionId}-response`,
		appId,
		sessionId,
		timestamp: Date.now(),
		inner: { role: IRole.Assistant, content: "" },
		files: [],
		tools: [],
		actions: [],
	};
	let chat = {
		intermediateResponse: Response.default(),
		responseMessage: message,
		attachments: new Map<string, IAttachment>(),
		tmpLocalState: null as unknown,
		tmpGlobalState: null as unknown,
		done: false,
		appId,
		eventId: event.id,
		sessionId,
	};
	let returned: unknown;
	let hasReturn = false;
	let text = "";
	let tooLarge = false;
	let failure: string | undefined;
	let completed = false;
	const seen = new Set<string>();
	const isChat = classifyAppEventInterface(event) === "chat";
	return {
		add(batch: IIntercomEvent[]): IInteractionRequest[] {
			const fresh = batch.filter((item) => {
				if (item.event_id && seen.has(item.event_id)) return false;
				if (item.event_id) seen.add(item.event_id);
				return true;
			});
			if (tooLarge) return [];
			const interactions: IInteractionRequest[] = [];
			if (isChat) {
				const result = processChatEvents(fresh, chat);
				chat = { ...chat, ...result };
				interactions.push(...(result.interactions ?? []));
			}
			for (const item of fresh) {
				if (!isChat && item.event_type === "interaction_request")
					interactions.push(item.payload as IInteractionRequest);
				if (item.event_type === "completed") {
					completed = true;
					if (
						typeof item.payload?.status !== "string" ||
						item.payload.status.toLowerCase() !== "completed"
					)
						failure =
							"The Event did not complete successfully. Open its run in Flow Like for details.";
				}
				if (item.event_type === "error")
					failure = "The Event failed. Open its run in Flow Like for details.";
				if (
					item.event_type === "generic_result" ||
					item.event_type === "return" ||
					item.event_type === "output" ||
					(item.event_type === "intercom" && item.payload?.type === "return")
				) {
					hasReturn = true;
					returned = item.payload;
				} else if (
					item.event_type === "text_output" ||
					item.event_type === "stream_text"
				) {
					const chunk =
						typeof item.payload === "string"
							? item.payload
							: item.payload?.text;
					if (typeof chunk === "string") text += chunk;
				}
			}
			tooLarge =
				new TextEncoder().encode(
					JSON.stringify({ returned, text, content: message.inner.content }),
				).length > 196_608;
			return interactions;
		},
		assertCompleted(requireTerminal: boolean) {
			if (requireTerminal && !completed)
				throw new Error(
					"The Event connection ended before completion was confirmed. Check the run in Flow Like before retrying.",
				);
		},
		result(): NativeEventOutput {
			if (failure) throw new Error(failure);
			if (tooLarge)
				throw new Error(
					"The Event response is too large for Shortcuts. Return a smaller output.",
				);
			const visibleText = joinContentText(message.inner.content);
			if (isChat) return { text: visibleText || text };
			if (hasReturn)
				return {
					text: typeof returned === "string" ? returned : text,
					json: JSON.stringify(returned) ?? "null",
				};
			return { text: visibleText || text };
		},
	};
}

export async function executeNativeEventWithResult(options: {
	request: NativePendingAction;
	appId: string;
	event: IEvent;
	input?: string;
	backend: IBackendState;
	engine: Pick<ExecutionEngineProvider, "executeEvent">;
	isCurrent: () => boolean;
	onUpdate?: (
		output: NativeEventOutput,
		interactions: IInteractionRequest[],
	) => void;
}): Promise<NativeEventOutput> {
	const { request, appId, event, input, backend, engine, isCurrent, onUpdate } =
		options;
	const assertCurrent = () => {
		if (!isCurrent())
			throw new Error("The active account or workspace changed.");
		if (
			request.responseDeadline !== undefined &&
			(!Number.isFinite(Date.parse(request.responseDeadline)) ||
				Date.parse(request.responseDeadline) <= Date.now())
		)
			throw new Error("This Shortcut request expired before it could run.");
	};
	assertCurrent();
	const kind = classifyAppEventInterface(event);
	if (kind === "page" || kind === "unavailable")
		throw new NativeEventInteractionRequired(
			"Open this Event in Flow Like to use its page. Pages do not return a Shortcut response.",
		);
	if (event.event_type === "mcp") {
		let args: unknown;
		try {
			args = JSON.parse(input?.trim() || "{}");
		} catch {
			throw new Error("MCP input must be a JSON object.");
		}
		const result = await executeNativeMcpOperation(
			backend,
			appId,
			event.id,
			request.action.operation || "",
			args,
			() => {
				assertCurrent();
				return true;
			},
		);
		if (!isCurrent())
			throw new Error("The active account or workspace changed.");
		if (result.status === "error")
			throw new Error("The MCP tool returned an error.");
		return { text: "", json: JSON.stringify(result.result) };
	}
	const sessionId = `native-${request.id}`;
	let payload: Record<string, unknown>;
	if (kind === "chat") {
		if (!input?.trim())
			throw new NativeEventInteractionRequired(
				"Provide a question for this chat Event.",
			);
		payload = {
			chat_id: sessionId,
			messages: [{ role: "user", content: input }],
			local_session: {},
			global_session: {},
			actions: [],
			tools: [],
			attachments: [],
			...(await buildWorkflowFrontendContext(appId, event.id)),
		};
	} else {
		const values = nativeQuickActionPayload(event, input);
		if (values === null)
			throw new NativeEventInteractionRequired(
				"This Event needs more inputs. Supply its required inputs in the Shortcut or open its form in Flow Like.",
			);
		payload = values;
	}
	assertCurrent();
	const output = createNativeEventOutput(appId, event, sessionId);
	const metadata = await engine.executeEvent(sessionId, {
		appId,
		eventId: event.id,
		payload: { id: event.node_id, payload },
		title: event.name,
		beforeDispatch: assertCurrent,
		interfaceType: kind === "chat" ? "chat" : "generic",
		path: `/use?id=${encodeURIComponent(appId)}&eventId=${encodeURIComponent(event.id)}`,
		onLiveEvents: (batch) => {
			if (!isCurrent()) return;
			const interactions = output.add(batch);
			try {
				onUpdate?.(output.result(), interactions);
			} catch {
				/* A large output is reported after the run finishes. */
			}
		},
	});
	if (!isCurrent()) throw new Error("The active account or workspace changed.");
	output.assertCompleted(metadata === undefined);
	const result = output.result();
	if (kind === "chat" && !result.text.trim())
		throw new NativeEventInteractionRequired(
			"This chat Event returned no text. Open the app to view its content or continue the request.",
		);
	return result;
}
