import { describe, expect, test } from "bun:test";
import {
	type AgentOperation,
	type AgentRead,
	type MetricsHistoryPage,
	lookupOfflineOperation,
	pruneArtifactRevisions,
	readArtifactUsage,
	readEventForm,
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
	{
		feature: "on_demand_events",
		read: (c, f) =>
			readEventForm(c, f, { placementId: "notes", eventId: "evt_notes_form" }),
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

/* run-more-2-design §1.8: the `event_form` literal; the agent's test carries the same. */
const EVENT_FORM = JSON.parse(
	`{"placement_id":"notes","config_revision":7,"event_id":"evt_notes_form","event_version":[1,0,0],"board_version":[3,0,1],
 "kind":"form","name":"New note","description":"",
 "fields":[{"name":"title","label":"Title","description":"","data_type":"String","value_type":"Normal",
            "optional":false,"sensitive":false,"default":null,"options":null}],
 "fields_truncated":false,"file_fields":0,"navigate_to_routes":[]}`,
);
const FORM_ON = { on_demand_events: 1 } as const;
const readForm = (reply: () => ManagementResponse) => {
	const { call, sent } = recorder(reply);
	return {
		sent,
		read: readEventForm(call, FORM_ON, {
			placementId: "notes",
			eventId: "evt_notes_form",
		}),
	};
};

test("the form of a run comes from the flow version the service runs", async () => {
	const { read, sent } = readForm(() => completed(EVENT_FORM));
	expect(await read).toEqual({ kind: "ok", data: EVENT_FORM });
	expect(sent).toEqual([
		{ type: "event_form", placement_id: "notes", event_id: "evt_notes_form" },
	]);
	const action = {
		...EVENT_FORM,
		kind: "action",
		fields: [],
		navigate_to_routes: ["/notes"],
	};
	expect(await readForm(() => completed(action)).read).toEqual({
		kind: "ok",
		data: action,
	});
});

test("a field keeps words this client does not know; a sensitive default is never shown", async () => {
	const [title] = EVENT_FORM.fields;
	const fields = [
		{ ...title, name: "where", data_type: "Geometry", value_type: "HashMap" },
		{ ...title, name: "pin", sensitive: true, default: "1234" },
		{ ...title, name: "size", options: ["S", "M"], default: "M" },
		{ ...title, name: "big", default: null, default_omitted: true },
	];
	const { read } = readForm(() => completed({ ...EVENT_FORM, fields }));
	const result = await read;
	if (result.kind !== "ok") throw new Error("expected a form");
	expect(result.data.fields.map((field) => field.default)).toEqual([
		null,
		null,
		"M",
		null,
	]);
	expect(result.data.fields[0]).toMatchObject({
		data_type: "Geometry",
		value_type: "HashMap",
	});
	expect(result.data.fields[3].default_omitted).toBe(true);
});

test("texts are bounded in characters as a device cuts them, so an emoji counts once", async () => {
	const [title] = EVENT_FORM.fields;
	const emoji = (count: number) => "😀".repeat(count);
	const form = {
		...EVENT_FORM,
		description: emoji(480),
		fields: [
			{
				...title,
				name: emoji(120),
				label: emoji(120),
				description: emoji(480),
				options: [emoji(64)],
			},
		],
	};
	expect(await readForm(() => completed(form)).read).toEqual({
		kind: "ok",
		data: form,
	});
	const over = { ...form, fields: [{ ...form.fields[0], label: emoji(121) }] };
	await expect(readForm(() => completed(over)).read).rejects.toThrow(
		"invalid form of event evt_notes_form",
	);
});

test("a form over its bounds or of another event is an invalid answer; a changed service is a coded refusal", async () => {
	const [title] = EVENT_FORM.fields;
	for (const form of [
		{ ...EVENT_FORM, event_id: "evt_other" },
		{ ...EVENT_FORM, kind: "wizard" },
		{ ...EVENT_FORM, fields: [{ ...title, data_type: "Str ing" }] },
		{ ...EVENT_FORM, fields: [{ ...title, label: "x".repeat(121) }] },
		{ ...EVENT_FORM, fields: [{ ...title, default: "x".repeat(1025) }] },
		{
			...EVENT_FORM,
			fields: [{ ...title, options: Array.from({ length: 33 }, () => "a") }],
		},
		{ ...EVENT_FORM, fields: Array.from({ length: 65 }, () => title) },
	])
		await expect(readForm(() => completed(form)).read).rejects.toThrow(
			"invalid form of event evt_notes_form",
		);
	const error = await readForm(() =>
		rejected("revision_conflict", "The service changed."),
	).read.catch((e) => e);
	expect(error).toBeInstanceOf(LiveCallError);
	expect(error.code).toBe("rejected_revision_conflict");
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

describe("artifact usage follows the device's cursor", () => {
	const digest = (index: number) => index.toString(16).padStart(64, "0");
	const budgets = {
		device: null,
		project: {
			bytes: { used: 30, max: 100 },
			entries: { used: 3, max: null },
			revisions: { used: 3, max: 128 },
		},
	};
	const row = (index: number) => ({
		revision: digest(index),
		bytes: 10,
		referenced_by: [],
		rollout: false,
	});
	const usageRequest = (after?: string) => ({
		type: "artifact",
		request: { kind: "usage", project_id: "app", ...(after ? { after } : {}) },
	});
	const read = (call: ManagementCall) =>
		readArtifactUsage(call, { artifact_capacity: 1 }, { projectId: "app" });

	test("pages are asked for with `after` and their revisions concatenated", async () => {
		const pages: Record<string, unknown>[] = [
			{ ...budgets, revisions: [row(1), row(2)], next: digest(2) },
			{ ...budgets, project: null, revisions: [row(3)], next: null },
		];
		const { call, sent } = recorder(() => completed(pages[sent.length - 1]));
		expect(await read(call)).toEqual({
			kind: "ok",
			data: { ...budgets, revisions: [row(1), row(2), row(3)] },
		});
		expect(sent).toEqual([usageRequest(), usageRequest(digest(2))]);
	});

	test("a cursor that does not advance stops the read", async () => {
		const { call, sent } = recorder(() =>
			completed({ ...budgets, revisions: [row(1)], next: digest(1) }),
		);
		await expect(read(call)).rejects.toThrow("repeated a page");
		expect(sent).toHaveLength(2);
	});

	test("the read gives up after 180 pages", async () => {
		const { call, sent } = recorder(() =>
			completed({
				...budgets,
				revisions: [row(sent.length)],
				next: digest(sent.length),
			}),
		);
		await expect(read(call)).rejects.toThrow("more than 180 pages");
		expect(sent).toHaveLength(180);
	});

	test("an invalid cursor is an invalid answer", async () => {
		const { call } = recorder(() =>
			completed({ ...budgets, revisions: [], next: "abc" }),
		);
		await expect(read(call)).rejects.toThrow("invalid app storage usage");
	});
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
