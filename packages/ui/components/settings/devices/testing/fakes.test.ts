import {
	afterAll,
	afterEach,
	describe,
	expect,
	setDefaultTimeout,
	test,
} from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { act, createElement } from "react";
import type { HubResult } from "../../../../lib/device-management/hub/endpoints";
import type {
	AppEventInput,
	AppInput,
} from "../../../../lib/device-management/model/app-plan";
import type {
	AgentFeature,
	AgentFeatures,
	AttentionItem,
	PlacementStatusPlus,
	ServiceAction,
	ServiceBot,
	ServiceSchedule,
} from "../../../../lib/device-management/model/types";
import type { ManagementPolicy } from "../../../../lib/device-management/types";
import type { IBackendState } from "../../../../state/backend-state";
import type { DeviceWorkspaceProviderProps } from "../workspace/device-workspace-provider";
import { byRole, click, installDom } from "./dom-harness";
import type { FakeDeviceApi, HubMethod } from "./fake-device-api";
import type { FakeWorkspace } from "./fake-workspace";

// Everything but types loads after the DOM exists, like the components under test.
const dom = installDom();
const { ApiResponseError } = await import("../../../../lib/api-error");
const {
	lookupOfflineOperation,
	pruneArtifactRevisions,
	readArtifactUsage,
	readHostOperation,
	readMetricsHistory,
	readEventForm,
	readOfflineOperations,
	readOperations,
	readRolloutHistory,
} = await import("../../../../lib/device-management/agent-reads");
const { cancelRun, readRun, runEvent } = await import(
	"../../../../lib/device-management/event-run"
);
const { readAcmeCertificates } = await import(
	"../../../../lib/device-management/certificate-acme"
);
const { readCertificateIssuers, readCertificateRequests } = await import(
	"../../../../lib/device-management/certificate-issuance"
);
const { readCertificates } = await import(
	"../../../../lib/device-management/certificates"
);
const { readDeploymentRollout, readExistingDeployment } = await import(
	"../../../../lib/device-management/deployment"
);
const {
	getArchiveUsage,
	getBillingEligibility,
	getBillingUsage,
	getDeviceUsage,
	getFleetCertificateInventory,
	getMyAccess,
	getResourceSummary,
	giveBackSchedule,
	listAccountBackups,
	listAppDevicePlacements,
	listCertificateNoticeMutes,
	listCertificateNotices,
	listEnrollments,
	muteCertificateNotices,
	publishFlowVersion,
	readCertificateInventory,
	readFlowVersion,
	readHubStandalone,
	readPolicyView,
	releaseSchedule,
	sendTestCertificateNotice,
	toHubError,
	unmuteCertificateNotices,
} = await import("../../../../lib/device-management/hub/endpoints");
const { APPS, SHOP_ONCE_AT, VISITOR_CATALOG } = await import(
	"../../../../lib/device-management/model/__fixtures__/apps"
);
const { SAMPLE_APPS, SAMPLE_IDS, SAMPLE_ME, SAMPLE_NOW, sampleFleet } =
	await import(
		"../../../../lib/device-management/model/__fixtures__/sample-fleet"
	);
const { readOfflineQueues } = await import(
	"../../../../lib/device-management/offline-queue"
);
const { prepareOnlineMetadata } = await import(
	"../../../../lib/device-management/online-metadata"
);
const { checkDeviceSetup } = await import(
	"../../../../lib/device-management/readiness"
);
const { readArchiveRoster } = await import(
	"../../../../lib/device-management/telemetry"
);
const { ManagementUnconfirmedError } = await import(
	"../../../../lib/device-management/transport"
);
const { managementRejection } = await import(
	"../../../../lib/device-management/types"
);
const { KeySessionError, OwnerPasswordRequiredError } = await import(
	"../../../../lib/device-management/workspace/keys"
);
const { loadDeviceResources } = await import(
	"../../../../lib/device-resources"
);
const { getDevice, listDevices, renameDevice, revokeDevice } = await import(
	"../../../../lib/devices"
);
const { boundedLists } = await import("./fake-events");
const { QUICK_REPLY, SHOP, serveQuickReplyOnEdge, serveShopOnEdge } =
	await import("./schedule-scenarios");
const { formAnswer, formField } = await import("./fake-runs");
const { FAKE_AGENT_COMMANDS, fakeDeviceApi, fakeHubRoutes, fakeKeys } =
	await import("./fake-device-api");
const { createFakeWorkspace } = await import("./fake-workspace");
const { cleanupDevices, mountDevices, preloadDevices } = await import(
	"./mount-devices"
);
await preloadDevices();
// Many lanes share this machine; a whole fleet unlock per test needs headroom.
setDefaultTimeout(30_000);

type KeyFailure = InstanceType<typeof KeySessionError>;

let fake: FakeWorkspace | undefined;
afterEach(async () => {
	await cleanupDevices();
	await fake?.dispose();
	fake = undefined;
	await dom.cleanup();
});
afterAll(dom.restore);

const { edge, warehouse, studio, lab, cold, oldKiosk } = SAMPLE_IDS;
const EDGE_GRANT = "d99ba88b-717b-445e-a719-a2084df3aec0";
const EDGE_BILLING = "b936e936-d443-4252-a8e6-9420d361c037";
const INVOICE_ROLLOUT = "a0e5259e-a9bf-4eef-99df-f9666dffbab4";

/** The routes plan §3.2 adds (E1, E2, E4 and E15 change existing ones), and those of "run more on devices". */
const ADDED_ROUTES = [
	"DELETE apps/:app/device-schedules/:event",
	"DELETE devices/:id/certificate-notices/mute",
	"GET apps/:app/board/:board/version/current",
	"GET apps/:app/device-placements",
	"GET devices/:id/billing-grants/:billing/usage",
	"GET devices/:id/certificate-notices",
	"GET devices/:id/certificate-notices/mute",
	"GET devices/:id/management/my-access",
	"GET devices/:id/resource-grants/:grant/billing/eligibility",
	"GET devices/archive-usage",
	"GET devices/certificate-inventory",
	"GET devices/controller-vaults",
	"GET devices/enrollments",
	"GET devices/resource-summary",
	"GET devices/usage",
	"PATCH devices/:id",
	"POST apps/:app/board/:board/version/current",
	"POST devices/:id/certificate-notices/test",
	"PUT apps/:app/device-schedules/:event",
	"PUT devices/:id/certificate-notices/mute",
];

const SAMPLE_PARAMS: Record<string, string> = {
	id: edge,
	key: fakeKeys.controller(SAMPLE_ME, edge).x,
	grant: EDGE_GRANT,
	billing: EDGE_BILLING,
	enrollment: "c382dd52-1611-4477-b983-4e1e1417b669",
	archive: "none",
	instance: "edd47b6d-a53a-4eae-9ade-ccab311c47e8",
	app: SAMPLE_APPS.invoiceAi,
};

const pathOf = (template: string) =>
	template.replace(/:(\w+)/g, (_match, name: string) =>
		encodeURIComponent(SAMPLE_PARAMS[name] ?? name),
	);

/** `ok`, or how the hub refused: a coded error is an answer, an uncoded 404/405 is a missing route. */
async function outcome(run: () => Promise<unknown>): Promise<string> {
	try {
		await run();
		return "ok";
	} catch (error) {
		if (!(error instanceof ApiResponseError)) throw error;
		return `${error.code ? "coded" : "uncoded"} ${error.status}`;
	}
}

const send = (api: FakeDeviceApi, method: HubMethod, template: string) =>
	api.fetch(api.profile, pathOf(template), {
		method,
		...(method === "GET" ? {} : { body: "{}" }),
	});

const data = <T>(result: HubResult<T>): T => {
	if (result.kind !== "ok") throw new Error("missing on hub");
	return result.data;
};

function seeded<T>(value: T | undefined): T {
	if (value === undefined) throw new Error("The sample fleet lacks this part.");
	return value;
}

