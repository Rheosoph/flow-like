import { IRole } from "@flow-like/flow-like-ui";
import { createCopilotStreamParser } from "@flow-like/flow-like-ui/components/flowpilot/copilot-stream-parser";
import { createId } from "@paralleldrive/cuid2";
import { Channel, invoke } from "@tauri-apps/api/core";
import {
	applyStreamEvent,
	createStreamAccumulator,
	mergeUsageStats,
	orderedSteps,
} from "../components/global-chat/copilot-stream-steps";
import {
	GLOBAL_CHAT_APP_ID,
	type IMessage,
	globalChatDb,
} from "./global-chat-db";
import { useGlobalChatStore } from "./global-chat-store";

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
}

/** Remember the run currently streaming, so a reload mid-response can re-attach to it. */
export function setActiveRun(conversationId: string, runId: string) {
	try {
		sessionStorage.setItem(
			ACTIVE_RUN_KEY,
			JSON.stringify({ conversationId, runId }),
		);
	} catch {
		// resumability is best-effort
	}
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
			return parsed as ActiveRun;
		}
	} catch {
		// ignore malformed pointer
	}
	return null;
}

export function clearActiveRun() {
	try {
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
		await globalChatDb.messages.put(message);
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
	/** True for a resume re-attach (guards against overwriting a restored checkpoint on a miss). */
	isResume?: boolean;
	/** Opens the Tauri command with the channel bound (global_chat or global_chat_resume). */
	startInvoke: (channel: Channel<string>) => Promise<unknown>;
}

/**
 * Drive one assistant turn: parse the streamed FlowPilot protocol into the message's content +
 * plan steps, mirror it into the store's `streamingMessage` (throttled checkpoints to IndexedDB),
 * and finalize into `messages` when the run ends. Safe to run detached from any component.
 */
export async function driveGlobalChatStream({
	responseMessage,
	isResume,
	startInvoke,
}: DriveOptions) {
	const store = useGlobalChatStore;
	const parser = createCopilotStreamParser();
	const acc = createStreamAccumulator();
	let lastCheckpoint = 0;

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
		const combinedUsage = mergeUsageStats(acc.usageStats, state.subUsageStats);
		responseMessage.usage_stats =
			combinedUsage.length > 0 ? combinedUsage : undefined;
		state.setStreamingMessage({ ...responseMessage });
		const now = Date.now();
		if (now - lastCheckpoint > 1_000) {
			lastCheckpoint = now;
			void persistGlobalChatMessage({ ...responseMessage });
		}
	};

	const channel = new Channel<string>();
	channel.onmessage = (chunk) => {
		for (const event of parser.push(chunk)) applyStreamEvent(acc, event);
		syncMessage();
	};

	let invokeResult: unknown;
	try {
		invokeResult = await startInvoke(channel);
	} catch (error) {
		const message = error instanceof Error ? error.message : String(error);
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
		for (const event of parser.flush()) applyStreamEvent(acc, event);
		for (const id of acc.stepOrder) {
			const step = acc.steps.get(id);
			if (step?.status === "progress") {
				acc.steps.set(id, { ...step, status: "done" });
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

		if (!resumeMissed) {
			responseMessage.inner.content = acc.content;
			responseMessage.plan_steps = [
				...orderedSteps(acc),
				...finalState.subPlanSteps.map((step) =>
					step.status === "progress"
						? { ...step, status: "done" as const }
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
			const finalUsage = mergeUsageStats(
				acc.usageStats,
				finalState.subUsageStats,
			);
			responseMessage.usage_stats =
				finalUsage.length > 0 ? finalUsage : undefined;
			const finalized = { ...responseMessage };
			finalState.commitMessage(finalized);
			void persistGlobalChatMessage(finalized);
			finalState.clearPendingAppRefs();
			finalState.clearSubPlanSteps();
			finalState.clearSubAttachments();
			finalState.clearSubUsageStats();
		}

		// Always release the stream regardless of commit vs. kept-checkpoint.
		finalState.setStreamingMessage(null);
		finalState.setStreaming(false);
		clearActiveRun();
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
	const state = useGlobalChatStore.getState();
	if (state.isStreaming) return;
	const active = readActiveRun();
	if (!active || active.conversationId !== state.activeConversationId) return;
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
		isResume: true,
		startInvoke: (channel) =>
			invoke("global_chat_resume", { runId: active.runId, channel }),
	});
}
