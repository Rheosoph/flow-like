import { describe, expect, test } from "bun:test";
import {
	APPS,
	CHANGES,
	HASH,
	PLACEMENTS,
	SERVICES,
	type SampleAppId,
	sampleDevices,
} from "./__fixtures__/apps";
import {
	type AppDeviceInput,
	type AppServiceRow,
	type AppUploadInput,
	type AppView,
	appMode,
	appUnknownOf,
	buildAppView,
} from "./app-plan";
import type { GateFailure } from "./types";

function view(
	appId: SampleAppId,
	options: {
		labUnlocked?: boolean;
		devices?: AppDeviceInput[];
		focus?: string[];
		noAccess?: string[];
		uploads?: AppUploadInput[];
	} = {},
): AppView {
	return buildAppView({
		app: APPS[appId],
		devices: options.devices ?? sampleDevices(options),
		placements: PLACEMENTS[appId],
		changes: CHANGES,
		focusDeviceIds: options.focus,
		noAccess: options.noAccess,
		uploads: options.uploads,
	});
}

function pinText(row: AppServiceRow): string[] | null {
	return (
		row.events?.map(
			(event) =>
				`${event.event_id} ${event.event_version.join(".")}/${event.board_version.join(".")}`,
		) ?? null
	);
}

function cloudText(row: AppServiceRow): string {
	const cloud = row.cloud;
	if (cloud.state !== "approved") return cloud.state;
	const billing = cloud.placement.billing;
	return [
		cloud.placement.grant.online_access ?? "models",
		billing ? `${billing.used_micros}/${billing.limit_micros}` : "no limit",
	].join(" · ");
}

const PICTURE: [
	SampleAppId,
	boolean,
	string,
	string[] | null,
	string | null,
	number | null,
	string,
	string,
	string,
][] = [
	[
		"app_support_portal",
		false,
		"support-bot@edge-berlin-01",
		["evt_support_chat 2.3.0/5.1.2", "evt_support_http 1.0.4/5.1.2"],
		"v2.3.0",
		1,
		"running→converged 2/2 max 4",
		"device since 1787209200",
		"none",
	],
	[
		"app_invoice_ai",
		false,
		"invoice-extractor@edge-berlin-01",
		["evt_extract_http 1.4.0/2.1.0"],
		"v1.4.0",
		1,
		"running→update_in_progress 1/1 max 1",
		"cloud · writes 0",
		"read_write · 7410000/25000000",
	],
	[
		"app_invoice_ai",
		true,
		"invoice-extractor-gpu@lab-gpu-02",
		["evt_gpu_extract 1.0.2/1.3.0"],
		"v1.3.0",
		2,
		"running→converged 1/1 max 1",
		"cloud",
		"hidden",
	],
	[
		"app_crm_sync",
		false,
		"nightly-sync@edge-berlin-01",
		["evt_crm_nightly 1.1.0/4.0.0"],
		"v3.0.0",
		1,
		"stopped→stopped_by_user 0/0 max 1",
		"device since 1788339600",
		"none",
	],
	[
		"app_warehouse_scan",
		false,
		"scanner-ingest@warehouse-pi",
		null,
		"v1.1.0",
		1,
		"running→crash_looping 1/1 max 1",
		"device since 1783497600",
		"none",
	],
	[
		"app_field_notes",
		false,
		"field-notes@studio-mac-mini",
		["evt_notes_http 1.2.0/3.0.1"],
		"v2.0.0",
		0,
		"running→converged 1/1 max 1",
		"cloud · writes 14 quarantined",
		"read_write · no limit",
	],
];

