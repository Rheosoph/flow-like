import { describe, expect, test } from "bun:test";
import { getI18n } from "@flow-like/locales";
import {
	attentionCopy,
	defaultCopyTime,
} from "../../../components/settings/devices/copy/attention-copy";
import type { DevicesT } from "../../../components/settings/devices/primitives/area-context";
import {
	SAMPLE_APPS,
	SAMPLE_IDS,
	SAMPLE_NOW,
	placement,
	resourceSummaryOf,
	sampleFleet,
	sampleFleetOlderAgent,
	sampleFleetOlderHub,
} from "./__fixtures__/sample-fleet";
import { generateFleet } from "./__fixtures__/sample-fleet-200";
import {
	ATTENTION_KEYS,
	ATTENTION_RULES,
	type AttentionInputExt,
	DEFERRED_KEYS,
	SNOOZE_S,
	compareAttention,
	computeAttention,
	countAttention,
	createAttentionMemory,
	filterAttention,
	snoozeAttention,
} from "./attention";
import { fleetFacts, rollupHealth } from "./device-view";
import type { AttentionItem, AttentionMemory } from "./types";

function memory(): AttentionMemory {
	return createAttentionMemory("test", undefined);
}

function label(input: AttentionInputExt, item: AttentionItem) {
	const deviceId =
		"deviceId" in item.subject ? item.subject.deviceId : undefined;
	const name = deviceId
		? (fleetFacts(input).byId.get(deviceId)?.name ?? deviceId)
		: item.subject.kind;
	return `${item.key}@${name}`;
}

function counted(input: AttentionInputExt, items: AttentionItem[]) {
	return items
		.filter((item) => item.severity !== "info")
		.map((item) => label(input, item))
		.sort();
}

const GOLDEN = [
	"account_backup_upload_pending@studio-mac-mini",
	"agent_update_available@warehouse-pi",
	"certificate_expired@warehouse-pi",
	"certificate_expiring@edge-berlin-01",
	"grant_expiring@edge-berlin-01",
	"history_paused_readers_expired@edge-berlin-01",
	"keys_not_backed_up_to_account@cold-storage-nas",
	"no_heartbeat_since_enrollment@cold-storage-nas",
	"offline_since@warehouse-pi",
	"offline_writes_conflict@studio-mac-mini",
	"offline_writes_quarantined@studio-mac-mini",
	"pending_setup_expired@setup",
	"service_crash_looping@warehouse-pi",
	"sharing_policy_waiting_for_device@studio-mac-mini",
	"you_still_pay_for_a_revoked_device@partner-edge",
];

/** Keys whose only inputs are BG fields (hub or agent) that the older variants drop. */
const NEEDS_NEW_HUB = [
	"device_slots_nearly_full",
	"backup_slots_nearly_full",
	"history_storage_nearly_full",
	"history_not_stored_by_plan",
	"online_files_read_only",
	"access_denied",
] as const;
const NEEDS_NEW_AGENT = [
	"background_task_failing",
	"device_operation_failed",
	"device_operation_unknown",
] as const;

describe("golden sample fleet (APP A10)", () => {
	test("exactly 2 critical · 15 total, Info never counted", () => {
		const input = sampleFleet();
		const items = computeAttention(input, memory());
		const counts = countAttention(items);
		expect(counts).toMatchObject({
			critical: 2,
			warning: 8,
			notice: 5,
			total: 15,
		});
		expect(counts.info).toBeGreaterThan(0);
		expect(counted(input, items)).toEqual(GOLDEN);
	});

	test("ids are unique (dedupe e) and the list is ordered", () => {
		const items = computeAttention(sampleFleet(), memory());
		expect(new Set(items.map((item) => item.id)).size).toBe(items.length);
		expect([...items].sort(compareAttention)).toEqual(items);
	});

	test("order: severity, then service > device > certificate > access > keys > setup, then age", () => {
		const input = sampleFleet();
		const items = computeAttention(input, memory());
		const head = items.slice(0, 2).map((item) => label(input, item));
		expect(head).toEqual([
			"service_crash_looping@warehouse-pi",
			"offline_since@warehouse-pi",
		]);
		const notices = items
			.filter((item) => item.severity === "notice")
			.map((item) => item.subject.kind);
		expect(notices).toEqual(["device", "access", "access", "keys", "setup"]);
		const access = items.filter(
			(item) => item.severity === "notice" && item.subject.kind === "access",
		);
		expect(access[0].firstSeenAt).toBeLessThanOrEqual(access[1].firstSeenAt);
	});

	test("an older hub and an older agent give the same counted items and no BG-only item", () => {
		for (const input of [sampleFleetOlderHub(), sampleFleetOlderAgent()]) {
			const items = computeAttention(input, memory());
			expect(counted(input, items)).toEqual(GOLDEN);
			const keys = new Set(items.map((item) => item.key));
			for (const key of [...NEEDS_NEW_HUB, ...NEEDS_NEW_AGENT])
				expect(keys.has(key)).toBe(false);
		}
	});

	test("older hub rows carry no BG1 relationship, so a foreign shared device stays unknown", () => {
		const input = sampleFleetOlderHub();
		expect(fleetFacts(input).byId.get(SAMPLE_IDS.lab)?.relationship).toBe(
			"unknown",
		);
		expect(input.devices.every((row) => row.relationship === undefined)).toBe(
			true,
		);
	});
});

