import { createId } from "@paralleldrive/cuid2";
import {
	FLOWPILOT_DEBUG_ENABLED,
	stripFlowPilotDebugReport,
} from "../../lib/flowpilot-debug";
import { isTauri } from "../../lib/platform";
import { IRole } from "../../lib/schema/llm/history";
import {
	createAgentDebugStreamRecorder,
	summarizeAgentDebugRootOutcomes,
} from "./agent-debug-report";
import {
	applyStreamEvent,
	createStreamAccumulator,
	mergeUsageStats,
	orderedSteps,
} from "./copilot-stream-steps";
import {
	GLOBAL_CHAT_APP_ID,
	type IMessage,
	globalChatDb,
} from "./global-chat-db";
import {
	type GlobalChatAgentSelection,
	beginGlobalChatTurnSelection,
	endGlobalChatTurnSelection,
	useGlobalChatStore,
} from "./global-chat-store";

// The global-chat streaming engine lives here — OUTSIDE any React component — so a turn keeps
// streaming, checkpointing, and finalizing even as the conversation morphs between the /chat page
// and the docked overlay (each unmounts the other) or the webview hard-reloads. All state is read
// from / written to the zustand singleton, never component-local refs. The Rust side mirrors every
// run into a resumable buffer keyed by `runId`; `resumeGlobalChatStream` re-attaches after a reload.

/** Session-scoped pointer to the active conversation, so reloads/navigation restore the transcript. */
export const LAST_CONVERSATION_KEY = "flow-like:global-chat:last-conversation";
/** Session-scoped pointer to an in-flight run so a reload can re-attach to the live Rust stream. */
const ACTIVE_RUN_KEY = "flow-like:global-chat:active-run";

interface ActiveRun {
	conversationId: string;
	runId: string;
	agentSelection?: GlobalChatAgentSelection;
}

/** Remember the run currently streaming, so a reload mid-response can re-attach to it. */
export function setActiveRun(
	conversationId: string,
	runId: string,
	agentSelection?: GlobalChatAgentSelection,
) {
	try {
		sessionStorage.setItem(
			ACTIVE_RUN_KEY,
			JSON.stringify({
				conversationId,
				runId,
				...(agentSelection
					? {
							agentSelection: {
								provider: agentSelection.provider,
								selectedModelId: agentSelection.selectedModelId,
								reasoningEffort: agentSelection.reasoningEffort,
							},
						}
					: {}),
			}),
		);
	} catch {
		// resumability is best-effort
	}
}

function readAgentSelection(
	value: unknown,
): GlobalChatAgentSelection | undefined {
	if (!value || typeof value !== "object") return undefined;
	const candidate = value as Record<string, unknown>;
	if (
		candidate.provider !== "bits" &&
		candidate.provider !== "github-copilot" &&
		candidate.provider !== "codex" &&
		candidate.provider !== "claude-code" &&
		candidate.provider !== "copilot"
	) {
		return undefined;
	}
	if (
		typeof candidate.selectedModelId !== "string" ||
		typeof candidate.reasoningEffort !== "string"
	) {
		return undefined;
	}
	return Object.freeze({
		provider: candidate.provider,
		selectedModelId: candidate.selectedModelId,
		reasoningEffort: candidate.reasoningEffort,
	});
}

export function readActiveRun(): ActiveRun | null {
	try {
		const raw = sessionStorage.getItem(ACTIVE_RUN_KEY);
		if (!raw) return null;
		const parsed = JSON.parse(raw);
		if (
			typeof parsed?.conversationId === "string" &&
			typeof parsed?.runId === "string"
		) {
			return {
				conversationId: parsed.conversationId,
				runId: parsed.runId,
				agentSelection: readAgentSelection(parsed.agentSelection),
			};
		}
	} catch {
		// ignore malformed pointer
	}
	return null;
}

export function clearActiveRun(runId?: string) {
	try {
		if (runId && readActiveRun()?.runId !== runId) return;
		sessionStorage.removeItem(ACTIVE_RUN_KEY);
	} catch {
		// best-effort
	}
}

export function makeGlobalChatMessage(
	role: IRole,
	content: string,
	sessionId: string,
): IMessage {
	return {
		id: createId(),
		appId: GLOBAL_CHAT_APP_ID,
		sessionId,
		inner: { role, content },
		files: [],
		tools: [],
		actions: [],
		timestamp: Date.now(),
	};
}

