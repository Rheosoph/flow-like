import { describe, expect, test } from "bun:test";
import type { DeploymentRolloutStatus } from "../deployment";
import {
	SAMPLE_IDS,
	SAMPLE_NOW,
	fleetState,
	observation,
	placement,
	sampleFleet,
	sampleFleetOlderAgent,
	sampleFleetOlderHub,
} from "./__fixtures__/sample-fleet";
import {
	type AttentionInputExt,
	computeAttention,
	createAttentionMemory,
} from "./attention";
import {
	attentionCandidate,
	buildDeviceView,
	buildServiceViews,
	compareVersions,
	currentRollout,
	fleetFacts,
	isLastKnown,
	keysLocked,
	rollupHealth,
	subjectId,
} from "./device-view";
import { classify } from "./freshness";
import type { AttentionItem, PlacementStatusPlus, ServiceView } from "./types";

const ID = SAMPLE_IDS;

function itemsOf(input: AttentionInputExt) {
	return computeAttention(input, createAttentionMemory("test", undefined));
}

function viewOf(input: AttentionInputExt, deviceId: string) {
	const view = buildDeviceView(deviceId, input, itemsOf(input));
	if (!view) throw new Error(`fixture: no view for ${deviceId}`);
	return view;
}

function servicesOf(input: AttentionInputExt, deviceId: string): ServiceView[] {
	const services = buildServiceViews(deviceId, input);
	if (!Array.isArray(services))
		throw new Error(`fixture: services of ${deviceId} not loaded`);
	return services;
}

function service(input: AttentionInputExt, deviceId: string, id: string) {
	const found = servicesOf(input, deviceId).find(
		(entry) => entry.serviceId === id,
	);
	if (!found) throw new Error(`fixture: no service ${id}`);
	return found;
}

/** Edge is live: patch its support-bot row in the live inspection. */
function edgeWith(patch: Partial<PlacementStatusPlus>) {
	const input = sampleFleet();
	const inspection = input.live[ID.edge].inspection;
	if (!inspection) throw new Error("fixture: edge inspection");
	inspection.value.placements[0] = {
		...inspection.value.placements[0],
		...patch,
	};
	return input;
}

function rollout(
	patch: Partial<DeploymentRolloutStatus> &
		Pick<DeploymentRolloutStatus, "rollout_id" | "state">,
): DeploymentRolloutStatus {
	return {
		placement_id: "support-bot",
		project_id: "app_support_portal",
		failure_code: null,
		active_revision: 7,
		base_revision: 6,
		previous_replicas: 2,
		candidate_replicas: 2,
		stabilization_seconds: 10,
		deadline_seconds: 120,
		created_at: SAMPLE_NOW - 3_600,
		updated_at: SAMPLE_NOW - 3_600,
		deadline_at: SAMPLE_NOW - 3_480,
		stable_since: null,
		...patch,
	};
}

function item(
	deviceId: string,
	severity: AttentionItem["severity"],
): AttentionItem {
	return {
		...attentionCandidate({
			key: "offline_since",
			severity,
			subject: { kind: "device", deviceId },
			source: classify("device_row", { now: SAMPLE_NOW, loaded: true }),
		}),
		firstSeenAt: SAMPLE_NOW,
	};
}

