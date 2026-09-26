import { z } from "zod";
import type { ManagementCall } from "./telemetry";

const integer = z.number().int().nonnegative().max(Number.MAX_SAFE_INTEGER);
const scope = z.string().regex(/^[a-f0-9]{64}$/);
const queuedOperation = z.object({
	sequence: integer,
	operation_id: z.string().uuid(),
	resource: z.string().max(4096),
	payload: z.null(),
	state: z.enum([
		"pending",
		"attempting",
		"blocked",
		"conflict",
		"outcome_unknown",
	]),
	attempts: integer,
	created_at: integer,
	error: z.string().max(2048).nullable(),
	local_version: integer.nullable(),
});
const queue = z.object({
	scope,
	quarantined: z.boolean(),
	pending_count: integer,
	pending_bytes: integer,
	oldest_at: integer.nullable(),
	mirror_error: z.string().max(2048).nullish(),
	head: queuedOperation.nullable(),
});
const page = z.object({
	placement_id: z.string(),
	queues: z.array(queue).max(2),
	next: scope.nullable(),
});
export type OfflineQueueStatus = z.infer<typeof queue>;

/** Read only queue metadata. File bodies and table rows never enter this view. */
export async function readOfflineQueues(
	call: ManagementCall,
	placement: string,
): Promise<OfflineQueueStatus[]> {
	const queues: OfflineQueueStatus[] = [];
	let after: string | null = null;
	for (let count = 0; count < 32; count++) {
		const response = await call({
			type: "offline_queue",
			placement_id: placement,
			after,
		});
		if (response.state !== "completed")
			throw new Error("The device could not read its offline queues.");
		const current = page.parse(response.result);
		if (current.placement_id !== placement)
			throw new Error("Offline queues belong to another placement.");
		for (const entry of current.queues) {
			if (
				(after !== null && entry.scope <= after) ||
				(queues.length > 0 && entry.scope <= queues[queues.length - 1].scope)
			)
				throw new Error(
					"Offline queue pages changed during transfer. Refresh the queues.",
				);
			queues.push(entry);
		}
		if (current.next === null) return queues;
		if (current.queues.length !== 2 || current.next !== current.queues[1].scope)
			throw new Error("Invalid offline queue page cursor.");
		after = current.next;
	}
	throw new Error("Offline queue inventory exceeds 64 authorization scopes.");
}

export function retryOfflineQueue(
	placement: string,
	status: OfflineQueueStatus,
): Record<string, unknown> {
	if (
		status.quarantined ||
		!status.head ||
		!["blocked", "conflict", "outcome_unknown"].includes(status.head.state)
	)
		throw new Error("Only an authorized, blocked queue head can be retried.");
	return {
		type: "offline_queue_retry",
		placement_id: placement,
		scope: status.scope,
		queued_operation_id: status.head.operation_id,
	};
}

export function skipOfflineQueue(
	placement: string,
	status: OfflineQueueStatus,
	reason: string,
	acknowledgeUncertain: boolean,
): Record<string, unknown> {
	if (status.quarantined || !status.head)
		throw new Error("This offline queue cannot be skipped.");
	const trimmed = reason.trim();
	// Leave room for the device to prepend the authenticated operator identity.
	if (!trimmed || new TextEncoder().encode(trimmed).length > 512)
		throw new Error("Enter a skip reason of at most 512 UTF-8 bytes.");
	if (status.head.attempts > 0 && !acknowledgeUncertain)
		throw new Error(
			"Acknowledge that skipping cannot undo a write that reached the cloud.",
		);
	return {
		type: "offline_queue_skip",
		placement_id: placement,
		scope: status.scope,
		queued_operation_id: status.head.operation_id,
		reason: trimmed,
		acknowledge_uncertain: acknowledgeUncertain,
	};
}