describe("rule coverage (IA §6.5)", () => {
	test("every condition key has exactly one rule or a deferral", () => {
		const ruleKeys = ATTENTION_RULES.map((rule) => rule.key);
		expect(new Set(ruleKeys).size).toBe(ruleKeys.length);
		for (const key of ATTENTION_KEYS) {
			const hasRule = ruleKeys.includes(key);
			const deferred = key in DEFERRED_KEYS;
			expect(`${key}:${hasRule || deferred}`).toBe(`${key}:true`);
			expect(hasRule && deferred).toBe(false);
		}
		expect(ruleKeys.every((key) => ATTENTION_KEYS.includes(key))).toBe(true);
	});

	test("an empty input yields nothing and never throws", () => {
		const input = sampleFleet();
		input.devices = [];
		input.pendingSetups = [];
		input.activity = [];
		input.accessRequests = [];
		input.local.authorities = [];
		input.local.vaults = [];
		expect(computeAttention(input, memory())).toEqual([]);
	});
});

describe("dedupe", () => {
	test("(a) offline: service items become last known, not-as-requested is dropped", () => {
		const input = sampleFleet();
		const status = input.fleet[SAMPLE_IDS.warehouse].status;
		if (!status) throw new Error("fixture: warehouse status");
		status.observations[0].placements.push(
			placement({
				id: "label-printer",
				project_id: SAMPLE_APPS.warehouseScanner,
				desired_state: "running",
				observed_state: "stopping",
			}),
			placement({
				id: "scale-reader",
				project_id: SAMPLE_APPS.warehouseScanner,
				desired_state: "running",
				observed_state: "running",
				desired_replicas: 2,
				ready_replicas: 1,
			}),
		);
		const mem = memory();
		computeAttention({ ...input, now: SAMPLE_NOW - 600 }, mem);
		const items = computeAttention(input, mem);
		const warehouse = items.filter(
			(item) =>
				item.subject.kind === "service" &&
				item.subject.deviceId === SAMPLE_IDS.warehouse,
		);
		expect(warehouse.map((item) => item.key).sort()).toEqual([
			"service_crash_looping",
			"service_degraded",
		]);
		expect(warehouse.every((item) => item.lastKnown)).toBe(true);
	});

	test("(b) expired access rules suppress history-paused items", () => {
		const input = sampleFleet();
		const policy = input.policies[SAMPLE_IDS.edge].policy;
		if (!policy) throw new Error("fixture: edge policy");
		policy.expires_at = SAMPLE_NOW - 60;
		const items = computeAttention(input, memory());
		const keys = items
			.filter(
				(item) =>
					"deviceId" in item.subject &&
					item.subject.deviceId === SAMPLE_IDS.edge,
			)
			.map((item) => item.key);
		expect(keys).toContain("sharing_policy_expired");
		expect(keys).not.toContain("history_paused_readers_expired");
		expect(
			items.find((item) => item.key === "sharing_policy_expired")?.severity,
		).toBe("critical");
	});

	test("(c) a revoked approval suppresses quarantined offline writes", () => {
		const input = sampleFleet();
		const studio = input.resources[SAMPLE_IDS.studio];
		if (!studio) throw new Error("fixture: studio resources");
		studio.grants[0].status = "revoked";
		input.resourceSummary = resourceSummaryOf(
			input.resources,
			input.me,
			input.now,
		);
		const keys = computeAttention(input, memory()).map((item) => item.key);
		expect(keys).not.toContain("offline_writes_quarantined");
		expect(keys).toContain("cloud_access_invalid");
		expect(keys).toContain("offline_writes_conflict");
	});

	test("(d) a revoked device keeps only consent and local-key items", () => {
		const input = sampleFleet();
		const edge = input.devices.find((row) => row.device_id === SAMPLE_IDS.edge);
		if (!edge) throw new Error("fixture: edge");
		edge.status = "revoked";
		const items = computeAttention(input, memory()).filter(
			(item) =>
				"deviceId" in item.subject && item.subject.deviceId === SAMPLE_IDS.edge,
		);
		expect(items.map((item) => item.key).sort()).toEqual([
			"revoked",
			"stale_local_keys",
			"you_still_pay_for_a_revoked_device",
		]);
	});
});

