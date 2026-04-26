import { dexieTauriBlobOffload } from "@flow-like/dexie-tauri-blob-offload";
import { chatDb } from "@tm9657/flow-like-ui/components/interfaces/chat-default/chat-db";
import { flowpilotDB } from "@tm9657/flow-like-ui/lib/flowpilot-db";
import { runtimeVarsDB } from "./runtime-vars-db";

let initialized = false;

/**
 * Apply the Tauri blob offload middleware to Dexie databases that store
 * potentially large values (base64 images, binary data, large strings).
 *
 * Call once at app startup before any DB operations.
 */
export function initBlobOffload(threshold = 200) {
	if (initialized) return;
	initialized = true;

	const middleware = dexieTauriBlobOffload(threshold);

	// FlowPilot: images[].data is raw base64
	flowpilotDB.use(middleware);

	// Chat: inner has embedded images, files has attachments
	chatDb.use(middleware);

	// Runtime vars: value is number[] which can be large byte arrays
	runtimeVarsDB.use(middleware);
}
