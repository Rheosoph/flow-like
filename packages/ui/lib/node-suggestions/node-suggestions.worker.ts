/**
 * Node-suggestion worker: fact extraction, training and ranking off the UI thread. All logic lives
 * in `worker-protocol.ts` so tests can drive it without a real Worker.
 */

import {
	type NodeSuggestionRequest,
	NodeSuggestionWorkerCore,
} from "./worker-protocol";

const core = new NodeSuggestionWorkerCore((response) =>
	self.postMessage(response),
);

self.onmessage = (event: MessageEvent<NodeSuggestionRequest>) => {
	void core.handle(event.data);
};