describe("APP §5.5 resulting picture per app", () => {
	for (const [
		appId,
		labUnlocked,
		where,
		events,
		label,
		behind,
		state,
		data,
		cloud,
	] of PICTURE) {
		test(`${APPS[appId].name}: ${where}`, () => {
			const result = view(appId, { labUnlocked });
			const row = result.services.find(
				(value) => `${value.serviceId}@${value.deviceId}` === where,
			);
			if (!row) throw new Error(`${where} is missing`);
			const writes =
				typeof row.writes === "object" && row.writes
					? ` · writes ${row.writes.pending}${row.writes.quarantined ? " quarantined" : ""}`
					: "";
			expect([
				pinText(row),
				row.version?.label ?? null,
				row.behind,
				`${row.view.desired}→${row.view.conv} ${row.view.instances.ready}/${row.view.instances.requested} max ${row.view.instances.max}`,
				`${row.data.where}${row.data.since ? ` since ${row.data.since}` : ""}${writes}`,
				cloudText(row),
			]).toEqual([events, label, behind, state, data, cloud]);
		});
	}

	test("Partner Reports: the revoked device is not counted and flows are unreadable", () => {
		const result = view("app_partner_reports");
		expect(result.app.canReadFlows).toBe(false);
		expect(result.versions).toEqual([]);
		expect(result.services).toEqual([]);
		expect(result.coverage.total).toBe(5);
		expect(result.events.rows.map((row) => row.eventId)).toEqual([
			"evt_render_report",
			"evt_report_tools",
		]);
		expect(result.events.ineligible.map((row) => row.eligibility.code)).toEqual(
			["type"],
		);
	});

	test("Visitor Check-in is never deployed and 2 of its 4 events can run", () => {
		const result = view("app_visitor_checkin");
		expect(result.layout).toBe("never");
		expect(result.app.mode).toBe("online");
		expect(result.newestRuns).toMatchObject({ services: 0, of: 0 });
		expect(result.versions[0].runningOn).toEqual([]);
		expect(result.events.rows.map((row) => row.eventId)).toEqual([
			"evt_visitor_page",
			"evt_badge_printer",
		]);
		expect(
			result.events.ineligible.map((row) => [
				row.eventId,
				row.eligibility.code,
			]),
		).toEqual([
			["evt_visitor_mail", "type"],
			["evt_visitor_report", "type"],
		]);
		expect(
			result.everywhereElse.notDeployed.map((row) => row.deviceId),
		).toEqual(["edge-berlin-01", "studio-mac-mini", "warehouse-pi"]);
		expect(
			result.everywhereElse.unknown.map((row) => [
				row.deviceId,
				row.unknown?.kind,
			]),
		).toEqual([["lab-gpu-02", "locked"]]);
		expect(
			result.everywhereElse.never.map((row) => [
				row.deviceId,
				row.unknown?.kind,
			]),
		).toEqual([["cold-storage-nas", "never"]]);
		expect(result.coverage).toEqual({
			total: 5,
			readable: 3,
			unknown: 2,
			locked: ["lab-gpu-02"],
			never: ["cold-storage-nas"],
		});
	});
});

describe("last known", () => {
	test("rows that are delayed or kept after a failed read are last known, like their attention items", () => {
		const lastKnown = (age: "live" | "current" | "delayed" | "error") => {
			const devices = sampleDevices().map((device) =>
				Array.isArray(device.services)
					? {
							...device,
							services: device.services.map((service) => ({
								...service,
								freshness: { ...service.freshness, age },
							})),
						}
					: device,
			);
			return view("app_support_portal", { devices }).services[0].lastKnown;
		};
		expect(lastKnown("live")).toBe(false);
		expect(lastKnown("current")).toBe(false);
		expect(lastKnown("delayed")).toBe(true);
		expect(lastKnown("error")).toBe(true);
	});
});

