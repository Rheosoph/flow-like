import { createId } from "@paralleldrive/cuid2";
import { Response } from "../../../lib/llm/response";
import type { IInteractionRequest } from "../../../lib/schema/interaction";
import {
	IContentType,
	type IContent,
	IRole,
} from "../../../lib/schema/llm/history";
import type { IResponseMessage } from "../../../lib/schema/llm/response";
import type {
	IAttachment,
	IChatUsageStat,
	IChatWidget,
	IMessage,
	IPlanStep,
} from "./chat-db";

export interface ProcessChatEventsResult {
	intermediateResponse: Response;
	responseMessage: IMessage;
	attachments: Map<string, IAttachment>;
	tmpLocalState: any;
	tmpGlobalState: any;
	done: boolean;
	shouldUpdate: boolean;
	interactions?: IInteractionRequest[];
}

function visibleResponseContent(
	message: IResponseMessage,
): string | IContent[] {
	const parts = message.content_parts ?? [];
	if (parts.length === 0) return message.content ?? "";
	if (
		message.content &&
		!parts.some((part) => part.type === IContentType.Text)
	) {
		return [{ type: IContentType.Text, text: message.content }, ...parts];
	}
	return [...parts];
}

interface BackendReasoning {
	plan: [number, string][];
	current_step: number;
	current_message: string;
}

function hasVisibleReasoning(
	reasoning: string | undefined,
): reasoning is string {
	return Boolean(reasoning && reasoning.trim() !== "");
}

function hasStructuredReasoning(reasoning: string): boolean {
	return reasoning
		.split(/\r?\n/)
		.map((line) => line.trim())
		.filter(Boolean)
		.some(
			(line) =>
				line.startsWith("```") ||
				/^#{1,6}\s/.test(line) ||
				/^[-*+]\s/.test(line) ||
				/^\d+\.\s/.test(line) ||
				/^>\s/.test(line) ||
				/^\|.*\|$/.test(line),
		);
}

function looksLikeTokenizedReasoning(reasoning: string): boolean {
	const lines = reasoning
		.split(/\r?\n/)
		.map((line) => line.trim())
		.filter(Boolean);

	if (lines.length < 6 || hasStructuredReasoning(reasoning)) {
		return false;
	}

	const shortLineCount = lines.filter((line) => {
		const wordCount = line.split(/\s+/).filter(Boolean).length;
		return wordCount <= 3 && line.length <= 24;
	}).length;

	return shortLineCount / lines.length >= 0.7;
}

function normalizeReasoningWhitespace(reasoning: string): string {
	let normalized = "";
	let pendingSpace = false;

	for (const ch of reasoning) {
		if (/\s/.test(ch)) {
			pendingSpace = normalized.length > 0;
			continue;
		}

		if (pendingSpace && !/[.,;:!?)}\]'\"]/.test(ch)) {
			normalized += " ";
		}

		pendingSpace = false;
		normalized += ch;
	}

	return normalized;
}

function sanitizeReasoningForDisplay(reasoning: string): string {
	return looksLikeTokenizedReasoning(reasoning)
		? normalizeReasoningWhitespace(reasoning)
		: reasoning;
}

function appendFallbackReasoningStep(
	responseMessage: IMessage,
	reasoning: string,
	replace = false,
) {
	const sanitizedReasoning = sanitizeReasoningForDisplay(reasoning);

	if (
		(!responseMessage.plan_steps || responseMessage.plan_steps.length === 0) &&
		!hasVisibleReasoning(sanitizedReasoning)
	) {
		return;
	}

	if (!responseMessage.plan_steps || responseMessage.plan_steps.length === 0) {
		responseMessage.plan_steps = [
			{
				id: "step-0",
				title: "Thinking",
				status: "progress",
				reasoning: sanitizedReasoning,
			},
		];
		responseMessage.current_step_id = "step-0";
		return;
	}

	const currentStep =
		responseMessage.plan_steps.find(
			(step) => step.id === responseMessage.current_step_id,
		) ??
		responseMessage.plan_steps.find((step) => step.status === "progress") ??
		responseMessage.plan_steps[responseMessage.plan_steps.length - 1];

	if (!currentStep) {
		return;
	}

	if (
		!hasVisibleReasoning(currentStep.reasoning) &&
		!hasVisibleReasoning(sanitizedReasoning)
	) {
		return;
	}

	currentStep.reasoning = replace
		? sanitizedReasoning
		: sanitizeReasoningForDisplay(
				(currentStep.reasoning || "") + sanitizedReasoning,
			);
	responseMessage.current_step_id = currentStep.id;
}