export async function persistGlobalChatMessage(message: IMessage) {
	try {
		if (FLOWPILOT_DEBUG_ENABLED) {
			await globalChatDb.messages.put(message);
			return;
		}
		// Defense in depth: callers can pass restored or backend-provided messages that still carry
		// an old report. Production must never write that diagnostic payload back to history.
		await globalChatDb.messages.put(stripFlowPilotDebugReport(message));
	} catch {
		// history persistence is best-effort in v1
	}
}

/** Create/update the conversation's session row so it shows up in the history list. */
export async function persistGlobalChatSession(
	sessionId: string,
	title: string,
) {
	try {
		const existing = await globalChatDb.sessions.get(sessionId);
		const now = Date.now();
		await globalChatDb.sessions.put({
			id: sessionId,
			appId: GLOBAL_CHAT_APP_ID,
			summarization: existing?.summarization || title.slice(0, 80),
			createdAt: existing?.createdAt ?? now,
			updatedAt: now,
		});
	} catch {
		// history persistence is best-effort in v1
	}
}

interface DriveOptions {
	/** The assistant message shell being streamed into; its id MUST equal the run id. */
	responseMessage: IMessage;
	/** Immutable parent-turn provider/model/effort, shared with all nested specialists. */
	agentSelection?: GlobalChatAgentSelection;
	/** True for a resume re-attach (guards against overwriting a restored checkpoint on a miss). */
	isResume?: boolean;
	/** Bounded/redacted user input metadata included in the persisted debug report. */
	inputPreview?: unknown;
	/**
	 * Transport hook. Drives the underlying run and forwards every raw FlowPilot stream chunk to
	 * `onChunk`; resolves with the transport's result (the desktop Tauri command's return value or,
	 * on the web, the final `UnifiedCopilotResponse`). This is the ONE seam that differs between the
	 * desktop (Tauri Channel — see {@link tauriStart}) and browser (HTTP+SSE — see
	 * `global-chat-web-transport.ts`) transports; everything else in the engine is shared.
	 */
	start: (onChunk: (chunk: string) => void) => Promise<unknown>;
}

/**
 * Desktop transport: bridge a Tauri `Channel<string>` to the engine's `onChunk` seam. Every chunk
 * the Rust command streams over the channel is forwarded to the parser. Returns a `start` function
 * suitable for {@link driveGlobalChatStream}.
 */
export function tauriStart(command: string, args: Record<string, unknown>) {
	return async (onChunk: (chunk: string) => void) => {
		// Tauri is imported lazily (mirrors use-copilot-sdk) so this module also loads on the web,
		// where the caller uses `webGlobalChatStart` instead and never reaches this path.
		const { Channel, invoke } = await import("@tauri-apps/api/core");
		const channel = new Channel<string>();
		channel.onmessage = onChunk;
		return invoke(command, { ...args, channel });
	};
}

/**
 * Drive one assistant turn: parse the streamed FlowPilot protocol into the message's content +
 * plan steps, mirror it into the store's `streamingMessage` (throttled checkpoints to IndexedDB),
 * and finalize into `messages` when the run ends. Safe to run detached from any component.
 */