describe("buildDeviceView on the golden sample", () => {
	test("health rollup per device (IA §6.5)", () => {
		const input = sampleFleet();
		const health = Object.fromEntries(
			Object.entries(ID).flatMap(([name, id]) => {
				const view = buildDeviceView(id, input, itemsOf(input));
				return view ? [[name, view.health]] : [];
			}),
		);
		expect(health).toEqual({
			edge: "attention",
			warehouse: "critical",
			studio: "attention",
			lab: "unknown",
			oldKiosk: "revoked",
			partner: "revoked",
			cold: "attention",
		});
	});

	test("a device missing from the hub list has no view; its services are not loaded", () => {
		const input = sampleFleet();
		expect(buildDeviceView(ID.miraRender, input, [])).toBeUndefined();
		expect(buildServiceViews(ID.miraRender, input)).toEqual({
			state: "notloaded",
		});
	});

	test("carries row, presence, relationship, keys, live, certificates and resources", () => {
		const input = sampleFleet();
		const edge = viewOf(input, ID.edge);
		expect(edge.row.name).toBe("edge-berlin-01");
		expect(edge.presence.kind).toBe("online");
		expect(edge.relationship).toBe("owner");
		expect(edge.keys.state).toBe("unlocked");
		expect(edge.live.kind).toBe("live");
		expect(edge.certificates?.revision).toBe(17);
		expect(edge.resources?.grants).toHaveLength(1);
		const lab = viewOf(input, ID.lab);
		expect(lab.relationship).toBe("shared");
		expect(lab.keys).toMatchObject({ state: "locked", role: "shared" });
		expect(lab.certificates).toBeUndefined();
		expect(lab.resources).toBeUndefined();
	});

	test("attention holds only the device's own items", () => {
		const input = sampleFleet();
		const items = itemsOf(input);
		const warehouse = buildDeviceView(ID.warehouse, input, items);
		expect(warehouse?.attention.length).toBeGreaterThan(0);
		for (const entry of warehouse?.attention ?? [])
			expect(entry.subject).toMatchObject({ deviceId: ID.warehouse });
	});

	test("agent version: the installed release from a live inspection, else the last live read as a snapshot", () => {
		const input = sampleFleet();
		expect(viewOf(input, ID.edge).agent).toMatchObject({
			version: "0.9.4",
			sequence: 44,
			source: { src: "live", age: "live" },
		});
		expect(viewOf(input, ID.warehouse).agent).toMatchObject({
			version: "0.9.2",
			source: { src: "saved", at: 1_790_676_000 },
		});
		expect(viewOf(input, ID.lab).agent).toBeUndefined();
	});

	test("the agent's constant crate version is never shown as its version", () => {
		const input = sampleFleet();
		const inspection = input.live[ID.edge].inspection;
		if (!inspection) throw new Error("fixture: edge inspection");
		expect(inspection.value.agentVersion).toBe("0.1.0");
		inspection.value.agent = {
			version: "0.1.0",
			release_version: null,
			release_sequence: null,
		};
		expect(viewOf(input, ID.edge).agent).toBeUndefined();
	});
});

describe("services are never [] for 'not loaded' (M-DATA §3.12)", () => {
	test("locked, never reported and revoked give a state object", () => {
		const input = sampleFleet();
		expect(buildServiceViews(ID.lab, input)).toEqual({
			state: "locked",
			reason: { code: "unlock_required" },
		});
		expect(buildServiceViews(ID.cold, input)).toEqual({
			state: "notloaded",
			reason: { code: "never_reported" },
		});
		expect(buildServiceViews(ID.oldKiosk, input)).toEqual({
			state: "notloaded",
		});
	});

	test("no keys on this computer", () => {
		const input = sampleFleet();
		input.keys = input.keys.filter((entry) => entry.deviceId !== ID.studio);
		input.local.vaults = input.local.vaults.filter(
			(entry) => entry.deviceId !== ID.studio,
		);
		delete input.live[ID.studio];
		expect(buildServiceViews(ID.studio, input)).toEqual({
			state: "notloaded",
			reason: { code: "no_keys_here" },
		});
		expect(viewOf(input, ID.studio).keys).toMatchObject({
			state: "none",
			role: "owner",
		});
	});

	test("unlocked but nothing read yet is 'not loaded', not empty", () => {
		const input = sampleFleet();
		delete input.live[ID.studio];
		expect(buildServiceViews(ID.studio, input)).toEqual({
			state: "notloaded",
		});
		expect(viewOf(input, ID.studio).health).not.toBe("healthy");
	});

	test("a snapshot without access, support or with an error keeps that state", () => {
		for (const [option, age] of [
			[{ noAccess: { code: "needs_capability" } }, "noaccess"],
			[{ unsupported: { code: "agent_update_needed" } }, "unsupported"],
			[{ error: { code: "network" } }, "error"],
		] as const) {
			const input = sampleFleet();
			const fleet = input.fleet[ID.warehouse];
			fleet.status = undefined;
			fleet.saved = undefined;
			fleet.freshness.status = classify("fleet_status", {
				now: SAMPLE_NOW,
				loaded: false,
				...option,
			});
			const services = buildServiceViews(ID.warehouse, input);
			expect(services).toMatchObject({ state: age });
			expect(Array.isArray(services)).toBe(false);
		}
	});

	test("a failed snapshot read keeps the last good rows and their items (IA §2.2)", () => {
		const input = sampleFleet();
		const fleet = input.fleet[ID.warehouse];
		const observedAt = fleet.status?.observedAt;
		if (observedAt === undefined) throw new Error("fixture: warehouse status");
		fleet.freshness.status = classify("fleet_status", {
			now: SAMPLE_NOW,
			at: observedAt,
			loaded: true,
			error: { code: "network", retryAt: SAMPLE_NOW + 60 },
		});
		fleet.error = {
			kind: "network",
			message: "Failed to fetch",
			at: SAMPLE_NOW - 5,
			retryAt: SAMPLE_NOW + 60,
		};
		const [scanner] = servicesOf(input, ID.warehouse);
		expect(scanner.conv).toBe("crash_looping");
		expect(scanner.freshness).toMatchObject({
			src: "snap",
			age: "error",
			error: { code: "network" },
			dataFrom: observedAt,
		});
		const crash = itemsOf(input).find(
			(item) => item.key === "service_crash_looping",
		);
		expect(crash).toMatchObject({ severity: "critical", lastKnown: true });
		expect(viewOf(input, ID.warehouse).health).toBe("critical");
	});

	test("an error without earlier rows names its cause", () => {
		const input = sampleFleet();
		const fleet = input.fleet[ID.warehouse];
		fleet.status = undefined;
		fleet.saved = undefined;
		fleet.freshness.status = classify("fleet_status", {
			now: SAMPLE_NOW,
			loaded: false,
			error: { code: "network" },
		});
		expect(buildServiceViews(ID.warehouse, input)).toEqual({
			state: "error",
			reason: { code: "network" },
		});
	});

	test("keys for access that ended read as no access, never as locked", () => {
		const input = sampleFleet();
		input.keys = input.keys.map((entry) =>
			entry.deviceId === ID.lab ? { ...entry, state: "stale" as const } : entry,
		);
		expect(buildServiceViews(ID.lab, input)).toEqual({
			state: "noaccess",
			reason: { code: "access_ended" },
		});
	});

	test("a loaded source with no placements is a real empty list", () => {
		const input = sampleFleet();
		input.fleet[ID.warehouse] = fleetState(ID.warehouse, {
			status: {
				observations: [observation(ID.warehouse, [], SAMPLE_NOW - 30, "b1")],
				observedAt: SAMPLE_NOW - 30,
				bootId: "b1",
				sequence: 1,
			},
		});
		expect(buildServiceViews(ID.warehouse, input)).toEqual([]);
		expect(viewOf(input, ID.warehouse).health).not.toBe("unknown");
	});
});