describe("memory: dwell, reloads, snooze", () => {
	function degradedFleet() {
		const input = sampleFleet();
		const inspection = input.live[SAMPLE_IDS.edge].inspection;
		if (!inspection) throw new Error("fixture: edge inspection");
		inspection.value.placements[0] = {
			...inspection.value.placements[0],
			ready_replicas: 1,
		};
		return input;
	}
	const degradedId = `service_degraded:${SAMPLE_IDS.edge}/support-bot`;

	test("a dwell condition shows after 2 min, also across a reload", () => {
		const store = new Map<string, string>();
		const storage = {
			getItem: (key: string) => store.get(key) ?? null,
			setItem: (key: string, value: string) => void store.set(key, value),
		};
		const first = computeAttention(
			degradedFleet(),
			createAttentionMemory("acct", storage),
		);
		expect(first.some((item) => item.id === degradedId)).toBe(false);
		const later = { ...degradedFleet(), now: SAMPLE_NOW + 121 };
		const reloaded = computeAttention(
			later,
			createAttentionMemory("acct", storage),
		);
		const item = reloaded.find((entry) => entry.id === degradedId);
		expect(item?.firstSeenAt).toBe(SAMPLE_NOW);
		expect(item?.severity).toBe("warning");
	});

	test("a reload that starts locked or still loading keeps the dwell and the first sighting", () => {
		const store = new Map<string, string>();
		const storage = {
			getItem: (key: string) => store.get(key) ?? null,
			setItem: (key: string, value: string) => void store.set(key, value),
		};
		computeAttention(degradedFleet(), createAttentionMemory("acct", storage));

		const loading = { ...degradedFleet(), now: SAMPLE_NOW + 20, devices: [] };
		const reloaded = createAttentionMemory("acct", storage);
		computeAttention(loading, reloaded);
		expect(reloaded.firstSeen[degradedId]).toBe(SAMPLE_NOW);

		const locked = { ...degradedFleet(), now: SAMPLE_NOW + 40 };
		locked.keys = locked.keys.map((entry) =>
			entry.deviceId === SAMPLE_IDS.edge
				? { ...entry, state: "locked" as const }
				: entry,
		);
		locked.live = {};
		computeAttention(locked, reloaded);
		expect(reloaded.firstSeen[degradedId]).toBe(SAMPLE_NOW);

		const unlocked = { ...degradedFleet(), now: SAMPLE_NOW + 121 };
		const item = computeAttention(unlocked, reloaded).find(
			(entry) => entry.id === degradedId,
		);
		expect(item?.firstSeenAt).toBe(SAMPLE_NOW);
	});

	test("an unreadable device does not keep a first sighting for ever", () => {
		const mem = memory();
		computeAttention(degradedFleet(), mem);
		const locked = { ...degradedFleet(), now: SAMPLE_NOW + 31 * 86_400 };
		locked.keys = locked.keys.map((entry) => ({
			...entry,
			state: "locked" as const,
		}));
		locked.live = {};
		computeAttention(locked, mem);
		expect(mem.firstSeen[degradedId]).toBeUndefined();
	});

	test("firstSeen resets when the condition clears", () => {
		const mem = memory();
		computeAttention(degradedFleet(), mem);
		expect(mem.firstSeen[degradedId]).toBe(SAMPLE_NOW);
		computeAttention({ ...sampleFleet(), now: SAMPLE_NOW + 60 }, mem);
		expect(mem.firstSeen[degradedId]).toBeUndefined();
		computeAttention({ ...degradedFleet(), now: SAMPLE_NOW + 200 }, mem);
		expect(mem.firstSeen[degradedId]).toBe(SAMPLE_NOW + 200);
	});

	test("a data-known start counts as first seen (policy saved 8 min ago)", () => {
		const items = computeAttention(sampleFleet(), memory());
		const waiting = items.find(
			(item) => item.key === "sharing_policy_waiting_for_device",
		);
		expect(waiting?.firstSeenAt).toBe(1_790_769_120);
	});

	test("notices snooze for 7 days on this computer; other severities don't", () => {
		const mem = memory();
		const items = computeAttention(sampleFleet(), mem);
		const notice = items.find((item) => item.severity === "notice");
		const warning = items.find((item) => item.severity === "warning");
		if (!notice || !warning) throw new Error("fixture: severities");
		expect(snoozeAttention(mem, warning, SAMPLE_NOW)).toBe(false);
		expect(snoozeAttention(mem, notice, SAMPLE_NOW)).toBe(true);
		expect(mem.snoozed[notice.id]).toBe(SAMPLE_NOW + SNOOZE_S);
		const snoozed = computeAttention(sampleFleet(), mem);
		expect(snoozed.some((item) => item.id === notice.id)).toBe(false);
		expect(countAttention(snoozed).total).toBe(14);
		const back = computeAttention(
			{ ...sampleFleet(), now: SAMPLE_NOW + SNOOZE_S },
			mem,
		);
		expect(back.some((item) => item.id === notice.id)).toBe(true);
		expect(mem.snoozed[notice.id]).toBeUndefined();
	});

	test("storage that throws degrades to memory only", () => {
		const broken = {
			getItem: () => {
				throw new Error("SecurityError");
			},
			setItem: () => {
				throw new Error("QuotaExceededError");
			},
		};
		const mem = createAttentionMemory("acct", broken);
		expect(() => computeAttention(sampleFleet(), mem)).not.toThrow();
		expect(Object.keys(mem.firstSeen).length).toBeGreaterThan(0);
	});

	test("corrupt stored memory is ignored", () => {
		const storage = {
			getItem: () => '{"firstSeen":{"a":"x","b":5},"snoozed":[1]}',
			setItem: () => undefined,
		};
		const mem = createAttentionMemory("acct", storage);
		expect(mem.firstSeen).toEqual({ b: 5 });
		expect(mem.snoozed).toEqual({});
	});
});