describe("unknown never means not deployed", () => {
	test("a snapshot without events gives null events with the snapshot reason", () => {
		const result = view("app_warehouse_scan");
		const [row] = result.services;
		expect([row.events, row.eventsWhy, row.lastKnown, row.rank]).toEqual([
			null,
			"snapshot",
			true,
			0,
		]);
		const ingest = result.events.rows.find(
			(value) => value.eventId === "evt_scan_ingest",
		);
		expect(ingest?.cells["warehouse-pi"]).toMatchObject({
			state: "unknown",
			unknown: { kind: "snapshot" },
		});
	});

	test("a locked device is unknown; one that never checked in is neither unknown nor Not deployed", () => {
		const result = view("app_invoice_ai");
		expect(result.groups.map((group) => [group.deviceId, group.rank])).toEqual([
			["edge-berlin-01", 2],
			["lab-gpu-02", 3],
		]);
		expect(
			result.everywhereElse.notDeployed.map((row) => row.deviceId),
		).toEqual(["studio-mac-mini", "warehouse-pi"]);
		expect(result.everywhereElse.never.map((row) => row.deviceId)).toEqual([
			"cold-storage-nas",
		]);
		expect(result.versions[0].unknownOn).toEqual(["lab-gpu-02"]);
	});

	test("every device locked has no service and nothing readable", () => {
		const devices = sampleDevices().map(
			(device): AppDeviceInput => ({
				...device,
				keyState: "locked",
				services: { state: "locked" },
			}),
		);
		const result = view("app_invoice_ai", { devices });
		expect(result.layout).toBe("all_unknown");
		expect(result.coverage).toEqual({
			total: 5,
			readable: 0,
			unknown: 5,
			locked: [
				"edge-berlin-01",
				"lab-gpu-02",
				"studio-mac-mini",
				"warehouse-pi",
			],
			never: ["cold-storage-nas"],
		});
	});

	test("devices that never checked in or are shared for another app don't ask for an unlock", () => {
		const [edge, lab, , , cold] = sampleDevices();
		const noAccess: AppDeviceInput = {
			...lab,
			services: { state: "noaccess" },
		};
		for (const devices of [[cold], [noAccess], [cold, noAccess]])
			expect(view("app_visitor_checkin", { devices }).layout).toBe("never");
		const locked: AppDeviceInput = { ...edge, services: { state: "locked" } };
		expect(
			view("app_visitor_checkin", { devices: [cold, locked] }).layout,
		).toBe("all_unknown");
	});

	test("a device that never checked in is a column only when asked for, as Not served with its gate", () => {
		const gate: GateFailure = {
			ok: false,
			gate: "G8",
			kind: "live",
			hide: false,
			copy: { code: "never_connected_needs_live" },
		};
		const devices = sampleDevices();
		devices[4] = { ...devices[4], deployGate: gate };
		const result = view("app_invoice_ai", {
			devices,
			focus: ["cold-storage-nas"],
		});
		expect(result.events.cols).toEqual([
			"edge-berlin-01",
			"lab-gpu-02",
			"cold-storage-nas",
		]);
		expect(result.events.rows[0].cells["cold-storage-nas"]).toEqual({
			deviceId: "cold-storage-nas",
			serviceIds: [],
			state: "not_served",
			gate,
		});
	});

	test("no devices", () => {
		expect(view("app_invoice_ai", { devices: [] }).layout).toBe("no_devices");
	});

	test("the unknown kind follows presence, keys and the plane's state", () => {
		const base = sampleDevices()[0];
		const kinds = (
			[
				[{ presence: { kind: "never" } }, "never"],
				[
					{
						services: { state: "locked", reason: { code: "no_keys_here" } },
					},
					"nokeys",
				],
				[{ keyState: "none", services: { state: "notloaded" } }, "nokeys"],
				[
					{ keyState: "held_elsewhere", services: { state: "notloaded" } },
					"locked",
				],
				[{ services: { state: "noaccess" } }, "noaccess"],
				[
					{
						relationship: "cloud_approval",
						keyState: "none",
						services: {
							state: "notloaded",
							reason: { code: "no_keys_here" },
						},
					},
					"noaccess",
				],
				[
					{
						presence: { kind: "offline", since: 5 },
						services: { state: "notloaded" },
					},
					"offline",
				],
				[{ services: { state: "error" } }, "error"],
				[{ services: { state: "notloaded" } }, "notloaded"],
			] as [Partial<AppDeviceInput>, string][]
		).map(([change, kind]) => [
			appUnknownOf({ ...base, ...change }).kind,
			kind,
		]);
		for (const [actual, expected] of kinds) expect(actual).toBe(expected);
	});
});

