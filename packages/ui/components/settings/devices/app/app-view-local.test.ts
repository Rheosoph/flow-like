import { describe, expect, test } from "bun:test";
import {
	APPS,
	CHANGES,
	HASH,
	NOW0,
	PLACEMENTS,
	SERVICES,
	sampleDevices,
	svc,
	v,
} from "../../../../lib/device-management/model/__fixtures__/apps";
import {
	type AppDeviceInput,
	buildAppView,
} from "../../../../lib/device-management/model/app-plan";
import {
	refineView,
	versionInputs,
} from "../../../../lib/device-management/model/app-versions";
import type { GateFailure } from "../../../../lib/device-management/model/types";
import type {
	ActivityItem,
	ActivityRun,
} from "../../../../lib/device-management/workspace/types";
import type { DeviceResources } from "../../../../lib/device-resources";
import {
	appRuns,
	appUploads,
	approvalsOf,
	blocksDeploy,
	capList,
	capNames,
	changesOf,
	cloudOf,
	eventsNowhere,
	headlineApp,
	openRunIds,
	runsElsewhere,
	shortMoney,
	stagedEndsAt,
	stagedVersionOf,
	updateAllGate,
	updateBatches,
	updateRows,
	uploadOf,
} from "./app-view-local";

const INVOICE = APPS.app_invoice_ai;
const SUPPORT = APPS.app_support_portal;
const ME = "usr_me";

function item(
	seed: Partial<ActivityItem> & Pick<ActivityItem, "id">,
): ActivityItem {
	return {
		kind: "upload",
		target: { deviceId: "edge-berlin-01", projectId: INVOICE.id },
		state: "done",
		label: { code: "upload" },
		startedAt: (NOW0 - 600) * 1000,
		updatedAt: (NOW0 - 500) * 1000,
		finishedAt: (NOW0 - 500) * 1000,
		startedBy: "you",
		actions: [],
		...seed,
	};
}

function invoiceView(options: { labUnlocked?: boolean } = {}) {
	const devices = sampleDevices(options);
	const services = devices.flatMap((device) =>
		Array.isArray(device.services)
			? device.services.filter((view) => view.projectId === INVOICE.id)
			: [],
	);
	const versions = versionInputs({
		label: "1.5.0",
		changedAt: NOW0 - 3600,
		events: INVOICE.events,
		services,
	});
	return refineView(
		buildAppView({
			app: { ...INVOICE, versions },
			devices,
			placements: PLACEMENTS.app_invoice_ai,
			changes: CHANGES,
		}),
	);
}

/* The version list and `refineView` are tested with the model (DM/model/app-versions.test.ts). */
describe("update gates on a refined view", () => {
	test("two revisions with the newest pins both count as newest", () => {
		const pin = (id: string, ver: string, board: string) => ({
			event_id: id,
			event_version: v(ver),
			board_version: v(board),
		});
		const a = svc({
			deviceId: "edge-berlin-01",
			serviceId: "extract",
			projectId: INVOICE.id,
			appVersion: { hash: "a".repeat(64) },
			events: [pin("evt_extract_http", "1.5.0", "2.2.0")],
		});
		const b = svc({
			deviceId: "studio-mac-mini",
			serviceId: "tools",
			projectId: INVOICE.id,
			appVersion: { hash: "b".repeat(64) },
			events: [pin("evt_invoice_mcp", "1.0.2", "2.2.0")],
		});
		const devices: AppDeviceInput[] = [
			{
				id: "edge-berlin-01",
				name: "edge-berlin-01",
				presence: { kind: "online", since: NOW0 },
				relationship: "owner",
				keyState: "unlocked",
				services: [a],
			},
			{
				id: "studio-mac-mini",
				name: "studio-mac-mini",
				presence: { kind: "online", since: NOW0 },
				relationship: "owner",
				keyState: "unlocked",
				services: [b],
			},
		];
		const versions = versionInputs({
			events: INVOICE.events,
			services: [a, b],
		});
		const view = refineView(
			buildAppView({ app: { ...INVOICE, versions }, devices }),
		);
		expect(view.versions).toHaveLength(1);
		expect(view.services.map((row) => row.behind)).toEqual([0, 0]);
		expect(view.versions[0].runningOn).toHaveLength(2);
		expect(view.versions[0].unknownOn).toEqual([]);
		expect(updateRows(view)).toEqual([]);
		expect(updateAllGate(view, false)).toBe("all_newest");
	});
});

