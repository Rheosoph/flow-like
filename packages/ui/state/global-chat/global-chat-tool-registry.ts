// Bridges the mounted `GlobalToolBridge` component's tool executor to callers that live outside it.
//
// On desktop, tool requests arrive on a Tauri event the bridge listens to directly, so this registry
// is unused. On the web the requests arrive inside the chat SSE stream (see `global-chat-web-transport`),
// which has no access to the bridge's React state — so the bridge registers its executor here and the
// web transport looks it up. There is a single global assistant, hence a single module-level slot.

import type {
	FrontendToolRequest,
	FrontendToolResponse,
} from "../../components/global-chat/global-tool-bridge";

export type GlobalChatToolExecutor = (
	request: FrontendToolRequest,
) => Promise<FrontendToolResponse>;

let executor: GlobalChatToolExecutor | null = null;

/** Called by the mounted `GlobalToolBridge` to (de)register its live executor. */
export function registerGlobalChatToolExecutor(
	fn: GlobalChatToolExecutor | null,
): void {
	executor = fn;
}

/** Run a browser tool request through the mounted bridge, or deny it if none is mounted. */
export function runGlobalChatTool(
	request: FrontendToolRequest,
): Promise<FrontendToolResponse> {
	if (!executor) {
		return Promise.resolve({
			requestId: request.requestId,
			approved: false,
			error: "The FlowPilot tool bridge is not mounted in this session.",
		});
	}
	return executor(request);
}
