import { describe, expect, test } from "bun:test";
import {
	type AgentOperation,
	type AgentRead,
	type MetricsHistoryPage,
	lookupOfflineOperation,
	pruneArtifactRevisions,
	readArtifactUsage,
	readHostOperation,
	readMetricsHistory,
	readOfflineOperations,
	readOperations,
	readRolloutHistory,
} from "./agent-reads";
import type { DeploymentRolloutStatus } from "./deployment";
import type {
	AgentFeature,
	AgentFeatures,
	HostOperationView,
} from "./model/types";
import type { ManagementCall } from "./telemetry";
import type { ManagementResponse } from "./types";
import { LiveCallError } from "./workspace/errors";

const scope = "a".repeat(64);
const revision = "b".repeat(64);
const queued = "00000000-0000-4000-8000-000000000001";

function completed(
	result: Record<string, unknown>,
	operation_id = "op",
): ManagementResponse {
	return { operation_id, state: "completed", result };
}
function rejected(code: string, error = "Refused."): ManagementResponse {
	return {
		operation_id: "op",
		state: "rejected",
		result: { code, error, retryable: false },
	};
}
function recorder(
	reply: (command: Record<string, unknown>) => ManagementResponse,
) {
	const sent: Record<string, unknown>[] = [];
	const call: ManagementCall = async (command, operationId) => {
		sent.push(command);
		const response = reply(command);
		return operationId ? { ...response, operation_id: operationId } : response;
	};
	return { call, sent };
}

const READS: {
	feature: AgentFeature;
	read: (
		call: ManagementCall,
		features: AgentFeatures | undefined,
	) => Promise<AgentRead<unknown>>;
}[] = [
	{ feature: "host_operation", read: (c, f) => readHostOperation(c, f) },
	{
		feature: "rollout_history",
		read: (c, f) => readRolloutHistory(c, f, { placementId: "p1" }),
	},
	{ feature: "operations", read: (c, f) => readOperations(c, f) },
	{
		feature: "metrics_history",
		read: (c, f) =>
			readMetricsHistory(c, f, { placementId: null, fields: ["cpu_percent"] }),
	},
	{
		feature: "offline_lookup",
		read: (c, f) => readOfflineOperations(c, f, { placementId: "p1", scope }),
	},
	{
		feature: "offline_lookup",
		read: (c, f) =>
			lookupOfflineOperation(c, f, {
				placementId: "p1",
				scope,
				queuedOperationId: queued,
			}),
	},
	{ feature: "artifact_capacity", read: (c, f) => readArtifactUsage(c, f) },
	{
		feature: "artifact_capacity",
		read: (c, f) =>
			pruneArtifactRevisions(c, f, { projectId: "app", revisions: [revision] }),
	},
];

describe("older agents", () => {
	for (const [index, { feature, read }] of READS.entries())
		test(`read ${index} (${feature}) is not sent without its feature flag`, async () => {
			const { call, sent } = recorder(() => completed({}));
			expect(await read(call, {})).toEqual({ kind: "unsupported", feature });
			expect(await read(call, undefined)).toEqual({
				kind: "unsupported",
				feature,
			});
			expect(sent).toHaveLength(0);
		});

	for (const [index, { feature, read }] of READS.entries())
		test(`read ${index} (${feature}) maps a rejected · unsupported reply to unsupported`, async () => {
			const { call, sent } = recorder(() => rejected("unsupported"));
			expect(await read(call, { [feature]: 1 })).toEqual({
				kind: "unsupported",
				feature,
			});
			expect(sent).toHaveLength(1);
		});
});

test("other refusals throw a coded live error", async () => {
	const { call } = recorder(() =>
		rejected("unauthorized", "Owner access is required."),
	);
	const error = await readOperations(call, { operations: 1 }).catch((e) => e);
	expect(error).toBeInstanceOf(LiveCallError);
	expect(error.code).toBe("rejected_unauthorized");
	expect(error.message).toBe("Owner access is required.");
});

test("an invalid result names the read and the field", async () => {
	const { call } = recorder(() => completed({ operation: { kind: "reboot" } }));
	await expect(readHostOperation(call, { host_operation: 1 })).rejects.toThrow(
		"invalid current device operation (operation.operation_id",
	);
});