describe("amounts", () => {
	test("cents only when there are any", () => {
		expect(shortMoney(25_000_000, "en")).toBe("€25");
		expect(shortMoney(7_410_000, "en")).toBe("€7.41");
	});
});

describe("this computer's records", () => {
	const transfer = item({
		id: "t1",
		resume: {
			type: "transfer",
			transferId: "tr",
			projectId: INVOICE.id,
			manifestSha256: HASH.invoice14,
			expiresAt: NOW0 + 3600,
		},
	});

	test("the last change per service comes from finished tray items", () => {
		const items = [
			item({
				id: "u1",
				target: {
					deviceId: "edge-berlin-01",
					serviceId: "invoice-extractor",
					projectId: INVOICE.id,
				},
				resume: transfer.resume,
			}),
			item({
				id: "s1",
				kind: "safe_update",
				finishedAt: (NOW0 - 100) * 1000,
				target: {
					deviceId: "edge-berlin-01",
					serviceId: "invoice-extractor",
					projectId: INVOICE.id,
				},
			}),
			item({
				id: "c1",
				kind: "command",
				target: {
					deviceId: "edge-berlin-01",
					serviceId: "invoice-extractor",
					projectId: INVOICE.id,
				},
			}),
			item({
				id: "f1",
				kind: "safe_update",
				state: "failed",
				target: {
					deviceId: "edge-berlin-01",
					serviceId: "invoice-extractor",
					projectId: INVOICE.id,
				},
			}),
		];
		expect(changesOf(items, INVOICE.id, ME)).toEqual({
			"edge-berlin-01/invoice-extractor": {
				at: NOW0 - 100,
				kind: "update",
				by: ME,
				seededAt: NOW0 - 500,
			},
		});
		expect(changesOf(items, SUPPORT.id, ME)).toEqual({});
	});

	test("a rolled-back update is recorded as a rollback", () => {
		const changes = changesOf(
			[
				item({
					id: "r1",
					kind: "safe_update",
					detail: { code: "rolled_back" },
					target: {
						deviceId: "edge-berlin-01",
						serviceId: "invoice-extractor",
						projectId: INVOICE.id,
					},
				}),
			],
			INVOICE.id,
			ME,
		);
		expect(changes["edge-berlin-01/invoice-extractor"].kind).toBe("rollback");
	});
});

describe("staged updates and uploads", () => {
	test("the staged version is the one this computer recorded", () => {
		const view = invoiceView({ labUnlocked: true });
		const [newest, older] = view.versions;
		expect(
			stagedVersionOf(
				{
					staged: true,
					lastChange: { at: 1, kind: "update", hash: older.hash },
				},
				view.versions,
			),
		).toBe(older);
		expect(
			stagedVersionOf({ staged: true, lastChange: null }, view.versions),
		).toBeNull();
		expect(
			stagedVersionOf(
				{
					staged: false,
					lastChange: { at: 1, kind: "update", hash: newest.hash },
				},
				view.versions,
			),
		).toBeNull();
	});

	test("a staged update is discarded a day after it was created", () => {
		const row = {
			view: svc({
				...SERVICES.supportBot,
				rollout: {
					rollout_id: "r",
					placement_id: "support-bot",
					project_id: SUPPORT.id,
					state: "staged",
					created_at: NOW0,
				},
			}),
		};
		expect(stagedEndsAt(row)).toBe(NOW0 + 86_400);
		expect(stagedEndsAt({ view: SERVICES.supportBot })).toBeUndefined();
	});

	test("an upload of the app joins the first service on its device", () => {
		const paused = item({
			id: "p1",
			state: "paused",
			progress: { done: 12, total: 38, unit: "files" },
			resume: {
				type: "transfer",
				transferId: "tr",
				projectId: INVOICE.id,
				manifestSha256: HASH.invoice15,
				expiresAt: NOW0 + 7200,
			},
		});
		const uploads = appUploads([paused, item({ id: "done" })], INVOICE.id);
		expect(uploads).toHaveLength(1);
		expect(uploads[0]).toMatchObject({
			state: "paused",
			done: 12,
			total: 38,
			expiresAt: NOW0 + 7200,
		});
		const services = [
			{ deviceId: "edge-berlin-01", serviceId: "a" },
			{ deviceId: "edge-berlin-01", serviceId: "b" },
			{ deviceId: "studio-mac-mini", serviceId: "c" },
		];
		expect(uploadOf(services[0], services, uploads)).toBe(uploads[0]);
		expect(uploadOf(services[1], services, uploads)).toBeNull();
		expect(uploadOf(services[2], services, uploads)).toBeNull();
	});
});