describe("By event matrix (APP §2.10)", () => {
	test("Invoice AI with lab-gpu-02 locked, then unlocked", () => {
		const locked = view("app_invoice_ai");
		const cell = (result: AppView, event: string, device: string) =>
			result.events.rows.find((row) => row.eventId === event)?.cells[device];
		expect(locked.events.cols).toEqual(["edge-berlin-01", "lab-gpu-02"]);
		expect(cell(locked, "evt_extract_http", "edge-berlin-01")).toMatchObject({
			state: "served",
			serviceIds: ["invoice-extractor"],
			pin: { eventVersion: [1, 4, 0], boardVersion: [2, 1, 0] },
			behind: true,
		});
		expect(cell(locked, "evt_extract_http", "lab-gpu-02")).toMatchObject({
			state: "unknown",
			unknown: { kind: "locked" },
		});
		expect(cell(locked, "evt_gpu_extract", "edge-berlin-01")?.state).toBe(
			"not_served",
		);
		const mcp = locked.events.rows.find(
			(row) => row.eventId === "evt_invoice_mcp",
		);
		expect(mcp?.newIn).toBe("v1.5.0");
		expect(
			locked.events.ineligible.map((row) => [
				row.eventId,
				row.eligibility.code,
			]),
		).toEqual([
			["evt_invoice_review", "latest_flow"],
			["evt_invoice_inbox", "type"],
			["evt_invoice_reconcile", "type"],
		]);
		const unlocked = view("app_invoice_ai", { labUnlocked: true });
		expect(cell(unlocked, "evt_gpu_extract", "lab-gpu-02")).toMatchObject({
			state: "served",
			serviceIds: ["invoice-extractor-gpu"],
			behind: true,
		});
		expect(cell(unlocked, "evt_extract_http", "lab-gpu-02")?.state).toBe(
			"not_served",
		);
		expect(unlocked.versions[2].runningOn.map((row) => row.serviceId)).toEqual([
			"invoice-extractor-gpu",
		]);
	});

	test("CRM Sync: the device's refusal is Can't run here; focus adds a column", () => {
		const result = view("app_crm_sync", { focus: ["studio-mac-mini"] });
		const watch = result.events.rows.find(
			(row) => row.eventId === "evt_crm_watch",
		);
		expect(watch?.newIn).toBe("v3.1.0");
		expect(watch?.cells["edge-berlin-01"]).toMatchObject({
			state: "cant_here",
			reason: expect.stringContaining("requires sandboxed services"),
		});
		expect(watch?.cells["studio-mac-mini"]?.state).toBe("not_served");
		expect(
			result.events.rows.find((row) => row.eventId === "evt_crm_nightly")
				?.cells["edge-berlin-01"],
		).toMatchObject({ state: "served", conv: "stopped_by_user" });
		expect(
			result.events.ineligible.map((row) => [
				row.eventId,
				row.eligibility.code,
			]),
		).toEqual([
			["evt_crm_hourly", "type"],
			["evt_crm_rest", "canary"],
		]);
	});

	test("Support Portal serves both events on edge; the paused mailbox can't run", () => {
		const result = view("app_support_portal");
		expect(
			result.events.rows.map((row) => [
				row.eventId,
				row.cells["edge-berlin-01"]?.state,
			]),
		).toEqual([
			["evt_support_chat", "served"],
			["evt_support_http", "served"],
		]);
		expect(
			result.events.ineligible.map((row) => [
				row.eventId,
				row.eligibility.code,
			]),
		).toEqual([
			["evt_support_reply", "type"],
			["evt_support_digest", "type"],
			["evt_support_mailbox", "paused"],
		]);
	});

	test("a conflict at the head of a write queue ranks the service as 'writes need you'", () => {
		const devices = sampleDevices();
		devices[2] = {
			...devices[2],
			services: [
				{
					...SERVICES.fieldNotes,
					offlineWrites: {
						pending: 2,
						quarantined: false,
						head: {
							queued_operation_id: "op-1",
							sequence: 4,
							operation_kind: "table_upsert",
							state: "conflict",
							created_at: 1,
							attempts: 1,
						} as never,
					},
				},
			],
		};
		expect(view("app_field_notes", { devices }).services[0].rank).toBe(1);
	});

	test("a staged rollout marks the cell and ranks the service as updating", () => {
		const devices = sampleDevices();
		devices[0] = {
			...devices[0],
			services: [
				{
					...SERVICES.supportBot,
					rollout: {
						rollout_id: "r1",
						placement_id: "support-bot",
						project_id: "app_support_portal",
						state: "staged",
					},
				},
			],
		};
		const result = view("app_support_portal", { devices });
		expect([result.services[0].staged, result.services[0].rank]).toEqual([
			true,
			2,
		]);
		expect(result.events.rows[0].cells["edge-berlin-01"]?.state).toBe("staged");
	});

	test("shared-for-another-app devices get a no-access column; gates reach Deploy here", () => {
		const gate: GateFailure = {
			ok: false,
			gate: "G6",
			kind: "live",
			hide: false,
			copy: { code: "device_offline" as never },
		};
		const devices = sampleDevices();
		devices[1] = { ...devices[1], services: { state: "noaccess" } };
		devices[2] = { ...devices[2], deployGate: gate };
		const result = view("app_visitor_checkin", {
			devices,
			focus: ["studio-mac-mini"],
		});
		expect(result.events.cols).toEqual(["lab-gpu-02", "studio-mac-mini"]);
		expect(result.events.rows[0].cells["lab-gpu-02"]?.state).toBe("no_access");
		expect(result.events.rows[0].cells["studio-mac-mini"]).toMatchObject({
			state: "not_served",
			gate,
		});
		expect(result.everywhereElse.noAccess.map((row) => row.deviceId)).toEqual([
			"lab-gpu-02",
		]);
		expect(
			result.everywhereElse.notDeployed.find(
				(row) => row.deviceId === "studio-mac-mini",
			),
		).toMatchObject({
			gate,
			runs: [{ serviceId: "field-notes", projectId: "app_field_notes" }],
		});
	});

	test("more than four device columns switch to list mode", () => {
		const devices = sampleDevices().map(
			(device): AppDeviceInput => ({
				...device,
				presence:
					device.presence.kind === "revoked"
						? { kind: "online" }
						: device.presence,
				services: { state: "locked" },
			}),
		);
		expect(view("app_invoice_ai", { devices }).events.listMode).toBe(true);
		expect(view("app_invoice_ai").events.listMode).toBe(false);
	});
});