describe("fake hub", () => {
	test("every device route answers, the routes of plan §3.2 included", async () => {
		const routes = fakeHubRoutes();
		expect(
			routes
				.filter((route) => route.added)
				.map((route) => `${route.method} ${route.template}`)
				.sort(),
		).toEqual(ADDED_ROUTES);
		for (const { method, template } of routes) {
			const answer = await outcome(() =>
				send(fakeDeviceApi(), method, template),
			);
			expect(`${method} ${template}: ${answer}`).not.toMatch(/uncoded 40[45]$/);
		}
	});

	test("the registry clients read the golden fleet", async () => {
		const api = fakeDeviceApi();
		const { profile, seed } = api;
		const rows = await listDevices(api, profile);
		expect(rows.map((row) => row.name)).toEqual([
			"cold-storage-nas",
			"studio-mac-mini",
			"warehouse-pi",
			"edge-berlin-01",
			"old-kiosk",
			"lab-gpu-02",
			"partner-edge",
		]);
		expect((await getDevice(api, profile, lab)).relationship).toBe("shared");
		expect((await checkDeviceSetup(api, profile)).ready).toBe(true);
		expect(
			data(await listEnrollments(api, profile)).map((row) => row.state),
		).toEqual(["pending", "expired"]);
		expect(data(await getDeviceUsage(api, profile)).usage).toEqual(
			seeded(seed.usage).usage,
		);
		expect(data(await getMyAccess(api, profile, lab)).role).toBe("grantee");
		expect(data(await getMyAccess(api, profile, edge)).role).toBe("owner");
		expect(data(await listAccountBackups(api, profile)).used).toBe(4);
		const standalone = await readHubStandalone(api.hub.origin, api.hubFetch);
		expect(standalone.enabled).toBe(true);
		expect(standalone.max_devices_per_user).toBe(100);
	});

	test("the certificate, history and cloud clients read the golden fleet", async () => {
		const api = fakeDeviceApi();
		const { profile, seed } = api;
		expect(
			data(await getFleetCertificateInventory(api, profile)).map(
				(row) => row.device_id,
			),
		).toEqual([edge, warehouse, studio, cold]);
		expect(data(await listCertificateNotices(api, profile, edge))).toEqual([]);
		expect(data(await getArchiveUsage(api, profile))).toEqual(
			seeded(seed.archiveUsage),
		);
		expect(data(await getResourceSummary(api, profile))).toEqual(
			seeded(seed.resourceSummary),
		);
		expect(
			data(await getBillingEligibility(api, profile, edge, EDGE_GRANT))
				.eligible,
		).toBe(true);
		expect(
			data(await getBillingUsage(api, profile, edge, EDGE_BILLING)).totals
				.used_micros,
		).toBe(7_412_300);
		expect(
			data(
				await listAppDevicePlacements(api, profile, SAMPLE_APPS.invoiceAi),
			).placements.map((row) => [row.device_id, row.placement_id]),
		).toEqual([[edge, "invoice-extractor"]]);
		expect((await readPolicyView(api, profile, edge)).version).toBe(5);
		expect((await readCertificateInventory(api, profile, edge)).revision).toBe(
			17,
		);
		expect((await loadDeviceResources(api, profile, edge)).grants).toHaveLength(
			1,
		);
	});

	test("an old hub answers 404 or 405 on every route of §3.2 and sends plain rows", async () => {
		const api = fakeDeviceApi({ hubVersion: "old" });
		const { profile } = api;
		const missing = [
			await listEnrollments(api, profile),
			await getDeviceUsage(api, profile),
			await getMyAccess(api, profile, edge),
			await listAccountBackups(api, profile),
			await getFleetCertificateInventory(api, profile),
			await listCertificateNotices(api, profile, edge),
			await listCertificateNoticeMutes(api, profile, edge),
			await muteCertificateNotices(api, profile, edge, null, null),
			await unmuteCertificateNotices(api, profile, edge, null),
			await sendTestCertificateNotice(api, profile, edge, "both"),
			await getArchiveUsage(api, profile),
			await getResourceSummary(api, profile),
			await getBillingEligibility(api, profile, edge, EDGE_GRANT),
			await getBillingUsage(api, profile, edge, EDGE_BILLING),
			await listAppDevicePlacements(api, profile, SAMPLE_APPS.invoiceAi),
		];
		expect(missing.map((result) => result.kind)).toEqual(
			missing.map(() => "missing_on_hub"),
		);
		for (const route of fakeHubRoutes().filter((entry) => entry.added)) {
			const answer = await outcome(() =>
				send(api, route.method, route.template),
			);
			expect(["coded 404", "uncoded 404", "uncoded 405"]).toContain(answer);
		}
		await expect(
			renameDevice(api, profile, edge, "Edge"),
		).rejects.toMatchObject({ status: 405 });
		const row = await getDevice(api, profile, edge);
		expect(row.relationship).toBeUndefined();
		expect(row.identity.management_key).toHaveLength(32);
		const { grants } = await loadDeviceResources(api, profile, edge);
		expect(grants[0]?.effective_expires_at).toBeUndefined();
		const standalone = await readHubStandalone(api.hub.origin, api.hubFetch);
		expect(standalone.max_devices_per_user).toBeUndefined();
	});

	test("writes change the hub and are recorded as writes", async () => {
		const api = fakeDeviceApi();
		const { profile } = api;
		const renamed = await renameDevice(api, profile, edge, "  Edge Berlin ");
		expect(renamed.display_name).toBe("Edge Berlin");
		await revokeDevice(api, profile, warehouse);
		const revoked = await getDevice(api, profile, warehouse);
		expect([revoked.status, revoked.revoked_at]).toEqual([
			"revoked",
			SAMPLE_NOW,
		]);
		await expect(revokeDevice(api, profile, lab)).rejects.toMatchObject({
			status: 404,
		});
		expect(api.writes().map(([method, path]) => `${method} ${path}`)).toEqual([
			`PATCH devices/${edge}`,
			`DELETE devices/${warehouse}`,
			`DELETE devices/${lab}`,
		]);
		expect(api.sent("PATCH")).toEqual([
			["PATCH", `devices/${edge}`, { display_name: "Edge Berlin" }],
		]);
	});

	test("a placement holds one active cloud approval: a second one is a conflict until the first is revoked", async () => {
		const api = fakeDeviceApi();
		const grants = `devices/${edge}/resource-grants`;
		const approve = (deploymentId: string) =>
			api.fetch<{ grant_id: string }>(api.profile, grants, {
				method: "POST",
				body: JSON.stringify({
					placement_id: "visitor-check-in",
					deployment_id: deploymentId,
					project_id: SAMPLE_APPS.invoiceAi,
				}),
			});
		const first = await approve("00000000-0000-4000-8000-0000000000d1");
		await expect(
			approve("00000000-0000-4000-8000-0000000000d2"),
		).rejects.toMatchObject({ status: 409 });
		await api.fetch(api.profile, `${grants}/${first.grant_id}`, {
			method: "DELETE",
		});
		const second = await approve("00000000-0000-4000-8000-0000000000d2");
		expect(second.grant_id).not.toBe(first.grant_id);
	});

	test("a device's certificate report needs the owner or whole-device access", async () => {
		const api = fakeDeviceApi();
		const report = (deviceId: string) =>
			outcome(() =>
				api.fetch(api.profile, `devices/${deviceId}/certificate-inventory`, {
					method: "GET",
				}),
			);
		expect(await report(edge)).toBe("ok");
		// lab-gpu-02 is shared for one app only.
		expect(await report(lab)).toBe("coded 403");
	});

	test("certificate reminders can be muted, listed and tested once per ten minutes", async () => {
		const api = fakeDeviceApi();
		const { profile } = api;
		const certificate = "93bcc1ef-5f49-4bb3-b90c-7b052822bf02";
		const until = SAMPLE_NOW + 86_400;
		expect(
			data(
				await muteCertificateNotices(api, profile, edge, certificate, until),
			),
		).toEqual({ certificate_id: certificate, until });
		await muteCertificateNotices(api, profile, edge, null, null);
		expect(data(await listCertificateNoticeMutes(api, profile, edge))).toEqual([
			{ certificate_id: certificate, until },
			{ certificate_id: null, until: null },
		]);
		await unmuteCertificateNotices(api, profile, edge, null);
		expect(
			data(await listCertificateNoticeMutes(api, profile, edge)),
		).toHaveLength(1);
		const unknown = "00000000-0000-4000-8000-000000000009";
		await expect(
			muteCertificateNotices(api, profile, edge, unknown, null),
		).rejects.toMatchObject({ code: "not_found" });
		api.hub.noticeChannels.email = false;
		expect(
			data(await sendTestCertificateNotice(api, profile, edge, "both")),
		).toEqual({
			sent: ["push"],
			skipped: [{ channel: "email", reason: "not_configured" }],
		});
		await expect(
			sendTestCertificateNotice(api, profile, edge, "push"),
		).rejects.toMatchObject({ code: "rate_limited", retryAfterS: 600 });
		const [notice] = data(await listCertificateNotices(api, profile, edge));
		expect([notice?.stage, notice?.channel]).toEqual(["test", "push"]);
		expect(
			data(await listCertificateNotices(api, profile, edge, certificate)),
		).toEqual([]);
	});

	test("hub conditions and injected failures answer like the real hub", async () => {
		const api = fakeDeviceApi();
		const { profile } = api;
		const code = async (run: () => Promise<unknown>) =>
			toHubError(await run().catch((error: unknown) => error)).code;
		api.fail({ method: "GET", path: "devices" }, undefined, 1);
		expect(await code(() => listDevices(api, profile))).toBe("server_error");
		expect(await listDevices(api, profile)).toHaveLength(7);
		api.mode.reachable = false;
		expect(await code(() => listDevices(api, profile))).toBe("network");
		api.mode.reachable = true;
		api.mode.signedIn = false;
		expect(await code(() => listDevices(api, profile))).toBe("unauthorized");
		api.mode.signedIn = true;
		api.mode.tokenRestricted = true;
		expect(await code(() => listDevices(api, profile))).toBe(
			"token_restricted",
		);
		api.mode.tokenRestricted = false;
		api.mode.devicesEnabled = false;
		expect(await code(() => listDevices(api, profile))).toBe("server_error");
		expect(
			(await readHubStandalone(api.hub.origin, api.hubFetch)).enabled,
		).toBe(false);
		api.mode.devicesEnabled = true;
		const release = api.hold({ path: "devices/usage" });
		let settled = false;
		const pending = getDeviceUsage(api, profile).then(() => {
			settled = true;
		});
		await new Promise((resolve) => setTimeout(resolve, 5));
		expect(settled).toBe(false);
		release();
		await pending;
		expect(settled).toBe(true);
	});

	test("an online app's definitions prepare for a deploy; a local-only or unreadable app has none", async () => {
		const api = fakeDeviceApi();
		const backend = { apiState: api } as unknown as IBackendState;
		const visitor = APPS.app_visitor_checkin.id;
		const prepared = await prepareOnlineMetadata(visitor, backend, api.profile);
		expect(prepared.catalog).toEqual(VISITOR_CATALOG);
		expect(prepared.app).toMatchObject({ id: visitor, bits: [] });
		expect(prepared.sha256).toMatch(/^[0-9a-f]{64}$/);
		const read = (app: string) =>
			outcome(() => send(api, "GET", `apps/${app}/device-metadata`));
		expect(await read(APPS.app_support_portal.id)).toBe("coded 400");
		expect(await read("app_unknown")).toBe("coded 403");
		api.hub.hiddenApps.add(visitor);
		expect(await read(visitor)).toBe("coded 403");
		expect(api.writes()).toEqual([]);
	});

	test("the export resolves only the Latest events it is asked for, and leaves out one that no longer fits", async () => {
		const api = fakeDeviceApi();
		const backend = { apiState: api } as unknown as IBackendState;
		const invoice = APPS.app_invoice_ai.id;
		const events = async (latest?: string[]) =>
			(
				await prepareOnlineMetadata(
					invoice,
					backend,
					api.profile,
					undefined,
					latest,
				)
			).catalog.events;
		// Without names, an event that follows Latest stays out, as on every hub before.
		expect((await events()).map((event) => event.id)).toEqual([
			"evt_extract_http",
			"evt_gpu_extract",
			"evt_invoice_mcp",
			"evt_invoice_reconcile",
		]);
		const named = await events(["evt_invoice_review"]);
		expect(
			named.find((event) => event.id === "evt_invoice_review"),
		).toMatchObject({
			eligible: true,
			kind: "served",
			event_version: [0, 9, 0],
			board_version: [0, 9, 2],
		});
		expect(
			named.find((event) => event.id === "evt_invoice_reconcile"),
		).toMatchObject({
			eligible: true,
			kind: "scheduled",
			schedule: { expression: "0 0 2 * * *", timezone: "Europe/Berlin" },
		});
		expect(api.sent("GET", /device-metadata/).at(-1)?.[1]).toBe(
			`apps/${invoice}/device-metadata?latest=evt_invoice_review`,
		);
		// The flow got edits: no version equals it, so the event stays out until one is published.
		api.hub.flows.edit(invoice, "flow_review");
		const ids = async () =>
			(await events(["evt_invoice_review"])).map((event) => event.id);
		expect(await ids()).not.toContain("evt_invoice_review");
		expect(
			data(await readFlowVersion(api, api.profile, invoice, "flow_review")),
		).toEqual({ current: null, newest: [0, 9, 2] });
		expect(
			await publishFlowVersion(api, api.profile, invoice, "flow_review"),
		).toEqual({ kind: "ok", data: { version: [0, 9, 3], created: true } });
		expect(
			await publishFlowVersion(api, api.profile, invoice, "flow_review"),
		).toEqual({ kind: "ok", data: { version: [0, 9, 3], created: false } });
		expect(
			(await events(["evt_invoice_review"])).find(
				(event) => event.id === "evt_invoice_review",
			)?.board_version,
		).toEqual([0, 9, 3]);
		// Its Page left the flow: the event is left out, the bundle stays valid.
		api.hub.unfitEvents.add("evt_invoice_review");
		expect(await ids()).not.toContain("evt_invoice_review");
		api.hub.unfitEvents.clear();
		// An older hub ignores the names and exports no schedule.
		api.hub.capabilities.latest = false;
		api.hub.capabilities.schedules = false;
		expect(await ids()).toEqual([
			"evt_extract_http",
			"evt_gpu_extract",
			"evt_invoice_mcp",
		]);
		expect(
			await readFlowVersion(api, api.profile, invoice, "flow_review"),
		).toEqual({ kind: "missing_on_hub" });
	});

	test("publishing a flow version can be refused: no right, an edit lock, a flow that never compares equal", async () => {
		const api = fakeDeviceApi();
		const invoice = APPS.app_invoice_ai.id;
		const publish = () =>
			publishFlowVersion(api, api.profile, invoice, "flow_review");
		api.hub.flows.edit(invoice, "flow_review");
		api.hub.flows.canPublish = false;
		expect(await publish()).toEqual({ kind: "flow_role" });
		api.hub.flows.canPublish = true;
		api.hub.flows.locked.add(`${invoice}/flow_review`);
		expect(await publish()).toEqual({ kind: "flow_busy" });
		api.hub.flows.locked.clear();
		api.hub.flows.incomparable.add(`${invoice}/flow_review`);
		expect(await publish()).toMatchObject({ kind: "flow_incomparable" });
		// A pinned flow reads as its newest pin; a flow nothing mentions has no version.
		expect(
			data(await readFlowVersion(api, api.profile, invoice, "flow_gpu")),
		).toEqual({ current: [1, 4, 0], newest: [1, 4, 0] });
		expect(
			data(await readFlowVersion(api, api.profile, invoice, "flow_unknown")),
		).toEqual({ current: null, newest: null });
	});

	test("a schedule moves to a device only after a person released it and the service claimed it", async () => {
		const api = fakeDeviceApi();
		const { hub } = api;
		const invoice = APPS.app_invoice_ai.id;
		const event = "evt_invoice_reconcile";
		const listed = async () =>
			data(await listAppDevicePlacements(api, api.profile, invoice)).schedules;
		expect(await listed()).toEqual([]);
		// A device can't take a schedule nobody released to its service.
		expect(hub.schedules.claim(edge, "invoice-extractor", [event])).toEqual({
			server_time: SAMPLE_NOW,
			claimed: [],
			held: [{ event_id: event, reason: "not_released" }],
		});
		expect(
			await releaseSchedule(
				api,
				api.profile,
				invoice,
				event,
				edge,
				"invoice-extractor",
			),
		).toEqual({ kind: "ok", data: { state: "released", since: SAMPLE_NOW } });
		expect(await listed()).toEqual([
			{
				event_id: event,
				state: "released",
				since: SAMPLE_NOW,
				device_id: edge,
				placement_id: "invoice-extractor",
			},
		]);
		// Another service is refused while this one is the place; a service without an approval can't ask.
		await api.post(api.profile, `devices/${studio}/resource-grants`, {
			placement_id: "invoice-reports",
			deployment_id: "dep-reports",
			project_id: invoice,
			app_id: invoice,
		});
		expect(
			hub.schedules.claim(studio, "invoice-reports", [event]).held,
		).toEqual([{ event_id: event, reason: "runs_elsewhere" }]);
		expect(() =>
			hub.schedules.claim(lab, "invoice-extractor-gpu", [event]),
		).toThrow("No working cloud approval");
		expect(
			hub.schedules.claim(edge, "invoice-extractor", [event]).claimed,
		).toEqual([{ event_id: event, since: SAMPLE_NOW }]);
		expect(await listed()).toEqual([
			{
				event_id: event,
				state: "device",
				since: SAMPLE_NOW,
				seen_at: SAMPLE_NOW,
				grant_id: EDGE_GRANT,
				device_id: edge,
				placement_id: "invoice-extractor",
			},
		]);
		// Releasing it again to the same service changes nothing; to another one is refused.
		expect(
			await releaseSchedule(
				api,
				api.profile,
				invoice,
				event,
				edge,
				"invoice-extractor",
			),
		).toEqual({ kind: "ok", data: { state: "device", since: SAMPLE_NOW } });
		expect(
			await releaseSchedule(
				api,
				api.profile,
				invoice,
				event,
				lab,
				"invoice-extractor-gpu",
			),
		).toEqual({ kind: "schedule_elsewhere" });
		// Giving it back starts the grace period; nobody else can take it meanwhile.
		// invoice-extractor is mid-update (no process runs), so the hub waits 5 minutes, not 65.
		const back = await giveBackSchedule(api, api.profile, invoice, event);
		expect(back).toEqual({
			kind: "ok",
			data: { hub_resumes_at: SAMPLE_NOW + 300 },
		});
		expect(await listed()).toEqual([
			{
				event_id: event,
				state: "returning",
				hub_resumes_at: SAMPLE_NOW + 300,
			},
		]);
		expect(
			await releaseSchedule(
				api,
				api.profile,
				invoice,
				event,
				lab,
				"invoice-extractor-gpu",
			),
		).toEqual({ kind: "schedule_returning" });
		expect(
			hub.schedules.claim(edge, "invoice-extractor", [event]).held,
		).toEqual([{ event_id: event, reason: "not_released" }]);
		// A person without the right to edit the app's events can neither move it nor give it back.
		hub.schedules.canEditEvents = false;
		expect(
			await releaseSchedule(
				api,
				api.profile,
				invoice,
				"evt_other",
				edge,
				"invoice-extractor",
			),
		).toEqual({ kind: "schedule_role" });
		expect(await giveBackSchedule(api, api.profile, invoice, event)).toEqual({
			kind: "schedule_role",
		});
		// An older hub has neither the routes nor the list.
		hub.capabilities.schedules = false;
		expect(
			await releaseSchedule(
				api,
				api.profile,
				invoice,
				event,
				edge,
				"invoice-extractor",
			),
		).toEqual({ kind: "missing_on_hub" });
		expect(await listed()).toBeUndefined();
	});

	test("a hub of round two says which types it hands to devices; an older hub says nothing", async () => {
		const api = fakeDeviceApi();
		const raw = () =>
			api.get<Record<string, unknown>>(
				api.profile,
				`apps/${APPS.app_invoice_ai.id}/device-placements`,
			);
		expect((await raw()).event_types).toEqual([
			"http",
			"simple_chat",
			"rest",
			"mcp",
			"daemon",
			"cron",
			"api",
			"quick_action",
			"generic_form",
			"telegram",
			"discord",
		]);
		api.hub.capabilities.eventTypes = false;
		expect(await raw()).not.toHaveProperty("event_types");
		expect(await raw()).toHaveProperty("schedules");
	});

	test("the export takes the types of round two it is asked for: out of five, each once", async () => {
		const api = fakeDeviceApi();
		const read = (query: string) =>
			outcome(() =>
				send(
					api,
					"GET",
					`apps/${APPS.app_invoice_ai.id}/device-metadata${query}`,
				),
			);
		for (const query of [
			"?types=generic_form",
			"?types=api,quick_action,generic_form,telegram,discord",
			"?types=",
			"?latest=evt_invoice_review&types=api",
		])
			expect(await read(query)).toBe("ok");
		for (const query of [
			"?types=cron",
			"?types=api,api",
			"?types=API",
			"?types=api,",
		])
			expect(await read(query)).toBe("coded 400");
		// A hub before round two has no such parameter: it reads past it.
		api.hub.capabilities.eventTypes = false;
		expect(await read("?types=cron")).toBe("ok");
		expect(api.writes()).toEqual([]);
	});

	test("an event of round two is in the bundle only when its type was named; an older hub never sends one", async () => {
		const api = fakeDeviceApi();
		const backend = { apiState: api } as unknown as IBackendState;
		const shop = APPS.app_shop_assistant.id;
		const ids = async (types: string[] = []) =>
			(
				await prepareOnlineMetadata(
					shop,
					backend,
					api.profile,
					undefined,
					[],
					types,
				)
			).catalog.events
				.map((event) => event.id)
				.sort();
		// The one-time schedule is a `cron` event: round one's bundle has it.
		expect(await ids()).toEqual(["evt_shop_prices"]);
		expect(await ids(["generic_form"])).toEqual([
			"evt_shop_prices",
			"evt_shop_return",
		]);
		expect(
			await ids(["telegram", "api", "discord", "generic_form", "quick_action"]),
		).toEqual([
			"evt_shop_discord",
			"evt_shop_orders",
			"evt_shop_prices",
			"evt_shop_return",
			"evt_shop_telegram",
		]);
		expect(api.sent("GET", /device-metadata/).at(-1)?.[1]).toBe(
			`apps/${shop}/device-metadata?types=api,discord,generic_form,quick_action,telegram`,
		);
		// What reaches a device of each: routes and bot settings, never a token.
		const bundle = await api.get<{
			documents: Record<string, { config?: number[] }>;
		}>(api.profile, `apps/${shop}/device-metadata?types=api,telegram,discord`);
		const configOf = (event: string) =>
			JSON.parse(
				new TextDecoder().decode(
					Uint8Array.from(
						bundle.documents[`events/${event}/versions/1/0/0`]?.config ?? [],
					),
				),
			) as Record<string, unknown>;
		expect(configOf("evt_shop_orders")).toEqual({
			sink_type: "http",
			method: "GET",
			path: "/orders",
			public_endpoint: false,
		});
		expect(configOf("evt_shop_telegram")).not.toHaveProperty("bot_token");
		expect(configOf("evt_shop_discord")).not.toHaveProperty("token");
		expect(configOf("evt_shop_telegram").chat_whitelist).toEqual([]);
		expect(JSON.stringify(bundle)).not.toContain("fixture-token");
		expect(JSON.stringify(bundle)).not.toContain("shop-orders-own-token");
		// The placements answer names what this hub hands to devices.
		expect(
			data(await listAppDevicePlacements(api, api.profile, shop)).event_types,
		).toContain("generic_form");
		api.hub.capabilities.eventTypes = false;
		expect(await ids(["generic_form", "telegram"])).toEqual([
			"evt_shop_prices",
		]);
		expect(
			data(await listAppDevicePlacements(api, api.profile, shop)).event_types,
		).toBeUndefined();
	});

	test("an exported event keeps its config without a credential", async () => {
		const api = fakeDeviceApi();
		const invoice = APPS.app_invoice_ai;
		const stored = {
			sink_type: "http",
			path: "/extract",
			method: "POST",
			public_endpoint: true,
			auth_token: "tok-auth",
			Bot_Token: "tok-bot",
			webhook_secret: "tok-hook",
			secret_key: "tok-key",
			password: "tok-pass",
			openai_api_key: "tok-api",
		};
		api.hub.apps = {
			...api.hub.apps,
			[invoice.id]: {
				...invoice,
				events: invoice.events.map((event) =>
					event.id === "evt_extract_http"
						? {
								...event,
								config: [...new TextEncoder().encode(JSON.stringify(stored))],
							}
						: event,
				),
			},
		};
		const bundle = await api.get<{
			documents: Record<string, { config?: number[] }>;
		}>(api.profile, `apps/${invoice.id}/device-metadata`);
		const config = bundle.documents["events/evt_extract_http/versions/1/5/0"]
			?.config as number[];
		expect(
			JSON.parse(new TextDecoder().decode(Uint8Array.from(config))),
		).toEqual({
			sink_type: "http",
			path: "/extract",
			method: "POST",
			public_endpoint: true,
		});
		expect(JSON.stringify(bundle)).not.toContain("tok-");
		// A schedule still travels as its config.
		const nightly = bundle.documents[
			"events/evt_invoice_reconcile/versions/1/0/0"
		]?.config as number[];
		expect(
			JSON.parse(new TextDecoder().decode(Uint8Array.from(nightly))),
		).toEqual({ expression: "0 0 2 * * *", timezone: "Europe/Berlin" });
	});

	test("a service that drops a schedule hands it back; revoking its cloud access does too", async () => {
		const api = fakeDeviceApi();
		const { hub } = api;
		const invoice = APPS.app_invoice_ai.id;
		const event = "evt_invoice_reconcile";
		const take = () => {
			hub.schedules.release(invoice, event, edge, "invoice-extractor");
			return hub.schedules.claim(edge, "invoice-extractor", [event]);
		};
		expect(take().claimed).toHaveLength(1);
		// Its next claim call no longer lists the event: confirmed by the device, 5 minutes.
		hub.schedules.claim(edge, "invoice-extractor", []);
		expect(hub.schedules.listing(invoice)).toEqual([
			{
				event_id: event,
				state: "returning",
				hub_resumes_at: SAMPLE_NOW + 300,
			},
		]);
		hub.now = () => SAMPLE_NOW + 301;
		expect(hub.schedules.listing(invoice)).toEqual([]);
		expect(take().claimed).toEqual([
			{ event_id: event, since: SAMPLE_NOW + 301 },
		]);
		await api.fetch(
			api.profile,
			`devices/${edge}/resource-grants/${EDGE_GRANT}`,
			{ method: "DELETE" },
		);
		// The release stays: a new approval for the same service can claim again.
		expect(hub.schedules.listing(invoice)).toMatchObject([
			{ event_id: event, state: "released", device_id: edge },
		]);
		expect(() =>
			hub.schedules.claim(edge, "invoice-extractor", [event]),
		).toThrow();
	});
});

