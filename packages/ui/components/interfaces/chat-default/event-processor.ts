import { createId } from "@paralleldrive/cuid2";
import { IRole, Response } from "../../../lib";
import type { IInteractionRequest } from "../../../lib/schema/interaction";
import type { IAttachment, IChatUsageStat, IMessage, IPlanStep } from "./chat-db";

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

interface BackendReasoning {
	plan: [number, string][];
	current_step: number;
	current_message: string;
}

function hasVisibleReasoning(reasoning: string | undefined): reasoning is string {
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

	currentStep.reasoning = sanitizeReasoningForDisplay(
		(currentStep.reasoning || "") + sanitizedReasoning,
	);
	responseMessage.current_step_id = currentStep.id;
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

	for (const ev of events) {
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
				responseMessage.inner.content = lastMessage.content ?? "";
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
					responseMessage.inner.content = lastMessage.content ?? "";
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
			continue;
		}
		if (ev.event_type === "chat_out") {
			done = true;
			if (ev.payload.response) {
				intermediateResponse = Response.fromObject(ev.payload.response);
				const lastMessage = intermediateResponse.lastMessageOfRole(
					IRole.Assistant,
				);
				const finalContent = lastMessage?.content ?? responseMessage.inner.content;
				if (finalContent !== responseMessage.inner.content) {
					responseMessage.inner.content = finalContent ?? "";
					shouldUpdate = true;
				}
			}

			if (ev.payload.attachments) {
				addAttachments(ev.payload.attachments);
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
