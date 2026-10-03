import { expect, test } from "bun:test";
import {
	hostOperationSchema,
	readDeviceInspection,
	snapshotDeviceFacts,
	snapshotPlacement,
} from "./inspection";
import type { ManagementCall } from "./telemetry";
function row(id: string) {
	return {
		id,
		project_id: "project",
		deployment_id: "deploy",
		revision: "v1",
		desired_state: "running",
		observed_state: "running",
		config_revision: 1,
		intent_revision: 1,
		applied_revision: 1,
		desired_replicas: 1,
		running_replicas: 1,
		ready_replicas: 1,
		max_replicas: 32,
		replicas: [{ slot: 0, observed_state: "running", applied_revision: 1 }],
	};
}
test("paged inspection preserves exact order and pins one boot/device", async () => {
	const requests: unknown[] = [];
	const call: ManagementCall = async (command) => {
		requests.push(command);
		return {
			operation_id: "op",
			state: "completed",
			result: {
				device_id: "device",
				boot_id: "boot",
				certificate_management: 1,
				can_manage_certificates: true,
				certificate_issuance: 1,
				certificate_acme: 1,
				can_delegate_certificate_renewal: true,
				placements: command.after ? [row("c")] : [row("a"), row("b")],
				next: command.after ? null : "b",
			},
		};
	};
	const result = await readDeviceInspection(call, "device");
	expect(result.placements.map((row) => row.id)).toEqual(["a", "b", "c"]);
	expect(result.boot_id).toBe("boot");
	expect(result.certificate_management).toBe(1);
	expect(result.can_manage_certificates).toBe(true);
	expect(result.certificate_issuance).toBe(1);
	expect(result.certificate_acme).toBe(1);
	expect(result.can_delegate_certificate_renewal).toBe(true);
	expect(requests).toEqual([
		{ type: "inspect_page", after: null, limit: 2 },
		{ type: "inspect_page", after: "b", limit: 2 },
	]);
});
test("fresh placements and starting replica slots preserve their unapplied revision", async () => {
	const fresh = {
		...row("fresh"),
		desired_state: "stopped",
		observed_state: "unknown",
		applied_revision: null,
		running_replicas: 0,
		ready_replicas: 0,
		replicas: [],
	};
	const starting = {
		...row("starting"),
		observed_state: "starting",
		applied_revision: null,
		ready_replicas: 0,
		replicas: [{ slot: 0, observed_state: "starting", applied_revision: null }],
	};
	const call: ManagementCall = async () => ({
		operation_id: "op",
		state: "completed",
		result: {
			device_id: "device",
			boot_id: null,
			placements: [fresh, starting],
			next: null,
		},
	});
	const result = await readDeviceInspection(call, "device");
	expect(result.placements).toEqual([fresh, starting]);
	expect(result.boot_id).toBeNull();
	expect(result.certificate_management).toBeUndefined();
	expect(result.certificate_issuance).toBeUndefined();
	expect(result.certificate_acme).toBeUndefined();
});

test("certificate mutation authority is absent unless this live session explicitly grants it", async () => {
	for (const authority of [undefined, false, "true", 1]) {
		const result = await readDeviceInspection(
			async () => ({
				operation_id: "op",
				state: "completed",
				result: {
					device_id: "device",
					boot_id: "boot",
					placements: [],
					next: null,
					certificate_management: 1,
					can_manage_certificates: authority,
					certificate_issuance: 1,
					certificate_acme: 1,
					can_delegate_certificate_renewal: authority,
				},
			}),
			"device",
		);
		expect(result.certificate_management).toBe(1);
		expect(result.can_manage_certificates).toBe(false);
		expect(result.can_delegate_certificate_renewal).toBe(false);
	}
});
test("inspection rejects reboot, duplicate cursors, wrong audiences and oversized pages", async () => {
	for (const changed of [
		{ boot_id: "other" },
		{ device_id: "other" },
		{ placements: [row("a")] },
		{ next: "bad" },
		{ placements: [], next: "a" },
		{ placements: [row("b"), row("c"), row("d")] },
	]) {
		let page = 0;
		const call: ManagementCall = async () => ({
			operation_id: "op",
			state: "completed",
			result: {
				device_id: "device",
				boot_id: "boot",
				placements: [row(page === 0 ? "a" : "b")],
				next: page++ === 0 ? "a" : null,
				...(page > 1 ? changed : {}),
			},
		});
		await expect(readDeviceInspection(call, "device")).rejects.toThrow();
	}
	const malformed: ManagementCall = async () => ({
		operation_id: "op",
		state: "completed",
		result: {
			device_id: "device",
			boot_id: "boot",
			placements: [
				{
					...row("a"),
					replicas: [
						{ slot: 0, observed_state: "running", applied_revision: 1 },
						{ slot: 0, observed_state: "running", applied_revision: 1 },
					],
				},
			],
			next: null,
		},
	});
	await expect(readDeviceInspection(malformed, "device")).rejects.toThrow();
});