describe("fake agent", () => {
	test("its default answers satisfy every device read client", async () => {
		fake = await createFakeWorkspace(sampleFleet());
		const { workspace } = fake;
		const call = workspace.live.call(edge);
		const features = workspace.live.inspection(edge)?.value.features;
		const support = await readExistingDeployment(
			call,
			"support-bot",
			SAMPLE_APPS.supportPortal,
		);
		expect(support.config.hosting?.port).toBe(8_443);
		const rollout = await readDeploymentRollout(call, {
			rollout_id: INVOICE_ROLLOUT,
			placement_id: "invoice-extractor",
			project_id: SAMPLE_APPS.invoiceAi,
		});
		expect(rollout.state).toBe("activating");
		expect((await readCertificates(call)).certificates).toHaveLength(2);
		expect(await readCertificateRequests(call)).toHaveLength(1);
		expect(await readCertificateIssuers(call)).toHaveLength(1);
		expect(await readAcmeCertificates(call)).toHaveLength(1);
		expect(
			await readOfflineQueues(workspace.live.call(studio), "field-notes"),
		).toHaveLength(2);
		const roster = await readArchiveRoster(call, "device", "metrics");
		expect(
			fake.crypto.verifyArchiveRosterHead(
				roster.text ?? "",
				fake.hub.ownerInvitationKey(edge),
			).expires_at,
		).toBe(1_790_683_200);
		const reads = [
			await readHostOperation(call, features),
			await readRolloutHistory(call, features, {
				placementId: "invoice-extractor",
			}),
			await readOperations(call, features),
			await readMetricsHistory(call, features, {
				placementId: null,
				fields: ["cpu_percent"],
			}),
			await readOfflineOperations(call, features, {
				placementId: "invoice-extractor",
				scope: "a".repeat(64),
			}),
			await lookupOfflineOperation(call, features, {
				placementId: "invoice-extractor",
				scope: "a".repeat(64),
				queuedOperationId: "f40615e0-b484-42d1-acd5-408b2c9e9938",
			}),
			await readArtifactUsage(call, features),
			await pruneArtifactRevisions(call, features, {
				projectId: SAMPLE_APPS.crmSync,
				revisions: ["a".repeat(64)],
			}),
		];
		expect(reads.map((read) => read.kind)).toEqual(reads.map(() => "ok"));
		expect(FAKE_AGENT_COMMANDS).toContain("inspect_page");
	});

	test("an agent from before the feature flags refuses every newer command", async () => {
		fake = await createFakeWorkspace(sampleFleet(), {
			agentFeatures: {},
			unlock: [edge],
		});
		const { workspace } = fake;
		const inspection = workspace.live.inspection(edge)?.value;
		expect(inspection?.features).toEqual({});
		expect([inspection?.agent, inspection?.tasks]).toEqual([
			undefined,
			undefined,
		]);
		expect(inspection?.placements[0]?.source).toBeUndefined();
		const call = workspace.live.call(edge);
		const usage = { kind: "usage", project_id: null };
		const prune = { kind: "prune", project_id: "app", revisions: [] };
		for (const command of [
			{ type: "host_operation" },
			{ type: "rollout_history", placement_id: "support-bot", limit: 8 },
			{ type: "operations", limit: 20 },
			{ type: "metrics_history", placement_id: null, after: 0, limit: 8 },
			{ type: "offline_queue_operations", placement_id: "support-bot" },
			{ type: "offline_queue_lookup", placement_id: "support-bot" },
			{ type: "artifact", request: usage },
			{ type: "artifact", request: prune },
		]) {
			const response = await call(command);
			expect([
				command.type,
				response.state,
				managementRejection(response)?.code,
			]).toEqual([command.type, "rejected", "unsupported"]);
		}
	});

	test("commands change what the device reports; refusals and a lost reply come back as on a device", async () => {
		fake = await createFakeWorkspace(sampleFleet(), { unlock: [edge] });
		const { workspace, api } = fake;
		const agent = api.agent(edge);
		const call = workspace.live.call(edge);
		const stop = { placement_id: "support-bot", expected_revision: 7 };
		expect((await call({ type: "stop", ...stop })).state).toBe("completed");
		await workspace.live.refreshInspection(edge);
		const stopped = workspace.live
			.inspection(edge)
			?.value.placements.find((row) => row.id === "support-bot");
		expect([stopped?.desired_state, stopped?.running_replicas]).toEqual([
			"stopped",
			0,
		]);
		const stale = await call({ type: "start", ...stop, expected_revision: 1 });
		expect(managementRejection(stale)?.code).toBe("revision_conflict");
		agent.reject("start", "host_policy", "Isolation is required here.");
		const refused = await call({ type: "start", ...stop });
		expect(managementRejection(refused)?.error).toBe(
			"Isolation is required here.",
		);
		agent.dropNext("restart");
		await expect(call({ type: "restart", ...stop })).rejects.toBeInstanceOf(
			ManagementUnconfirmedError,
		);
		expect(
			workspace.activity
				.list()
				.some(
					(item) => item.state === "unknown" && item.target.deviceId === edge,
				),
		).toBe(true);
		expect(api.commands.map(([, type]) => type)).toContain("restart");
	});

	test("a quick update honours `start`; a service is removed only once it is asked to stop and has stopped", async () => {
		fake = await createFakeWorkspace(sampleFleet(), { unlock: [edge] });
		const { workspace, api } = fake;
		const agent = api.agent(edge);
		const call = workspace.live.call(edge);
		const row = () => agent.placement("support-bot");
		const states = () => [row()?.desired_state, row()?.observed_state];
		const { project_id, deployment_id, revision } = seeded(row());
		const config = { id: "support-bot", project_id, deployment_id, revision };
		const apply = (expected_revision: number, start: boolean) =>
			call({ type: "apply", config, expected_revision, start });

		expect((await apply(7, true)).state).toBe("completed");
		expect(states()).toEqual(["running", "running"]);
		expect((await apply(8, false)).state).toBe("completed");
		expect(states()).toEqual(["stopped", "stopped"]);
		expect((await apply(9, true)).state).toBe("completed");

		const service = { placement_id: "support-bot", expected_revision: 10 };
		const early = await call({ type: "remove", ...service });
		expect(managementRejection(early)?.error).toContain(
			"must be stopped before removal",
		);

		agent.stopReads = 2;
		expect((await call({ type: "stop", ...service })).state).toBe("completed");
		expect(states()).toEqual(["stopped", "stopping"]);
		const stillStopping = await call({ type: "remove", ...service });
		expect(managementRejection(stillStopping)?.code).toBe("invalid");
		let reads = 0;
		while (row()?.observed_state === "stopping" && reads < 6) {
			await workspace.live.refreshInspection(edge);
			reads += 1;
		}
		expect(reads).toBeGreaterThan(0);
		expect(states()).toEqual(["stopped", "stopped"]);
		expect((await call({ type: "remove", ...service })).state).toBe(
			"completed",
		);
		expect(row()).toBeUndefined();
	});

	test("a service reports its schedules only while it runs, armed or held; its claim on the hub stays through a stop", async () => {
		fake = await createFakeWorkspace(sampleFleet(), { unlock: [edge] });
		const { workspace, api } = fake;
		const agent = api.agent(edge);
		const call = workspace.live.call(edge);
		const invoice = SAMPLE_APPS.invoiceAi;
		const event = "evt_invoice_reconcile";
		const row = () => seeded(agent.placement("invoice-extractor"));
		const base = seeded(agent.placement("invoice-extractor"));
		const config = {
			id: "invoice-extractor",
			project_id: invoice,
			deployment_id: base.deployment_id,
			revision: base.revision,
			source: "online",
			events: [
				{
					event_id: "evt_extract_http",
					event_version: [1, 5, 0],
					board_version: [2, 2, 0],
				},
				{ event_id: event, event_version: [1, 0, 0], board_version: [1, 3, 0] },
			],
		};
		const revision = base.config_revision;
		// Nobody released it: the service runs, the schedule is held and the hub keeps running it.
		await call({
			type: "apply",
			config,
			expected_revision: revision,
			start: true,
		});
		expect(row().events?.map((value) => value.event_id)).toEqual([
			"evt_extract_http",
			event,
		]);
		expect(row().schedules).toMatchObject([
			{ event_id: event, hold: "not_released", next_at: null },
		]);
		expect(api.hub.schedules.listing(invoice)).toEqual([]);
		// Released, then the next start claims and arms it.
		api.hub.schedules.release(invoice, event, edge, "invoice-extractor");
		await call({
			type: "restart",
			placement_id: "invoice-extractor",
			expected_revision: revision + 1,
		});
		expect(row().schedules).toEqual([
			{
				event_id: event,
				expression: "0 0 2 * * *",
				timezone: "Europe/Berlin",
				hold: null,
				// 02:00 in Berlin is 00:00 UTC in summer time; the sample's now is 2026-09-30 12:00 UTC.
				next_at: Date.UTC(2026, 9, 1, 0, 0, 0) / 1_000,
				running: false,
				last_at: null,
				last_outcome: null,
				runs: 0,
				failed: 0,
				skipped: 0,
				last_skip: null,
				clock_behind: false,
			},
		]);
		expect(row().schedules_truncated).toBe(false);
		expect(api.hub.schedules.listing(invoice)).toMatchObject([
			{ event_id: event, state: "device", device_id: edge },
		]);
		// The live row carries it; a snapshot only what does not change by itself.
		await workspace.live.refreshInspection(edge);
		expect(
			workspace.live
				.inspection(edge)
				?.value.placements.find((value) => value.id === "invoice-extractor")
				?.schedules?.[0],
		).toMatchObject({ event_id: event, hold: null, runs: 0 });
		api.hub.publishStatus(edge, agent);
		const [stream] = api.hub.streams.get(edge) ?? [];
		const snapshot = (
			stream?.payload as {
				inspection: {
					features?: Record<string, number>;
					placements: { id: string; schedules?: unknown[] }[];
				};
			}
		).inspection;
		expect(snapshot.features?.scheduled_events).toBe(1);
		expect(
			snapshot.placements.find((value) => value.id === "invoice-extractor")
				?.schedules,
		).toEqual([
			{
				event_id: event,
				expression: "0 0 2 * * *",
				timezone: "Europe/Berlin",
				hold: null,
				last_outcome: null,
			},
		]);
		// Stop: no schedule fact, and the hub does not take it over.
		await call({
			type: "stop",
			placement_id: "invoice-extractor",
			expected_revision: revision + 1,
		});
		expect(row().schedules).toBeUndefined();
		expect(api.hub.schedules.listing(invoice)).toMatchObject([
			{ event_id: event, state: "device" },
		]);
		// An update that drops the schedule hands it back once the service runs again.
		await call({
			type: "apply",
			config: { ...config, events: config.events.slice(0, 1) },
			expected_revision: revision + 1,
			start: true,
		});
		expect(row().schedules).toBeUndefined();
		expect(api.hub.schedules.listing(invoice)).toMatchObject([
			{ event_id: event, state: "returning" },
		]);
	});

	test("an agent without the schedule flag fails a service that has one, and refuses schedules at discovery", async () => {
		const withoutFlag = {
			placement_events: 1,
			placement_diagnostics: 1,
		} as const;
		fake = await createFakeWorkspace(sampleFleet(), {
			unlock: [edge],
			agentFeatures: withoutFlag,
		});
		const { workspace, api } = fake;
		const agent = api.agent(edge);
		const call = workspace.live.call(edge);
		const base = seeded(agent.placement("nightly-sync"));
		await call({
			type: "apply",
			config: {
				id: "nightly-sync",
				project_id: SAMPLE_APPS.crmSync,
				deployment_id: base.deployment_id,
				revision: base.revision,
				source: "offline",
				events: [
					{
						event_id: "evt_crm_hourly",
						event_version: [1, 0, 0],
						board_version: [4, 1, 0],
					},
				],
			},
			expected_revision: base.config_revision,
			start: true,
		});
		expect(agent.placement("nightly-sync")).toMatchObject({
			desired_state: "running",
			observed_state: "failed",
			running_replicas: 0,
			has_error: true,
			last_error: "Event evt_crm_hourly needs an unsupported cron sink.",
		});
		expect(agent.placement("nightly-sync")?.schedules).toBeUndefined();
		const describe = async () => {
			const answer = await call({
				type: "artifact",
				request: {
					kind: "describe",
					project_id: SAMPLE_APPS.crmSync,
					revision: "a".repeat(64),
					event_id: null,
					after: null,
				},
			});
			return (answer.result.items as Record<string, unknown>[]).find(
				(item) => item.id === "evt_crm_hourly",
			);
		};
		const old = await describe();
		expect(old).toMatchObject({
			eligible: false,
			readiness_kind: "unsupported",
			rollout_supported: false,
		});
		expect(old).not.toHaveProperty("kind");
		agent.features = { ...withoutFlag, scheduled_events: 1 };
		expect(await describe()).toMatchObject({
			eligible: true,
			readiness_kind: "explicit",
			rollout_supported: true,
			kind: "scheduled",
			schedule: { expression: "0 0 * * * *", timezone: "UTC" },
			ineligible_code: null,
		});
	});

	test("two services of one device never both run a schedule: the second holds it", async () => {
		fake = await createFakeWorkspace(sampleFleet(), { unlock: [edge] });
		const { workspace, api } = fake;
		const agent = api.agent(edge);
		const call = workspace.live.call(edge);
		const config = (id: string) => ({
			id,
			project_id: SAMPLE_APPS.crmSync,
			deployment_id: `dep-${id}`,
			revision: "r1",
			source: "offline",
			events: [
				{
					event_id: "evt_crm_hourly",
					event_version: [1, 0, 0],
					board_version: [4, 1, 0],
				},
			],
		});
		for (const id of ["hourly-a", "hourly-b"])
			await call({
				type: "apply",
				config: config(id),
				expected_revision: 0,
				start: true,
			});
		expect(agent.placement("hourly-a")?.schedules?.[0].hold).toBeNull();
		expect(agent.placement("hourly-b")?.schedules?.[0].hold).toBe(
			"other_service",
		);
	});
});