describe("filters (APP A10)", () => {
	const input = sampleFleet();
	const items = computeAttention(input, memory());
	const keysFor = (appId: string) =>
		filterAttention(items, { appId, minSeverity: "notice" }, input)
			.map((item) => label(input, item))
			.sort();

	test("app scope keeps app services and device items of devices that run it", () => {
		expect(keysFor(SAMPLE_APPS.warehouseScanner)).toEqual([
			"agent_update_available@warehouse-pi",
			"offline_since@warehouse-pi",
			"service_crash_looping@warehouse-pi",
		]);
		expect(keysFor(SAMPLE_APPS.fieldNotes)).toEqual([
			"offline_writes_conflict@studio-mac-mini",
			"offline_writes_quarantined@studio-mac-mini",
		]);
		expect(keysFor(SAMPLE_APPS.invoiceAi)).toEqual([]);
		expect(
			filterAttention(items, { appId: SAMPLE_APPS.invoiceAi }, input).map(
				(item) => item.key,
			),
		).toEqual(["rollout_in_progress"]);
	});

	test("device, service and severity filters", () => {
		const device = filterAttention(items, { deviceId: SAMPLE_IDS.cold }, input);
		expect(device.map((item) => item.key).sort()).toEqual([
			"keys_not_backed_up_to_account",
			"no_heartbeat_since_enrollment",
		]);
		const service = filterAttention(
			items,
			{ deviceId: SAMPLE_IDS.studio, serviceId: "field-notes" },
			input,
		);
		expect(service).toHaveLength(2);
		expect(
			filterAttention(items, { minSeverity: "critical" }, input),
		).toHaveLength(2);
	});
});

