import type { IGenericCommand } from "@flow-like/flow-like-ui";

/**
 * Keep a complete logical mutation below every production ingress limit. The AWS API
 * is a synchronous Lambda whose 6 MiB event contains the HTTP JSON body as an escaped
 * string, so the 16 MiB Axum route limit is not the effective deployment limit. Four
 * MiB is the raw command-body ceiling requested for large plans. An independent
 * escaped-envelope check below keeps both the Function URL request event and the
 * echoed command response below Lambda's 6 MiB synchronous payload limit.
 *
 * A FlowScript command plan is atomic: setup commands are emitted before its pin
 * updates and connections, so splitting the plan across independently persisted
 * requests can expose a board containing all nodes but none of its edges.
 */
export const MAX_COMMAND_SYNC_BODY_BYTES = 4 * 1024 * 1024;

/** AWS Lambda synchronous request/response payload ceiling. */
export const MAX_LAMBDA_SYNC_PAYLOAD_BYTES = 6 * 1024 * 1024;

/**
 * Space for the Function URL request context, auth headers, and response metadata.
 * The command body itself is measured after JSON-string escaping, not estimated.
 */
export const LAMBDA_SYNC_ENVELOPE_RESERVE_BYTES = 256 * 1024;

/** Historical chunk size used only to finish an outbox tail created by older clients. */
export const MAX_LEGACY_COMMAND_SYNC_BODY_BYTES = 800 * 1024;

export const OFFLINE_SYNC_COMMAND_MAX_AGE_MS = 7 * 24 * 60 * 60 * 1000;

/**
 * Undo/redo must ship the full command list in one request — a partial (chunked)
 * undo would diverge the board. They use the same raw-body and escaped-Lambda
 * envelope validation as forward mutations.
 */
export const MAX_UNDO_REDO_SYNC_BODY_BYTES = MAX_COMMAND_SYNC_BODY_BYTES;
export const MAX_SINGLE_COMMAND_SYNC_BODY_BYTES = MAX_COMMAND_SYNC_BODY_BYTES;

export class CommandSyncPayloadTooLargeError extends Error {
	constructor(
		readonly bodyBytes: number,
		readonly commandCount: number,
		readonly lambdaEnvelopeBytes?: number,
	) {
		const detail =
			bodyBytes > MAX_COMMAND_SYNC_BODY_BYTES
				? `serializes to ${bodyBytes} bytes, above the ${MAX_COMMAND_SYNC_BODY_BYTES}-byte raw plan limit`
				: `expands to ${lambdaEnvelopeBytes} bytes in the escaped Lambda envelope, above the ${MAX_LAMBDA_SYNC_PAYLOAD_BYTES}-byte synchronous payload limit`;
		super(
			`The atomic board command batch (${commandCount} command${commandCount === 1 ? "" : "s"}) exceeds the safe sync limit: it ${detail}.`,
		);
		this.name = "CommandSyncPayloadTooLargeError";
	}
}

const utf8Bytes = (value: string): number =>
	new TextEncoder().encode(value).length;

interface CommandSyncPayloadSizes {
	bodyBytes: number;
	lambdaEnvelopeBytes: number;
}

/**
 * Measure both sides of the Function URL contract. Lambda embeds an HTTP body as
 * a JSON string in its request event, and this endpoint echoes the submitted
 * command array as a JSON response body. Quotes and backslashes therefore count
 * again in the Lambda payload even though they count only once on the wire.
 */
function commandSyncPayloadSizes(
	commands: IGenericCommand[],
): CommandSyncPayloadSizes {
	const requestBody = JSON.stringify({ commands });
	const responseBody = JSON.stringify(commands);
	const escapedRequestBytes = utf8Bytes(JSON.stringify(requestBody));
	const escapedResponseBytes = utf8Bytes(JSON.stringify(responseBody));
	return {
		bodyBytes: utf8Bytes(requestBody),
		lambdaEnvelopeBytes:
			Math.max(escapedRequestBytes, escapedResponseBytes) +
			LAMBDA_SYNC_ENVELOPE_RESERVE_BYTES,
	};
}