test("host operation reads the current operation or none", async () => {
	const operation: HostOperationView = {
		operation_id: "op-1",
		kind: "reboot",
		state: "draining",
		created_at: 10,
		issued_by: "owner",
	};
	const { call, sent } = recorder(() => completed({ operation }));
	expect(await readHostOperation(call, { host_operation: 1 })).toEqual({
		kind: "ok",
		data: operation,
	});
	expect(sent).toEqual([{ type: "host_operation" }]);
	const none = recorder(() => completed({ operation: null }));
	expect(await readHostOperation(none.call, { host_operation: 1 })).toEqual({
		kind: "ok",
		data: null,
	});
});

test("rollout history pages one placement and refuses rows of another", async () => {
	const row: DeploymentRolloutStatus = {
		rollout_id: "r2",
		placement_id: "p1",
		project_id: "app",
		state: "healthy",
		stable_since: 5,
	};
	const { call, sent } = recorder(() =>
		completed({ placement_id: "p1", rollouts: [row], next: "r2" }),
	);
	const page = await readRolloutHistory(
		call,
		{ rollout_history: 1 },
		{ placementId: "p1", before: "r3", limit: 1 },
	);
	expect(page).toEqual({
		kind: "ok",
		data: { placement_id: "p1", rollouts: [row], next: "r2" },
	});
	expect(sent).toEqual([
		{ type: "rollout_history", placement_id: "p1", before: "r3", limit: 1 },
	]);
	const other = recorder(() =>
		completed({
			placement_id: "p1",
			rollouts: [{ ...row, placement_id: "p2" }],
			next: null,
		}),
	);
	await expect(
		readRolloutHistory(
			other.call,
			{ rollout_history: 1 },
			{ placementId: "p1" },
		),
	).rejects.toThrow("another placement");
});

test("limits are checked before anything is sent", async () => {
	const { call, sent } = recorder(() => completed({}));
	expect(() =>
		readRolloutHistory(
			call,
			{ rollout_history: 1 },
			{
				placementId: "p1",
				limit: 17,
			},
		),
	).toThrow(RangeError);
	expect(() => readOperations(call, { operations: 1 }, { limit: 0 })).toThrow(
		"from 1 to 50",
	);
	expect(() =>
		readMetricsHistory(
			call,
			{ metrics_history: 1 },
			{
				placementId: "p1",
				fields: ["memory_used_bytes" as "memory_bytes"],
			},
		),
	).toThrow("memory_used_bytes");
	expect(sent).toHaveLength(0);
});

test("operations parse actors and keep kind null until the agent records it", async () => {
	const operation: AgentOperation = {
		operation_id: "op-1",
		kind: null,
		actor: { role: "grant", user_id: "usr_jonas", grant_id: "g1" },
		project_id: "app",
		placement_id: "p1",
		accepted_at: 1727760000,
		state: "accepted",
	};
	const { call, sent } = recorder(() =>
		completed({ operations: [operation], next: null }),
	);
	expect(
		await readOperations(call, { operations: 1 }, { after: "op-0" }),
	).toEqual({ kind: "ok", data: { operations: [operation], next: null } });
	expect(sent).toEqual([{ type: "operations", after: "op-0", limit: 20 }]);
});

const metrics: MetricsHistoryPage = {
	fields: ["cpu_percent", "memory_bytes"],
	points: [
		[100, 12.5, 1024],
		[160, null, 2048],
	],
	next: 9,
	evicted_through: 2,
};
const placementMetrics = {
	placementId: "p1",
	fields: ["cpu_percent", "memory_bytes"],
} as const;

test("metric history projects the requested fields", async () => {
	const { call, sent } = recorder(() => completed({ ...metrics }));
	expect(
		await readMetricsHistory(
			call,
			{ metrics_history: 1 },
			{ ...placementMetrics, after: 4, limit: 2 },
		),
	).toEqual({ kind: "ok", data: metrics });
	expect(sent).toEqual([
		{
			type: "metrics_history",
			placement_id: "p1",
			after: 4,
			limit: 2,
			fields: ["cpu_percent", "memory_bytes"],
		},
	]);
});

test("metric history refuses points and columns that do not match the request", async () => {
	for (const [result, message] of [
		[{ ...metrics, points: [[100, 12.5]] }, "point width"],
		[{ ...metrics, fields: ["memory_bytes", "cpu_percent"] }, "fields differ"],
	] as const) {
		const { call } = recorder(() => completed({ ...result }));
		await expect(
			readMetricsHistory(call, { metrics_history: 1 }, placementMetrics),
		).rejects.toThrow(message);
	}
});