describe("generated fleet of 200 (SPEC §7.3)", () => {
	const { input, kinds } = generateFleet(200);

	test("200 devices, deterministic per seed", () => {
		expect(input.devices).toHaveLength(200);
		expect(generateFleet(200).input.devices.map((row) => row.name)).toEqual(
			input.devices.map((row) => row.name),
		);
		expect(generateFleet(43).input.devices).toHaveLength(43);
	});

	test("health rollup matches the generated distribution", () => {
		const mem = memory();
		computeAttention({ ...input, now: SAMPLE_NOW - 900 }, mem);
		const items = computeAttention(input, mem);
		const facts = fleetFacts(input);
		const levels = {
			critical: 0,
			attention: 0,
			unknown: 0,
			healthy: 0,
			revoked: 0,
		};
		for (const device of facts.devices)
			levels[rollupHealth(items, device.row, Array.isArray(device.services))]++;
		expect(levels).toEqual({
			critical: 4,
			attention: 21,
			unknown: 12,
			healthy: 156,
			revoked: 7,
		});
		const expected = {
			critical: "critical",
			"attention-off": "attention",
			attention: "attention",
			locked: "unknown",
			setup: "unknown",
			revoked: "revoked",
			healthy: "healthy",
		} as const;
		for (const [id, kind] of kinds) {
			const device = facts.byId.get(id);
			if (!device) throw new Error(`generated device ${id} missing`);
			expect(
				`${device.name}:${rollupHealth(items, device.row, Array.isArray(device.services))}`,
			).toBe(`${device.name}:${expected[kind]}`);
		}
		expect(new Set(items.map((item) => item.id)).size).toBe(items.length);
	});

	test("one computation over 200 devices stays well under budget", () => {
		const fresh = generateFleet(200).input;
		const started = performance.now();
		computeAttention(fresh, memory());
		expect(performance.now() - started).toBeLessThan(250);
	});
});

describe("copy (SPEC §6.3, R3)", () => {
	const t = getI18n().getFixedT("en", "devices") as DevicesT;
	const ctx = {
		time: defaultCopyTime(),
		personName: (id: string) => `person ${id.slice(-4)}`,
		appName: (id: string) => `app ${id.slice(-4)}`,
	};

	function expectReadable(item: AttentionItem) {
		const copy = attentionCopy(t, item, ctx);
		const text = [copy.sentence, copy.action ?? "", copy.secondary ?? ""].join(
			" ",
		);
		expect(copy.sentence.trim().length).toBeGreaterThan(0);
		expect(text).not.toMatch(/\{\{|\bundefined\b|\bNaN\b|\[object Object\]/);
		if (item.key.includes("_")) expect(text).not.toContain(item.key);
		if (item.action) expect(copy.action?.trim().length).toBeGreaterThan(0);
		if (item.secondary)
			expect(copy.secondary?.trim().length).toBeGreaterThan(0);
	}

	test("every item of every sample fleet reads as a sentence", () => {
		for (const input of [
			sampleFleet(),
			sampleFleetOlderHub(),
			sampleFleetOlderAgent(),
			generateFleet(200).input,
		])
			for (const item of computeAttention(input, memory()))
				expectReadable(item);
	});

	test("every condition key renders even without params", () => {
		for (const key of ATTENTION_KEYS)
			expectReadable({
				id: key,
				key,
				severity: "notice",
				subject: { kind: "hub" },
				copy: { code: key },
				source: { src: "hub", age: "current" },
				lastKnown: false,
				firstSeenAt: SAMPLE_NOW,
			});
	});

	test("the golden critical sentences", () => {
		const items = computeAttention(sampleFleet(), memory());
		const critical = items.filter((item) => item.severity === "critical");
		expect(
			critical.map((item) => attentionCopy(t, item, ctx).sentence),
		).toEqual([
			expect.stringContaining("scanner-ingest"),
			expect.stringContaining("warehouse-pi"),
		]);
	});
});
