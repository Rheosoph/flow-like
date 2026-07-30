import { createId } from "@paralleldrive/cuid2";
import {
	isAgentBackendProvider,
	normalizeAIProvider,
} from "../../components/flowpilot/types";
import { copilotBackendConnectionCoordinator } from "../../hooks/copilot-backend-coordinator";
import {
	FLOWPILOT_DEBUG_ENABLED,
	stripFlowPilotDebugReport,
} from "../../lib/flowpilot-debug";
import {
	classifyAgentBackendError,
	formatAgentBackendFailure,
	shouldPersistAgentBackendDiagnostic,
} from "../../lib/flowpilot/agent-backend-diagnostics";
import { isTauri } from "../../lib/platform";
import { IRole } from "../../lib/schema/llm/history";
import {
	createAgentDebugStreamRecorder,
	summarizeAgentDebugRootOutcomes,
} from "./agent-debug-report";
import {
	applyStreamEvent,
	createStreamAccumulator,
	orderedSteps,
} from "./copilot-stream-steps";
import {
	GLOBAL_CHAT_APP_ID,
	type IMessage,
	globalChatDb,
} from "./global-chat-db";
import {
	getGlobalChatRunControl,
	registerGlobalChatRunControl,
	takeUnconsumedSteering,
	tauriGlobalChatRunControl,
	unregisterGlobalChatRunControl,
} from "./global-chat-run-control";
import {
	type GlobalChatAgentSelection,
	LAST_CONVERSATION_KEY,
	beginGlobalChatTurnSelection,
	useGlobalChatStore,
} from "./global-chat-store";

// The global-chat streaming engine lives here — OUTSIDE any React component — so a turn keeps
// streaming, checkpointing, and finalizing even as the conversation morphs between the /chat page
// and the docked overlay (each unmounts the other) or the webview hard-reloads. All state is read
// from / written to the zustand store, never component-local refs, and every write is addressed by
// run id so N turns can stream at once. The Rust side mirrors every run into a resumable buffer
// keyed by `runId`; `resumeGlobalChatStream` re-attaches to all of them after a reload.

/** Session-scoped pointers to in-flight runs so a reload can re-attach to the live Rust streams. */
const ACTIVE_RUN_KEY = "flow-like:global-chat:active-run";

interface ActiveRun {
	conversationId: string;
	runId: string;
	agentSelection?: GlobalChatAgentSelection;
}

function writeActiveRuns(runs: ActiveRun[]) {
	try {
		if (runs.length === 0) sessionStorage.removeItem(ACTIVE_RUN_KEY);
		else sessionStorage.setItem(ACTIVE_RUN_KEY, JSON.stringify(runs));
	} catch {
		// resumability is best-effort
	}
}

