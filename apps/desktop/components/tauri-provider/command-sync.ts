import type { IGenericCommand } from "@flow-like/flow-like-ui";

/**
 * Keep each command-sync POST safely under the server's request body limit (axum
 * defaults to 2MB); large FlowScript applies can produce thousands of commands whose
 * single-body push previously failed with HTTP 413 and stranded the board unsynced.
 */
export const MAX_COMMAND_SYNC_BODY_BYTES = 800 * 1024;

export const OFFLINE_SYNC_COMMAND_MAX_AGE_MS = 7 * 24 * 60 * 60 * 1000;

/** Split a command batch into order-preserving chunks that each serialize below the body cap. */
export function chunkCommandsForSync(
	commands: IGenericCommand[],
): IGenericCommand[][] {
	const chunks: IGenericCommand[][] = [];
	let current: IGenericCommand[] = [];
	let currentBytes = 0;

	for (const command of commands) {
		const commandBytes = JSON.stringify(command).length;
		if (
			current.length > 0 &&
			currentBytes + commandBytes > MAX_COMMAND_SYNC_BODY_BYTES
		) {
			chunks.push(current);
			current = [];
			currentBytes = 0;
		}
		current.push(command);
		currentBytes += commandBytes;
	}

	if (current.length > 0) {
		chunks.push(current);
	}

	return chunks;
}