function page(result: Record<string, unknown>): ManagementCall {
	return async () => ({
		operation_id: "op",
		state: "completed",
		result: {
			device_id: "device",
			boot_id: "boot",
			next: null,
			...result,
		},
	});
}

const diagnosed = {
	...row("diag"),
	process_id: 4211,
	last_error: "listen failed in <state>",
	has_error: true,
	restarts: {
		failures: 3,
		max_restarts: 5,
		crash_looping: true,
		retry_in_seconds: 40,
		last_started_at: 1727770000,
	},
	offline_writes: {
		scopes: 1,
		pending_count: 13,
		pending_bytes: 2048,
		oldest_at: 1727760000,
		quarantined_scopes: 0,
		needs_attention: 1,
		mirror_error: false,
	},
	source: "online",
	events: [
		{ event_id: "event-1", event_version: [1, 2, 0], board_version: [3, 0, 1] },
	],
	events_truncated: false,
	online_metadata_sha256: "a".repeat(64),
	replicas: [
		{
			slot: 0,
			observed_state: "backoff",
			applied_revision: 1,
			process_id: null,
			last_error: "exit 1",
			restarts: {
				failures: 3,
				max_restarts: 5,
				crash_looping: true,
				retry_in_seconds: 40,
				last_started_at: 1727770000,
			},
		},
	],
};

const deviceScope = {
	agent_version: "0.9.3",
	host_operations: { reboot: true, update_agent: false },
	host_isolation: "optional",
	isolation: {
		platform: "linux",
		sandbox_available: true,
		require_isolation: false,
		placement_preflight_required: true,
		network_boundary: "loopback",
		disk_requirement: "quota",
		landlock_abi: 4,
		reason: null,
	},
	features: { placement_diagnostics: 1, task_health: 1, placement_events: 1 },
	agent: { version: "0.9.3", release_version: "1.4.0", release_sequence: 44 },
	host: { booted_at: 1727700000, agent_started_at: 1727700100 },
	tasks: [
		{
			name: "fleet_publisher",
			state: "failing",
			since: 1727760000,
			consecutive_failures: 4,
			category: "hub_unreachable",
		},
	],
	host_operation: {
		operation_id: "op-1",
		kind: "reboot",
		state: "requested",
		created_at: 1727760000,
		issued_by: "you",
	},
	network: {
		interfaces: [
			{ name: "eth0", addresses: ["192.168.1.20"], loopback: false },
		],
	},
};

test("inspection keeps every agent fact a newer agent sends", async () => {
	const result = await readDeviceInspection(
		page({ ...deviceScope, placements: [diagnosed] }),
		"device",
		{ now: () => 1234 },
	);
	expect<unknown>(result.placements).toEqual([diagnosed]);
	expect(result.observed_at).toBe(1234);
	expect<unknown>(result.features).toEqual(deviceScope.features);
	expect(result.agentVersion).toBe("0.9.3");
	expect(result.hostOperations).toEqual(deviceScope.host_operations);
	expect(result.hostIsolation).toBe("optional");
	expect(result.isolation).toEqual(deviceScope.isolation);
	expect(result.agent).toEqual(deviceScope.agent);
	expect(result.host).toEqual(deviceScope.host);
	expect<unknown>(result.tasks).toEqual(deviceScope.tasks);
	expect<unknown>(result.hostOperation).toEqual(deviceScope.host_operation);
	expect(result.network).toEqual(deviceScope.network);
});

test("an older agent's page passes unchanged with no feature flags", async () => {
	const result = await readDeviceInspection(
		page({ placements: [row("old")] }),
		"device",
	);
	expect(result.placements).toEqual([row("old")]);
	expect(result.features).toEqual({});
	for (const key of [
		"agentVersion",
		"hostOperations",
		"hostIsolation",
		"isolation",
		"agent",
		"host",
		"tasks",
		"hostOperation",
		"network",
	])
		expect(key in result).toBe(false);
});