describe("versions and drift (APP §2.11, A7)", () => {
	test("each version lists the event changes against the next older one", () => {
		const result = view("app_invoice_ai");
		expect(result.versions.map((row) => [row.label, row.short])).toEqual([
			["v1.5.0", "7c2d1e90"],
			["v1.4.0", "491e8acf"],
			["v1.3.0", "684d0cd5"],
		]);
		expect(
			result.versions[0].diff?.map((row) => [
				row.eventId,
				row.kind,
				row.from?.eventVersion.join("."),
				row.to?.eventVersion.join("."),
			]),
		).toEqual([
			["evt_extract_http", "changed", "1.4.0", "1.5.0"],
			["evt_gpu_extract", "changed", "1.0.2", "1.1.0"],
			["evt_invoice_mcp", "added", undefined, "1.0.2"],
		]);
		expect(result.versions[1].diff).toEqual([
			{
				eventId: "evt_extract_http",
				kind: "changed",
				from: { eventVersion: [1, 3, 0], boardVersion: [2, 0, 0] },
				to: { eventVersion: [1, 4, 0], boardVersion: [2, 1, 0] },
			},
		]);
		expect(result.versions[2].diff).toBeNull();
		expect(result.versions[1].runningOn).toEqual([
			{
				deviceId: "edge-berlin-01",
				serviceId: "invoice-extractor",
				conv: "update_in_progress",
				lastKnown: false,
			},
		]);
		expect(result.newestRuns).toMatchObject({ services: 0, of: 1 });
		expect(result.howRuns).toMatchObject({
			visibility: "Prototype",
			mode: "online",
			newest: { label: "v1.5.0" },
		});
	});

	test("a removed event shows in the newer version's diff", () => {
		const app = {
			...APPS.app_crm_sync,
			versions: [
				{ ...APPS.app_crm_sync.versions[1], hash: "f".repeat(64) },
				APPS.app_crm_sync.versions[0],
			],
		};
		const result = buildAppView({ app, devices: [] });
		expect(
			result.versions[0].diff?.filter((row) => row.kind === "removed"),
		).toEqual([
			{
				eventId: "evt_crm_watch",
				kind: "removed",
				from: { eventVersion: [0, 3, 0], boardVersion: [4, 1, 0] },
			},
		]);
	});

	test("without a hash, pins identify a version only when exactly one matches", () => {
		const devices = sampleDevices({ labUnlocked: true });
		devices[1] = {
			...devices[1],
			services: [{ ...SERVICES.invoiceGpu, appVersion: null }],
		};
		devices[0] = {
			...devices[0],
			services: [{ ...SERVICES.invoiceExtractor, appVersion: null }],
		};
		const result = view("app_invoice_ai", { devices });
		const byService = Object.fromEntries(
			result.services.map((row) => [row.serviceId, row.behind]),
		);
		expect(byService).toEqual({
			"invoice-extractor": 1,
			"invoice-extractor-gpu": null,
		});
		expect(result.versions[0].unknownOn).toContain(
			"lab-gpu-02/invoice-extractor-gpu",
		);
	});

	test("short hashes of eight characters or more match the full hash", () => {
		const devices = sampleDevices();
		devices[2] = {
			...devices[2],
			services: [
				{
					...SERVICES.fieldNotes,
					appVersion: { hash: HASH.notes19.slice(0, 8).toUpperCase() },
				},
			],
		};
		expect(view("app_field_notes", { devices }).services[0].behind).toBe(1);
	});

	test("cloud access is unknown until E20 loads", () => {
		const result = buildAppView({
			app: APPS.app_invoice_ai,
			devices: sampleDevices(),
		});
		expect(result.services[0].cloud).toEqual({ state: "unknown" });
		expect(result.services[0].lastChange).toBeNull();
	});
});