describe("fake agent, round two", () => {
	const R2_FLAGS = {
		api_events: 1,
		scheduled_once: 1,
		on_demand_events: 1,
		telegram_bots: 1,
		discord_bots: 1,
	} as const;
	type Triple = [number, number, number];
	const bytes = (value: unknown) => [
		...new TextEncoder().encode(JSON.stringify(value)),
	];
	const kindsEvent = (
		id: string,
		name: string,
		type: string,
		version: Triple,
		board: Triple,
		config?: unknown,
	): AppEventInput => ({
		id,
		name,
		active: true,
		event_type: type,
		event_version: version,
		board_version: board,
		default_page_id: null,
		boardId: "flow_main",
		...(config === undefined ? {} : { config: bytes(config) }),
	});
	const KINDS = "app_kinds";
	/** One event of each kind of round two, as the rows of design §1.5 name them. */
	const KIND_EVENTS = {
		orders: kindsEvent("evt_orders", "Orders", "api", [0, 0, 2], [0, 0, 5], {
			sink_type: "http",
			method: "GET",
			path: "/orders",
			public_endpoint: true,
			auth_token: "tok-endpoint",
		}),
		once: kindsEvent("evt_once", "Migration", "cron", [0, 0, 4], [0, 0, 7], {
			scheduled_for: { date: "2026-09-24", time: "09:00" },
			timezone: "Europe/Berlin",
		}),
		form: kindsEvent(
			"evt_notes_form",
			"New note",
			"generic_form",
			[1, 0, 0],
			[3, 0, 1],
		),
		helper: kindsEvent(
			"evt_helper",
			"Helper",
			"telegram",
			[0, 0, 1],
			[0, 0, 3],
			{
				chat_whitelist: [],
				bot_token: "123456789:tok-telegram-abcdefghijklmn",
			},
		),
	};
	const kindsApp = (extra: AppEventInput[] = []): AppInput => ({
		id: KINDS,
		name: "Kinds",
		visibility: "Offline",
		versions: null,
		events: [...Object.values(KIND_EVENTS), ...extra],
	});
	const pin = (event: AppEventInput) => ({
		event_id: event.id,
		event_version: event.event_version,
		board_version: event.board_version,
	});
	const tokenKey = (eventId: string) => `event.${eventId}.bot_token`;

	/** edge-berlin-01 with every flag of round two and the app of `kindsApp`; `lacking(flag)` takes one away. */
	async function onEdge(extra: AppEventInput[] = []) {
		fake = await createFakeWorkspace(sampleFleet(), { unlock: [edge] });
		const agent = fake.api.agent(edge);
		const full: AgentFeatures = { ...agent.features, ...R2_FLAGS };
		agent.features = full;
		const lacking = (...flags: AgentFeature[]) => {
			agent.features = Object.fromEntries(
				Object.entries(full).filter(
					([flag]) => !flags.includes(flag as AgentFeature),
				),
			) as AgentFeatures;
		};
		fake.hub.apps = { ...fake.hub.apps, [KINDS]: kindsApp(extra) };
		const call = fake.workspace.live.call(edge);
		const deploy = async (
			events: AppEventInput[],
			options: {
				id?: string;
				start?: boolean;
				secrets?: Record<string, string>;
			} = {},
		) => {
			const id = options.id ?? "kinds";
			return call({
				type: "apply",
				config: {
					id,
					project_id: KINDS,
					deployment_id: `dep-${id}`,
					revision: "r1",
					source: "offline",
					events: events.map(pin),
					secret_overrides: options.secrets ?? {},
				},
				expected_revision: agent.placement(id)?.config_revision ?? 0,
				start: options.start !== false,
			});
		};
		const row = (id = "kinds") => seeded(agent.placement(id));
		return {
			agent,
			call,
			deploy,
			row,
			lacking,
			api: fake.api,
			hub: fake.hub,
		};
	}

	test("discovery rows say the kind, route and once of round two (the literals of design §1.5)", async () => {
		const { call, lacking } = await onEdge();
		const describeKinds = async () =>
			(
				await call({
					type: "artifact",
					request: {
						kind: "describe",
						project_id: KINDS,
						revision: "a".repeat(64),
						event_id: null,
						after: null,
					},
				})
			).result.items as Record<string, unknown>[];
		const literal = [
			{
				id: "evt_helper",
				name: "Helper",
				event_type: "telegram",
				event_version: [0, 0, 1],
				board_version: [0, 0, 3],
				hosted: false,
				eligible: true,
				readiness_kind: "explicit",
				rollout_supported: true,
				readiness_error: null,
				kind: "bot",
				ineligible_code: null,
			},
			{
				id: "evt_notes_form",
				name: "New note",
				event_type: "generic_form",
				event_version: [1, 0, 0],
				board_version: [3, 0, 1],
				hosted: false,
				eligible: true,
				readiness_kind: "explicit",
				rollout_supported: true,
				readiness_error: null,
				kind: "on_demand",
				ineligible_code: null,
			},
			{
				id: "evt_once",
				name: "Migration",
				event_type: "cron",
				event_version: [0, 0, 4],
				board_version: [0, 0, 7],
				hosted: false,
				eligible: true,
				readiness_kind: "explicit",
				rollout_supported: true,
				readiness_error: null,
				kind: "scheduled",
				once: {
					date: "2026-09-24",
					time: "09:00",
					at: 1790233200,
					timezone: "Europe/Berlin",
				},
				ineligible_code: null,
			},
			{
				id: "evt_orders",
				name: "Orders",
				event_type: "api",
				event_version: [0, 0, 2],
				board_version: [0, 0, 5],
				hosted: true,
				eligible: true,
				readiness_kind: "listener",
				rollout_supported: true,
				readiness_error: null,
				kind: "served",
				route: { method: "GET", path: "/orders" },
				ineligible_code: null,
			},
		];
		expect(await describeKinds()).toEqual(literal);
		// An agent built without a part answers that type as it answers any type it doesn't know.
		const unknown = (id: string) => ({
			eligible: false,
			readiness_kind: "unsupported",
			rollout_supported: false,
			readiness_error: null,
			ineligible_code: null,
			hosted: false,
			id,
		});
		for (const [flag, id] of [
			["api_events", "evt_orders"],
			["on_demand_events", "evt_notes_form"],
			["telegram_bots", "evt_helper"],
		] as const) {
			lacking(flag);
			const row = (await describeKinds()).find((item) => item.id === id);
			expect(row).toMatchObject(unknown(id));
			expect(row).not.toHaveProperty("kind");
			expect(row).not.toHaveProperty("route");
		}
		// Without one-time schedules it says why, and sends no instant.
		lacking("scheduled_once");
		const once = (await describeKinds()).find((item) => item.id === "evt_once");
		expect(once).toMatchObject({
			eligible: false,
			readiness_kind: "explicit",
			rollout_supported: false,
			kind: "scheduled",
			ineligible_code: "schedule_once",
		});
		expect(once).not.toHaveProperty("once");
		expect(once).not.toHaveProperty("schedule");
	});

	test("a device describes the copy of a revision it holds, not an event record edited since", async () => {
		const { call, deploy, hub } = await onEdge();
		await deploy([KIND_EVENTS.orders]);
		hub.apps = {
			...hub.apps,
			[KINDS]: {
				...kindsApp(),
				events: kindsApp().events.map((event) =>
					event.id === "evt_orders"
						? { ...event, config: bytes({ path: "/orders-v2", method: "GET" }) }
						: event,
				),
			},
		};
		const routeAt = async (revision: string) =>
			(
				(
					await call({
						type: "artifact",
						request: {
							kind: "describe",
							project_id: KINDS,
							revision,
							event_id: null,
							after: null,
						},
					})
				).result.items as { id: string; route?: unknown }[]
			).find((item) => item.id === "evt_orders")?.route;
		expect(await routeAt("r1")).toEqual({ method: "GET", path: "/orders" });
		expect(await routeAt("r2")).toEqual({ method: "GET", path: "/orders-v2" });
	});

	test("a route, a bot setting or a one-time instant the device can't read is refused with its code", async () => {
		const broken = [
			kindsEvent("evt_no_path", "No path", "api", [1, 0, 0], [1, 0, 0], {}),
			kindsEvent("evt_trace", "Trace", "api", [1, 0, 0], [1, 0, 0], {
				path: "/x",
				method: "TRACE",
			}),
			kindsEvent("evt_ui", "UI", "http", [1, 0, 0], [1, 0, 0], {
				path: "/ui/x",
				method: "GET",
			}),
			kindsEvent("evt_bad_bot", "Bad bot", "discord", [1, 0, 0], [1, 0, 0], {
				intents: ["Guilds", "Telepathy"],
			}),
			kindsEvent("evt_gap", "Gap", "cron", [1, 0, 0], [1, 0, 0], {
				scheduled_for: { date: "2027-03-28", time: "02:30" },
				timezone: "Europe/Berlin",
			}),
		];
		const { call, deploy, row, hub } = await onEdge(broken);
		const items = (
			await call({
				type: "artifact",
				request: {
					kind: "describe",
					project_id: KINDS,
					revision: "a".repeat(64),
					event_id: null,
					after: null,
				},
			})
		).result.items as Record<string, unknown>[];
		const codes = Object.fromEntries(
			items.map((item) => [item.id, [item.eligible, item.ineligible_code]]),
		);
		expect(codes).toMatchObject({
			evt_no_path: [false, "route_missing"],
			evt_trace: [false, "route_invalid"],
			evt_ui: [false, "route_reserved"],
			evt_bad_bot: [false, "bot_invalid"],
			evt_gap: [false, "schedule_invalid"],
		});
		// Validation at start refuses each, with the device's sentence.
		for (const event of broken) {
			await deploy([event], { id: `svc-${event.id.replaceAll("_", "-")}` });
			expect(row(`svc-${event.id.replaceAll("_", "-")}`)).toMatchObject({
				observed_state: "failed",
				has_error: true,
			});
		}
		// Two events of one service on one method and path.
		const twin = kindsEvent(
			"evt_orders_2",
			"Orders 2",
			"api",
			[1, 0, 0],
			[1, 0, 0],
			{
				path: "/orders",
				method: "get",
			},
		);
		hub.apps = { ...hub.apps, [KINDS]: kindsApp([twin]) };
		await deploy([KIND_EVENTS.orders, twin], { id: "twins" });
		expect(row("twins").last_error).toBe(
			"Two events claim the same method and service path",
		);
	});

	test("an agent without a flag fails a service of that type; a bot token key belongs to a bot of the service", async () => {
		const { agent, call, deploy, row, lacking } = await onEdge();
		const failed = () => [row().observed_state, row().last_error];
		for (const [flag, event, sentence] of [
			[
				"api_events",
				KIND_EVENTS.orders,
				"Event evt_orders needs an unsupported api sink.",
			],
			[
				"scheduled_once",
				KIND_EVENTS.once,
				"Event evt_once has a one-time schedule this device can't run.",
			],
			[
				"on_demand_events",
				KIND_EVENTS.form,
				"Event evt_notes_form needs an unsupported generic_form sink.",
			],
			[
				"telegram_bots",
				KIND_EVENTS.helper,
				"Event evt_helper needs an unsupported telegram sink.",
			],
		] as const) {
			lacking(flag);
			await deploy([event], {
				secrets:
					event === KIND_EVENTS.helper ? { [tokenKey(event.id)]: "s1" } : {},
			});
			expect(failed()).toEqual(["failed", sentence]);
		}
		lacking();
		await deploy([KIND_EVENTS.helper]);
		expect(failed()).toEqual(["failed", "Event evt_helper has no bot token."]);
		await deploy([KIND_EVENTS.orders, KIND_EVENTS.helper], {
			secrets: {
				[tokenKey("evt_helper")]: "s1",
				[tokenKey("evt_orders")]: "s2",
			},
		});
		expect(failed()).toEqual([
			"failed",
			"placement variable event.evt_orders.bot_token is absent from the selected pinned boards",
		]);
		await deploy([KIND_EVENTS.orders, KIND_EVENTS.helper], {
			secrets: { [tokenKey("evt_helper")]: "s1" },
		});
		expect(failed()).toEqual(["running", undefined]);
		// A safe update the device refuses fails validation and leaves the running revision.
		const before = row().config_revision;
		const staged = await call({
			type: "stage_rollout",
			config: { ...agent.configs.get("kinds"), secret_overrides: {} },
			expected_revision: before,
		});
		const rolloutId = (staged.result as { rollout_id: string }).rollout_id;
		await call({ type: "activate_rollout", rollout_id: rolloutId });
		expect(
			(await call({ type: "rollout", rollout_id: rolloutId })).result,
		).toMatchObject({ state: "validating" });
		expect(
			(await call({ type: "rollout", rollout_id: rolloutId })).result,
		).toMatchObject({ state: "failed", failure_code: "validation_failed" });
		expect([row().config_revision, row().observed_state]).toEqual([
			before,
			"running",
		]);
	});

	test("a bot connects once nothing holds it; a second service of the device holds it; a snapshot says only ok", async () => {
		const { agent, api, call, deploy, row, lacking } = await onEdge();
		const secrets = { [tokenKey("evt_helper")]: "bot-helper" };
		await deploy([KIND_EVENTS.helper], { secrets });
		expect(row().bots).toEqual([
			{
				event_id: "evt_helper",
				provider: "telegram",
				state: "connected",
				hold: null,
				bot_name: "Helper",
				connected_at: SAMPLE_NOW,
				last_message_at: null,
				last_outcome: null,
				running: 0,
				runs: 0,
				runs_today: 0,
				failed: 0,
				dropped: 0,
			},
		]);
		expect(row().bots_truncated).toBe(false);
		await deploy([KIND_EVENTS.helper], { id: "kinds-b", secrets });
		expect(row("kinds-b").bots?.[0]).toMatchObject({
			state: "waiting",
			hold: "other_service",
			connected_at: null,
		});
		// The provider refused the token: the bot stays off until the service restarts.
		agent.botFacts.set("evt_helper", {
			state: "token_refused",
			bot_name: null,
		});
		await call({
			type: "restart",
			placement_id: "kinds",
			expected_revision: row().config_revision,
		});
		expect(row().bots?.[0]).toMatchObject({
			state: "token_refused",
			bot_name: null,
			connected_at: null,
		});
		// What a test says about a bot applies at its next start, or at once with `report`.
		agent.botFacts.clear();
		agent.report(row());
		api.hub.publishStatus(edge, agent);
		const snapshot = (
			api.hub.streams.get(edge)?.at(-1)?.payload as {
				inspection: { placements: PlacementStatusPlus[] };
			}
		).inspection.placements;
		expect(snapshot.find((value) => value.id === "kinds")?.bots).toEqual([
			{ event_id: "evt_helper", provider: "telegram", hold: null, state: "ok" },
		]);
		expect(snapshot.find((value) => value.id === "kinds-b")?.bots).toEqual([
			{
				event_id: "evt_helper",
				provider: "telegram",
				hold: "other_service",
				state: "waiting",
			},
		]);
		// The token saved on the event never reaches a fact, a snapshot or a command.
		expect(
			JSON.stringify([api.hub.streams.get(edge), api.commands, agent.rows()]),
		).not.toContain("tok-telegram");
		// An agent without a bot flag sends no bot list.
		lacking("telegram_bots", "discord_bots");
		const wire = agent.rows().find((value) => value.id === "kinds");
		expect(wire).not.toHaveProperty("bots");
	});

	test("an online service claims its bots with its schedules, and connects a bot only after a person released it", async () => {
		fake = await createFakeWorkspace(sampleFleet(), { unlock: [edge] });
		const { api, workspace } = fake;
		const agent = api.agent(edge);
		agent.features = { ...agent.features, ...R2_FLAGS };
		const invoice = APPS.app_invoice_ai;
		const bot = kindsEvent(
			"evt_invoice_bot",
			"Invoice bot",
			"discord",
			[1, 0, 0],
			[2, 2, 0],
			{
				channel_whitelist: ["123"],
			},
		);
		api.hub.apps = {
			...api.hub.apps,
			[invoice.id]: { ...invoice, events: [...invoice.events, bot] },
		};
		const call = workspace.live.call(edge);
		const base = seeded(agent.placement("invoice-extractor"));
		const existing = await readExistingDeployment(
			call,
			"invoice-extractor",
			invoice.id,
		);
		await call({
			type: "apply",
			config: {
				...existing.config,
				events: [
					{
						event_id: "evt_invoice_reconcile",
						event_version: [1, 0, 0],
						board_version: [1, 3, 0],
					},
					pin(bot),
				],
				secret_overrides: { [tokenKey(bot.id)]: "bot-secret" },
			},
			expected_revision: base.config_revision,
			start: true,
		});
		const row = () => seeded(agent.placement("invoice-extractor"));
		expect(row().bots).toMatchObject([
			{
				event_id: bot.id,
				provider: "discord",
				state: "waiting",
				hold: "not_released",
			},
		]);
		expect(row().schedules).toMatchObject([
			{ event_id: "evt_invoice_reconcile", hold: "not_released" },
		]);
		api.hub.schedules.release(invoice.id, bot.id, edge, "invoice-extractor");
		await call({
			type: "restart",
			placement_id: "invoice-extractor",
			expected_revision: row().config_revision,
		});
		expect(row().bots?.[0]).toMatchObject({ state: "connected", hold: null });
		expect(api.hub.schedules.listing(invoice.id)).toMatchObject([
			{ event_id: bot.id, state: "device", device_id: edge },
		]);
		// One place per bot: it can't be released to another service while this one holds it.
		expect(() =>
			api.hub.schedules.release(invoice.id, bot.id, studio, "invoice-reports"),
		).toThrow(`Schedule ${bot.id} runs on another service`);
		const listing = await listAppDevicePlacements(api, api.profile, invoice.id);
		expect(data(listing).schedules).toMatchObject([
			{ event_id: bot.id, state: "device" },
		]);
	});

	test("a one-time schedule runs once, also across a stop and a start; one deployed after its time has passed", async () => {
		const ahead = kindsEvent("evt_soon", "Soon", "cron", [1, 0, 0], [1, 0, 0], {
			scheduled_for: { date: "2026-10-01", time: "09:00" },
			timezone: "Europe/Berlin",
		});
		const { api, agent, call, hub, deploy, row } = await onEdge([ahead]);
		await deploy([KIND_EVENTS.once]);
		expect(row().schedules).toEqual([
			{
				event_id: "evt_once",
				once_at: 1790233200,
				timezone: "Europe/Berlin",
				once_state: "passed",
				hold: null,
				next_at: null,
				running: false,
				last_at: null,
				last_outcome: null,
			},
		]);
		await deploy([ahead], { id: "soon" });
		const at = Date.UTC(2026, 9, 1, 7) / 1000;
		expect(row("soon").schedules?.[0]).toMatchObject({
			once_at: at,
			once_state: "pending",
			next_at: at,
		});
		// Its time comes while it runs (by the device's clock): it runs, once.
		agent.online = true;
		hub.now = () => at + 60;
		agent.rows();
		expect(row("soon").schedules?.[0]).toMatchObject({
			once_state: "ran",
			last_outcome: "succeeded",
			next_at: null,
		});
		await call({
			type: "stop",
			placement_id: "soon",
			expected_revision: row("soon").config_revision,
		});
		// A stopped service still reports the finished one, and nothing else.
		expect(row("soon").schedules).toMatchObject([
			{ event_id: "evt_soon", once_state: "ran" },
		]);
		await call({
			type: "start",
			placement_id: "soon",
			expected_revision: row("soon").config_revision,
		});
		expect(row("soon").schedules?.[0]).toMatchObject({
			once_state: "ran",
			last_at: at + 60,
		});
		api.hub.publishStatus(edge, agent);
		const snapshot = (
			api.hub.streams.get(edge)?.at(-1)?.payload as {
				inspection: { placements: PlacementStatusPlus[] };
			}
		).inspection.placements.find((value) => value.id === "soon");
		expect(snapshot?.schedules).toEqual([
			{
				event_id: "evt_soon",
				once_at: at,
				timezone: "Europe/Berlin",
				once_state: "ran",
				hold: null,
				last_outcome: "succeeded",
			},
		]);
	});

	test("a one-time schedule whose service was down over its time is missed", async () => {
		const ahead = kindsEvent("evt_soon", "Soon", "cron", [1, 0, 0], [1, 0, 0], {
			scheduled_for: { date: "2026-10-01", time: "09:00" },
			timezone: "Europe/Berlin",
		});
		const { agent, call, hub, deploy, row } = await onEdge([ahead]);
		agent.online = true;
		await deploy([ahead]);
		await call({
			type: "stop",
			placement_id: "kinds",
			expected_revision: row().config_revision,
		});
		expect(row().schedules).toBeUndefined();
		const at = Date.UTC(2026, 9, 1, 7) / 1000;
		hub.now = () => at + 901;
		await call({
			type: "start",
			placement_id: "kinds",
			expected_revision: row().config_revision,
		});
		expect(row().schedules?.[0]).toMatchObject({
			once_state: "missed",
			hold: null,
		});
	});

	test("a person-started run: accepted at once, read until it ends, counted on the row", async () => {
		const { agent, api, call, deploy, row } = await onEdge();
		await deploy([KIND_EVENTS.form]);
		const revision = row().config_revision;
		// The literals of design §1.6c and §1.8, for this service and revision.
		expect(row().actions).toEqual([
			{
				event_id: "evt_notes_form",
				kind: "form",
				fields: 1,
				file_fields: 0,
				running: 0,
				last_at: null,
				last_outcome: null,
				runs: 0,
				failed: 0,
			},
		]);
		const form = await call({
			type: "event_form",
			placement_id: "kinds",
			event_id: "evt_notes_form",
		});
		expect(form.state).toBe("completed");
		expect(form.result).toEqual({
			placement_id: "kinds",
			config_revision: revision,
			event_id: "evt_notes_form",
			event_version: [1, 0, 0],
			board_version: [3, 0, 1],
			kind: "form",
			name: "New note",
			description: "",
			fields: [
				{
					name: "title",
					label: "Title",
					description: "",
					data_type: "String",
					value_type: "Normal",
					optional: false,
					sensitive: false,
					default: null,
					options: null,
				},
			],
			fields_truncated: false,
			file_fields: 0,
			navigate_to_routes: [],
		});

		agent.runs.script = {
			queuedReads: 1,
			runningReads: 1,
			end: { output: { id: 42 } },
		};
		const sent = await call({
			type: "run_event",
			placement_id: "kinds",
			event_id: "evt_notes_form",
			expected_revision: revision,
			payload: { title: "Hello" },
		});
		expect(sent.state).toBe("accepted");
		expect(sent.result).toMatchObject({
			command: "run_event",
			placement_id: "kinds",
			event_id: "evt_notes_form",
			run: "queued",
		});
		const operationId = sent.operation_id as string;
		const read = () => call({ type: "operation", operation_id: operationId });
		expect((await read()).result.run).toBe("queued");
		const running = await read();
		expect([
			running.state,
			running.result.run,
			running.result.started_at,
		]).toEqual(["accepted", "running", SAMPLE_NOW]);
		expect(row().actions?.[0]?.running).toBe(1);
		const done = await read();
		expect(done).toMatchObject({
			operation_id: operationId,
			state: "completed",
			result: {
				run: "succeeded",
				started_at: SAMPLE_NOW,
				finished_at: SAMPLE_NOW,
				output_bytes: 9,
				truncated: false,
				attachments: 0,
				output: { json: { id: 42 } },
			},
		});
		expect(row().actions?.[0]).toMatchObject({
			running: 0,
			runs: 1,
			failed: 0,
			last_at: SAMPLE_NOW,
			last_outcome: "succeeded",
		});
		// The journal keeps the row without the output; the owner's list names who ran what.
		expect(agent.journal.get(operationId)?.result).not.toHaveProperty("output");
		const listed = (await call({ type: "operations", limit: 20 })).result
			.operations as Record<string, unknown>[];
		expect(listed.at(-1)).toMatchObject({
			kind: "run_event",
			event_id: "evt_notes_form",
			state: "completed",
		});
		// The agent restarted: the row stays, the output is gone.
		agent.restartAgent();
		expect(agent.sessions).toBe(0);
		const after = await call({ type: "operation", operation_id: operationId });
		expect(after.result).toMatchObject({ run: "succeeded", output_gone: true });
		expect(after.result).not.toHaveProperty("output");
		expect(JSON.stringify(api.commands)).toContain('"title":"Hello"');
	});

	test("a run ends with each failure the device reports, and Stop this run cancels it", async () => {
		const { agent, call, deploy, row } = await onEdge();
		await deploy([KIND_EVENTS.form]);
		const revision = row().config_revision;
		const start = async (payload: Record<string, unknown> = { title: "x" }) =>
			(
				await call({
					type: "run_event",
					placement_id: "kinds",
					event_id: "evt_notes_form",
					expected_revision: revision,
					payload,
				})
			).operation_id as string;
		const finish = async (operationId: string) => {
			for (let index = 0; index < 5; index++) {
				const answer = await call({
					type: "operation",
					operation_id: operationId,
				});
				if (answer.state !== "accepted") return answer;
			}
			throw new Error("The run did not end.");
		};
		// The device checks the fields itself.
		expect(
			(await finish(await start({ title: 7, colour: "red" }))).result,
		).toMatchObject({
			run: "failed",
			code: "invalid_fields",
			fields: ["colour", "title"],
		});
		expect((await finish(await start({}))).result).toMatchObject({
			code: "invalid_fields",
			fields: ["title"],
		});
		for (const code of [
			"flow_failed",
			"timed_out",
			"interrupted",
			"not_started",
			"needs_interaction",
		] as const) {
			agent.runs.script = { end: { code } };
			const answer = await finish(await start());
			expect([answer.state, answer.result.run, answer.result.code]).toEqual([
				"failed",
				code === "timed_out" ? "timed_out" : "failed",
				code,
			]);
			expect(answer.result).not.toHaveProperty("output");
		}
		// A result larger than 8 KiB as it travels is cut until it fits.
		agent.runs.script = { runningReads: 0, end: { text: '"'.repeat(9_000) } };
		const cut = (await finish(await start())).result;
		expect(cut.truncated).toBe(true);
		expect(
			new TextEncoder().encode(JSON.stringify(cut.output)).length,
		).toBeLessThanOrEqual(8_192);
		// Stop this run.
		agent.runs.script = { runningReads: null };
		const open = await start();
		await call({ type: "operation", operation_id: open });
		expect(
			(await call({ type: "cancel_run", operation_id: open })).result,
		).toEqual({ command: "cancel_run", cancelled: true });
		expect((await finish(open)).result).toMatchObject({
			run: "cancelled",
			code: "cancelled",
		});
		expect(
			(await call({ type: "cancel_run", operation_id: open })).result,
		).toEqual({ command: "cancel_run", cancelled: false });
		expect(
			managementRejection(
				await call({ type: "cancel_run", operation_id: "op-unknown" }),
			)?.code,
		).toBe("invalid");
		// A test ends an open run when it wants to.
		const manual = await start();
		expect(agent.runs.end(manual, { output: { ok: true } })).toBe(true);
		expect((await finish(manual)).result.output).toEqual({
			json: { ok: true },
		});
		// A stop cuts a running run off.
		const cutOff = await start();
		await call({ type: "operation", operation_id: cutOff });
		await call({
			type: "stop",
			placement_id: "kinds",
			expected_revision: revision,
		});
		expect((await finish(cutOff)).result).toMatchObject({
			run: "failed",
			code: "interrupted",
		});
	});

	test("run_event is refused before anything is journaled when the device can't take it", async () => {
		const { agent, call, deploy, row, lacking } = await onEdge([
			kindsEvent("evt_ping", "Ping", "quick_action", [1, 0, 0], [1, 0, 0]),
		]);
		await deploy([KIND_EVENTS.form, KIND_EVENTS.orders]);
		const revision = row().config_revision;
		const send = (command: Record<string, unknown>) =>
			call({
				type: "run_event",
				placement_id: "kinds",
				event_id: "evt_notes_form",
				expected_revision: revision,
				payload: { title: "x" },
				...command,
			});
		const code = async (command: Record<string, unknown>) =>
			managementRejection(await send(command))?.code;
		expect(await code({ expected_revision: revision + 1 })).toBe(
			"revision_conflict",
		);
		expect(await code({ event_id: "evt_orders" })).toBe("invalid");
		expect(await code({ event_id: "evt_ping" })).toBe("invalid");
		expect(await code({ payload: ["x"] })).toBe("invalid");
		expect(
			await code({
				payload: Object.fromEntries(
					Array.from({ length: 65 }, (_, index) => [`k${index}`, 1]),
				),
			}),
		).toBe("invalid");
		expect(await code({ payload: { title: "x".repeat(12_300) } })).toBe(
			"invalid",
		);
		agent.runs.script = { runningReads: null };
		for (let index = 0; index < 12; index++)
			expect((await send({})).state).toBe("accepted");
		const busy = managementRejection(await send({}));
		expect([busy?.code, busy?.retryable]).toEqual(["busy", true]);
		const refused = await send({ expected_revision: 0 });
		expect(
			managementRejection(
				await call({
					type: "operation",
					operation_id: refused.operation_id as string,
				}),
			)?.code,
		).toBe("invalid");
		await call({
			type: "stop",
			placement_id: "kinds",
			expected_revision: revision,
		});
		expect(await code({})).toBe("revision_conflict");
		expect(
			managementRejection(
				await call({
					type: "event_form",
					placement_id: "kinds",
					event_id: "evt_notes_form",
				}),
			)?.code,
		).toBe("revision_conflict");
		lacking("on_demand_events");
		for (const command of [
			{ type: "run_event", placement_id: "kinds" },
			{ type: "cancel_run", operation_id: "op" },
			{ type: "event_form", placement_id: "kinds", event_id: "evt_notes_form" },
		])
			expect(managementRejection(await call(command))?.code).toBe(
				"unsupported",
			);
	});

	test("a sensitive field's default never leaves the device, and its value never reaches the recorded commands", async () => {
		const { api, call, deploy, row } = await onEdge();
		api.hub.eventForms.evt_notes_form = {
			fields: [
				formField("title"),
				formField("pin", { sensitive: true, optional: true, default: "1234" }),
				formField("photo", { data_type: "PathBuf", optional: true }),
			],
		};
		await deploy([KIND_EVENTS.form]);
		const form = await call({
			type: "event_form",
			placement_id: "kinds",
			event_id: "evt_notes_form",
		});
		expect(form.result).toMatchObject({ kind: "form", file_fields: 1 });
		expect(
			(form.result.fields as { name: string; default: unknown }[]).find(
				(field) => field.name === "pin",
			)?.default,
		).toBeNull();
		expect(row().actions?.[0]).toMatchObject({ fields: 3, file_fields: 1 });
		await call({
			type: "run_event",
			placement_id: "kinds",
			event_id: "evt_notes_form",
			expected_revision: row().config_revision,
			payload: { title: "visible", pin: "9876" },
		});
		const recorded = JSON.stringify(api.commands);
		expect(recorded).toContain("visible");
		expect(recorded).not.toContain("9876");
		expect(recorded).not.toContain("1234");
		// A file never fits a run from Devices.
		const sent = await call({
			type: "run_event",
			placement_id: "kinds",
			event_id: "evt_notes_form",
			expected_revision: row().config_revision,
			payload: { title: "x", photo: "data:image/png;base64,AAAA" },
		});
		for (let index = 0; index < 3; index++)
			await call({
				type: "operation",
				operation_id: sent.operation_id as string,
			});
		expect(
			(
				await call({
					type: "operation",
					operation_id: sent.operation_id as string,
				})
			).result,
		).toMatchObject({ code: "invalid_fields", fields: ["photo"] });
	});

	test("scenarios: Shop Assistant with one event of each kind, and Support Portal's quick action, on edge-berlin-01", async () => {
		fake = await createFakeWorkspace(sampleFleet(), { unlock: [edge] });
		const shop = await serveShopOnEdge(fake);
		expect(shop).toMatchObject({
			observed_state: "running",
			source: "online",
			bots: [
				{ event_id: SHOP.telegram, state: "connected", hold: null },
				{ event_id: SHOP.discord, state: "connected", hold: null },
			],
			schedules: [
				{
					event_id: SHOP.once,
					once_state: "pending",
					next_at: SHOP_ONCE_AT,
				},
			],
			actions: [
				{ event_id: SHOP.form, kind: "form", fields: 3, file_fields: 1 },
			],
		});
		expect(fake.hub.schedules.listing(SHOP.app)).toMatchObject([
			{ event_id: SHOP.telegram, state: "device", device_id: edge },
			{ event_id: SHOP.discord, state: "device", device_id: edge },
			{ event_id: SHOP.once, state: "device", device_id: edge },
		]);
		const placements = data(
			await listAppDevicePlacements(fake.api, fake.api.profile, SHOP.app),
		);
		expect(placements.placements.map((row) => row.placement_id)).toEqual([
			SHOP.service,
		]);
		const reply = await serveQuickReplyOnEdge(fake);
		expect(reply?.actions).toEqual([
			{
				event_id: QUICK_REPLY.event,
				kind: "action",
				fields: 0,
				file_fields: 0,
				running: 0,
				last_at: null,
				last_outcome: null,
				runs: 0,
				failed: 0,
			},
		]);
		expect(reply?.observed_state).toBe("running");
		// Unreleased, the service holds what moves off the hub.
		await fake.dispose();
		fake = await createFakeWorkspace(sampleFleet(), { unlock: [edge] });
		const held = await serveShopOnEdge(fake, {
			events: [SHOP.telegram],
			release: false,
		});
		expect(held?.bots).toMatchObject([
			{ event_id: SHOP.telegram, state: "waiting", hold: "not_released" },
		]);
	});

	test("the client library reads what the fake answers: the form, a run, its stop, the row facts", async () => {
		const { agent, call, deploy, row } = await onEdge();
		await deploy([KIND_EVENTS.form, KIND_EVENTS.helper], {
			secrets: { [tokenKey("evt_helper")]: "bot-helper" },
		});
		const placement = { placementId: "kinds", eventId: "evt_notes_form" };
		const form = await readEventForm(call, agent.features, placement);
		expect(form).toMatchObject({
			kind: "ok",
			data: { kind: "form", fields: [{ name: "title", data_type: "String" }] },
		});
		agent.runs.script = { runningReads: 1, end: { text: "Saved." } };
		const started = await runEvent(
			call,
			{
				...placement,
				expectedRevision: row().config_revision,
				payload: { title: "Hi" },
			},
			"op-run-1",
		);
		expect(started).toMatchObject({
			kind: "accepted",
			run: { operationId: "op-run-1", run: "queued" },
		});
		expect(await readRun(call, "op-run-1")).toMatchObject({ run: "running" });
		expect(await readRun(call, "op-run-1")).toMatchObject({
			run: "succeeded",
			output: { text: "Saved." },
			outputBytes: 6,
		});
		expect(await readRun(call, "op-unknown")).toBeNull();
		agent.runs.script = { runningReads: null };
		await runEvent(
			call,
			{
				...placement,
				expectedRevision: row().config_revision,
				payload: { title: "x" },
			},
			"op-run-2",
		);
		expect(await cancelRun(call, "op-run-2")).toBe(true);
		expect(await readRun(call, "op-run-2")).toMatchObject({
			run: "cancelled",
			code: "cancelled",
		});
		expect(await cancelRun(call, "op-run-2")).toBe(false);
		const refused = await runEvent(
			call,
			{ ...placement, expectedRevision: 0, payload: { title: "x" } },
			"op-run-3",
		);
		expect(refused).toMatchObject({
			kind: "rejected",
			code: "revision_conflict",
		});
		// The device took the run but its answer was lost: the run is found by its id, and a
		// repeated send answers that run instead of starting a second one.
		agent.loseReplyNext("run_event");
		const input = {
			...placement,
			expectedRevision: row().config_revision,
			payload: { title: "lost" },
		};
		await expect(runEvent(call, input, "op-run-4")).rejects.toBeInstanceOf(
			ManagementUnconfirmedError,
		);
		const found = await readRun(call, "op-run-4");
		expect(found).toMatchObject({ operationId: "op-run-4", run: "running" });
		const again = await runEvent(call, input, "op-run-4");
		expect(again).toMatchObject({
			kind: "accepted",
			run: { runId: found?.runId },
		});
		expect(agent.runs.open("kinds")).toHaveLength(1);
		// The live row as the client parses it.
		await (fake as FakeWorkspace).workspace.live.refreshInspection(edge);
		const parsed = (fake as FakeWorkspace).workspace.live
			.inspection(edge)
			?.value.placements.find((value) => value.id === "kinds");
		expect(parsed?.bots).toMatchObject([
			{ event_id: "evt_helper", state: "connected", hold: null },
		]);
		expect(parsed?.actions).toMatchObject([
			{ event_id: "evt_notes_form", kind: "form", runs: 2, failed: 0 },
		]);
		// Without the flag the client never asks.
		expect(await readEventForm(call, {}, placement)).toEqual({
			kind: "unsupported",
			feature: "on_demand_events",
		});
	});

	test("a form answer keeps 64 fields at most and 12 KiB as it is sent", () => {
		const head = {
			placement_id: "kinds",
			config_revision: 1,
			event_id: "evt_big",
			event_version: [1, 0, 0],
			board_version: [1, 0, 0],
			name: "Big",
		};
		const many = formAnswer(head, {
			fields: Array.from({ length: 80 }, (_, index) => formField(`f${index}`)),
		});
		expect([(many.fields as unknown[]).length, many.fields_truncated]).toEqual([
			64,
			true,
		]);
		const long = formAnswer(head, {
			fields: Array.from({ length: 30 }, (_, index) =>
				formField(`f${index}`, { description: "d".repeat(480) }),
			),
		});
		expect(long.fields_truncated).toBe(true);
		expect(
			new TextEncoder().encode(JSON.stringify(long)).length,
		).toBeLessThanOrEqual(12 * 1024);
		expect(formAnswer(head, { fields: [] })).toMatchObject({
			kind: "action",
			fields: [],
			fields_truncated: false,
		});
	});

	test("a row's lists keep their counts and share 4 KiB: actions go first, then schedules, then bots", async () => {
		const forms = Array.from({ length: 20 }, (_, index) =>
			kindsEvent(
				`evt_form_${String(index).padStart(2, "0")}`,
				`Form ${index}`,
				"generic_form",
				[1, 0, 0],
				[1, 0, 0],
			),
		);
		const { deploy, row } = await onEdge(forms);
		await deploy(forms);
		expect([row().actions?.length, row().actions_truncated]).toEqual([
			16,
			true,
		]);
		expect(row().schedules).toBeUndefined();

		const long = (prefix: string, index: number) =>
			`${prefix}_${String(index).padStart(2, "0")}_${"x".repeat(110)}`;
		const action = (index: number): ServiceAction => ({
			event_id: long("evt_action", index),
			kind: "action",
			fields: 0,
			file_fields: 0,
		});
		const schedule = (index: number): ServiceSchedule => ({
			event_id: long("evt_schedule", index),
			expression: "0 0 * * * *",
			timezone: "UTC",
			hold: null,
		});
		const bot = (index: number): ServiceBot => ({
			event_id: long("evt_bot", index),
			provider: "telegram",
			state: "ok",
			hold: null,
		});
		const lists = boundedLists({
			schedules: Array.from({ length: 16 }, (_, index) => schedule(index)),
			bots: Array.from({ length: 8 }, (_, index) => bot(index)),
			actions: Array.from({ length: 10 }, (_, index) => action(index)),
		});
		expect(
			new TextEncoder().encode(
				JSON.stringify([lists.actions, lists.schedules, lists.bots]),
			).length,
		).toBeLessThanOrEqual(4_096);
		expect([lists.actions?.length, lists.actions_truncated]).toEqual([0, true]);
		expect(lists.schedules?.length).toBeLessThan(16);
		expect(lists.schedules_truncated).toBe(true);
		expect([lists.bots?.length, lists.bots_truncated]).toEqual([8, false]);
		expect(boundedLists({ bots: [bot(0)] })).toEqual({
			bots: [bot(0)],
			bots_truncated: false,
		});
	});
});