export async function driveGlobalChatStream({
	responseMessage,
	agentSelection,
	isResume,
	inputPreview,
	start,
}: DriveOptions) {
	const store = useGlobalChatStore;
	const acc = createStreamAccumulator();
	let lastCheckpoint = 0;
	let streamFailure: string | undefined;
	const turnSelection = beginGlobalChatTurnSelection(
		responseMessage.id,
		agentSelection,
	);
	const initialState = store.getState();
	initialState.beginDebugReport(responseMessage.id, {
		provider: turnSelection.provider,
		model: turnSelection.selectedModelId,
		reasoningEffort: turnSelection.reasoningEffort,
		inputPreview,
	});
	initialState.recordDebugEvent(responseMessage.id, {
		id: `main:${responseMessage.id}:lifecycle:start`,
		kind: "lifecycle",
		stage: isResume ? "resume_started" : "run_started",
		status: "progress",
		timestamp_ms: Date.now(),
		started_at_ms: Date.now(),
		summary: isResume
			? "Re-attaching to a previously started agent run. Earlier frontend-only milestones may be unavailable."
			: "Agent turn started.",
	});
	const debugStream = createAgentDebugStreamRecorder({
		scope: "main",
		requestId: responseMessage.id,
		record: (event) =>
			store.getState().recordDebugEvent(responseMessage.id, event),
	});

	const syncMessage = () => {
		const state = store.getState();
		responseMessage.inner.content = acc.content;
		// Nested sub-agent activity (flowpilot_board) is published by the tool bridge into
		// subPlanSteps — render it inline after this response's own steps.
		responseMessage.plan_steps = [...orderedSteps(acc), ...state.subPlanSteps];
		responseMessage.current_step_id = acc.currentStepId;
		responseMessage.tools = acc.currentStepId ? ["working"] : [];
		responseMessage.app_refs =
			state.pendingAppRefs.length > 0 ? [...state.pendingAppRefs] : undefined;
		responseMessage.files = state.subAttachments;
		responseMessage.widgets =
			state.subWidgets.length > 0 ? state.subWidgets : undefined;
		const combinedUsage = mergeUsageStats(acc.usageStats, state.subUsageStats);
		responseMessage.usage_stats =
			combinedUsage.length > 0 ? combinedUsage : undefined;
		responseMessage.debug_report =
			state.debugReport?.message_id === responseMessage.id
				? state.debugReport
				: undefined;
		state.setStreamingMessage({ ...responseMessage });
		const now = Date.now();
		if (now - lastCheckpoint > 1_000) {
			lastCheckpoint = now;
			void persistGlobalChatMessage({ ...responseMessage });
		}
	};

	const onChunk = (chunk: string) => {
		for (const event of debugStream.push(chunk)) {
			applyStreamEvent(acc, event);
		}
		syncMessage();
	};

	let invokeResult: unknown;
	try {
		invokeResult = await start(onChunk);
	} catch (error) {
		const message = error instanceof Error ? error.message : String(error);
		streamFailure = message;
		store.getState().recordDebugEvent(responseMessage.id, {
			id: `main:${responseMessage.id}:lifecycle:stream-error`,
			kind: "lifecycle",
			stage: "stream_error",
			status: "error",
			timestamp_ms: Date.now(),
			ended_at_ms: Date.now(),
			error: message,
		});
		// Surface mid-stream failures even when partial content already arrived — a silent stop
		// reads as a crash. The failed step keeps the panel open.
		acc.stepOrder.push("stream-error");
		acc.steps.set("stream-error", {
			id: "stream-error",
			title: "Stream failed",
			description: message.slice(0, 300),
			status: "failed",
			timestamp: Date.now(),
		});
		if (!acc.content) {
			acc.content = `Something went wrong: ${message}`;
		}
	} finally {
		// Emit any held-back partial-tag fragment so replies ending in '<...' are not lost.
		for (const event of debugStream.flush()) {
			applyStreamEvent(acc, event);
		}
		for (const id of acc.stepOrder) {
			const step = acc.steps.get(id);
			if (step?.status === "progress") {
				acc.steps.set(id, {
					...step,
					status: streamFailure ? "failed" : "done",
				});
			}
		}
		const finalState = store.getState();

		// A resume that found no live run server-side (already GC'd) streams nothing — keep the
		// restored checkpoint intact rather than overwriting it with an empty finalized message.
		const resumeMissed =
			isResume &&
			typeof invokeResult === "object" &&
			invokeResult !== null &&
			(invokeResult as { attached?: boolean }).attached === false;

		if (resumeMissed) {
			store.getState().recordDebugEvent(responseMessage.id, {
				id: `main:${responseMessage.id}:lifecycle:resume-gap`,
				kind: "lifecycle",
				stage: "resume_gap",
				status: "error",
				timestamp_ms: Date.now(),
				error:
					"The live run was no longer available. The restored checkpoint was preserved, but frontend-only milestones after the last checkpoint may be missing.",
			});
		}
		store.getState().recordDebugEvent(responseMessage.id, {
			id: `main:${responseMessage.id}:lifecycle:finish`,
			kind: "lifecycle",
			stage: "run_finished",
			status: streamFailure || resumeMissed ? "error" : "done",
			timestamp_ms: Date.now(),
			ended_at_ms: Date.now(),
			error: streamFailure,
			summary: resumeMissed
				? "Resume did not find a live run."
				: streamFailure
					? "The agent stream ended with an error."
					: "The agent turn completed.",
		});
		const reportBeforeFinalize = store.getState().debugReport;
		const reportEvents =
			reportBeforeFinalize?.message_id === responseMessage.id
				? reportBeforeFinalize.events
				: [];
		const { recordedTimeout, recordedPartial, recordedError } =
			summarizeAgentDebugRootOutcomes(reportEvents);
		const debugOutcome = recordedTimeout
			? "timeout"
			: streamFailure || resumeMissed || recordedError
				? "error"
				: recordedPartial
					? "partial"
					: "ok";
		store.getState().finalizeDebugReport(responseMessage.id, {
			outcome: debugOutcome,
			terminalStage: resumeMissed
				? "resume_gap"
				: streamFailure
					? "stream_error"
					: recordedTimeout
						? "frontend_tool_timeout"
						: recordedError
							? "completed_with_errors"
							: recordedPartial
								? "completed_partial"
								: "completed",
			terminalCode: resumeMissed
				? "RUN_NOT_FOUND"
				: streamFailure
					? "STREAM_FAILED"
					: recordedTimeout
						? "FRONTEND_TOOL_TIMEOUT"
						: recordedError
							? "COMPLETED_WITH_ERRORS"
							: recordedPartial
								? "PARTIAL"
								: "OK",
			summary: resumeMissed
				? "The live run could not be resumed."
				: streamFailure
					? streamFailure
					: recordedTimeout
						? "The agent turn ended after a frontend tool timeout; late mutations were blocked."
						: recordedError
							? "The agent turn completed with one or more recorded errors."
							: recordedPartial
								? "The agent turn completed partially because an action was partial, denied, or cancelled."
								: "Agent turn completed.",
			outputPreview: acc.content,
		});

		if (!resumeMissed) {
			const reportState = store.getState();
			responseMessage.inner.content = acc.content;
			responseMessage.plan_steps = [
				...orderedSteps(acc),
				...finalState.subPlanSteps.map((step) =>
					step.status === "progress"
						? {
								...step,
								status: streamFailure ? ("failed" as const) : ("done" as const),
							}
						: step,
				),
			];
			responseMessage.current_step_id = undefined;
			responseMessage.tools = [];
			responseMessage.app_refs =
				finalState.pendingAppRefs.length > 0
					? [...finalState.pendingAppRefs]
					: undefined;
			responseMessage.files = finalState.subAttachments;
			responseMessage.widgets =
				finalState.subWidgets.length > 0
					? [...finalState.subWidgets]
					: undefined;
			const finalUsage = mergeUsageStats(
				acc.usageStats,
				finalState.subUsageStats,
			);
			responseMessage.usage_stats =
				finalUsage.length > 0 ? finalUsage : undefined;
			responseMessage.debug_report =
				reportState.debugReport?.message_id === responseMessage.id
					? reportState.debugReport
					: undefined;
			const finalized = { ...responseMessage };
			finalState.commitMessage(finalized);
			void persistGlobalChatMessage(finalized);
			finalState.clearPendingAppRefs();
			finalState.clearSubPlanSteps();
			finalState.clearSubAttachments();
			finalState.clearSubUsageStats();
			finalState.clearSubWidgets();
		}

		// Always release the stream regardless of commit vs. kept-checkpoint.
		finalState.setStreamingMessage(null);
		finalState.setStreaming(false);
		finalState.clearDebugReport(responseMessage.id);
		clearActiveRun(responseMessage.id);
		endGlobalChatTurnSelection(responseMessage.id);
	}
}

