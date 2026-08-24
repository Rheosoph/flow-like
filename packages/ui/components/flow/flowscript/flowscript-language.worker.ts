/**
 * FlowScript language worker: runs document analysis, the client-side linter,
 * semantic-token encoding and the other per-keystroke language products off
 * the UI thread. All logic lives in `flowscript-worker-protocol.ts` so tests
 * can exercise it without a real Worker.
 */

import {
	type FlowScriptWorkerRequest,
	createFlowScriptWorkerState,
	handleFlowScriptWorkerMessage,
} from "./flowscript-worker-protocol";

const state = createFlowScriptWorkerState();

self.onmessage = (event: MessageEvent<FlowScriptWorkerRequest>) => {
	const response = handleFlowScriptWorkerMessage(state, event.data);
	if (response) self.postMessage(response);
};
