import { describe, expect, test } from "bun:test";
import type { ActivityItem } from "../workspace/types";
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
} from "./__fixtures__/apps";
import { type AppEventInput, buildAppView } from "./app-plan";
import {
	definitionHash,
	hasUnpublishedEdits,
	newestPins,
	refineView,
	revisionsSent,
	versionInputs,
	versionLabel,
	versionName,
} from "./app-versions";
import type { ServiceView } from "./types";

const INVOICE = APPS.app_invoice_ai;
const SUPPORT = APPS.app_support_portal;

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

describe("versions without a hub history", () => {
	test("the newest version pins every event that can run on a device", () => {
		expect(newestPins(INVOICE.events).map((pin) => pin.eventId)).toEqual([
			"evt_invoice_review",
			"evt_extract_http",
			"evt_gpu_extract",
			"evt_invoice_mcp",
			"evt_invoice_reconcile",
		]);
		expect(versionLabel("1.5.0")).toBe("v1.5.0");
		expect(versionLabel("Autumn")).toBe("Autumn");
		expect(versionLabel(" ")).toBeNull();
	});

	test("an event that follows Latest is pinned to the flow version that equals its flow", () => {
		const [review, ...pinned] = INVOICE.events;
		const withFlow = (flow: AppEventInput["flow"]) => [
			{ ...review, flow },
			...pinned,
		];
		expect(newestPins(INVOICE.events)[0]).toEqual({
			eventId: "evt_invoice_review",
			eventVersion: [0, 9, 0],
			boardVersion: [0, 9, 2],
		});
		expect(hasUnpublishedEdits(INVOICE.events)).toBe(false);
		// Unpublished edits: no version holds the flow yet, so the event has no pin and the newest row is "current edits".
		const edited = withFlow({ current: null, newest: [0, 9, 2] });
		expect(newestPins(edited).map((pin) => pin.eventId)).not.toContain(
			"evt_invoice_review",
		);
		expect(hasUnpublishedEdits(edited)).toBe(true);
		expect(versionInputs({ events: edited, services: [] })[0].unpublished).toBe(
			true,
		);
		expect(
			versionInputs({ events: INVOICE.events, services: [] })[0],
		).not.toHaveProperty("unpublished");
		// Not read yet, or a hub that can't say: unknown, never "no edits" and never "edits".
		for (const flow of [undefined, "missing_on_hub"] as const) {
			const unknown = withFlow(flow);
			expect(newestPins(unknown).map((pin) => pin.eventId)).not.toContain(
				"evt_invoice_review",
			);
			expect(hasUnpublishedEdits(unknown)).toBe(false);
			// A service that serves it is neither on the newest version nor behind it.
			const serving = {
				appVersion: { hash: "a".repeat(64) },
				events: [
					{
						event_id: "evt_invoice_review",
						event_version: [0, 9, 0] as [number, number, number],
						board_version: [0, 9, 1] as [number, number, number],
					},
				],
			};
			expect(
				versionInputs({ events: unknown, services: [serving] }).map(
					(version) => version.hash,
				),
			).not.toContain(serving.appVersion.hash);
			expect(
				versionInputs({ events: INVOICE.events, services: [serving] }).map(
					(version) => version.hash,
				),
			).toContain(serving.appVersion.hash);
		}
		// A paused Latest event with edits does not make the app's newest version "current edits".
		expect(
			hasUnpublishedEdits([
				{
					...review,
					active: false,
					flow: { current: null, newest: [0, 9, 2] },
				},
			]),
		).toBe(false);
		// An older hub can't hand schedules over: its schedule is in no version.
		expect(
			newestPins(INVOICE.events, { hubSchedules: false }).map(
				(pin) => pin.eventId,
			),
		).not.toContain("evt_invoice_reconcile");
	});

	test("a version of unpublished edits alone is still listed", () => {
		const [review] = INVOICE.events;
		const versions = versionInputs({
			label: "1.5.0",
			events: [{ ...review, flow: { current: null, newest: null } }],
			services: [],
		});
		expect(versions).toMatchObject([{ pins: [], unpublished: true }]);
		const view = buildAppView({
			app: { ...INVOICE, events: [review], versions },
			devices: [],
		});
		expect(view.versions[0].unpublished).toBe(true);
	});

	test("against an older version, an event with flow edits reads as edited, never as removed", () => {
		const [review, ...rest] = INVOICE.events;
		const pinned = [
			{ ...review, flow: { current: [0, 9, 2], newest: [0, 9, 2] } },
			...rest,
		] as typeof INVOICE.events;
		const edited = [
			{ ...review, flow: { current: null, newest: [0, 9, 2] } },
			...rest,
		] as typeof INVOICE.events;
		const older = versionInputs({ events: pinned, services: [] })[0];
		const [current] = versionInputs({ events: edited, services: [] });
		expect(current.edited).toEqual(["evt_invoice_review"]);
		const view = buildAppView({
			app: {
				...INVOICE,
				events: edited,
				versions: [current, { ...older, hash: "b".repeat(64) }],
			},
			devices: [],
		});
		const rows = view.versions[0].diff ?? [];
		expect(rows.find((row) => row.eventId === "evt_invoice_review")).toEqual({
			eventId: "evt_invoice_review",
			kind: "edits",
			from: { eventVersion: [0, 9, 0], boardVersion: [0, 9, 2] },
		});
		expect(rows.some((row) => row.kind === "removed")).toBe(false);
	});

	test("the definition hash is stable and order-independent", () => {
		const pins = newestPins(INVOICE.events);
		expect(definitionHash(pins)).toMatch(/^[a-f0-9]{64}$/);
		expect(definitionHash([...pins].reverse())).toBe(definitionHash(pins));
		expect(definitionHash(pins.slice(1))).not.toBe(definitionHash(pins));
	});

	test("older revisions come from what readable services still run", () => {
		const versions = versionInputs({
			label: "1.5.0",
			changedAt: NOW0,
			events: INVOICE.events,
			services: [SERVICES.invoiceExtractor, SERVICES.invoiceGpu],
		});
		expect(versions.map((version) => version.label)).toEqual([
			"v1.5.0",
			null,
			null,
		]);
		expect(versions[0].hash).toBe(definitionHash(newestPins(INVOICE.events)));
		expect(versions.slice(1).map((version) => version.hash)).toEqual([
			HASH.invoice14,
			HASH.invoice13,
		]);
	});

	test("this computer's upload time orders older revisions", () => {
		const versions = versionInputs({
			events: INVOICE.events,
			services: [SERVICES.invoiceExtractor, SERVICES.invoiceGpu],
			sentAt: { [HASH.invoice13]: NOW0 - 10, [HASH.invoice14]: NOW0 - 9000 },
		});
		expect(versions.slice(1).map((version) => version.hash)).toEqual([
			HASH.invoice13,
			HASH.invoice14,
		]);
		expect(versions[1].builtAt).toBe(NOW0 - 10);
	});

	test("a revision that serves only newest pins is the newest version", () => {
		const current: ServiceView = svc({
			deviceId: "edge-berlin-01",
			serviceId: "extractor",
			projectId: INVOICE.id,
			appVersion: { hash: "a".repeat(64) },
			events: [
				{
					event_id: "evt_extract_http",
					event_version: v("1.5.0"),
					board_version: v("2.2.0"),
				},
			],
		});
		const versions = versionInputs({
			events: INVOICE.events,
			services: [current, SERVICES.invoiceGpu],
		});
		expect(versions).toHaveLength(2);
		expect(versions[0].hash).toBe("a".repeat(64));
	});

	test("a service without an event list adds no version", () => {
		const versions = versionInputs({
			events: APPS.app_warehouse_scan.events,
			services: [SERVICES.scannerIngest],
		});
		expect(versions).toHaveLength(1);
	});

	test("an app with nothing to run and nothing running has no versions", () => {
		expect(versionInputs({ events: [], services: [] })).toEqual([]);
	});
});