describe("update everywhere", () => {
	test("offers the services that don't run the newest version", () => {
		const view = invoiceView({ labUnlocked: true });
		expect(updateRows(view)).toMatchObject([
			{ serviceId: "invoice-extractor", blocked: "busy" },
			{ serviceId: "invoice-extractor-gpu", blocked: null },
		]);
		expect(updateAllGate(view, false)).toBeNull();
		expect(updateAllGate(view, true)).toBe("run_active");
	});

	test("a staged update on every service gates the whole action", () => {
		const view = invoiceView({ labUnlocked: true });
		const staged = {
			...view,
			services: view.services.map((row) => ({ ...row, staged: true })),
		};
		expect(updateRows(staged).map((row) => row.blocked)).toEqual([
			"staged",
			"staged",
		]);
		expect(updateAllGate(staged, false)).toBe("all_staged");
	});

	test("nothing deployed is its own gate", () => {
		const view = refineView(
			buildAppView({
				app: {
					...APPS.app_visitor_checkin,
					versions: versionInputs({
						events: APPS.app_visitor_checkin.events,
						services: [],
					}),
				},
				devices: sampleDevices(),
			}),
		);
		expect(updateAllGate(view, false)).toBe("nothing_deployed");
		// While no device is readable the reason is "unlock first", never "not on any device".
		expect(updateAllGate({ ...view, layout: "all_unknown" }, false)).toBe(
			"unreadable",
		);
		expect(updateAllGate({ ...view, layout: "no_devices" }, false)).toBe(
			"no_devices",
		);
	});

	test("one plan takes one service per device", () => {
		const rows = [
			{ deviceId: "a", serviceId: "1" },
			{ deviceId: "a", serviceId: "2" },
			{ deviceId: "b", serviceId: "3" },
		];
		expect(updateBatches(rows)).toEqual([[rows[0], rows[2]], [rows[1]]]);
		expect(updateBatches([])).toEqual([]);
	});
});

describe("cloud access", () => {
	const view = invoiceView({ labUnlocked: true });
	const [edge, lab] = ["invoice-extractor", "invoice-extractor-gpu"].map((id) =>
		view.services.find((row) => row.serviceId === id),
	);

	test("the hub's app list wins", () => {
		if (!edge || !lab) throw new Error("fixture rows missing");
		const cloud = cloudOf(edge, INVOICE.id, undefined, ME);
		expect(cloud.state).toBe("approved");
		if (cloud.state !== "approved") return;
		expect(cloud.approval).toMatchObject({
			serviceId: "invoice-extractor",
			active: true,
			files: "read_write",
			billing: { limit: 25_000_000, used: 7_410_000, payerIsMe: true },
		});
		expect(cloudOf(lab, INVOICE.id, undefined, ME)).toEqual({
			state: "hidden",
		});
	});

	/** What the device's own list holds: one approval of invoice-extractor with its spending limit. */
	const resources: DeviceResources = {
		grants: [
			{
				grant_id: "g1",
				device_id: "edge-berlin-01",
				placement_id: "invoice-extractor",
				deployment_id: "dep",
				project_id: INVOICE.id,
				app_id: INVOICE.id,
				delegating_user_id: ME,
				authz_version: 1,
				model_ids: ["m"],
				online_access: "read_only",
				max_instances: 2,
				expires_at: NOW0 + 100,
				status: "active",
			},
		],
		billing: [
			{
				billing_grant_id: "b1",
				grant_id: "g1",
				payer_id: ME,
				authz_version: 1,
				limit_micros: 10,
				used_micros: 4,
				reserved_micros: 1,
				expires_at: NOW0 + 100,
				status: "active",
			},
		],
		instances: [],
	};

	test("an older hub is read device by device", () => {
		if (!edge) throw new Error("fixture row missing");
		const unknown = { ...edge, cloud: { state: "unknown" as const } };
		expect(cloudOf(unknown, INVOICE.id, undefined, ME)).toEqual({
			state: "unknown",
		});
		const cloud = cloudOf(unknown, INVOICE.id, resources, ME);
		expect(cloud).toMatchObject({
			state: "approved",
			approval: {
				grantId: "g1",
				files: "read_only",
				approvedBy: ME,
				leases: 0,
				billing: { id: "b1", limit: 10, used: 4, reserved: 1, payerIsMe: true },
			},
		});
		expect(
			cloudOf(unknown, INVOICE.id, { ...resources, grants: [] }, ME),
		).toEqual({ state: "hidden" });
	});

	test("on the viewer's own device nothing listed means no cloud access, not a hidden one", () => {
		if (!edge || !lab) throw new Error("fixture rows missing");
		const unlisted = { ...edge, cloud: { state: "hidden" as const } };
		expect(cloudOf(unlisted, INVOICE.id, undefined, ME, true)).toEqual({
			state: "none",
		});
		expect(cloudOf(lab, INVOICE.id, undefined, ME, false)).toEqual({
			state: "hidden",
		});
		const unknown = { ...edge, cloud: { state: "unknown" as const } };
		const empty: DeviceResources = { grants: [], billing: [], instances: [] };
		expect(cloudOf(unknown, INVOICE.id, empty, ME, true)).toEqual({
			state: "none",
		});
		expect(
			approvalsOf(
				{ app: view.app, services: [unlisted] },
				() => undefined,
				ME,
				() => true,
			).map((entry) => entry.cloud.state),
		).toEqual(["none"]);
	});
});