describe("fake workspace", () => {
	test("the golden fleet opens through the real managers", async () => {
		fake = await createFakeWorkspace(sampleFleet());
		const { workspace } = fake;
		expect(
			[edge, warehouse, studio, lab, cold, oldKiosk].map(
				(id) => workspace.keys.snapshot(id).state,
			),
		).toEqual([
			"unlocked",
			"unlocked",
			"unlocked",
			"locked",
			"locked",
			"locked",
		]);
		expect(workspace.live.state(edge).kind).toBe("live");
		expect(
			workspace.live.inspection(edge)?.value.placements.map((row) => row.id),
		).toEqual(["invoice-extractor", "nightly-sync", "support-bot"]);
		const status = workspace.fleet.get(warehouse);
		expect([status?.status?.sequence, status?.saved?.observedAt]).toEqual([
			2_210, 1_790_676_000,
		]);
		expect(workspace.local.summary().vaults).toHaveLength(7);
		expect(Date.now()).toBe(SAMPLE_NOW * 1000);
	});

	test("a wrong password, a lock held in another window and a lock release the keys", async () => {
		fake = await createFakeWorkspace(sampleFleet(), { unlock: "none" });
		const { workspace } = fake;
		const failure = await workspace.keys
			.unlock(edge, "not the password")
			.catch((error: unknown) => error);
		expect((failure as KeyFailure).keyError.code).toBe("wrong_password");
		const letGo = fake.holdElsewhere(cold);
		const held = await workspace.keys
			.unlock(cold, fake.password)
			.catch((error: unknown) => error);
		expect((held as KeyFailure).keyError.code).toBe("held_elsewhere");
		letGo();
		await fake.unlock(warehouse);
		expect(workspace.keys.snapshot(warehouse).state).toBe("unlocked");
		workspace.keys.lockAll();
		expect(fake.crypto.controllers.length).toBeGreaterThan(0);
		expect(
			fake.crypto.controllers.every(
				(controller) => controller.closed && controller.freed,
			),
		).toBe(true);
		expect(failure).toBeInstanceOf(KeySessionError);
	});

	test("an open owner session signs without the password; an older crypto bundle asks for it", async () => {
		const policy: ManagementPolicy = {
			version: 1,
			device_id: warehouse,
			policy_version: 1,
			previous_policy_digest: null,
			grants: [],
			issued_at: SAMPLE_NOW,
			expires_at: SAMPLE_NOW + 86_400,
		};
		fake = await createFakeWorkspace(sampleFleet(), { unlock: [warehouse] });
		expect(fake.workspace.keys.snapshot(warehouse).canSign).toBe(true);
		const signed = await fake.workspace.keys
			.signer(warehouse)
			?.signPolicy(policy);
		expect(
			fake.crypto.verifyManagementPolicy(
				signed ?? "",
				fake.hub.ownerInvitationKey(warehouse),
			),
		).toEqual(policy);
		const view = await fake.api.put<{ version: number }>(
			fake.profile,
			`devices/${warehouse}/management/policy`,
			{ policy_jws: signed },
		);
		expect(view.version).toBe(1);
		await fake.dispose();

		fake = await createFakeWorkspace(sampleFleet(), {
			unlock: [warehouse],
			heldSigner: false,
		});
		const signer = fake.workspace.keys.signer(warehouse);
		expect(fake.workspace.keys.snapshot(warehouse).canSign).toBe(false);
		await expect(signer?.signPolicy(policy)).rejects.toBeInstanceOf(
			OwnerPasswordRequiredError,
		);
		expect(await signer?.signPolicy(policy, fake.password)).toBe(signed);
	});

	test("no secret reaches the recorded calls and commands", async () => {
		fake = await createFakeWorkspace(sampleFleet(), { unlock: "none" });
		const { workspace, api } = fake;
		await fake.unlock(cold, { backupToAccount: true });
		await fake.unlock(edge, { connectLive: true });
		const call = workspace.live.call(edge);
		await call({
			type: "set_secret",
			placement_id: "support-bot",
			expected_revision: 8,
			name: "variable-1",
			value: "s3cr3t-value",
		});
		await call({
			type: "put_certificate",
			certificate_id: "24f6fe22-c6c2-4e15-9d37-7a41a379afb9",
			label: "edge-api",
			expected_revision: 4,
			certificate_chain_pem: "-----BEGIN CERTIFICATE-----",
			private_key_pem: "-----BEGIN PRIVATE KEY-----",
		});
		expect(api.sent("PUT", `devices/controller-vaults/${cold}`)).toHaveLength(
			1,
		);
		const recorded = JSON.stringify([api.calls, api.commands]);
		expect(recorded).toContain("set_secret");
		for (const secret of [fake.password, "s3cr3t-value", "PRIVATE KEY"])
			expect(recorded).not.toContain(secret);
	});

	test("the fakes use no module mocks", () => {
		const banned = ["mock", "module("].join(".");
		for (const file of [
			"fake-device-api.ts",
			"fake-workspace.ts",
			"fake-schedules.ts",
			"fake-runs.ts",
			"fake-sinks.ts",
			"mount-devices.tsx",
			"schedule-scenarios.ts",
			"fakes.test.ts",
		])
			expect(readFileSync(join(import.meta.dir, file), "utf8")).not.toContain(
				banned,
			);
	});
});