/** Remember a run that is streaming, so a reload mid-response can re-attach to it. */
export function setActiveRun(
	conversationId: string,
	runId: string,
	agentSelection?: GlobalChatAgentSelection,
) {
	const entry: ActiveRun = {
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
	};
	writeActiveRuns([
		...readActiveRuns().filter((run) => run.runId !== runId),
		entry,
	]);
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

function parseActiveRun(value: unknown): ActiveRun | null {
	if (!value || typeof value !== "object") return null;
	const parsed = value as Record<string, unknown>;
	if (
		typeof parsed.conversationId !== "string" ||
		typeof parsed.runId !== "string"
	) {
		return null;
	}
	return {
		conversationId: parsed.conversationId,
		runId: parsed.runId,
		agentSelection: readAgentSelection(parsed.agentSelection),
	};
}

/** Every run that was in flight when the pointer was last written. */
export function readActiveRuns(): ActiveRun[] {
	try {
		const raw = sessionStorage.getItem(ACTIVE_RUN_KEY);
		if (!raw) return [];
		const parsed = JSON.parse(raw);
		// Tolerate the pre-concurrency single-object shape so an in-progress reload still resumes.
		const entries = Array.isArray(parsed) ? parsed : [parsed];
		return entries
			.map(parseActiveRun)
			.filter((run): run is ActiveRun => run !== null);
	} catch {
		return [];
	}
}

/** The most recently started in-flight run, or null. */
export function readActiveRun(): ActiveRun | null {
	const runs = readActiveRuns();
	return runs.length > 0 ? runs[runs.length - 1] : null;
}

export function clearActiveRun(runId?: string) {
	if (!runId) {
		writeActiveRuns([]);
		return;
	}
	const remaining = readActiveRuns().filter((run) => run.runId !== runId);
	writeActiveRuns(remaining);
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
	/** Short human label for this run's stop/steer controls (usually the user's prompt). */
	label?: string;
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
 * plan steps, mirror it into the store's per-run `streamingMessages` view (throttled checkpoints
 * to IndexedDB),
 * and finalize into `messages` when the run ends. Safe to run detached from any component.
 */
export async function driveGlobalChatStream({
	responseMessage,
	agentSelection,
	label,
	isResume,
	inputPreview,
	start,
}: DriveOptions) {
	const store = useGlobalChatStore;
	const runId = responseMessage.id;
	const acc = createStreamAccumulator();
	let lastCheckpoint = 0;
	let streamFailure: string | undefined;
	const turnSelection = beginGlobalChatTurnSelection(runId, agentSelection);
	// Register the run BEFORE anything streams: every per-run store write (sub-agent buffers, debug
	// events, the bubble itself) is addressed by run id and is a no-op until the record exists.
	store.getState().startRun({
		runId,
		conversationId: responseMessage.sessionId,
		selection: turnSelection,
		label: label?.trim() || "Assistant turn",
		message: { ...responseMessage },
	});
	// Desktop control is derivable from the run id alone. The web transport replaces this with an
	// SSE-addressed control once the server hands back its own run id.
	if (isTauri()) {
		registerGlobalChatRunControl(runId, tauriGlobalChatRunControl(runId));
	}
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

	// The engine only writes the fields IT owns. Nested sub-agent activity (plan steps, widgets,
	// attachments, usage, app refs) and the debug report are folded in by the store, per run, so a
	// concurrent turn's nested output can never leak into this bubble.
	const syncMessage = () => {
		const state = store.getState();
		// New `inner` identity per sync: snapshots in the store must not share a mutating object,
		// and MessageComponent's memo compares `inner.content` by identity.
		responseMessage.inner = { ...responseMessage.inner, content: acc.content };
		responseMessage.plan_steps = orderedSteps(acc);
		responseMessage.current_step_id = acc.currentStepId;
		responseMessage.tools = acc.currentStepId ? ["working"] : [];
		responseMessage.usage_stats =
			acc.usageStats.length > 0 ? acc.usageStats : undefined;
		state.setRunMessage(runId, { ...responseMessage });
		const now = Date.now();
		if (now - lastCheckpoint > 1_000) {
			lastCheckpoint = now;
			// Checkpoint the FOLDED bubble so a restored conversation keeps the nested output too.
			const folded = store.getState().runs[runId]?.message;
			if (folded) void persistGlobalChatMessage({ ...folded });
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
		const normalizedProvider = normalizeAIProvider(turnSelection.provider);
		let message = error instanceof Error ? error.message : String(error);
		if (isAgentBackendProvider(normalizedProvider)) {
			const diagnostic = classifyAgentBackendError(normalizedProvider, error);
			if (diagnostic && shouldPersistAgentBackendDiagnostic(diagnostic)) {
				copilotBackendConnectionCoordinator.reportFailure(
					normalizedProvider,
					error,
				);
			}
			message = formatAgentBackendFailure(turnSelection.provider, error);
		}
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
			content_offset: acc.content.length,
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
		const reportEvents =
			store.getState().runs[runId]?.debugReport?.events ?? [];
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
			// Settle this run's own steps, push them through the store so the sub-agent buffers fold
			// in one last time, then read the folded bubble back as the message to commit.
			responseMessage.inner = {
				...responseMessage.inner,
				content: acc.content,
			};
			responseMessage.plan_steps = orderedSteps(acc);
			responseMessage.current_step_id = undefined;
			responseMessage.tools = [];
			responseMessage.usage_stats =
				acc.usageStats.length > 0 ? acc.usageStats : undefined;
			store.getState().setRunMessage(runId, { ...responseMessage });
			const folded = store.getState().runs[runId]?.message ?? responseMessage;
			const finalized: IMessage = {
				...folded,
				plan_steps: folded.plan_steps?.map((step) =>
					step.status === "progress"
						? {
								...step,
								status: streamFailure ? ("failed" as const) : ("done" as const),
							}
						: step,
				),
			};
			store.getState().commitMessage(finalized);
			void persistGlobalChatMessage(finalized);
		}

		// A turn can end before it reaches a boundary where steering could be folded in. Recover
		// anything the backend never consumed and re-send it as its own turn — the user watched
		// that instruction get accepted, so dropping it here would be the worst outcome.
		const conversationId = responseMessage.sessionId;
		if (isTauri()) {
			void takeUnconsumedSteering(runId).then((leftovers) => {
				for (const content of leftovers) {
					store.getState().enqueueMessage({ conversationId, content });
				}
				if (leftovers.length > 0) void drainGlobalChatQueue?.(conversationId);
			});
		}

		// Always release the run regardless of commit vs. kept-checkpoint. endRun drops the record
		// (and with it every per-run buffer), so no explicit clearSub* calls are needed.
		store.getState().clearDebugReport(runId);
		store.getState().endRun(runId);
		unregisterGlobalChatRunControl(runId);
		clearActiveRun(runId);
		void drainGlobalChatQueue?.(conversationId);
	}
}

/**
 * Hook the chat surface installs so a finishing run pulls the next queued message. Lives here (not
 * in the component) because the run that finishes may outlive the surface that started it.
 */
type GlobalChatQueueDrain = (conversationId: string) => void | Promise<void>;

let drainGlobalChatQueue: GlobalChatQueueDrain | undefined;

export function setGlobalChatQueueDrain(drain: GlobalChatQueueDrain) {
	drainGlobalChatQueue = drain;
}

/**
 * Release the hook only if `drain` still owns the slot. The /chat page and the docked overlay are
 * both chat surfaces and can be mounted together; without the ownership check, whichever unmounts
 * first would tear out the survivor's hook and the queue would stop draining.
 */
export function clearGlobalChatQueueDrain(drain: GlobalChatQueueDrain) {
	if (drainGlobalChatQueue === drain) drainGlobalChatQueue = undefined;
}

/**
 * Stop one in-flight turn. The bubble stays on screen and finalizes with whatever it had — a
 * cancelled turn is a partial answer, not a disappearance.
 */
export async function cancelGlobalChatRun(runId: string): Promise<boolean> {
	const store = useGlobalChatStore;
	const run = store.getState().runs[runId];
	if (!run || run.status === "cancelling") return false;
	store.getState().setRunStatus(runId, "cancelling");
	// Tear the transport down locally first so the stream stops rendering even if the backend
	// request fails; the run's own finally block still commits the partial reply.
	try {
		run.abort?.();
	} catch {
		// teardown is best-effort
	}
	const control = getGlobalChatRunControl(runId);
	if (!control) return false;
	try {
		await control.cancel();
		return true;
	} catch {
		return false;
	}
}

/**
 * Push a user instruction into a turn that is already running. The text is committed to the
 * transcript on success so the next turn's history contains what the user actually said.
 */
export async function steerGlobalChatRun(
	runId: string,
	content: string,
): Promise<boolean> {
	const trimmed = content.trim();
	if (!trimmed) return false;
	const store = useGlobalChatStore;
	const run = store.getState().runs[runId];
	if (!run || run.status !== "streaming") return false;

	const steerId = createId();
	store.getState().addRunSteer(runId, {
		id: steerId,
		content: trimmed,
		status: "pending",
		createdAt: Date.now(),
	});

	const fail = (message: string) => {
		store.getState().setRunSteerStatus(runId, steerId, "failed", message);
		return false;
	};

	const control = getGlobalChatRunControl(runId);
	if (!control) {
		return fail("This run cannot take mid-run messages.");
	}
	try {
		await control.steer(trimmed);
	} catch (error) {
		return fail(error instanceof Error ? error.message : String(error));
	}
	store.getState().setRunSteerStatus(runId, steerId, "delivered");
	// Commit it as a real user turn: the agent is acting on it, so the transcript (and therefore
	// the next turn's history) has to contain it.
	const steerMessage = makeGlobalChatMessage(
		IRole.User,
		trimmed,
		run.conversationId,
	);
	store.getState().appendMessage(steerMessage);
	void persistGlobalChatMessage(steerMessage);
	return true;
}

/**
 * If responses were still streaming when the webview reloaded, re-attach to EVERY live Rust run of
 * the active conversation and keep rendering them (the generations never stopped — they just lost
 * their channels). Runs already attached in this session are skipped, so it is safe to call from
 * several mounted surfaces; a run the server has already GC'd resolves `attached: false`, leaving
 * the restored checkpoint untouched.
 */
export function resumeGlobalChatStream() {
	// Resume re-attaches to a live Rust run registry that only exists on the desktop; browser runs
	// are ephemeral (non-resumable), so this is a no-op there.
	if (!isTauri()) return;
	const state = useGlobalChatStore.getState();
	const pending = readActiveRuns().filter(
		(active) =>
			active.conversationId === state.activeConversationId &&
			// Already attached (another surface got here first, or the run never lost its channel).
			!state.runs[active.runId],
	);
	for (const active of pending) {
		const agentSelection =
			active.agentSelection ??
			Object.freeze({
				provider: state.provider,
				selectedModelId: state.selectedModelId,
				reasoningEffort: state.reasoningEffort,
			});
		const responseMessage = makeGlobalChatMessage(
			IRole.Assistant,
			"",
			active.conversationId,
		);
		// The message id IS the run id — the Rust replay rebuilds this message from the buffer, and
		// the finalized result upserts over the restored checkpoint (same id) rather than duplicating.
		responseMessage.id = active.runId;
		// Seed the live bubble from the restored checkpoint: the partial reply stays on screen
		// (instead of an empty "Thinking…" bubble) until the Rust buffer replay catches up, and the
		// original timestamp is kept so the finalized message doesn't reorder below later turns.
		const checkpoint = state.messages.find(
			(message) => message.id === active.runId,
		);
		if (checkpoint) {
			responseMessage.inner = { ...checkpoint.inner };
			responseMessage.plan_steps = checkpoint.plan_steps;
			responseMessage.current_step_id = checkpoint.current_step_id;
			responseMessage.usage_stats = checkpoint.usage_stats;
			responseMessage.files = checkpoint.files ?? [];
			responseMessage.widgets = checkpoint.widgets;
			responseMessage.app_refs = checkpoint.app_refs;
			responseMessage.timestamp = checkpoint.timestamp;
		}
		void driveGlobalChatStream({
			responseMessage,
			agentSelection,
			isResume: true,
			label: "Resumed turn",
			start: tauriStart("global_chat_resume", { runId: active.runId }),
		});
	}
}

/**
 * Mid-stream checkpoints persist unsettled steps — settle them so a restored message doesn't
 * render an eternal spinner when its run is long gone. A run that IS still live gets re-attached
 * right after and rebuilds the live statuses from the Rust replay buffer.
 */
export function normalizeRestoredCheckpoint(message: IMessage): IMessage {
	return {
		...message,
		current_step_id: undefined,
		tools: [],
		plan_steps: message.plan_steps?.map((step) =>
			step.status === "progress" || step.status === "planned"
				? { ...step, status: "done" as const }
				: step,
		),
	};
}

/**
 * The ONE way to bring a persisted conversation back on screen — used by the mount-restore path
 * and the history popover. Loads + normalizes the transcript, repoints the reload-restore key,
 * and re-attaches any of the conversation's still-live runs.
 *
 * With `skipIfBusy`, the load is dropped when the store picked up messages, a live run, or a
 * pending draft while Dexie was reading — mount-time restore must never clobber those.
 */
export async function restoreGlobalChatConversation(
	conversationId: string,
	options?: { skipIfBusy?: boolean },
): Promise<boolean> {
	const restored = await globalChatDb.messages
		.where("sessionId")
		.equals(conversationId)
		.sortBy("timestamp");
	const state = useGlobalChatStore.getState();
	if (
		options?.skipIfBusy &&
		(restored.length === 0 ||
			state.messages.length > 0 ||
			state.isStreaming ||
			state.draft !== null)
	) {
		return false;
	}
	state.loadConversation(
		conversationId,
		restored.map(normalizeRestoredCheckpoint),
	);
	try {
		sessionStorage.setItem(LAST_CONVERSATION_KEY, conversationId);
	} catch {
		// restore pointer is best-effort
	}
	resumeGlobalChatStream();
	return true;
}
