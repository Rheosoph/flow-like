/*
 * SPEC §7.3 item 17: the sample fleet plus generated devices (seeded LCG, same
 * names and distribution as the prototype's `_generateFleet`). For 200: 3 critical,
 * 9 offline + 9 other "needs attention", 8 locked, 5 revoked, 3 setting up, the rest healthy.
 */
import type { AttentionInputExt } from "../attention";
import type { DeviceRow, PlacementStatusPlus } from "../types";
import {
	SAMPLE_NOW,
	SAMPLE_PEOPLE,
	fleetState,
	identityOf,
	keySession,
	observation,
	placement,
	sampleFleet,
	vault,
} from "./sample-fleet";

export type GeneratedKind =
	| "critical"
	| "attention-off"
	| "attention"
	| "locked"
	| "revoked"
	| "setup"
	| "healthy";

const NOW = SAMPLE_NOW;
const DAY = 86_400;
const CITIES = [
	"munich",
	"hamburg",
	"vienna",
	"zurich",
	"prague",
	"lyon",
	"milan",
	"oslo",
	"ghent",
	"porto",
	"basel",
	"graz",
	"bremen",
	"leipzig",
	"linz",
];
const BASE_APPS = [
	"app_support_portal",
	"app_warehouse_scan",
	"app_field_notes",
	"app_crm_sync",
	"app_invoice_ai",
];
const STRESS_APPS = [
	"app_store_pos",
	"app_camera_ingest",
	"app_kiosk_ui",
	"app_line_telemetry",
	"app_shelf_vision",
	"app_delivery_notes",
	"app_returns_desk",
	"app_energy_monitor",
	"app_door_access",
	"app_cold_chain",
];
const SERVICE_NAMES: Record<string, string> = {
	app_support_portal: "support-bot",
	app_warehouse_scan: "scanner",
	app_field_notes: "notes",
	app_crm_sync: "crm-sync",
	app_invoice_ai: "invoice",
	app_store_pos: "pos-sync",
	app_camera_ingest: "cam-ingest",
	app_kiosk_ui: "kiosk-ui",
	app_line_telemetry: "telemetry",
	app_shelf_vision: "shelf-vision",
	app_delivery_notes: "delivery",
	app_returns_desk: "returns",
	app_energy_monitor: "energy",
	app_door_access: "door",
	app_cold_chain: "cold-chain",
};

function random(seed: number) {
	let state = seed >>> 0;
	const next = () => {
		state = (Math.imul(state, 1_664_525) + 1_013_904_223) >>> 0;
		return state / 4_294_967_296;
	};
	const hex = (length: number) => {
		let out = "";
		while (out.length < length)
			out += Math.floor(next() * 4_294_967_296)
				.toString(16)
				.padStart(8, "0");
		return out.slice(0, length);
	};
	const uuid = () => {
		const h = hex(32);
		return `${h.slice(0, 8)}-${h.slice(8, 12)}-4${h.slice(13, 16)}-a${h.slice(17, 20)}-${h.slice(20, 32)}`;
	};
	return { next, hex, uuid };
}

function names(extra: number, big: boolean): string[] {
	const out: string[] = [];
	if (!big) {
		for (let i = 1; i <= 18; i++)
			out.push(`store-01${String(i).padStart(2, "0")}`);
		for (let i = 1; i <= 6; i++) out.push(`line-${i}-gateway`);
		out.push(
			"kiosk-hall-a",
			"kiosk-hall-b",
			"kiosk-hall-c",
			"cam-dock-01",
			"cam-dock-02",
			"cam-dock-03",
		);
		for (const city of CITIES.slice(0, 6)) out.push(`edge-${city}-01`);
	} else {
		for (let i = 1; i <= 110; i++)
			out.push(`store-${String(100 + i).padStart(4, "0")}`);
		for (let i = 1; i <= 24; i++) out.push(`line-${i}-gateway`);
		for (let i = 1; i <= 16; i++)
			out.push(`kiosk-${String(i).padStart(2, "0")}`);
		for (let i = 1; i <= 16; i++)
			out.push(`cam-ingest-${String(i).padStart(2, "0")}`);
		for (const city of CITIES) out.push(`edge-${city}-01`, `edge-${city}-02`);
		for (let i = 1; out.length < extra; i++)
			out.push(`lab-rpi-${String(i).padStart(2, "0")}`);
	}
	return out.slice(0, extra);
}