test("malformed agent facts are dropped without failing the inspection", async () => {
	const result = await readDeviceInspection(
		page({
			agent_version: 7,
			host_isolation: "maybe",
			isolation: null,
			host_operation: null,
			tasks: [{ name: "Bad Name", state: "ok" }],
			placements: [
				{
					...row("odd"),
					process_id: -1,
					last_error: "x".repeat(2000),
					source: "cloud",
					events: [{ event_id: "e", event_version: [1], board_version: [] }],
					online_metadata_sha256: "not-hex",
					has_error: false,
					restarts: { failures: 1, max_restarts: 5, crash_looping: false },
				},
			],
		}),
		"device",
	);
	const [odd] = result.placements;
	expect(odd).toEqual({
		...row("odd"),
		has_error: false,
		restarts: {
			failures: 1,
			max_restarts: 5,
			crash_looping: false,
			retry_in_seconds: null,
			last_started_at: null,
		},
	});
	expect(result.agentVersion).toBeUndefined();
	expect(result.hostIsolation).toBeUndefined();
	expect(result.isolation).toBeNull();
	expect(result.hostOperation).toBeNull();
	expect(result.tasks).toBeUndefined();
});

test("replica error flags are validated and a task without its failure counter keeps the task list", async () => {
	const replica = (slot: number, has_error: unknown) => ({
		slot,
		observed_state: "backoff",
		applied_revision: 1,
		has_error,
	});
	const tasks = [
		{ name: "fleet_publisher", state: "stopped", since: 1727760000 },
		{
			name: "archive_publisher",
			state: "failing",
			since: 1727760000,
			consecutive_failures: 2,
			category: "hub_refused",
		},
	];
	const result = await readDeviceInspection(
		page({
			tasks,
			placements: [
				{
					...row("status-only"),
					desired_replicas: 2,
					max_replicas: 2,
					has_error: true,
					replicas: [replica(0, true), replica(1, "yes")],
				},
			],
		}),
		"device",
	);
	expect<unknown>(result.placements[0].replicas).toEqual([
		replica(0, true),
		{ slot: 1, observed_state: "backoff", applied_revision: 1 },
	]);
	expect<unknown>(result.tasks).toEqual(tasks);
});

test("page progress reports reads and an estimate from the previous count", async () => {
	const progress: [number, number | null][] = [];
	await readDeviceInspection(
		async (command) => ({
			operation_id: "op",
			state: "completed",
			result: {
				device_id: "device",
				boot_id: "boot",
				placements: command.after ? [row("c")] : [row("a"), row("b")],
				next: command.after ? null : "b",
			},
		}),
		"device",
		{
			expectedPlacements: 6,
			onPage: (done, total) => progress.push([done, total]),
		},
	);
	expect(progress).toEqual([
		[1, 3],
		[2, 2],
	]);
});

test("the host operation schema is shared with the command reader", () => {
	expect(
		hostOperationSchema.safeParse(deviceScope.host_operation).success,
	).toBe(true);
	expect(
		hostOperationSchema.safeParse({
			...deviceScope.host_operation,
			kind: "shutdown",
		}).success,
	).toBe(false);
});

/* run-more-design §1.5: the literals an agent sends for the schedules of a running service. */
const LIVE_SCHEDULES = JSON.parse(
	`{"schedules":[{"event_id":"evt_report","expression":"0 0 2 * * *","timezone":"Europe/Berlin",
  "hold":null,"next_at":1790121600,"running":false,"last_at":1790035200,"last_outcome":"succeeded",
  "runs":12,"failed":1,"skipped":2,"last_skip":{"at":1789948800,"reason":"missed"},"clock_behind":false}],
"schedules_truncated":false}`,
);
const SNAPSHOT_SCHEDULES = JSON.parse(
	`{"schedules":[{"event_id":"evt_report","expression":"0 0 2 * * *","timezone":"Europe/Berlin","hold":null,"last_outcome":"succeeded"}]}`,
);

test("a live row carries the schedules its process reported, with times and counters", async () => {
	const result = await readDeviceInspection(
		page({
			features: { scheduled_events: 1 },
			placements: [{ ...row("reports"), ...LIVE_SCHEDULES }],
		}),
		"device",
	);
	expect<unknown>(result.features).toEqual({ scheduled_events: 1 });
	const [placement] = result.placements;
	expect(placement.schedules).toEqual(LIVE_SCHEDULES.schedules);
	expect(placement.schedules_truncated).toBe(false);
});

test("a row without schedules has no schedule fact, never an empty list", async () => {
	const result = await readDeviceInspection(
		page({
			features: { scheduled_events: 1 },
			placements: [row("stopped")],
		}),
		"device",
	);
	expect("schedules" in result.placements[0]).toBe(false);
	expect("schedules_truncated" in result.placements[0]).toBe(false);
});