describe("cloud access on the viewer's own device (APP §2.13)", () => {
	const unapproved = (labUnlocked: boolean) =>
		buildAppView({
			app: APPS.app_invoice_ai,
			devices: sampleDevices({ labUnlocked }),
			placements: { server_time: 0, placements: [] },
		}).services.map((row) => [row.deviceId, row.cloud.state]);

	test("nothing listed means none on an owned device and hidden on a shared one", () => {
		expect(unapproved(true)).toEqual([
			["edge-berlin-01", "none"],
			["lab-gpu-02", "hidden"],
		]);
	});
});

describe("staged version and uploads (APP §2.8, §2.9)", () => {
	const staged = (changes: typeof CHANGES | undefined) => {
		const devices = sampleDevices();
		devices[0] = {
			...devices[0],
			services: [
				{
					...SERVICES.supportBot,
					rollout: {
						rollout_id: "r1",
						placement_id: "support-bot",
						project_id: "app_support_portal",
						state: "staged",
					},
				},
			],
		};
		return buildAppView({ app: APPS.app_support_portal, devices, changes })
			.services[0];
	};

	test("the staged version is the one this computer recorded", () => {
		expect(staged(CHANGES).stagedVersion?.label).toBe("v2.3.0");
		expect(staged(undefined).stagedVersion).toBeNull();
		expect(
			staged({
				"edge-berlin-01/support-bot": { at: 1, kind: "settings" },
			}).stagedVersion,
		).toBeNull();
		expect(view("app_support_portal").services[0]).not.toHaveProperty(
			"stagedVersion",
		);
	});

	test("an upload joins its own service, else the first service of the app on that device", () => {
		const upload = (
			id: string,
			deviceId: string,
			serviceId?: string,
		): AppUploadInput => ({
			id,
			deviceId,
			...(serviceId ? { serviceId } : {}),
			state: "paused",
			done: 5,
			total: 10,
			expiresAt: 9,
		});
		const devices = sampleDevices({ labUnlocked: true });
		devices[0] = {
			...devices[0],
			services: [
				{ ...SERVICES.invoiceExtractor, serviceId: "invoice-extractor-b" },
				SERVICES.invoiceExtractor,
			],
		};
		const uploads = [
			upload("device-wide", "edge-berlin-01"),
			upload("own", "lab-gpu-02", "invoice-extractor-gpu"),
			upload("gone", "lab-gpu-02", "removed-service"),
			upload("elsewhere", "studio-mac-mini"),
		];
		const result = view("app_invoice_ai", { devices, uploads });
		expect(
			result.services.map((row) => [row.serviceId, row.upload?.id ?? null]),
		).toEqual([
			["invoice-extractor", "device-wide"],
			["invoice-extractor-b", null],
			["invoice-extractor-gpu", "own"],
		]);
		expect(view("app_invoice_ai").services[0]).not.toHaveProperty("upload");
	});
});

