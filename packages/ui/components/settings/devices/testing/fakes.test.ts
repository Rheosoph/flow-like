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
import type { AttentionItem } from "../../../../lib/device-management/model/types";
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
	readOfflineOperations,
	readOperations,
	readRolloutHistory,
} = await import("../../../../lib/device-management/agent-reads");
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
	listAccountBackups,
	listAppDevicePlacements,
	listCertificateNoticeMutes,
	listCertificateNotices,
	listEnrollments,
	muteCertificateNotices,
	readCertificateInventory,
	readHubStandalone,
	readPolicyView,
	sendTestCertificateNotice,
	toHubError,
	unmuteCertificateNotices,
} = await import("../../../../lib/device-management/hub/endpoints");
const { APPS, VISITOR_CATALOG } = await import(
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

/** The routes plan §3.2 adds (E1, E2, E4 and E15 change existing ones). */
const ADDED_ROUTES = [
	"DELETE devices/:id/certificate-notices/mute",
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
	"POST devices/:id/certificate-notices/test",
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
			"mount-devices.tsx",
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
