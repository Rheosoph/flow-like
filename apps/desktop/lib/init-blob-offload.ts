import { dexieTauriBlobOffload } from "@flow-like/dexie-tauri-blob-offload";
import { patchDexieDependencies } from "@flow-like/dexie-tauri-blob-offload/idb-sqlite";
import { chatDb } from "@flow-like/flow-like-ui/components/interfaces/chat-default/chat-db";
import { flowpilotDB } from "@flow-like/flow-like-ui/lib/flowpilot-db";
import Dexie from "dexie";
import { runtimeVarsDB } from "./runtime-vars-db";

let initialized = false;

const CHAT_ROW_BLOB_THRESHOLD = 64 * 1024;

/**
 * Apply the Tauri blob offload middleware to Dexie databases that store
 * potentially large values (base64 images, binary data, large strings).
 *
 * Call once at app startup before any DB operations.
 */
export function initBlobOffload(threshold = 200) {
	if (initialized) return;
	initialized = true;

	// Safety net for import-order slips: Dexie snapshots indexedDB/IDBKeyRange
	// at module evaluation, which may predate the SQLite shim install.
	patchDexieDependencies(Dexie);

	const middleware = dexieTauriBlobOffload(threshold);

	// FlowPilot: images[].data is raw base64
	flowpilotDB.use(middleware);

	// Chat rows are one JSON string each, so the default threshold would send
	// every message through the blob store and cost one IPC per row on every
	// session read. Keep messages inline and offload only image-sized payloads.
	chatDb.use(dexieTauriBlobOffload(CHAT_ROW_BLOB_THRESHOLD));

	// Runtime vars: value is number[] which can be large byte arrays
	runtimeVarsDB.use(middleware);
}
