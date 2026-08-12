/**
 * Per-run control channel: how to cancel a live turn, and how to push a steering instruction into
 * one that is already running.
 *
 * This exists as its own module because the two transports address a run differently. On the
 * desktop the run id the frontend minted IS the id Rust registered, so control is a pair of Tauri
 * commands. On the web the server mints its own run id and hands it back in the opening SSE frame,
 * so only the transport knows the address to POST to — it registers a control here under the
 * *client* run id once that frame arrives. Callers (the composer, the stop button) only ever see
 * the client run id and never have to care which transport is underneath.
 */

export interface GlobalChatRunControl {
	/** Stop the run. Resolves once the request is delivered, not once the run has torn down. */
	cancel: () => Promise<void>;
	/** Push a user instruction into the running turn. Rejects when the backend refuses it. */
	steer: (content: string) => Promise<void>;
}

const controls = new Map<string, GlobalChatRunControl>();

export function registerGlobalChatRunControl(
	runId: string,
	control: GlobalChatRunControl,
) {
	controls.set(runId, control);
}

export function unregisterGlobalChatRunControl(runId: string) {
	controls.delete(runId);
}

export function getGlobalChatRunControl(
	runId: string,
): GlobalChatRunControl | undefined {
	return controls.get(runId);
}

/**
 * Instructions a finished run never folded in — it ended before reaching a round/idle boundary, or
 * an external CLI run never restarted a phase. The caller re-sends them as their own turn so a
 * steering message the user watched get accepted is never silently swallowed.
 */
export async function takeUnconsumedSteering(runId: string): Promise<string[]> {
	try {
		const { invoke } = await import("@tauri-apps/api/core");
		const leftovers = await invoke<string[]>(
			"global_chat_take_unconsumed_steering",
			{ runId },
		);
		return Array.isArray(leftovers) ? leftovers : [];
	} catch {
		return [];
	}
}

/**
 * Desktop control: both operations address the Rust run registry by the run id the frontend
 * generated (which is also the assistant message id).
 */
export function tauriGlobalChatRunControl(runId: string): GlobalChatRunControl {
	// Tauri is imported lazily so this module also loads in the browser bundle.
	const core = () => import("@tauri-apps/api/core");
	return {
		cancel: async () => {
			const { invoke } = await core();
			await invoke("cancel_copilot_chat", { requestId: runId });
		},
		steer: async (content: string) => {
			const { invoke } = await core();
			const accepted = await invoke<boolean>("global_chat_steer", {
				runId,
				message: content,
			});
			if (!accepted) {
				throw new Error("The run finished before it could take this message.");
			}
		},
	};
}