/**
 * Upsert `incoming` widgets into `existing` by instance id. chat_out /
 * chat_stream re-send each widget as snapshotted at push time, without updates
 * that streamed live after the push. Both update arrays are prefixes of the
 * same emission-ordered sequence, so the longer one is the more complete
 * state — never regress it.
 */
export function mergeChatWidgets(
	existing: IChatWidget[] | undefined,
	incoming: IChatWidget[] | undefined,
): IChatWidget[] {
	const byId = new Map(
		(existing ?? []).map((widget) => [widget.instance_id, widget]),
	);
	for (const widget of incoming ?? []) {
		if (!widget?.instance_id) continue;
		const prior = byId.get(widget.instance_id);
		const priorUpdates = prior?.updates ?? [];
		const incomingUpdates = widget.updates ?? [];
		byId.set(
			widget.instance_id,
			priorUpdates.length > incomingUpdates.length
				? { ...widget, updates: priorUpdates }
				: widget,
		);
	}
	return Array.from(byId.values());
}

function widgetContainsChild(widget: IChatWidget, childId: string): boolean {
	const inlineDef = (widget.component as Record<string, unknown>)
		?.inlineWidgetDef as { components?: Array<{ id?: string }> } | undefined;
	const suffix = `-${childId}`;
	return (
		inlineDef?.components?.some(
			(c) => c?.id === childId || (c?.id?.endsWith(suffix) ?? false),
		) ?? false
	);
}

function a2uiUpdateTargetsWidget(
	widget: IChatWidget,
	payload: Record<string, unknown>,
): boolean {
	switch (payload.type) {
		case "upsertElement": {
			const elementId = payload.element_id as string | undefined;
			if (!elementId) return false;
			if (elementId.includes("/")) {
				const surfaceId = elementId.split("/", 2)[0];
				return (
					surfaceId === widget.surface_id || surfaceId === widget.instance_id
				);
			}
			return (
				elementId === widget.instance_id ||
				widgetContainsChild(widget, elementId)
			);
		}
		case "dataModelUpdate":
		case "createElement":
		case "removeElement": {
			const surfaceId = (payload.surfaceId ?? payload.surface_id) as
				| string
				| undefined;
			return !!surfaceId && surfaceId === widget.surface_id;
		}
		default:
			return false;
	}
}

function hasUsageStat(
	usageStats: IChatUsageStat[] | undefined,
	stat: IChatUsageStat,
): boolean {
	if (!usageStats || usageStats.length === 0) {
		return false;
	}

	const signature = JSON.stringify(stat);
	return usageStats.some((existing) => JSON.stringify(existing) === signature);
}

function parseBackendPlan(reasoning: BackendReasoning): {
	steps: IPlanStep[];
	currentStepId: string | undefined;
} {
	const steps: IPlanStep[] = [];
	let currentStepId: string | undefined;

	for (const [stepId, stepText] of reasoning.plan) {
		const id = `step-${stepId}`;

		// Parse "title: description" format
		const colonIndex = stepText.indexOf(":");
		const title =
			colonIndex > 0 ? stepText.substring(0, colonIndex).trim() : stepText;
		const description =
			colonIndex > 0 ? stepText.substring(colonIndex + 1).trim() : undefined;

		// Determine status based on current_step
		let status: "planned" | "progress" | "done" | "failed";
		if (stepId < reasoning.current_step) {
			status = "done";
		} else if (stepId === reasoning.current_step) {
			status = "progress";
			currentStepId = id;
		} else {
			status = "planned";
		}

		steps.push({
			id,
			title,
			description,
			status,
			reasoning:
				stepId === reasoning.current_step &&
				hasVisibleReasoning(reasoning.current_message)
					? sanitizeReasoningForDisplay(reasoning.current_message)
					: undefined,
		});
	}

	return { steps, currentStepId };
}