describe("source of the service rows", () => {
	test("live while the session is open", () => {
		const services = servicesOf(sampleFleet(), ID.edge);
		expect(services.map((entry) => entry.serviceId)).toEqual([
			"support-bot",
			"invoice-extractor",
			"nightly-sync",
		]);
		for (const entry of services)
			expect(entry.freshness).toMatchObject({ src: "live", age: "live" });
	});

	test("without a session: the newest of snapshot and saved inventory", () => {
		const [scanner] = servicesOf(sampleFleet(), ID.warehouse);
		expect(scanner.freshness.src).toBe("snap");
		expect(isLastKnown(scanner.freshness)).toBe(true);
		expect(scanner.conv).toBe("crash_looping");
		const newerSaved = sampleFleet();
		const saved = newerSaved.fleet[ID.warehouse].saved;
		if (!saved) throw new Error("fixture: saved inventory");
		saved.observedAt = SAMPLE_NOW - 60;
		const [fromSaved] = servicesOf(newerSaved, ID.warehouse);
		expect(fromSaved.freshness.src).toBe("saved");
		expect(fromSaved.conv).toBe("converged");
	});

	test("a closed session falls back to the newest source, the last live read included", () => {
		const closed = () => {
			const input = sampleFleet();
			input.live[ID.edge].state = { kind: "idle" };
			return input;
		};
		const [bot] = servicesOf(closed(), ID.edge);
		expect(bot.freshness.src).toBe("live");
		expect(isLastKnown(bot.freshness)).toBe(true);
		const newerSnapshot = closed();
		const status = newerSnapshot.fleet[ID.edge].status;
		if (!status) throw new Error("fixture: edge status");
		status.observedAt = SAMPLE_NOW;
		expect(servicesOf(newerSnapshot, ID.edge)[0].freshness.src).toBe("snap");
	});

	test("the newest observation wins per placement", () => {
		const input = sampleFleet();
		const running = placement({
			id: "scanner-ingest",
			project_id: "app_warehouse_scan",
			desired_state: "running",
			observed_state: "running",
		});
		const status = input.fleet[ID.warehouse].status;
		if (!status) throw new Error("fixture: warehouse status");
		status.observations.push(
			observation(ID.warehouse, [running], SAMPLE_NOW - 20, "b2"),
		);
		expect(servicesOf(input, ID.warehouse)).toHaveLength(1);
		expect(servicesOf(input, ID.warehouse)[0].conv).toBe("converged");
	});
});

