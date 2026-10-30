import { createId } from "@paralleldrive/cuid2";
import { Response } from "../../../lib/llm/response";
import type { IIntercomEvent } from "../../../lib/schema/events/intercom-event";
import type { IInteractionRequest } from "../../../lib/schema/interaction";
import {
	type IContent,
	IContentType,
	IRole,
} from "../../../lib/schema/llm/history";
import type { IResponseMessage } from "../../../lib/schema/llm/response";
import type {
	IAttachment,
	IChatMainWidget,
	IChatUsageStat,
	IChatWidget,
	IGlobalState,
	ILocalChatState,
	IMessage,
	IPlanStep,
} from "./chat-db";
import { joinContentText } from "./inline-segments";
import { nextMainWidgetRevision } from "./main-widget-state";
import { sanitizeReasoningForDisplay } from "./reasoning-format";

export interface ProcessChatEventsResult {
	intermediateResponse: Response;
	responseMessage: IMessage;
	responseMessages: IMessage[];
	attachments: Map<string, IAttachment>;
	tmpLocalState: ILocalChatState | null | undefined;
	tmpGlobalState: IGlobalState | null | undefined;
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

/**
 * Messages whose plan is the single synthesized "Thinking" step rather than a real backend plan.
 * Only those may be replaced wholesale with the run-wide accumulated reasoning — backend steps
 * are scoped per step and would otherwise absorb every earlier step's text.
 */
const syntheticReasoningPlans = new WeakSet<IMessage>();

function appendFallbackReasoningStep(
	responseMessage: IMessage,
	reasoning: string,
	replace = false,
) {
	if (
		(!responseMessage.plan_steps || responseMessage.plan_steps.length === 0) &&
		!hasVisibleReasoning(reasoning)
	) {
		return;
	}

	if (!responseMessage.plan_steps || responseMessage.plan_steps.length === 0) {
		const step: IPlanStep = {
			id: "step-0",
			title: "Thinking",
			status: "progress",
			reasoning,
		};
		// A live delta marks where the thinking started; a replayed transcript does not.
		if (!replace) {
			step.content_offset = joinContentText(
				responseMessage.inner.content,
			).length;
		}
		responseMessage.plan_steps = [step];
		responseMessage.current_step_id = "step-0";
		syntheticReasoningPlans.add(responseMessage);
		return;
	}

	// The run-wide reasoning transcript may only overwrite the synthesized step. Real backend
	// plans carry their own per-step text, which applyBackendPlan already keeps up to date.
	if (replace && !syntheticReasoningPlans.has(responseMessage)) {
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
		!hasVisibleReasoning(reasoning)
	) {
		return;
	}

	currentStep.reasoning = replace
		? reasoning
		: (currentStep.reasoning || "") + reasoning;
	responseMessage.current_step_id = currentStep.id;
}

/**
 * The tokenized-reasoning repair is a display heuristic for rows persisted before the producers
 * were fixed. Running it on the whole transcript for every chunk made the processor superlinear
 * in stream length, so it runs exactly once, on the settled message.
 */
export function finalizePlanSteps(responseMessage: IMessage) {
	for (const step of responseMessage.plan_steps ?? []) {
		if (step.status === "progress") {
			step.status = "done";
			if (!step.endTime) {
				step.endTime = Date.now();
			}
		}
		if (step.reasoning) {
			step.reasoning = sanitizeReasoningForDisplay(step.reasoning);
		}
	}
	responseMessage.current_step_id = undefined;
}

function withContentOffset(
	widget: IChatWidget,
	contentOffset: number | undefined,
): IChatWidget {
	if (widget.content_offset === contentOffset) return widget;
	const { content_offset: _replaced, ...rest } = widget;
	return contentOffset === undefined
		? rest
		: { ...rest, content_offset: contentOffset };
}

/**
 * Upsert `incoming` widgets into `existing` by instance id. chat_out /
 * chat_stream re-send each widget as snapshotted at push time, without updates
 * that streamed live after the push. Both update arrays are prefixes of the
 * same emission-ordered sequence, so the longer one is the more complete
 * state — never regress it.
 *
 * Anchors are frozen at first sight: a widget already on the message keeps its
 * `content_offset` (or the lack of one) whatever the re-send carries, and only
 * a first-seen widget takes `anchor` when one is given.
 */
export function mergeChatWidgets(
	existing: IChatWidget[] | undefined,
	incoming: IChatWidget[] | undefined,
	anchor?: number,
): IChatWidget[] {
	const byId = new Map(
		(existing ?? []).map((widget) => [widget.instance_id, widget]),
	);
	let changed = false;
	for (const widget of incoming ?? []) {
		if (!widget?.instance_id) continue;
		const prior = byId.get(widget.instance_id);
		const priorUpdates = prior?.updates ?? [];
		const incomingUpdates = widget.updates ?? [];
		// Keeping the prior OBJECT (not just its updates) preserves identity for
		// downstream memoization; the component is the push-time snapshot and
		// re-registrations ride the updates array, so an equal-or-shorter
		// incoming widget carries nothing new.
		if (prior && priorUpdates.length >= incomingUpdates.length) continue;
		const contentOffset = prior
			? prior.content_offset
			: (anchor ?? widget.content_offset);
		byId.set(widget.instance_id, withContentOffset(widget, contentOffset));
		changed = true;
	}
	// Same rule for the array: a re-send that adds nothing must not re-render
	// every embedded widget list.
	if (existing && !changed) return existing;
	return Array.from(byId.values());
}

function mergeMainWidget(
	existing: IChatMainWidget | undefined,
	incoming: IChatMainWidget,
): IChatMainWidget {
	const sameCommand =
		existing?.command_id === incoming.command_id &&
		existing?.title === incoming.title &&
		existing?.display === incoming.display;
	if (!incoming.widget) {
		return existing && !existing.widget && sameCommand ? existing : incoming;
	}
	if (!existing?.widget) return incoming;
	const prior = existing.widget;
	const next = incoming.widget;
	if (
		prior.instance_id !== next.instance_id ||
		prior.surface_id !== next.surface_id ||
		prior.widget_id !== next.widget_id ||
		JSON.stringify(prior.component) !== JSON.stringify(next.component)
	) {
		return incoming;
	}
	const widget = mergeChatWidgets([prior], [next])[0];
	if (widget === prior && sameCommand) {
		return existing;
	}
	return { ...incoming, widget };
}

function inlineChildIds(component: unknown): string[] {
	const inlineDef = (component as Record<string, unknown> | undefined)
		?.inlineWidgetDef as { components?: Array<{ id?: string }> } | undefined;
	return (
		inlineDef?.components
			?.map((c) => c?.id)
			.filter((id): id is string => typeof id === "string") ?? []
	);
}

function collectBoundPaths(value: unknown, paths: Set<string>) {
	if (Array.isArray(value)) {
		for (const item of value) collectBoundPaths(item, paths);
		return;
	}
	if (!value || typeof value !== "object") return;
	const record = value as Record<string, unknown>;
	const keys = Object.keys(record);
	if (
		typeof record.path === "string" &&
		keys.every((k) => k === "path" || k === "defaultValue")
	) {
		paths.add(record.path);
	}
	for (const item of Object.values(record)) collectBoundPaths(item, paths);
}

function pathsOverlap(updated: string, bound: string): boolean {
	return (
		updated === bound ||
		bound.startsWith(`${updated}/`) ||
		updated.startsWith(`${bound}/`)
	);
}

interface WidgetUpdateTargets {
	instanceIds: Set<string>;
	childIds: Set<string>;
	boundPaths: Set<string>;
}

/** A pushed instance id (not one of the widget's own inline children) joins the replay. */
function notePushedInstance(
	{ instanceIds, childIds }: WidgetUpdateTargets,
	value: Record<string, unknown>,
) {
	if (value.type !== "pushChild" && value.type !== "insertChildAt") return;
	if (typeof value.childId !== "string" || childIds.has(value.childId)) return;
	instanceIds.add(value.childId);
}

/** Registering a tracked instance contributes its inline children and bindings. */
function noteCreatedComponent(
	{ instanceIds, childIds, boundPaths }: WidgetUpdateTargets,
	elementId: unknown,
	value: Record<string, unknown>,
) {
	if (value.type !== "createComponent") return;
	if (typeof elementId !== "string" || !instanceIds.has(elementId)) return;
	for (const id of inlineChildIds(value.component)) childIds.add(id);
	collectBoundPaths(value.component, boundPaths);
}

/** Folds one replay entry into the target sets; order matters, so callers apply entries in sequence. */
function extendWidgetUpdateTargets(
	targets: WidgetUpdateTargets,
	update: unknown,
): WidgetUpdateTargets {
	const record = update as Record<string, unknown>;
	const value =
		record.type === "upsertElement"
			? (record.value as Record<string, unknown> | undefined)
			: undefined;
	if (!value) return targets;
	notePushedInstance(targets, value);
	noteCreatedComponent(targets, record.element_id, value);
	return targets;
}

// Keyed by widget object: the processor replaces the object whenever its
// updates grow, so a stale entry can never be served for new content.
const widgetTargetsCache = new WeakMap<IChatWidget, WidgetUpdateTargets>();

/**
 * Ids and binding paths a live a2ui update may legitimately target for one
 * chat widget. Mirrors the backend `ChatWidget::attach_update_log` fixpoint:
 * instances pushed into this widget's containers (and their children) belong
 * to its replay, and data-model updates are matched by bound path because
 * `Data Update` writes to a board-chosen surface id, never the instance id.
 *
 * Computing this walks the whole component tree plus every prior update, so
 * it is cached per widget object and extended incrementally when an update is
 * appended — a stream of element updates otherwise re-walked everything per
 * event.
 */
function widgetUpdateTargets(widget: IChatWidget): WidgetUpdateTargets {
	const cached = widgetTargetsCache.get(widget);
	if (cached) return cached;
	const targets: WidgetUpdateTargets = {
		instanceIds: new Set([widget.instance_id]),
		childIds: new Set(inlineChildIds(widget.component)),
		boundPaths: new Set<string>(),
	};
	collectBoundPaths(widget.component, targets.boundPaths);
	for (const update of widget.updates ?? []) {
		extendWidgetUpdateTargets(targets, update);
	}
	widgetTargetsCache.set(widget, targets);
	return targets;
}

function appendWidgetUpdate(
	widget: IChatWidget,
	targets: WidgetUpdateTargets,
	update: unknown,
): IChatWidget {
	const next = { ...widget, updates: [...(widget.updates ?? []), update] };
	widgetTargetsCache.set(
		next,
		extendWidgetUpdateTargets(
			{
				instanceIds: new Set(targets.instanceIds),
				childIds: new Set(targets.childIds),
				boundPaths: new Set(targets.boundPaths),
			},
			update,
		),
	);
	return next;
}

/**
 * Returns the payload to append to the widget's replay when the update targets
 * it, or null. Data-model updates are re-addressed to the widget's surface —
 * the reducer only applies them when the surface ids match.
 */
function a2uiUpdateForWidget(
	widget: IChatWidget,
	payload: Record<string, unknown>,
	targets: ReturnType<typeof widgetUpdateTargets>,
): Record<string, unknown> | null {
	const { instanceIds, childIds, boundPaths } = targets;
	switch (payload.type) {
		case "upsertElement": {
			const elementId = payload.element_id as string | undefined;
			if (!elementId) return null;
			if (elementId.includes("/")) {
				const scope = elementId.split("/", 2)[0];
				return scope === widget.surface_id || instanceIds.has(scope)
					? payload
					: null;
			}
			if (instanceIds.has(elementId)) return payload;
			const suffix = `-${elementId}`;
			for (const child of childIds) {
				if (child === elementId || child.endsWith(suffix)) return payload;
			}
			return null;
		}
		case "createElement":
		case "removeElement": {
			const surfaceId = (payload.surfaceId ?? payload.surface_id) as
				| string
				| undefined;
			if (!surfaceId) return null;
			return surfaceId === widget.surface_id || instanceIds.has(surfaceId)
				? payload
				: null;
		}
		case "dataModelUpdate": {
			const surfaceId = (payload.surfaceId ?? payload.surface_id) as
				| string
				| undefined;
			const targeted =
				!!surfaceId &&
				(surfaceId === widget.surface_id || instanceIds.has(surfaceId));
			const path = payload.path as string | undefined;
			const contents = payload.contents as Array<{ key?: string }> | undefined;
			const bound =
				!targeted &&
				(path
					? [...boundPaths].some((b) => pathsOverlap(path, b))
					: (contents?.some((entry) => {
							const key = entry?.key;
							return (
								typeof key === "string" &&
								[...boundPaths].some((b) => pathsOverlap(key, b))
							);
						}) ?? false));
			if (!targeted && !bound) return null;
			if (surfaceId === widget.surface_id) return payload;
			return {
				...payload,
				surfaceId: widget.surface_id,
				surface_id: widget.surface_id,
			};
		}
		default:
			return null;
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

/**
 * Rebuild the plan from a backend snapshot — Push Step, Push Reasoning, Push Text To Step and
 * Remove Step all re-send the whole plan. Anchors are frozen at first sight: a step already on the
 * message keeps whatever `content_offset` it has (none, once Push Response detached it), and only
 * a step seen for the first time anchors at the current text length, so it renders after the text
 * that preceded it and before the text that follows.
 */
function applyBackendPlan(
	responseMessage: IMessage,
	reasoning: BackendReasoning,
) {
	const priorSteps = new Map(
		(responseMessage.plan_steps ?? []).map((step) => [step.id, step]),
	);
	const textLength = joinContentText(responseMessage.inner.content).length;
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

		const prior = priorSteps.get(id);
		steps.push({
			id,
			title,
			description,
			status,
			reasoning:
				stepId === reasoning.current_step &&
				hasVisibleReasoning(reasoning.current_message)
					? reasoning.current_message
					: undefined,
			content_offset: prior ? prior.content_offset : textLength,
		});
	}

	responseMessage.plan_steps = steps;
	responseMessage.current_step_id = currentStepId;
	syntheticReasoningPlans.delete(responseMessage);
}

/**
 * Push Response replaces the whole text, so offsets stamped against the streamed text no longer
 * point at anything. Detached steps fall back to the block above the text; steps pushed afterwards
 * anchor against the replaced text again.
 */
function detachPlanAnchors(responseMessage: IMessage): boolean {
	const steps = responseMessage.plan_steps;
	if (!steps?.some((step) => typeof step.content_offset === "number")) {
		return false;
	}
	responseMessage.plan_steps = steps.map(
		({ content_offset: _detached, ...step }) => step,
	);
	return true;
}

/** Same rule for widgets: detached ones fall back to the block after the text. */
function detachWidgetAnchors(responseMessage: IMessage): boolean {
	const widgets = responseMessage.widgets;
	if (!widgets?.some((widget) => typeof widget.content_offset === "number")) {
		return false;
	}
	responseMessage.widgets = widgets.map((widget) => {
		const detached = withContentOffset(widget, undefined);
		const targets = widgetTargetsCache.get(widget);
		if (detached !== widget && targets) {
			widgetTargetsCache.set(detached, targets);
		}
		return detached;
	});
	return true;
}

function detachAnchors(responseMessage: IMessage): boolean {
	const steps = detachPlanAnchors(responseMessage);
	const widgets = detachWidgetAnchors(responseMessage);
	return steps || widgets;
}

export function processChatEvents(
	events: Pick<IIntercomEvent, "event_type" | "payload">[],
	initialState: {
		intermediateResponse: Response;
		responseMessage: IMessage;
		responseMessages?: IMessage[];
		attachments: Map<string, IAttachment>;
		tmpLocalState: ILocalChatState | null | undefined;
		tmpGlobalState: IGlobalState | null | undefined;
		done: boolean;
		appId: string;
		eventId: string;
		sessionId: string;
		/** Persistence folds data without answering frontend requests. */
		handleRequests?: boolean;
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
	const responseMessages = initialState.responseMessages ?? [responseMessage];
	const root = responseMessages[0] ?? responseMessage;
	const rootId = root.stream_root_id ?? root.id;
	const rootTimestamp = root.stream_root_timestamp ?? root.timestamp;
	root.stream_root_id = rootId;
	root.stream_root_timestamp = rootTimestamp;

	const selectMessage = (messageId: unknown) => {
		if (typeof messageId !== "string" || !messageId) return;
		if (!responseMessage.stream_message_id) {
			responseMessage.stream_message_id = messageId;
			return;
		}
		if (responseMessage.stream_message_id === messageId) return;

		finalizePlanSteps(responseMessage);
		responseMessage = {
			id: `${rootId}:${messageId}`,
			main_widget: responseMessage.main_widget,
			stream_root_id: rootId,
			stream_root_timestamp: rootTimestamp,
			stream_message_id: messageId,
			trigger_message_id: root.trigger_message_id,
			appId,
			sessionId,
			explicit_name: root.explicit_name,
			timestamp: rootTimestamp + responseMessages.length,
			inner: { role: IRole.Assistant, content: "" },
			files: [],
			tools: [],
			actions: [],
		};
		responseMessages.push(responseMessage);
		intermediateResponse = Response.default();
		attachments = new Map();
		shouldUpdate = true;
	};

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

	// A live push anchors where the text stood (like applyBackendPlan's first sight); the final
	// flush carries no position, so widgets it introduces stay in the after-text block.
	const addWidgets = (
		newWidgets: IChatWidget[] | undefined,
		anchored: boolean,
	) => {
		if (!newWidgets?.length) return;
		responseMessage.widgets = mergeChatWidgets(
			responseMessage.widgets,
			newWidgets,
			anchored
				? joinContentText(responseMessage.inner.content).length
				: undefined,
		);
	};

	const setMainWidget = (incoming: IChatMainWidget | null | undefined) => {
		// Null on the outer field is an older/default response, not a clear command.
		if (!incoming || typeof incoming !== "object" || !("widget" in incoming)) {
			return;
		}
		if (incoming.widget && !incoming.widget.instance_id) return;
		const next = mergeMainWidget(responseMessage.main_widget, incoming);
		if (next !== responseMessage.main_widget) {
			const previous = responseMessage.main_widget;
			const revision = nextMainWidgetRevision(previous);
			const sameCommand =
				previous &&
				previous.command_id === next.command_id &&
				previous.title === next.title &&
				previous.display === next.display &&
				previous.widget?.instance_id === next.widget?.instance_id;
			responseMessage.main_widget = {
				...next,
				updated_at: revision,
				command_at: sameCommand
					? (previous.command_at ?? previous.updated_at ?? revision)
					: revision,
			};
			shouldUpdate = true;
		}
	};

	// Appends a live a2ui update (streamed after the widget was pushed) to the
	// matching widget so the render-time replay picks it up. Updates fired
	// before the push are attached by the backend instead.
	const attachA2UIUpdate = (payload: Record<string, unknown>): boolean => {
		const widgets = responseMessage.widgets;
		let changed = false;
		const next = widgets?.map((widget) => {
			const targets = widgetUpdateTargets(widget);
			const update = a2uiUpdateForWidget(widget, payload, targets);
			if (!update) return widget;
			changed = true;
			return appendWidgetUpdate(widget, targets, update);
		});
		if (changed) {
			responseMessage.widgets = next;
		}
		const main = responseMessage.main_widget;
		if (main?.widget) {
			const targets = widgetUpdateTargets(main.widget);
			const update = a2uiUpdateForWidget(main.widget, payload, targets);
			if (update) {
				responseMessage.main_widget = {
					...main,
					widget: appendWidgetUpdate(main.widget, targets, update),
					updated_at: nextMainWidgetRevision(main),
				};
				changed = true;
			}
		}
		return changed;
	};

	for (const ev of events) {
		if (
			!done &&
			[
				"chat_stream_partial",
				"chat_stream",
				"chat_out",
				"chat_usage_stat",
			].includes(ev.event_type)
		) {
			selectMessage(ev.payload?.message_id);
		}
		if (ev.event_type === "a2ui") {
			if (
				ev.payload?.type === "widgetQuery" ||
				ev.payload?.type === "requestElements" ||
				ev.payload?.type === "deviceCommand"
			)
				continue;
			if (attachA2UIUpdate(ev.payload as Record<string, unknown>)) {
				shouldUpdate = true;
			}
			continue;
		}
		if (ev.event_type === "chat_stream_partial") {
			if (done) continue;
			setMainWidget(ev.payload.main_widget);

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
				applyBackendPlan(responseMessage, ev.payload.plan as BackendReasoning);
				shouldUpdate = true;
			}

			// Handle attachments
			if (ev.payload.attachments) {
				addAttachments(ev.payload.attachments);
				shouldUpdate = true;
			}

			// Handle embedded widgets
			if (ev.payload.widgets) {
				addWidgets(ev.payload.widgets, true);
				shouldUpdate = true;
			}
			continue;
		}
		if (ev.event_type === "chat_stream") {
			if (done) continue;
			if (ev.payload.attachments) {
				addAttachments(ev.payload.attachments);
				shouldUpdate = true;
			}
			setMainWidget(ev.payload.main_widget);
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
				// Push Response replaced the text: every anchor stamped so far is void.
				if (detachAnchors(responseMessage)) {
					shouldUpdate = true;
				}
			}
			// Handle plan in chat_stream as well
			if (ev.payload.plan) {
				applyBackendPlan(responseMessage, ev.payload.plan as BackendReasoning);
				shouldUpdate = true;
			}
			if (ev.payload.widgets) {
				addWidgets(ev.payload.widgets, true);
				shouldUpdate = true;
			}
			continue;
		}
		if (ev.event_type === "chat_out") {
			done = true;
			setMainWidget(ev.payload.main_widget);
			if (ev.payload.response) {
				intermediateResponse = Response.fromObject(ev.payload.response);
				const lastMessage = intermediateResponse.lastMessageOfRole(
					IRole.Assistant,
				);
				const streamedText = joinContentText(responseMessage.inner.content);
				const finalContent = lastMessage
					? visibleResponseContent(lastMessage)
					: responseMessage.inner.content;
				if (finalContent !== responseMessage.inner.content) {
					responseMessage.inner.content = finalContent ?? "";
					shouldUpdate = true;
				}
				// The final flush normally re-sends exactly what streamed, so anchors stay
				// valid. Anything else (a chunk lost to sampling, a rewrite) leaves them
				// pointing into text that no longer exists.
				if (
					!joinContentText(responseMessage.inner.content).startsWith(
						streamedText,
					) &&
					detachAnchors(responseMessage)
				) {
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
				addWidgets(ev.payload.widgets, false);
				shouldUpdate = true;
			}

			if (responseMessage.plan_steps) {
				finalizePlanSteps(responseMessage);
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
				"has_channel:",
				!!interaction.channel,
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
		responseMessages,
		attachments,
		tmpLocalState,
		tmpGlobalState,
		done,
		shouldUpdate,
		interactions: interactions.length > 0 ? interactions : undefined,
	};
}