function kinds(
	extra: number,
	big: boolean,
	next: () => number,
): GeneratedKind[] {
	const list: GeneratedKind[] = [];
	const add = (kind: GeneratedKind, count: number) => {
		for (let i = 0; i < count; i++) list.push(kind);
	};
	if (big) {
		add("critical", 3);
		add("attention-off", 9);
		add("attention", 9);
		add("locked", 8);
		add("revoked", 5);
		add("setup", 3);
		add("healthy", extra - 37);
	} else {
		add("critical", 2);
		add("attention-off", 2);
		add("attention", 8);
		add("locked", 1);
		add("healthy", extra - 13);
	}
	for (let i = list.length - 1; i > 0; i--) {
		const j = Math.floor(next() * (i + 1));
		[list[i], list[j]] = [list[j], list[i]];
	}
	return list;
}

export interface GeneratedFleet {
	input: AttentionInputExt;
	/** Generated device id → its intended health class. */
	kinds: ReadonlyMap<string, GeneratedKind>;
}

type Random = ReturnType<typeof random>;

interface Generation {
	input: AttentionInputExt;
	apps: readonly string[];
	rnd: Random;
	lastReads: NonNullable<AttentionInputExt["agentLastRead"]>;
}

interface GeneratedDevice {
	id: string;
	name: string;
	index: number;
	kind: GeneratedKind;
	/** For "attention" devices: 0 = certificate expiring, 1 = service degraded, 2 = older agent. */
	variant: number;
	shared: boolean;
}

interface ReadTimes {
	certificateNotAfter: number;
	statusAt: number;
	certificatesAt: number;
	agentReadAt: number;
}

type ServiceShape = Pick<
	PlacementStatusPlus,
	"desired_state" | "observed_state" | "running_replicas" | "ready_replicas"
>;

const SERVICE_SHAPES = {
	running: {
		desired_state: "running",
		observed_state: "running",
		running_replicas: 1,
		ready_replicas: 1,
	},
	crashing: {
		desired_state: "running",
		observed_state: "backoff",
		running_replicas: 0,
		ready_replicas: 0,
	},
	degraded: {
		desired_state: "running",
		observed_state: "running",
		running_replicas: 1,
		ready_replicas: 0,
	},
	stopped: {
		desired_state: "stopped",
		observed_state: "stopped",
		running_replicas: 0,
		ready_replicas: 0,
	},
} satisfies Record<string, ServiceShape>;

const isOffline = (kind: GeneratedKind) =>
	kind === "critical" || kind === "attention-off";
const isActive = (kind: GeneratedKind) =>
	kind !== "setup" && kind !== "revoked";
const isUnlocked = (kind: GeneratedKind) => isActive(kind) && kind !== "locked";

function registration(
	kind: GeneratedKind,
	rnd: Random,
): { registeredAt: number; lastSeen: number | null } {
	const registeredAt = NOW - Math.floor(20 + rnd.next() * 320) * DAY;
	if (kind === "setup")
		return {
			registeredAt: NOW - Math.floor(120 + rnd.next() * 360),
			lastSeen: null,
		};
	if (kind === "revoked")
		return {
			registeredAt,
			lastSeen: registeredAt + Math.floor(rnd.next() * 60) * DAY,
		};
	const lastSeen = isOffline(kind)
		? NOW - Math.floor(1_800 + rnd.next() * 90_000)
		: NOW - Math.floor(5 + rnd.next() * 80);
	return { registeredAt, lastSeen };
}

function serviceShape(device: GeneratedDevice, slot: number): ServiceShape {
	if (device.kind === "attention-off") return SERVICE_SHAPES.stopped;
	if (slot > 0) return SERVICE_SHAPES.running;
	if (device.kind === "critical") return SERVICE_SHAPES.crashing;
	return device.kind === "attention" && device.variant === 1
		? SERVICE_SHAPES.degraded
		: SERVICE_SHAPES.running;
}

function services(
	gen: Generation,
	device: GeneratedDevice,
): PlacementStatusPlus[] {
	if (!isActive(device.kind)) return [];
	const { apps, rnd } = gen;
	const count = 1 + Math.floor(rnd.next() * 2);
	return Array.from({ length: count }, (_, slot) => {
		const app = apps[(device.index + slot * 3) % apps.length];
		const revision = 2 + Math.floor(rnd.next() * 9);
		return placement({
			id: slot ? `${SERVICE_NAMES[app]}-${slot + 1}` : SERVICE_NAMES[app],
			project_id: app,
			deployment_id: rnd.uuid(),
			revision: rnd.hex(64),
			...serviceShape(device, slot),
			config_revision: revision,
			applied_revision: revision,
			max_replicas: 2,
		});
	});
}