describe("ServiceView fields", () => {
	test("requested, ready, running and max instances", () => {
		const input = sampleFleet();
		expect(service(input, ID.edge, "support-bot").instances).toEqual({
			requested: 2,
			ready: 2,
			running: 2,
			max: 4,
		});
		expect(service(input, ID.edge, "invoice-extractor").instances).toEqual({
			requested: 1,
			ready: 0,
			running: 1,
			max: 1,
		});
		const bare = service(
			edgeWith({
				desired_replicas: undefined,
				running_replicas: undefined,
				ready_replicas: undefined,
				max_replicas: undefined,
			}),
			ID.edge,
			"support-bot",
		);
		expect(bare.instances).toEqual({
			requested: 1,
			ready: 1,
			running: 1,
			max: 1,
		});
	});

	test("convergence, settings, source, events and version", () => {
		const input = sampleFleet();
		const bot = service(input, ID.edge, "support-bot");
		expect(bot).toMatchObject({
			deviceId: ID.edge,
			projectId: "app_support_portal",
			desired: "running",
			observed: "running",
			conv: "converged",
			settings: { applied: 7, latest: 7 },
			source: "offline",
		});
		expect(bot.events).toHaveLength(2);
		expect(bot.appVersion?.hash).toMatch(/^71c6216b/);
		expect(service(input, ID.edge, "nightly-sync").conv).toBe(
			"stopped_by_user",
		);
	});

	test("an active rollout is attached and drives the convergence", () => {
		const extractor = service(sampleFleet(), ID.edge, "invoice-extractor");
		expect(extractor.conv).toBe("update_in_progress");
		expect(extractor.rollout?.state).toBe("activating");
		expect(service(sampleFleet(), ID.edge, "support-bot").rollout).toBe(
			undefined,
		);
	});

	test("offline writes: live queues (worst head first), placement summary, else not loaded", () => {
		const input = sampleFleet();
		expect(
			service(input, ID.studio, "field-notes").offlineWrites,
		).toMatchObject({
			pending: 17,
			quarantined: true,
			head: { state: "conflict" },
		});
		const summary = service(
			edgeWith({
				offline_writes: {
					scopes: 2,
					pending_count: 5,
					pending_bytes: 1_024,
					oldest_at: SAMPLE_NOW - 60,
					quarantined_scopes: 1,
					needs_attention: 1,
					mirror_error: false,
				},
			}),
			ID.edge,
			"support-bot",
		);
		expect(summary.offlineWrites).toEqual({ pending: 5, quarantined: true });
		expect(service(input, ID.edge, "support-bot").offlineWrites).toBe(
			"not_loaded",
		);
	});

	test("diagnostics from the placement or its replicas (BG7)", () => {
		const restarts = {
			failures: 3,
			max_restarts: 5,
			crash_looping: false,
			retry_in_seconds: 8,
			last_started_at: SAMPLE_NOW - 30,
		};
		const fromReplica = service(
			edgeWith({
				replicas: [
					{ slot: 0, observed_state: "running", applied_revision: 7 },
					{
						slot: 1,
						observed_state: "backoff",
						applied_revision: 7,
						last_error: "exit status 1",
						restarts,
					},
				],
			}),
			ID.edge,
			"support-bot",
		);
		expect(fromReplica.diagnostics).toEqual({
			hasError: true,
			lastError: "exit status 1",
			restarts,
		});
		const flagged = service(
			edgeWith({ has_error: false, last_error: null }),
			ID.edge,
			"support-bot",
		);
		expect(flagged.diagnostics).toEqual({ hasError: false });
		const snapshotReplica = {
			slot: 0,
			observed_state: "backoff",
			applied_revision: 7,
			has_error: true,
		};
		expect(
			service(edgeWith({ replicas: [snapshotReplica] }), ID.edge, "support-bot")
				.diagnostics,
		).toEqual({ hasError: true });
		expect(
			service(sampleFleet(), ID.edge, "support-bot").diagnostics,
		).toBeUndefined();
	});

	test("schedules: what the process reported, else what the missing fact means on this plane", () => {
		const reported = [
			{
				event_id: "evt_support_digest",
				expression: "0 0 8 * * 1-5",
				timezone: "Europe/Berlin",
				hold: null,
				next_at: SAMPLE_NOW + 3_600,
				runs: 4,
			},
		];
		const live = service(
			edgeWith({ schedules: reported, schedules_truncated: true }),
			ID.edge,
			"support-bot",
		);
		expect(live.schedules).toEqual(reported);
		expect(live.schedulesTruncated).toBe(true);
		expect(
			service(edgeWith({ schedules: [] }), ID.edge, "support-bot"),
		).toMatchObject({ schedules: [] });
		expect(
			service(edgeWith({ schedules: [] }), ID.edge, "support-bot"),
		).not.toHaveProperty("schedulesTruncated");
		// The agent can say and the row carries none: not running, or not armed yet.
		expect(service(sampleFleet(), ID.edge, "support-bot").schedules).toBe(
			"not_reported",
		);
		expect(viewOf(sampleFleet(), ID.edge).features?.scheduled_events).toBe(1);
		// An older agent can't say.
		const older = sampleFleetOlderAgent();
		expect(service(older, ID.edge, "support-bot").schedules).toBe(
			"needs_agent",
		);
		expect(viewOf(older, ID.edge).features).toEqual({});
		// A snapshot says it only when it carries the feature map (a device-scope reader).
		const [snapshot] = servicesOf(sampleFleet(), ID.warehouse);
		expect(snapshot.freshness.src).toBe("snap");
		expect(snapshot.schedules).toBe("not_reported");
		const [unknown] = servicesOf(older, ID.warehouse);
		expect(unknown.schedules).toBe("not_loaded");
		expect(viewOf(older, ID.warehouse).features).toBeUndefined();
		// Saved inventory carries no flags: unknown, never "too old".
		const saved = sampleFleet();
		const inventory = saved.fleet[ID.warehouse].saved;
		if (!inventory) throw new Error("fixture: saved inventory");
		inventory.observedAt = SAMPLE_NOW - 60;
		expect(servicesOf(saved, ID.warehouse)[0].schedules).toBe("not_loaded");
	});

	test("bots and actions follow the same rule, each with its own flags", () => {
		const bots = [
			{
				event_id: "evt_helper",
				provider: "telegram" as const,
				state: "connected" as const,
				hold: null,
				bot_name: null,
			},
		];
		const actions = [
			{
				event_id: "evt_support_reply",
				kind: "action" as const,
				fields: 0,
				file_fields: 0,
				runs: 3,
			},
		];
		const live = service(
			edgeWith({
				bots,
				bots_truncated: true,
				actions,
				actions_truncated: true,
			}),
			ID.edge,
			"support-bot",
		);
		expect(live).toMatchObject({
			bots,
			botsTruncated: true,
			actions,
			actionsTruncated: true,
		});
		const none = service(sampleFleet(), ID.edge, "support-bot");
		expect([none.bots, none.actions]).toEqual(["not_reported", "not_reported"]);
		expect(none).not.toHaveProperty("botsTruncated");
		const older = service(sampleFleetOlderAgent(), ID.edge, "support-bot");
		expect([older.bots, older.actions]).toEqual(["needs_agent", "needs_agent"]);
		// One bot flag is enough to report bots; actions have their own.
		const input = sampleFleet();
		const inspection = input.live[ID.edge].inspection;
		if (!inspection) throw new Error("fixture: edge inspection");
		inspection.value.features = { discord_bots: 1, scheduled_events: 1 };
		const partial = service(input, ID.edge, "support-bot");
		expect([partial.bots, partial.actions]).toEqual([
			"not_reported",
			"needs_agent",
		]);
		const [unknown] = servicesOf(sampleFleetOlderAgent(), ID.warehouse);
		expect([unknown.bots, unknown.actions]).toEqual([
			"not_loaded",
			"not_loaded",
		]);
	});
});