describe("access that covers other apps only (APP §2.12)", () => {
	const noAccess = ["lab-gpu-02"];

	test("a locked device shared for another app is no access, not unknown", () => {
		expect(
			view("app_support_portal").everywhereElse.unknown.map(
				(row) => row.deviceId,
			),
		).toEqual(["lab-gpu-02"]);
		const result = view("app_support_portal", { noAccess });
		expect(result.everywhereElse.unknown).toEqual([]);
		expect(result.everywhereElse.noAccess).toMatchObject([
			{ deviceId: "lab-gpu-02", gate: null, unknown: { kind: "noaccess" } },
		]);
		expect(result.groups.map((group) => group.deviceId)).toEqual([
			"edge-berlin-01",
		]);
		expect(result.versions[0].unknownOn).toEqual([]);
		expect(result.coverage.locked).toEqual([]);
		expect(result.events.cols).toContain("lab-gpu-02");
		expect(
			result.events.rows.every(
				(row) => row.cells["lab-gpu-02"]?.state === "no_access",
			),
		).toBe(true);
	});

	test("unlocked, it is never counted as not deployed", () => {
		const open = { labUnlocked: true };
		expect(
			view("app_support_portal", open).everywhereElse.notDeployed.map(
				(row) => row.deviceId,
			),
		).toContain("lab-gpu-02");
		const result = view("app_support_portal", { ...open, noAccess });
		expect(
			result.everywhereElse.notDeployed.map((row) => row.deviceId),
		).not.toContain("lab-gpu-02");
		expect(result.everywhereElse.noAccess).toMatchObject([
			{
				deviceId: "lab-gpu-02",
				gate: null,
				unknown: { kind: "noaccess" },
				runs: [{ serviceId: "invoice-extractor-gpu" }],
			},
		]);
		expect(result.coverage.readable).toBe(
			view("app_support_portal", open).coverage.readable,
		);
	});

	test("a device that runs the app, or never checked in, stays where it is", () => {
		const result = view("app_invoice_ai", {
			labUnlocked: true,
			noAccess: ["lab-gpu-02", "cold-storage-nas"],
		});
		expect(result.groups.map((group) => group.deviceId)).toContain(
			"lab-gpu-02",
		);
		expect(result.everywhereElse.never.map((row) => row.deviceId)).toEqual([
			"cold-storage-nas",
		]);
		expect(result.everywhereElse.noAccess).toEqual([]);
	});

	test("all-unknown becomes never-deployed when nothing can be unlocked", () => {
		const devices = sampleDevices().filter(
			(device) => device.id === "lab-gpu-02",
		);
		expect(view("app_visitor_checkin", { devices }).layout).toBe("all_unknown");
		expect(view("app_visitor_checkin", { devices, noAccess }).layout).toBe(
			"never",
		);
	});
});

describe("version foot and update size", () => {
	test("services whose version can't be told are counted apart", () => {
		const devices = sampleDevices({ labUnlocked: true });
		devices[1] = {
			...devices[1],
			services: [{ ...SERVICES.invoiceGpu, appVersion: null }],
		};
		expect(view("app_invoice_ai", { devices }).newestRuns).toMatchObject({
			services: 0,
			of: 2,
			unknown: 1,
		});
		expect(view("app_invoice_ai").newestRuns?.unknown).toBe(0);
	});

	test("a version carries what an update sends when the caller knows it", () => {
		const [newest, older] = APPS.app_crm_sync.versions;
		const sends = { bytes: 91_226_112, files: 38 };
		const result = buildAppView({
			app: { ...APPS.app_crm_sync, versions: [{ ...newest, sends }, older] },
			devices: [],
		});
		expect(result.versions[0].sends).toEqual(sends);
		expect(result.versions[1]).not.toHaveProperty("sends");
	});
});

test("the mode follows visibility (A1)", () => {
	expect(
		(
			[
				"Offline",
				"Private",
				"Prototype",
				"PublicRequestAccess",
				"Public",
			] as const
		).map(appMode),
	).toEqual(["offline", "online", "online", "online", "online"]);
});