describe("gates on links into the deploy flow", () => {
	const gate = (code: string): GateFailure =>
		({
			ok: false,
			gate: "G8",
			kind: "live",
			hide: false,
			copy: { code },
		}) as GateFailure;

	test("what the wizard handles itself never blocks a link", () => {
		expect(blocksDeploy(null)).toBe(false);
		expect(blocksDeploy(gate("locked_change"))).toBe(false);
		expect(blocksDeploy(gate("connect_first"))).toBe(false);
		expect(blocksDeploy(gate("offline_needs_live"))).toBe(true);
		expect(blocksDeploy(gate("never_connected_needs_live"))).toBe(true);
	});
});

describe("headline facts and lists", () => {
	test("events nowhere ignores unknown devices", () => {
		const locked = invoiceView();
		expect(eventsNowhere(locked)).toEqual([]);
		const open = invoiceView({ labUnlocked: true });
		expect(eventsNowhere(open)).toEqual(["Invoice tools (MCP)"]);
		expect(headlineApp(open)).toMatchObject({
			appId: INVOICE.id,
			localOnly: false,
			events: { total: 6, eligible: 3 },
			latestLabel: "v1.5.0",
			olderServices: 2,
		});
	});

	test("lists are capped", () => {
		expect(capList([1, 2, 3], 2)).toEqual({ shown: [1, 2], rest: [3] });
		expect(capNames(["a", "b"], 3)).toEqual({ names: ["a", "b"], more: 0 });
		expect(capNames(["a", "b", "c", "d", "e"], 3)).toEqual({
			names: ["a", "b", "c"],
			more: 2,
		});
	});

	test("who notices a stop names where else the app runs", () => {
		const view = invoiceView({ labUnlocked: true });
		expect(runsElsewhere(view, view.services[0])).toEqual([
			{ serviceId: "invoice-extractor-gpu", deviceId: "lab-gpu-02" },
		]);
	});
});

describe("runs tracked on this computer", () => {
	test("only open multi-device runs of the app are listed", () => {
		const run = (id: string, updatedAt: number): ActivityRun => ({
			id,
			title: { code: "deploy" },
			itemIds: (items[id] ?? []).map((entry) => entry.id),
			oneAtATime: true,
			stopOnFail: true,
			startedAt: 0,
			updatedAt,
		});
		const items: Record<string, ActivityItem[]> = {
			open: [
				item({ id: "a", state: "done" }),
				item({ id: "b", state: "active" }),
			],
			finished: [item({ id: "c" }), item({ id: "d" })],
			single: [item({ id: "e", state: "active" })],
			other: [
				item({
					id: "f",
					state: "active",
					target: { deviceId: "x", projectId: SUPPORT.id },
				}),
				item({
					id: "g",
					state: "active",
					target: { deviceId: "y", projectId: SUPPORT.id },
				}),
			],
		};
		const all = [
			run("open", 2),
			run("finished", 3),
			run("single", 4),
			run("other", 5),
		];
		const runs = appRuns(all, Object.values(items).flat(), INVOICE.id);
		expect(runs.map((entry) => entry.run.id)).toEqual(["open"]);
		expect(runs[0]).toMatchObject({ done: 1, total: 2, open: true });
		// What keeps the next Update everywhere waiting: every open run of the app, one device or many.
		expect([
			...openRunIds(all, Object.values(items).flat(), INVOICE.id),
		]).toEqual(["open", "single"]);
	});
});