describe("older hub and older agent", () => {
	test("an older agent gives views without diagnostics, events or source", () => {
		const input = sampleFleetOlderAgent();
		const bot = service(input, ID.edge, "support-bot");
		expect(bot.source).toBeNull();
		expect(bot.events).toBeNull();
		expect(bot.diagnostics).toBeUndefined();
		// Without BG8 the agent only sends its constant crate version: no release is known.
		expect(viewOf(input, ID.edge).agent).toBeUndefined();
		input.agentLastRead = {
			...input.agentLastRead,
			[ID.edge]: { version: "0.9.3", at: SAMPLE_NOW - 60 },
		};
		expect(viewOf({ ...input }, ID.edge).agent?.version).toBe("0.9.3");
		expect(viewOf(input, ID.warehouse).health).toBe("critical");
	});

	test("an older hub builds every view without throwing", () => {
		const input = sampleFleetOlderHub();
		const items = itemsOf(input);
		for (const device of fleetFacts(input).devices) {
			const view = buildDeviceView(device.id, input, items);
			expect(view?.row.device_id).toBe(device.id);
			if (Array.isArray(view?.services)) continue;
			expect(view?.services.state).toBeDefined();
		}
		expect(viewOf(input, ID.edge).relationship).toBe("owner");
	});
});