test("a schedule entry with a hold or a result this client does not know is dropped, not the row", async () => {
	const [entry] = LIVE_SCHEDULES.schedules;
	const held = { ...entry, event_id: "evt_held", hold: "not_released" };
	const result = await readDeviceInspection(
		page({
			placements: [
				{
					...row("reports"),
					schedules: [
						{ ...entry, event_id: "evt_later_hold", hold: "paused_by_owner" },
						{ ...entry, event_id: "evt_later_result", last_outcome: "skipped" },
						{ ...entry, event_id: "evt_bad_text", expression: "0 0 2 * * ?" },
						{ ...entry, event_id: "evt_no_hold", hold: undefined },
						held,
						{
							...entry,
							event_id: "evt_later_skip",
							last_skip: { at: 1, reason: "throttled" },
						},
						"not an entry",
					],
					schedules_truncated: "yes",
				},
			],
		}),
		"device",
	);
	const [placement] = result.placements;
	expect(placement.id).toBe("reports");
	expect(placement.schedules?.map((value) => value.event_id)).toEqual([
		"evt_held",
		"evt_later_skip",
	]);
	expect(placement.schedules?.[0].hold).toBe("not_released");
	// An unknown skip reason loses only that fact.
	expect(placement.schedules?.[1].last_skip).toBeUndefined();
	expect("schedules_truncated" in placement).toBe(false);
	const malformed = await readDeviceInspection(
		page({ placements: [{ ...row("reports"), schedules: "none" }] }),
		"device",
	);
	expect("schedules" in malformed.placements[0]).toBe(false);
});

test("a snapshot row keeps only the schedule facts that do not change by themselves", () => {
	const base = row("reports");
	expect(
		snapshotPlacement(base, { ...base, ...SNAPSHOT_SCHEDULES }).schedules,
	).toEqual(SNAPSHOT_SCHEDULES.schedules);
	// A snapshot that carried live values would show a countdown nobody refreshes.
	expect(
		snapshotPlacement(base, { ...base, ...LIVE_SCHEDULES }).schedules,
	).toEqual(SNAPSHOT_SCHEDULES.schedules);
	expect("schedules" in snapshotPlacement(base, base)).toBe(false);
	expect(
		snapshotPlacement(base, {
			...base,
			schedules: [{ ...SNAPSHOT_SCHEDULES.schedules[0], hold: "later" }],
		}).schedules,
	).toEqual([]);
});

/* run-more-2-design §1.7: the literals an agent sends for one-time schedules, bots and actions. */
const ONCE_LIVE = JSON.parse(
	`{"event_id":"evt_once","once_at":1790233200,"timezone":"Europe/Berlin","once_state":"pending",
 "hold":null,"next_at":1790233200,"running":false,"last_at":null,"last_outcome":null}`,
);
const ONCE_SNAPSHOT = JSON.parse(
	`{"event_id":"evt_once","once_at":1790233200,"timezone":"Europe/Berlin","once_state":"pending","hold":null,"last_outcome":null}`,
);
const BOTS_LIVE = JSON.parse(
	`{"bots":[{"event_id":"evt_helper","provider":"telegram","state":"connected","hold":null,"bot_name":"helper_bot",
  "connected_at":1790000003,"last_message_at":1790003600,"last_outcome":"succeeded",
  "running":1,"runs":57,"runs_today":12,"failed":2,"dropped":3}],"bots_truncated":false}`,
);
const BOTS_SNAPSHOT = JSON.parse(
	`{"bots":[{"event_id":"evt_helper","provider":"telegram","hold":null,"state":"ok"}]}`,
);
const ACTIONS_LIVE = JSON.parse(
	`{"actions":[{"event_id":"evt_notes_form","kind":"form","fields":1,"file_fields":0,"running":0,
  "last_at":1790035200,"last_outcome":"succeeded","runs":12,"failed":1}],"actions_truncated":false}`,
);
const ACTIONS_SNAPSHOT = JSON.parse(
	`{"actions":[{"event_id":"evt_notes_form","kind":"form","fields":1,"file_fields":0}]}`,
);

