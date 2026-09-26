import { expect, test } from "bun:test";
import {
	type OfflineQueueStatus,
	readOfflineQueues,
	retryOfflineQueue,
	skipOfflineQueue,
} from "./offline-queue";
import type { ManagementCall } from "./telemetry";

const status = (scope = "a".repeat(64)): OfflineQueueStatus => ({
	scope,
	quarantined: false,
	pending_count: 2,
	pending_bytes: 1048576,
	oldest_at: 100,
	head: {
		sequence: 1,
		operation_id: "00000000-0000-4000-8000-000000000001",
		resource: '{"kind":"file","purpose":"files","path":"exports/report"}',
		payload: null,
		state: "conflict",
		attempts: 1,
		created_at: 100,
		error: "Cloud file changed",
		local_version: 1,
	},
});
const response = (
	queues: OfflineQueueStatus[],
	next: string | null = null,
) => ({
	operation_id: "read",
	state: "completed",
	result: { placement_id: "placement", queues, next },
});

test("offline queue inventory reads bounded ordered metadata pages", async () => {
	const calls: Record<string, unknown>[] = [];
	const result = await readOfflineQueues(async (command) => {
		calls.push(command);
		return calls.length === 1
			? response([status(), status("b".repeat(64))], "b".repeat(64))
			: response([status("c".repeat(64))]);
	}, "placement");
	expect(result).toHaveLength(3);
	expect(result.reduce((count, queue) => count + queue.pending_count, 0)).toBe(
		6,
	);
	expect(calls).toEqual([
		{ type: "offline_queue", placement_id: "placement", after: null },
		{ type: "offline_queue", placement_id: "placement", after: "b".repeat(64) },
	]);
});

test("queue status rejects bodies, another placement, and nonadvancing pages", async () => {
	const body = status();
	const withBody = {
		...body,
		head: { ...body.head, payload: { secret: "must remain on the device" } },
	};
	await expect(
		readOfflineQueues(
			(async () =>
				response([
					withBody as unknown as OfflineQueueStatus,
				])) as ManagementCall,
			"placement",
		),
	).rejects.toThrow();
	await expect(
		readOfflineQueues(async () => response([status()]), "another"),
	).rejects.toThrow("another placement");
	await expect(
		readOfflineQueues(
			async () => response([status(), status("b".repeat(64))], "a".repeat(64)),
			"placement",
		),
	).rejects.toThrow("cursor");
	await expect(
		readOfflineQueues(
			async () => response([status(), status("b".repeat(64))], "b".repeat(64)),
			"placement",
		),
	).rejects.toThrow("changed");
});

test("retry and skip bind to the queue head and require an uncertain-effect acknowledgement", () => {
	const queue = status();
	expect(retryOfflineQueue("placement", queue)).toEqual({
		type: "offline_queue_retry",
		placement_id: "placement",
		scope: queue.scope,
		queued_operation_id: queue.head?.operation_id,
	});
	expect(() =>
		skipOfflineQueue("placement", queue, "Reviewed conflict", false),
	).toThrow("Acknowledge");
	expect(() => skipOfflineQueue("placement", queue, " ", true)).toThrow(
		"reason",
	);
	expect(() =>
		skipOfflineQueue("placement", queue, "é".repeat(257), true),
	).toThrow("512");
	expect(
		skipOfflineQueue("placement", queue, " Reviewed conflict ", true),
	).toEqual({
		type: "offline_queue_skip",
		placement_id: "placement",
		scope: queue.scope,
		queued_operation_id: queue.head?.operation_id,
		reason: "Reviewed conflict",
		acknowledge_uncertain: true,
	});
	expect(() =>
		retryOfflineQueue("placement", { ...queue, quarantined: true }),
	).toThrow("authorized");
	expect(() =>
		skipOfflineQueue(
			"placement",
			{ ...queue, quarantined: true },
			"reviewed",
			true,
		),
	).toThrow("cannot");
});