describe("mountDevices", () => {
	test("hooks read the golden fleet: 2 critical · 15 total", async () => {
		const { useDeviceRows, useReleaseTrust } = await import(
			"../workspace/use-hub"
		);
		const { useAttention, useAttentionCounts } = await import(
			"../workspace/use-attention"
		);
		const seen: {
			counts?: unknown;
			items?: AttentionItem[];
			release?: string;
		} = {};
		function Probe() {
			seen.counts = useAttentionCounts();
			seen.items = useAttention();
			seen.release = useReleaseTrust().data?.manifest.release_version;
			return createElement("output", null, useDeviceRows().rows?.length);
		}
		const view = await mountDevices(createElement(Probe));
		expect(view.container.textContent).toBe("7");
		expect(seen.counts).toEqual({
			critical: 2,
			warning: 8,
			notice: 5,
			info: 8,
			total: 15,
		});
		expect(seen.items?.slice(0, 2).map((item) => item.key)).toEqual([
			"service_crash_looping",
			"offline_since",
		]);
		expect(seen.release).toBe("0.9.4");
		expect(view.fake.api.writes().map(([method]) => method)).toEqual([
			"PUT",
			"PUT",
			"PUT",
		]);

		// Renewing a certificate happens on the device; renewing a person's access is a page to go to.
		const renewals = (seen.items ?? []).filter(
			(item) => item.action?.code === "renew",
		);
		const offlineCertificate = renewals.find(
			(item) =>
				item.subject.kind === "certificate" &&
				item.subject.deviceId === warehouse,
		);
		expect(offlineCertificate?.key).toBe("certificate_expired");
		expect(offlineCertificate?.action?.gate).toMatchObject({ ok: false });
		const access = renewals.find((item) => item.key === "grant_expiring");
		expect(access?.subject.kind).not.toBe("certificate");
		expect(access?.action?.gate).toBeUndefined();
	});

	test("a hub copy of the access rules reads verified, pending or rejected", async () => {
		const { usePolicy } = await import("../workspace/use-hub");
		const { useAttentionState } = await import("../workspace/use-attention");
		type Read = ReturnType<typeof usePolicy>;
		const seen: {
			view?: Read["data"];
			state?: ReturnType<typeof useAttentionState>["policyState"];
		} = {};
		function Probe() {
			const read = usePolicy(edge);
			seen.view = read.data;
			seen.state = useAttentionState().policyState;
			return createElement("output", null, read.verification ?? "unread");
		}
		const view = await mountDevices(createElement(Probe));
		for (
			let round = 0;
			round < 40 && view.container.textContent !== "verified";
			round++
		)
			await view.settle();
		expect(view.container.textContent).toBe("verified");

		const copy = seen.view as NonNullable<Read["data"]>;
		const state = seen.state as NonNullable<typeof seen.state>;
		const forged = {
			...copy,
			policy_jws: `${copy.policy_jws}x`,
			digest: "forged",
		};
		expect(state(edge, forged)).toBe("rejected");
		expect(state(lab, copy)).toBe("pending");
		expect(
			state(edge, { ...copy, policy_jws: null, version: 0, digest: null }),
		).toBe("pending");
	});

	test("the app view knows what a shared device's access covers before any unlock", async () => {
		const { useAppView } = await import("../workspace/use-app");
		type View = NonNullable<ReturnType<typeof useAppView>["view"]>;
		const seen: Record<string, View | undefined> = {};
		function Probe() {
			seen.covered = useAppView(SAMPLE_APPS.invoiceAi).view;
			seen.other = useAppView(SAMPLE_APPS.supportPortal).view;
			return createElement("output", null, seen.other ? "read" : "loading");
		}
		const view = await mountDevices(createElement(Probe), { unlock: "none" });
		expect(view.container.textContent).toBe("read");
		const asked = view.fake.api.calls
			.map(([method, path]) => `${method} ${path}`)
			.filter((call) => call.endsWith("/my-access"));
		expect(asked).toEqual([`GET devices/${lab}/management/my-access`]);

		const idsOf = (rows: readonly { deviceId: string }[]) =>
			rows.map((row) => row.deviceId);
		// lab-gpu-02 is shared for Invoice AI only: for another app it says nothing, locked or not.
		const other = seen.other as View;
		expect(idsOf(other.everywhereElse.noAccess)).toContain(lab);
		expect(idsOf(other.everywhereElse.unknown)).not.toContain(lab);
		const covered = seen.covered as View;
		expect(idsOf(covered.everywhereElse.noAccess)).not.toContain(lab);
	});

	test("on an old hub the hooks report what is missing and still render", async () => {
		const { useAppPlacements, useDeviceRows, useDeviceUsage } = await import(
			"../workspace/use-hub"
		);
		const { useAttentionCounts } = await import("../workspace/use-attention");
		const seen: { missing?: boolean[]; critical?: number } = {};
		function Probe() {
			const rows = useDeviceRows().rows ?? [];
			seen.missing = [
				useDeviceUsage().missingOnHub,
				useAppPlacements(SAMPLE_APPS.invoiceAi).missingOnHub,
			];
			seen.critical = useAttentionCounts().critical;
			return createElement(
				"output",
				null,
				rows.map((row) => row.relationship ?? "unknown").join(" "),
			);
		}
		const view = await mountDevices(createElement(Probe), {
			hubVersion: "old",
		});
		expect(view.container.textContent).toBe(
			Array.from({ length: 7 }, () => "unknown").join(" "),
		);
		expect(seen.missing).toEqual([true, true]);
		expect(seen.critical).toBe(2);
	});

	test("navigation is recorded, a signed-out host shows the sign-in gate, a node can mount its own provider", async () => {
		const { useDevicesRoute } = await import("../routing/use-devices-route");
		const { DeviceWorkspaceProvider, useDeviceScope } = await import(
			"../workspace/device-workspace-provider"
		);
		let route: ReturnType<typeof useDevicesRoute> | undefined;
		function RouteProbe() {
			route = useDevicesRoute();
			return createElement("output", null, route.route.screen);
		}
		const routed = await mountDevices(createElement(RouteProbe), {
			unlock: "none",
		});
		await act(async () =>
			route?.navigate({ screen: "device", deviceId: edge, tab: "overview" }),
		);
		expect(routed.container.textContent).toBe("device");
		expect(routed.navigations.map((entry) => entry.mode)).toEqual(["push"]);
		expect(routed.navigations[0]?.href).toContain(edge);
		await routed.unmount();

		const signedOut = await mountDevices(
			createElement("output", null, "screen"),
			{ unlock: "none", signedIn: false },
		);
		expect(signedOut.container.textContent).not.toContain("screen");
		await click(byRole("button", "Sign in"));
		expect(signedOut.signIns()).toBe(1);
		await signedOut.unmount();

		function ScopeProbe() {
			return createElement("output", null, useDeviceScope()?.account);
		}
		const own = await mountDevices(
			({ overrides }) =>
				createElement(
					DeviceWorkspaceProvider,
					{ overrides } as DeviceWorkspaceProviderProps,
					createElement(ScopeProbe),
				),
			{ unlock: "none", providers: false },
		);
		expect(own.container.textContent).toBe(SAMPLE_ME);
	});

	test("a provider without overrides binds to the fake workspace through the registry", async () => {
		const { DeviceWorkspaceProvider, useDeviceWorkspace } = await import(
			"../workspace/device-workspace-provider"
		);
		let bound: unknown;
		function WorkspaceProbe() {
			bound = useDeviceWorkspace();
			return createElement("output", null, "bound");
		}
		const view = await mountDevices(
			createElement(
				DeviceWorkspaceProvider,
				{} as DeviceWorkspaceProviderProps,
				createElement(WorkspaceProbe),
			),
			{ unlock: [warehouse], providers: false },
		);
		expect(view.container.textContent).toBe("bound");
		expect(bound).toBe(view.fake.workspace);
		expect(view.fake.workspace.keys.snapshot(warehouse).state).toBe("unlocked");
	});

	test("the viewer owns every app unless the test gives another role, or none", async () => {
		const { useBackendStore } = await import("../../../../state/backend-state");
		const { useAppPermissions } = await import(
			"../../../../hooks/use-app-permissions"
		);
		const backend = () => useBackendStore.getState().backend as IBackendState;
		const seen: { isOwner?: boolean } = {};
		function RoleProbe() {
			seen.isOwner = useAppPermissions(APPS.app_invoice_ai.id).isOwner;
			return createElement("output", null, "role");
		}
		const owner = await mountDevices(createElement(RoleProbe), {
			unlock: "none",
		});
		expect(await backend().roleState.getOwnRole("any_app")).toEqual({
			role_id: "role_owner",
			role_name: "Owner",
			permissions: 1,
			is_owner: true,
			can_leave: false,
		});
		expect(seen.isOwner).toBe(true);
		await owner.unmount();

		const member = {
			role_id: "role_member",
			role_name: "Member",
			permissions: 0,
			is_owner: false,
			can_leave: true,
		};
		const other = await mountDevices(createElement("output"), {
			unlock: "none",
			backend: {
				roleState: {
					getOwnRole: async () => member,
				} as unknown as IBackendState["roleState"],
			},
		});
		expect(await backend().roleState.getOwnRole("any_app")).toBe(member);
		await other.unmount();

		await mountDevices(createElement("output"), {
			unlock: "none",
			backend: { roleState: undefined },
		});
		expect(backend().roleState).toBeUndefined();
	});

	test("this computer's triggers: the desktop lists and stops them, the web has none", async () => {
		const { useBackendStore } = await import("../../../../state/backend-state");
		const backend = () => useBackendStore.getState().backend as IBackendState;
		const hourly = "evt_crm_hourly";
		const desktop = await mountDevices(createElement("output"), {
			unlock: "none",
			platform: "desktop",
			localTriggers: [hourly],
		});
		expect(await backend().eventState.isEventSinkActive(hourly)).toBe(true);
		expect(
			await backend().eventState.isEventSinkActive("evt_crm_webhook"),
		).toBe(false);
		expect(await backend().sinkState?.listEventSinks()).toEqual([
			{
				event_id: hourly,
				name: "Hourly sync",
				type: "cron",
				created_at: SAMPLE_NOW,
				updated_at: SAMPLE_NOW,
				config: { expression: "0 0 * * * *" },
				offline: true,
				app_id: SAMPLE_APPS.crmSync,
			},
		]);
		await backend().sinkState?.removeEventSink(hourly);
		expect(desktop.fake.sinks.removed).toEqual([hourly]);
		expect(await backend().sinkState?.isEventSinkActive(hourly)).toBe(false);
		expect(await backend().eventState.isEventSinkActive(hourly)).toBe(false);
		desktop.fake.sinks.failure = new Error("The desktop command failed.");
		await expect(backend().sinkState?.listEventSinks()).rejects.toThrow(
			"The desktop command failed.",
		);
		await desktop.unmount();

		await mountDevices(createElement("output"), {
			unlock: "none",
			platform: "web",
			localTriggers: [hourly],
		});
		expect(backend().sinkState).toBeUndefined();
	});

	test("the apps given to the mount are the apps the hub exports", async () => {
		const visitor = APPS.app_visitor_checkin;
		const view = await mountDevices(createElement("output"), {
			unlock: "none",
			apps: { [visitor.id]: visitor },
		});
		const read = (app: string) =>
			outcome(() => send(view.fake.api, "GET", `apps/${app}/device-metadata`));
		expect(await read(visitor.id)).toBe("ok");
		expect(await read(APPS.app_invoice_ai.id)).toBe("coded 403");
	});
});