test("a live row carries one-time schedules, bots and actions as their processes reported them", async () => {
	const result = await readDeviceInspection(
		page({
			placements: [
				{
					...row("notes"),
					schedules: [LIVE_SCHEDULES.schedules[0], ONCE_LIVE],
					schedules_truncated: false,
					...BOTS_LIVE,
					...ACTIONS_LIVE,
				},
			],
		}),
		"device",
	);
	const [placement] = result.placements;
	expect(placement.schedules).toEqual([LIVE_SCHEDULES.schedules[0], ONCE_LIVE]);
	expect(placement.bots).toEqual(BOTS_LIVE.bots);
	expect(placement.bots_truncated).toBe(false);
	expect(placement.actions).toEqual(ACTIONS_LIVE.actions);
	expect(placement.actions_truncated).toBe(false);
	const empty = await readDeviceInspection(
		page({ placements: [row("stopped")] }),
		"device",
	);
	for (const fact of ["bots", "bots_truncated", "actions", "actions_truncated"])
		expect(fact in empty.placements[0]).toBe(false);
});

test("an entry with a state, kind or outcome this client does not know is dropped, not the row", async () => {
	const [bot] = BOTS_LIVE.bots;
	const [action] = ACTIONS_LIVE.actions;
	const result = await readDeviceInspection(
		page({
			placements: [
				{
					...row("notes"),
					schedules: [
						ONCE_LIVE,
						{ ...ONCE_LIVE, event_id: "evt_later", once_state: "postponed" },
						{ ...ONCE_LIVE, event_id: "evt_both", expression: "0 0 2 * * *" },
						{ ...ONCE_LIVE, event_id: "evt_no_state", once_state: undefined },
						{ ...ONCE_LIVE, event_id: "evt_1999", once_at: 915148800 },
						{ ...ONCE_LIVE, event_id: "evt_ran", once_state: "ran" },
					],
					bots: [
						bot,
						{ ...bot, event_id: "evt_slack", provider: "slack" },
						{ ...bot, event_id: "evt_later", state: "sleeping" },
						{ ...bot, event_id: "evt_later_hold", hold: "paused_by_owner" },
						{ ...bot, event_id: "evt_odd_name", bot_name: "Bot ✨ <b>" },
						{ ...bot, event_id: "evt_no_name", bot_name: null },
					],
					actions: [
						action,
						{ ...action, event_id: "evt_wizard", kind: "wizard" },
						{ ...action, event_id: "evt_later", last_outcome: "skipped" },
						{ ...action, event_id: "evt_action", kind: "action", fields: 0 },
					],
					bots_truncated: "no",
				},
			],
		}),
		"device",
	);
	const [placement] = result.placements;
	expect(placement.schedules?.map((entry) => entry.event_id)).toEqual([
		"evt_once",
		"evt_ran",
	]);
	expect(placement.bots?.map((entry) => entry.event_id)).toEqual([
		"evt_helper",
		"evt_odd_name",
		"evt_no_name",
	]);
	// A name a device may not show reads as no name; the bot keeps its status.
	expect(placement.bots?.[1].bot_name).toBeNull();
	expect(placement.bots?.[2].bot_name).toBeNull();
	expect(placement.actions?.map((entry) => entry.event_id)).toEqual([
		"evt_notes_form",
		"evt_action",
	]);
	expect("bots_truncated" in placement).toBe(false);
});

test("a snapshot row keeps the one-time state, the bot's settled state and the action counts only", () => {
	const base = row("notes");
	const snapshot = snapshotPlacement(base, {
		...base,
		schedules: [ONCE_LIVE],
		...BOTS_LIVE,
		...ACTIONS_LIVE,
	});
	expect(snapshot.schedules).toEqual([ONCE_SNAPSHOT]);
	expect(snapshot.bots).toEqual([
		{
			event_id: "evt_helper",
			provider: "telegram",
			state: "connected",
			hold: null,
		},
	]);
	expect(snapshot.actions).toEqual(ACTIONS_SNAPSHOT.actions);
	expect(
		snapshotPlacement(base, {
			...base,
			schedules: [ONCE_SNAPSHOT],
			...BOTS_SNAPSHOT,
			...ACTIONS_SNAPSHOT,
		}),
	).toMatchObject({
		schedules: [ONCE_SNAPSHOT],
		bots: BOTS_SNAPSHOT.bots,
		actions: ACTIONS_SNAPSHOT.actions,
	});
});

test("snapshot device facts carry the feature map only when the device sent one", () => {
	expect(
		snapshotDeviceFacts({
			features: { scheduled_events: 1, a_later_flag: 1, off: 0 },
		}).features,
	).toEqual({ scheduled_events: 1, a_later_flag: 1 } as never);
	expect(snapshotDeviceFacts({ features: {} }).features).toEqual({});
	// No map: an older agent, or a reader below device scope. Unknown, never "too old".
	for (const source of [{}, { features: null }, { features: [] }, null])
		expect("features" in snapshotDeviceFacts(source)).toBe(false);
});