function assertCommandBatchFitsSync(commands: IGenericCommand[]): void {
	const { bodyBytes, lambdaEnvelopeBytes } = commandSyncPayloadSizes(commands);
	if (
		bodyBytes > MAX_COMMAND_SYNC_BODY_BYTES ||
		lambdaEnvelopeBytes > MAX_LAMBDA_SYNC_PAYLOAD_BYTES
	) {
		throw new CommandSyncPayloadTooLargeError(
			bodyBytes,
			commands.length,
			lambdaEnvelopeBytes,
		);
	}
}

/**
 * Return the exact logical mutation as one transport unit.
 *
 * The function name is retained for persisted-outbox compatibility, but new mutations
 * must never be split: the API executes and saves each request independently. Legacy
 * multi-chunk outbox rows can still drain through the existing recovery path.
 */
export function chunkCommandsForSync(
	commands: IGenericCommand[],
): IGenericCommand[][] {
	if (commands.length === 0) return [];

	assertCommandBatchFitsSync(commands);

	return [commands];
}

/**
 * Finish a flat outbox tail written by the pre-atomic transport.
 *
 * This must never be used for a new logical mutation. A legacy row means the server
 * may already expose a prefix, so preserving its old order and completing the exact
 * tail is the least destructive automatic recovery available. New rows persist an
 * explicit `chunks` array and always contain exactly one atomic chunk.
 */
export function chunkLegacyCommandsForRecovery(
	commands: IGenericCommand[],
): IGenericCommand[][] {
	const chunks: IGenericCommand[][] = [];
	let current: IGenericCommand[] = [];

	for (const command of commands) {
		const candidate = [...current, command];
		const candidateBytes = utf8Bytes(JSON.stringify({ commands: candidate }));
		if (
			current.length > 0 &&
			candidateBytes > MAX_LEGACY_COMMAND_SYNC_BODY_BYTES
		) {
			chunks.push(current);
			current = [command];
		} else {
			current = candidate;
		}

		assertCommandBatchFitsSync(current);
	}

	if (current.length > 0) chunks.push(current);
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

export interface CommandSyncRemoteIdentity {
	remoteIdentityVersion?: 1;
	remoteProfileId?: string;
	remotePrincipalId?: string;
	remoteHub?: string;
}

export interface CommandSyncMutationState {
	commands?: IGenericCommand[];
	chunks?: IGenericCommand[][];
	blockedReason?: string;
}

/** Completed receipt tombstones are recovery metadata, not unsent semantic edits. */
export function commandSyncHasPendingMutation(
	record: CommandSyncMutationState,
): boolean {
	return (
		Boolean(record.blockedReason) ||
		(record.commands?.length ?? 0) > 0 ||
		(record.chunks?.length ?? 0) > 0
	);
}

/**
 * Refuse to drain a durable mutation through a different account or Hub. Legacy
 * rows remain compatible; versioned rows compare even missing fields, so a new
 * mutation created while signed out cannot later attach itself to an arbitrary user.
 */
export function evaluateCommandSyncRemoteIdentity(
	recorded: CommandSyncRemoteIdentity,
	current: CommandSyncRemoteIdentity,
): LineageDecision {
	const strictlyBound = recorded.remoteIdentityVersion === 1;
	if (strictlyBound && !recorded.remotePrincipalId) {
		return {
			apply: false,
			refusalReason: "queued mutation has no bound remote account",
		};
	}
	for (const [label, expected, actual] of [
		["profile", recorded.remoteProfileId, current.remoteProfileId],
		["account", recorded.remotePrincipalId, current.remotePrincipalId],
		["Hub", recorded.remoteHub, current.remoteHub],
	] as const) {
		const exactAccountMismatch =
			strictlyBound && label === "account" && expected !== actual;
		if (exactAccountMismatch || (expected && expected !== actual)) {
			return {
				apply: false,
				refusalReason: `queued mutation belongs to a different remote ${label}`,
			};
		}
	}
	return { apply: true };
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
