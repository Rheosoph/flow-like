import type { IGenericCommand } from "@flow-like/flow-like-ui";

/**
 * Keep each command-sync POST safely under the server's request body limit (axum
 * defaults to 2MB); large FlowScript applies can produce thousands of commands whose
 * single-body push previously failed with HTTP 413 and stranded the board unsynced.
 */
export const MAX_COMMAND_SYNC_BODY_BYTES = 800 * 1024;

export const OFFLINE_SYNC_COMMAND_MAX_AGE_MS = 7 * 24 * 60 * 60 * 1000;

/**
 * Undo/redo must ship the full command list in one request — a partial (chunked)
 * undo would diverge the board. Fail fast below the server's 16MB board-route
 * limit instead of surfacing a raw HTTP 413.
 */
export const MAX_UNDO_REDO_SYNC_BODY_BYTES = 12 * 1024 * 1024;

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

export interface SystemTimeLike {
	secs_since_epoch?: number;
	nanos_since_epoch?: number;
}

export const systemTimeToNanos = (time?: SystemTimeLike | null): number => {
	if (!time) return 0;
	return (
		(time.secs_since_epoch ?? 0) * 1_000_000_000 + (time.nanos_since_epoch ?? 0)
	);
};

export interface LineageDecision {
	apply: boolean;
	refusalReason?: string;
}

/**
 * Lineage guard layered on top of the updated_at last-writer-wins checks: once
 * this client has applied or pushed past a remote revision, only a strictly
 * newer remote revision may overwrite local state. A missing cache leaves the
 * guard inert, so it can only add refusals — it never applies a remote board
 * the existing guards would reject.
 */
export function evaluateBoardLineage(
	remoteUpdatedAtNs: number,
	cachedLineageNs: number | null | undefined,
): LineageDecision {
	if (!cachedLineageNs || cachedLineageNs <= 0) {
		return { apply: true };
	}

	if (remoteUpdatedAtNs <= 0) {
		return {
			apply: false,
			refusalReason:
				"remote board has no updated_at but this client already synced past a known revision",
		};
	}

	if (remoteUpdatedAtNs <= cachedLineageNs) {
		return {
			apply: false,
			refusalReason:
				remoteUpdatedAtNs === cachedLineageNs
					? "remote board updated_at equals the last synced lineage revision"
					: "remote board updated_at is older than the last synced lineage revision",
		};
	}

	return { apply: true };
}