function readTimes(
	device: GeneratedDevice,
	lastSeen: number | null,
	rnd: Random,
): ReadTimes {
	const expiring = device.kind === "attention" && device.variant === 0;
	const certificateNotAfter =
		NOW +
		Math.floor(expiring ? 2 + rnd.next() * 5 : 12 + rnd.next() * 80) * DAY;
	if (isOffline(device.kind)) {
		const seen = lastSeen ?? NOW;
		return {
			certificateNotAfter,
			statusAt: seen - 10,
			certificatesAt: seen,
			agentReadAt: seen,
		};
	}
	return {
		certificateNotAfter,
		statusAt: NOW - Math.floor(10 + rnd.next() * 50),
		certificatesAt: NOW - Math.floor(600 + rnd.next() * 3_000),
		agentReadAt: NOW - 30,
	};
}

function generatedRow(
	device: GeneratedDevice,
	registeredAt: number,
	lastSeen: number | null,
): DeviceRow {
	const { id, kind, shared } = device;
	return {
		device_id: id,
		owner_id: shared ? SAMPLE_PEOPLE.mira : SAMPLE_PEOPLE.felix,
		name: device.name,
		status: kind === "revoked" ? "revoked" : "active",
		registered_at: registeredAt,
		last_seen_at: lastSeen,
		auth_epoch: kind === "revoked" ? 2 : 1,
		identity: identityOf(id),
		relationship: shared ? "shared" : "owner",
		access_expires_at: shared ? NOW + 3 * DAY : null,
	};
}

function addLocalRecords(input: AttentionInputExt, device: GeneratedDevice) {
	const { id, kind, shared } = device;
	const role = shared ? "shared" : "owner";
	const grantId = `${shared ? "grant" : "owner"}-${id.slice(0, 8)}`;
	const keyState =
		kind === "revoked" ? "stale" : isUnlocked(kind) ? "unlocked" : "locked";
	input.keys.push(keySession(id, keyState, role, grantId));
	input.local.vaults.push(vault(id, role, grantId));
	input.local.backups[id] = { localRevision: 1, pending: false };
	input.accountBackups[id] = { revision: 1 };
}

function addReportedFacts(
	gen: Generation,
	device: GeneratedDevice,
	placements: PlacementStatusPlus[],
	times: ReadTimes,
) {
	const { input, rnd } = gen;
	const { id, kind } = device;
	input.certInventory[id] = {
		revision: 1,
		updated_at: times.certificatesAt,
		certificates: [
			{
				certificate_id: rnd.uuid(),
				revision: 1,
				fingerprint_sha256: rnd.hex(64),
				not_after: times.certificateNotAfter,
			},
		],
	};
	if (isUnlocked(kind)) {
		const bootId = rnd.hex(16);
		input.fleet[id] = fleetState(id, {
			status: {
				observations: [observation(id, placements, times.statusAt, bootId)],
				observedAt: times.statusAt,
				bootId,
				sequence: 1 + Math.floor(rnd.next() * 9_000),
			},
		});
	}
	gen.lastReads[id] = {
		version: kind === "attention" && device.variant === 2 ? "0.9.3" : "0.9.4",
		at: times.agentReadAt,
	};
}

function addDevice(gen: Generation, device: GeneratedDevice) {
	const { registeredAt, lastSeen } = registration(device.kind, gen.rnd);
	const placements = services(gen, device);
	const times = readTimes(device, lastSeen, gen.rnd);
	gen.input.devices.push(generatedRow(device, registeredAt, lastSeen));
	addLocalRecords(gen.input, device);
	if (isActive(device.kind)) addReportedFacts(gen, device, placements, times);
}

/** `n` devices in total: the 7 sample devices plus `n − 7` generated ones (43 or 200 in the spec). */
export function generateFleet(n: number, seed = 891): GeneratedFleet {
	const input = sampleFleet();
	const rnd = random(seed);
	const extra = Math.max(0, n - input.devices.length);
	const big = n > 43;
	const plan = kinds(extra, big, rnd.next);
	const lastReads = input.agentLastRead ?? {};
	input.agentLastRead = lastReads;
	const gen: Generation = {
		input,
		rnd,
		lastReads,
		apps: big ? [...BASE_APPS, ...STRESS_APPS] : BASE_APPS,
	};
	const generatedKinds = new Map<string, GeneratedKind>();

	names(extra, big).forEach((name, index) => {
		const kind = plan[index];
		const id = rnd.uuid();
		generatedKinds.set(id, kind);
		addDevice(gen, {
			id,
			name,
			index,
			kind,
			variant: index % 3,
			shared: big && kind === "healthy" && index % 17 === 3,
		});
	});

	if (input.usage) {
		const active = input.devices.filter(
			(row) => row.status === "active",
		).length;
		input.usage = {
			limits: { ...input.usage.limits, max_devices: Math.max(100, 2 * n) },
			usage: { ...input.usage.usage, active_devices: active },
		};
		input.hub = { ...input.hub, ...input.usage };
	}
	return { input, kinds: generatedKinds };
}