describe("refineView", () => {
	test("drift follows the pins and unknown stays unknown", () => {
		const view = invoiceView({ labUnlocked: true });
		const rows = Object.fromEntries(
			view.services.map((row) => [row.serviceId, row.behind]),
		);
		expect(rows).toEqual({
			"invoice-extractor": 1,
			"invoice-extractor-gpu": 2,
		});
		expect(view.versions[0].runningOn).toEqual([]);
		expect(view.newestRuns).toMatchObject({ services: 0, of: 2 });
		expect(view.howRuns.newest?.label).toBe("v1.5.0");
	});

	test("a partial older revision never claims an event is new", () => {
		const view = invoiceView({ labUnlocked: true });
		expect(view.versions[0].diff?.map((row) => row.kind)).toEqual(["changed"]);
		expect(view.events.rows.every((row) => row.newIn === null)).toBe(true);
	});

	test("a view without versions is returned as it is", () => {
		const view = buildAppView({
			app: { ...INVOICE, versions: [] },
			devices: sampleDevices(),
		});
		expect(refineView(view)).toBe(view);
	});

	test("refining twice changes nothing more", () => {
		const once = invoiceView({ labUnlocked: true });
		expect(refineView(once)).toEqual(once);
	});

	test("a locked device is unknown for every version, and an older version is named by its hash", () => {
		const view = invoiceView();
		expect(view.versions[0].unknownOn).toEqual(["lab-gpu-02"]);
		expect(versionName(view.versions[0])).toBe("v1.5.0");
		expect(versionName(view.versions[1])).toBe(HASH.invoice14.slice(0, 8));
	});

	test("a device without access to the app is unknown for no version", () => {
		const versions = versionInputs({
			events: SUPPORT.events,
			services: [SERVICES.supportBot],
		});
		const view = (noAccess: string[]) =>
			refineView(
				buildAppView({
					app: { ...SUPPORT, versions },
					devices: sampleDevices({ labUnlocked: false }),
					noAccess,
				}),
			);
		expect(view([]).versions[0].unknownOn).toEqual(["lab-gpu-02"]);
		const fixed = view(["lab-gpu-02"]);
		expect(fixed.versions[0].unknownOn).toEqual([]);
		expect(fixed.groups.map((group) => group.deviceId)).toEqual([
			"edge-berlin-01",
		]);
		expect(fixed.everywhereElse.noAccess.map((row) => row.deviceId)).toEqual([
			"lab-gpu-02",
		]);
	});
});

describe("revisions this computer sent", () => {
	const transfer: ActivityItem = {
		id: "t1",
		kind: "upload",
		target: { deviceId: "edge-berlin-01", projectId: INVOICE.id },
		state: "done",
		label: { code: "upload" },
		startedAt: (NOW0 - 600) * 1000,
		updatedAt: (NOW0 - 500) * 1000,
		finishedAt: (NOW0 - 500) * 1000,
		startedBy: "you",
		actions: [],
		resume: {
			type: "transfer",
			transferId: "tr",
			projectId: INVOICE.id,
			manifestSha256: HASH.invoice14,
			expiresAt: NOW0 + 3600,
		},
	};

	test("are read from finished uploads of this app", () => {
		expect(revisionsSent([transfer], INVOICE.id)).toEqual({
			[HASH.invoice14]: NOW0 - 500,
		});
		expect(revisionsSent([transfer], SUPPORT.id)).toEqual({});
		expect(
			revisionsSent([{ ...transfer, state: "paused" }], INVOICE.id),
		).toEqual({});
	});
});