export function processChatEvents(
	events: any[],
	initialState: {
		intermediateResponse: Response;
		responseMessage: IMessage;
		attachments: Map<string, IAttachment>;
		tmpLocalState: any;
		tmpGlobalState: any;
		done: boolean;
		appId: string;
		eventId: string;
		sessionId: string;
	},
): ProcessChatEventsResult {
	let {
		intermediateResponse,
		responseMessage,
		attachments,
		tmpLocalState,
		tmpGlobalState,
		done,
	} = initialState;
	let shouldUpdate = false;
	const interactions: IInteractionRequest[] = [];
	const { appId, eventId, sessionId } = initialState;

	const addAttachments = (newAttachments: IAttachment[]) => {
		for (const attachment of newAttachments) {
			if (typeof attachment === "string" && !attachments.has(attachment)) {
				attachments.set(attachment, attachment);
			}

			if (typeof attachment !== "string" && !attachments.has(attachment.url)) {
				attachments.set(attachment.url, attachment);
			}
		}
		responseMessage.files = Array.from(attachments.values());
	};

	const addWidgets = (newWidgets: IChatWidget[] | undefined) => {
		if (!newWidgets?.length) return;
		responseMessage.widgets = mergeChatWidgets(
			responseMessage.widgets,
			newWidgets,
		);
	};

	// Appends a live a2ui update (streamed after the widget was pushed) to the
	// matching widget so the render-time replay picks it up. Updates fired
	// before the push are attached by the backend instead.
	const attachA2UIUpdate = (payload: Record<string, unknown>): boolean => {
		const widgets = responseMessage.widgets;
		if (!widgets?.length) return false;
		let changed = false;
		const next = widgets.map((widget) => {
			if (!a2uiUpdateTargetsWidget(widget, payload)) return widget;
			changed = true;
			return { ...widget, updates: [...(widget.updates ?? []), payload] };
		});
		if (changed) {
			responseMessage.widgets = next;
		}
		return changed;
	};

	for (const ev of events) {
		if (ev.event_type === "a2ui") {
			if (attachA2UIUpdate(ev.payload as Record<string, unknown>)) {
				shouldUpdate = true;
			}
			continue;
		}
		if (ev.event_type === "chat_stream_partial") {
			if (done) continue;

			// Handle response chunks
			if (ev.payload.chunk) {
				intermediateResponse.pushChunk(ev.payload.chunk);
				shouldUpdate = true;

				// Extract reasoning from chunk delta
				const delta = ev.payload.chunk?.choices?.[0]?.delta;
				if (delta?.reasoning && !ev.payload.plan) {
					appendFallbackReasoningStep(responseMessage, delta.reasoning);
					shouldUpdate = true;
				}
			}

			// Update message content from response
			const lastMessage = intermediateResponse.lastMessageOfRole(
				IRole.Assistant,
			);
			if (lastMessage) {
				responseMessage.inner.content = visibleResponseContent(lastMessage);
				if (lastMessage.reasoning && !ev.payload.plan) {
					appendFallbackReasoningStep(
						responseMessage,
						lastMessage.reasoning,
						true,
					);
				}
			}

			// Handle plan updates
			if (ev.payload.plan) {
				const planData = ev.payload.plan as BackendReasoning;
				const { steps, currentStepId } = parseBackendPlan(planData);
				responseMessage.plan_steps = steps;
				responseMessage.current_step_id = currentStepId;
				shouldUpdate = true;
			}

			// Handle attachments
			if (ev.payload.attachments) {
				addAttachments(ev.payload.attachments);
				shouldUpdate = true;
			}

			// Handle embedded widgets
			if (ev.payload.widgets) {
				addWidgets(ev.payload.widgets);
				shouldUpdate = true;
			}
			continue;
		}
		if (ev.event_type === "chat_stream") {
			if (done) continue;
			if (ev.payload.response) {
				intermediateResponse = Response.fromObject(ev.payload.response);
				const lastMessage = intermediateResponse.lastMessageOfRole(
					IRole.Assistant,
				);
				if (lastMessage) {
					responseMessage.inner.content = visibleResponseContent(lastMessage);
					if (lastMessage.reasoning && !ev.payload.plan) {
						appendFallbackReasoningStep(
							responseMessage,
							lastMessage.reasoning,
							true,
						);
					}
					shouldUpdate = true;
				}
			}
			// Handle plan in chat_stream as well
			if (ev.payload.plan) {
				const planData = ev.payload.plan as BackendReasoning;
				const { steps, currentStepId } = parseBackendPlan(planData);
				responseMessage.plan_steps = steps;
				responseMessage.current_step_id = currentStepId;
				shouldUpdate = true;
			}
			if (ev.payload.widgets) {
				addWidgets(ev.payload.widgets);
				shouldUpdate = true;
			}
			continue;
		}
		if (ev.event_type === "chat_out") {
			done = true;
			if (ev.payload.response) {
				intermediateResponse = Response.fromObject(ev.payload.response);
				const lastMessage = intermediateResponse.lastMessageOfRole(
					IRole.Assistant,
				);
				const finalContent = lastMessage
					? visibleResponseContent(lastMessage)
					: responseMessage.inner.content;
				if (finalContent !== responseMessage.inner.content) {
					responseMessage.inner.content = finalContent ?? "";
					shouldUpdate = true;
				}
				if (lastMessage?.reasoning && !ev.payload.plan) {
					appendFallbackReasoningStep(
						responseMessage,
						lastMessage.reasoning,
						true,
					);
					shouldUpdate = true;
				}
			}

			if (ev.payload.attachments) {
				addAttachments(ev.payload.attachments);
				shouldUpdate = true;
			}

			if (ev.payload.widgets) {
				addWidgets(ev.payload.widgets);
				shouldUpdate = true;
			}

			// Finalize plan steps - mark all as done if not already
			if (responseMessage.plan_steps) {
				for (const step of responseMessage.plan_steps) {
					if (step.status === "progress") {
						step.status = "done";
						if (!step.endTime) {
							step.endTime = Date.now();
						}
					}
				}
				responseMessage.current_step_id = undefined;
				shouldUpdate = true;
			}
		}

		if (ev.event_type === "chat_local_session") {
			if (tmpLocalState) {
				tmpLocalState = {
					...tmpLocalState,
					localState: ev.payload,
				};
			} else {
				tmpLocalState = {
					id: createId(),
					appId,
					eventId: eventId,
					sessionId: sessionId,
					localState: ev.payload,
				};
			}
		}

		if (ev.event_type === "interaction_request") {
			const interaction = ev.payload as IInteractionRequest;
			console.debug(
				"[Chat] Received interaction_request:",
				interaction.id,
				interaction.interaction_type?.type,
				"status:",
				interaction.status,
				"expires_at:",
				interaction.expires_at,
				"has_jwt:",
				!!interaction.responder_jwt,
			);
			interactions.push(interaction);
			shouldUpdate = true;
		}

		if (ev.event_type === "chat_global_session") {
			if (tmpGlobalState) {
				tmpGlobalState = {
					...tmpGlobalState,
					globalState: ev.payload,
				};
			} else {
				tmpGlobalState = {
					id: createId(),
					appId,
					eventId: eventId,
					globalState: ev.payload,
				};
			}
		}

		if (ev.event_type === "chat_usage_stat") {
			const stat = ev.payload as IChatUsageStat;
			if (!responseMessage.usage_stats) {
				responseMessage.usage_stats = [];
			}
			if (!hasUsageStat(responseMessage.usage_stats, stat)) {
				responseMessage.usage_stats.push(stat);
				shouldUpdate = true;
			}
		}
	}

	return {
		intermediateResponse,
		responseMessage,
		attachments,
		tmpLocalState,
		tmpGlobalState,
		done,
		shouldUpdate,
		interactions: interactions.length > 0 ? interactions : undefined,
	};
}