describe("rollupHealth", () => {
	const row = { device_id: ID.edge, status: "active" as const };

	test("revoked wins, critical beats attention, Info is never counted", () => {
		expect(
			rollupHealth(
				[item(ID.edge, "critical")],
				{ ...row, status: "revoked" },
				true,
			),
		).toBe("revoked");
		expect(
			rollupHealth(
				[item(ID.edge, "notice"), item(ID.edge, "critical")],
				row,
				true,
			),
		).toBe("critical");
		expect(rollupHealth([item(ID.edge, "notice")], row, false)).toBe(
			"attention",
		);
		expect(rollupHealth([item(ID.edge, "info")], row, true)).toBe("healthy");
		expect(rollupHealth([item(ID.edge, "info")], row, false)).toBe("unknown");
	});

	test("other devices' items don't count", () => {
		expect(rollupHealth([item(ID.studio, "critical")], row, true)).toBe(
			"healthy",
		);
	});
});

describe("helpers", () => {
	test("currentRollout: an active rollout, else the most recently updated", () => {
		const old = rollout({ rollout_id: "a", state: "healthy" });
		const newer = rollout({
			rollout_id: "b",
			state: "failed",
			updated_at: SAMPLE_NOW - 60,
		});
		const active = rollout({
			rollout_id: "c",
			state: "staged",
			updated_at: SAMPLE_NOW - 7_200,
		});
		const other = rollout({
			rollout_id: "d",
			state: "activating",
			placement_id: "nightly-sync",
		});
		expect(currentRollout([old, newer, other], "support-bot")?.rollout_id).toBe(
			"b",
		);
		expect(
			currentRollout([old, newer, active], "support-bot")?.rollout_id,
		).toBe("c");
		expect(currentRollout(undefined, "support-bot")).toBeUndefined();
	});

	test("keysLocked covers every key state an unlock can open", () => {
		for (const state of [
			"locked",
			"unlocking",
			"held_elsewhere",
			"blocked",
		] as const)
			expect(keysLocked({ state })).toBe(true);
		for (const state of ["unlocked", "none", "stale"] as const)
			expect(keysLocked({ state })).toBe(false);
	});

	test("candidate ids are key, subject and part", () => {
		const source = classify("device_row", { now: SAMPLE_NOW, loaded: true });
		expect(
			attentionCandidate({
				key: "history_paused_readers_expired",
				severity: "warning",
				subject: { kind: "device", deviceId: ID.edge },
				source,
				part: "logs",
			}).id,
		).toBe(`history_paused_readers_expired:${ID.edge}:logs`);
		expect(
			subjectId({
				kind: "service",
				deviceId: ID.edge,
				serviceId: "support-bot",
				projectId: "app_support_portal",
			}),
		).toBe(`${ID.edge}/support-bot`);
		expect(subjectId({ kind: "certificate", deviceId: ID.edge })).toBe(
			`${ID.edge}/*`,
		);
		expect(subjectId({ kind: "hub" })).toBe("hub");
	});

	test("compareVersions is numeric per part", () => {
		expect(compareVersions("0.9.10", "0.9.4")).toBeGreaterThan(0);
		expect(compareVersions("0.9.2", "0.9.4")).toBeLessThan(0);
		expect(compareVersions("1.0", "1.0.0")).toBe(0);
		expect(compareVersions("1.2.0-beta", "1.2.0")).toBe(0);
	});

	test("facts are computed once per input object", () => {
		const input = sampleFleet();
		expect(fleetFacts(input)).toBe(fleetFacts(input));
		expect(fleetFacts(sampleFleet())).not.toBe(fleetFacts(input));
	});
});