/**
 * If a response was still streaming when the webview reloaded, re-attach to the Rust run and keep
 * rendering it live (the generation never stopped — it just lost its channel). No-op when nothing is
 * in flight, the pending run is for another conversation, or a turn is already streaming. Safe to
 * call from multiple mounted surfaces — the first to flip `isStreaming` claims it; a run the server
 * has already GC'd resolves `attached: false`, leaving the restored checkpoint untouched.
 */
export function resumeGlobalChatStream() {
	// Resume re-attaches to a live Rust run registry that only exists on the desktop; browser runs
	// are ephemeral (non-resumable), so this is a no-op there.
	if (!isTauri()) return;
	const state = useGlobalChatStore.getState();
	if (state.isStreaming) return;
	const active = readActiveRun();
	if (!active || active.conversationId !== state.activeConversationId) return;
	const agentSelection =
		active.agentSelection ??
		Object.freeze({
			provider: state.provider,
			selectedModelId: state.selectedModelId,
			reasoningEffort: state.reasoningEffort,
		});
	beginGlobalChatTurnSelection(active.runId, agentSelection);
	// Claim the resume synchronously so a concurrently-mounted surface can't double-attach.
	state.setStreaming(true);
	const responseMessage = makeGlobalChatMessage(
		IRole.Assistant,
		"",
		active.conversationId,
	);
	// The message id IS the run id — the Rust replay rebuilds this message from the buffer, and the
	// finalized result upserts over the restored checkpoint (same id) instead of duplicating it.
	responseMessage.id = active.runId;
	state.setStreamingMessage({ ...responseMessage });
	void driveGlobalChatStream({
		responseMessage,
		agentSelection,
		isResume: true,
		start: tauriStart("global_chat_resume", { runId: active.runId }),
	});
}