const tombstone = {
	state: "superseded",
	superseded_by: queued,
	error: null,
	error_code: null,
};

test("queued write pages never carry payloads", async () => {
	const summary = {
		sequence: 3,
		operation_id: queued,
		resource: '{"table":"orders"}',
		mutation_kind: "table_upsert",
		state: "superseded",
		attempts: 1,
		created_at: 50,
		bytes: 120,
		error: null,
		error_code: null,
	};
	const { call, sent } = recorder(() =>
		completed({
			operations: [{ ...summary, payload: { secret: "row" } }],
			next: 4,
		}),
	);
	const page = await readOfflineOperations(
		call,
		{ offline_lookup: 1 },
		{ placementId: "p1", scope, afterSequence: 2, terminal: true },
	);
	expect(page).toEqual({
		kind: "ok",
		data: { operations: [summary], next: 4 },
	});
	expect(JSON.stringify(page)).not.toContain("secret");
	expect(sent).toEqual([
		{
			type: "offline_queue_operations",
			placement_id: "p1",
			scope,
			after_sequence: 2,
			limit: 20,
			terminal: true,
		},
	]);
});

test("a queued write lookup reports its tombstone", async () => {
	const { call, sent } = recorder(() => completed({ ...tombstone }));
	expect(
		await lookupOfflineOperation(
			call,
			{ offline_lookup: 1 },
			{ placementId: "p1", scope, queuedOperationId: queued },
		),
	).toEqual({ kind: "ok", data: tombstone });
	expect(sent).toEqual([
		{
			type: "offline_queue_lookup",
			placement_id: "p1",
			scope,
			queued_operation_id: queued,
		},
	]);
});

test("artifact usage asks for one app or the device only", async () => {
	const usage = {
		device: {
			bytes: { used: 10, max: 100 },
			entries: { used: 2, max: null },
			revisions: { used: 1, max: 8 },
		},
		project: null,
		revisions: [{ revision, bytes: 10, referenced_by: ["p1"], rollout: false }],
	};
	const { call, sent } = recorder(() => completed(usage));
	expect(
		await readArtifactUsage(
			call,
			{ artifact_capacity: 1 },
			{ projectId: "app" },
		),
	).toEqual({ kind: "ok", data: usage });
	expect(await readArtifactUsage(call, { artifact_capacity: 1 })).toEqual({
		kind: "ok",
		data: usage,
	});
	expect(sent).toEqual([
		{ type: "artifact", request: { kind: "usage", project_id: "app" } },
		{ type: "artifact", request: { kind: "usage", project_id: null } },
	]);
});

const pruneInput = { projectId: "app", revisions: [revision] };
const pruned =
	(result: Record<string, unknown>): ManagementCall =>
	async (_command, operationId) =>
		completed(result, operationId);

test("pruning names exact revisions and is journaled", async () => {
	const ids: (string | undefined)[] = [];
	const call: ManagementCall = async (command, operationId) => {
		ids.push(operationId);
		return pruned({ project_id: "app", pruned: [revision], freed_bytes: 64 })(
			command,
			operationId,
		);
	};
	expect(
		await pruneArtifactRevisions(
			call,
			{ artifact_capacity: 1 },
			{ projectId: "app", revisions: [revision, revision] },
		),
	).toEqual({
		kind: "ok",
		data: { project_id: "app", pruned: [revision], freed_bytes: 64 },
	});
	expect(ids[0]).toMatch(/^[0-9a-f-]{36}$/u);
});

test("pruning refuses an empty list, an unrequested removal and reports busy uploads", async () => {
	const greedy = pruned({
		project_id: "app",
		pruned: [revision, "c".repeat(64)],
		freed_bytes: 1,
	});
	expect(() =>
		pruneArtifactRevisions(
			greedy,
			{ artifact_capacity: 1 },
			{ ...pruneInput, revisions: [] },
		),
	).toThrow("Choose 1 to 64");
	await expect(
		pruneArtifactRevisions(greedy, { artifact_capacity: 1 }, pruneInput),
	).rejects.toThrow("not requested");
	const busy = recorder(() => rejected("busy", "Uploads are in progress."));
	const error = await pruneArtifactRevisions(
		busy.call,
		{ artifact_capacity: 1 },
		pruneInput,
	).catch((e) => e);
	expect(error.code).toBe("rejected_busy");
});
