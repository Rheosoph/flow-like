import { afterAll, afterEach, describe, expect, test } from "bun:test";
import { act } from "react";
import { ApiResponseError } from "../../../../lib/api-error";
import type {
	DeployDraft,
	DeployPlan,
	PlanApp,
	PlanDevice,
	PlanEntry,
} from "../../../../lib/device-management/model/deploy-plan";
import type { DeployRunState } from "../../../../lib/device-management/model/deploy-run";
import { installDom, settle } from "../testing/dom-harness";
import type { FakeAgent } from "../testing/fake-device-api";
import type { FakeWorkspace } from "../testing/fake-workspace";
import type { MountDevicesOptions } from "../testing/mount-devices";
import type { DeployPrepared } from "./step-props";
import type * as Run from "./use-deploy-run";

const dom = installDom();
const { cleanupDevices, mountDevices, preloadDevices } = await import(
	"../testing/mount-devices"
);
await preloadDevices();
const run = await import("./use-deploy-run");
const { createFakeWorkspace } = await import("../testing/fake-workspace");
const {
	forgetArtifactTransfer,
	pendingArtifactTransfers,
	prepareProjectArtifact,
} = await import("../../../../lib/device-management/artifacts");
const { makePlan, resolvePlan } = await import(
	"../../../../lib/device-management/model/deploy-plan"
);
const { deployRunResult } = await import(
	"../../../../lib/device-management/model/deploy-run"
);
const {
	APPS,
	CRM_CATALOG,
	CRM_PLAN_APP,
	NOW0,
	PLAN_DEVICES,
	VISITOR_CATALOG,
	VISITOR_PLAN_APP,
	configBytes,
} = await import("../../../../lib/device-management/model/__fixtures__/apps");
const { withSavedBotTokens } = await import("./deploy-facts");
const { SAMPLE_IDS } = await import(
	"../../../../lib/device-management/model/__fixtures__/sample-fleet"
);
const { ACTIVITY_STORAGE_PREFIX } = await import(
	"../../../../lib/device-management/workspace/activity"
);
const { accountStorageKey } = await import(
	"../../../../lib/device-management/storage"
);

const EDGE = SAMPLE_IDS.edge;
const STUDIO = SAMPLE_IDS.studio;
const SHA = "e647322ba8a0deb7e20632b5a4d4d61a356f13ee81ca306c8a9f58bf47d7e14d";

const DEVICES: Record<string, PlanDevice> = {
	[EDGE]: { ...PLAN_DEVICES["edge-berlin-01"], id: EDGE },
	[STUDIO]: { ...PLAN_DEVICES["studio-mac-mini"], id: STUDIO },
};

afterEach(async () => {
	await cleanupDevices();
	await dom.cleanup();
});
afterAll(dom.restore);

let deployments = 0;

function planOf(
	app: PlanApp,
	route: Partial<PlanEntry["route"]> & { deviceIds: string[] },
	change: Partial<DeployDraft> = {},
	extra: Partial<PlanEntry> = {},
): DeployPlan {
	deployments += 1;
	const draft = {
		...makePlan({
			scope: { kind: "app", appId: app.id },
			route: { appId: app.id, ...route },
			app,
			deploymentId: `dep-run-${deployments}`,
			now: NOW0,
			...extra,
		}),
		...change,
	};
	return resolvePlan(draft, {
		app,
		devices: DEVICES,
		platform: "desktop",
		now: NOW0,
	});
}

async function visitorBundle(): Promise<DeployPrepared> {
	const project = "app_visitor_checkin";
	const artifact = await prepareProjectArtifact(
		project,
		[
			{
				path: `apps/${project}/online-source.json`,
				file: new Blob(['{"version":1}']),
			},
		],
		undefined,
		undefined,
		"online",
	);
	return {
		artifact,
		approved: {
			app: { id: project } as never,
			file: {
				path: `apps/${project}/online-metadata.json`,
				file: new Blob(["{}"]),
			},
			sha256: SHA,
			catalog: VISITOR_CATALOG,
		},
		preparedAt: 0,
	};
}

/** The device takes a bundle at once, and reports a copy's events and settings. */
function serveArtifacts(agent: FakeAgent) {
	agent.handle("artifact", (command, { operationId }) => {
		const request = command.request as Record<string, unknown>;
		if (request.kind === "begin") {
			const descriptor = request.descriptor as {
				project_id: string;
				manifest_sha256: string;
				manifest_size: number;
			};
			return {
				state: "completed",
				result: {
					transfer_id: operationId,
					descriptor,
					state: "committed",
					expires_at: agent.now() + 86_400,
					manifest_ready: true,
					file_index: null,
					offset: descriptor.manifest_size,
					complete: true,
					project_path: `/private/projects/${descriptor.project_id}/revisions/${descriptor.manifest_sha256}`,
				},
			};
		}
		const eventId = request.event_id as string | null;
		const variables = CRM_CATALOG.variables as Record<string, unknown[]>;
		return {
			state: "completed",
			result: {
				project_id: request.project_id,
				revision: request.revision,
				event_id: eventId,
				items: eventId ? [...(variables[eventId] ?? [])] : CRM_CATALOG.events,
				next: null,
			},
		};
	});
}

interface Sink {
	handle?: Run.DeployRunHandle;
	details?: Run.DeployRunDetails;
}

function Probe({
	plan,
	options,
	sink,
}: Readonly<{
	plan: DeployPlan;
	options: Partial<Run.DeployRunOptions>;
	sink: Sink;
}>) {
	sink.handle = run.useDeployRun(plan, {
		title: run.deployRunTitleRef(plan),
		oneAtATime: true,
		stopOnFail: true,
		...options,
	});
	sink.details = run.useDeployRunDetails(plan.draft.deploymentId);
	return null;
}

async function mountRun(
	plan: DeployPlan,
	options: Partial<Run.DeployRunOptions> = {},
	mount: MountDevicesOptions = {},
) {
	const sink: Sink = {};
	const mounted = await mountDevices(
		<Probe plan={plan} options={options} sink={sink} />,
		mount,
	);
	const state = () => sink.handle?.state as DeployRunState;
	return { ...mounted, sink, state };
}

async function until(done: () => boolean, what: string, rounds = 400) {
	for (let round = 0; round < rounds; round++) {
		if (done()) return;
		await settle();
	}
	throw new Error(`Timed out waiting for ${what}.`);
}

const sent = (fake: FakeWorkspace, deviceId: string, type?: string) =>
	fake.api.commands
		.filter((command) => command[0] === deviceId)
		.filter((command) => !type || command[1] === type);

const DEPLOY_COMMANDS = [
	"apply",
	"set_secret",
	"start",
	"stage_rollout",
	"rollout_secret",
	"activate_rollout",
];

/** The commands a deploy sends, in order; an upload counts once, by its `begin`. */
const types = (fake: FakeWorkspace, deviceId: string) =>
	sent(fake, deviceId)
		.filter(
			([, type, command]) =>
				DEPLOY_COMMANDS.includes(type) ||
				(type === "artifact" &&
					(command.request as { kind?: string }).kind === "begin"),
		)
		.map((command) => command[1]);

const grants = (fake: FakeWorkspace, deviceId: string) =>
	fake.api.sent("POST", `devices/${deviceId}/resource-grants`);

async function start(sink: Sink) {
	await act(async () => {
		sink.handle?.start();
	});
}

function visitorPlan(deviceIds: string[], change: Partial<DeployDraft> = {}) {
	return planOf(
		VISITOR_PLAN_APP,
		{ deviceIds, eventId: "evt_visitor_page" },
		change,
	);
}

describe("useDeployRun", () => {
	test("a settings-only update of a stopped service applies once, uploads nothing and leaves it stopped", async () => {
		const plan = planOf(
			CRM_PLAN_APP,
			{ deviceIds: [EDGE], serviceId: "nightly-sync" },
			{
				version: "keep",
				vars: { var_batch_size: "50" },
				edited: ["var_batch_size"],
			},
			{ updateEvents: ["evt_crm_nightly"] },
		);
		const { fake, sink, state } = await mountRun(plan);
		serveArtifacts(fake.agent(EDGE));
		expect(state().status).toBe("idle");
		expect(state().shared).toBeNull();

		await start(sink);
		await until(() => state().status === "finished", "the update to finish");

		const [row] = state().rows;
		expect(row?.state).toBe("done");
		expect(row?.phases).toEqual(["stop"]);
		expect(types(fake, EDGE)).toEqual(["apply"]);
		const [apply] = sent(fake, EDGE, "apply");
		const command = apply?.[2] as {
			expected_revision: number;
			config: { variables: Record<string, unknown> };
		};
		expect(command.expected_revision).toBe(3);
		expect(command.config.variables).toEqual({ var_batch_size: 50 });
		const detail = sink.details?.[`${EDGE}/nightly-sync`];
		expect(detail?.strategy).toBe("quick");
		expect(detail?.reasons).toEqual([{ code: "stopped" }]);
		expect(detail?.stopped).toBe("as_asked");
		expect(detail?.toSettings).toBe(4);
		expect(deployRunResult(state())?.outcome).toBe("all");
		expect(grants(fake, EDGE)).toEqual([]);

		const [tray] = fake.workspace.activity.runs();
		expect(tray?.itemIds).toHaveLength(1);
		const [item] = fake.workspace.activity.runItems(tray?.id ?? "");
		expect(item?.state).toBe("done");
		expect(item?.href).toMatchObject({ screen: "deploy", step: "rollout" });
		expect(state().id).toBe(tray?.id ?? "");
	});

	test("a service that changed in the meantime fails with that reason; a retry reads it again and applies on top", async () => {
		const plan = planOf(
			CRM_PLAN_APP,
			{ deviceIds: [EDGE], serviceId: "nightly-sync" },
			{
				version: "keep",
				vars: { var_batch_size: "50" },
				edited: ["var_batch_size"],
			},
			{ updateEvents: ["evt_crm_nightly"] },
		);
		const { fake, sink, state } = await mountRun(plan);
		const agent = fake.agent(EDGE);
		serveArtifacts(agent);
		const release = agent.hold("apply");

		await start(sink);
		await until(
			() => sent(fake, EDGE, "apply").length === 1,
			"the apply to leave",
		);
		// Someone else changes the service after this deploy read it.
		const placement = agent.placement("nightly-sync");
		if (!placement) throw new Error("The fixture has no nightly-sync.");
		placement.config_revision += 1;
		await act(async () => release());
		await until(() => state().status === "finished", "the conflict to show");

		const [row] = state().rows;
		expect(row?.state).toBe("failed");
		expect(row?.error?.code).toBe("stale");
		const reads = sent(fake, EDGE, "placement_configuration").length;

		await act(async () => sink.handle?.retry(row?.target ?? ""));
		await until(() => state().rows[0]?.state === "done", "the retry to finish");
		expect(sent(fake, EDGE, "placement_configuration").length).toBeGreaterThan(
			reads,
		);
		const again = sent(fake, EDGE, "apply").at(-1)?.[2] as {
			expected_revision: number;
		};
		expect(again.expected_revision).toBe(4);
	});

	test("new online services get their own approval, the handed-over bundle, a token each and start, one device at a time", async () => {
		const plan = visitorPlan([EDGE, STUDIO]);
		const { fake, sink, state } = await mountRun(plan);
		serveArtifacts(fake.agent(EDGE));
		serveArtifacts(fake.agent(STUDIO));
		run.setDeployRunExtras(plan.draft.deploymentId, {
			prepared: await visitorBundle(),
		});
		expect(state().shared?.phase).toBe("approve_definitions");

		await start(sink);
		await until(() => state().status === "finished", "both devices to finish");

		expect(state().rows.map((row) => row.state)).toEqual(["done", "done"]);
		for (const deviceId of [EDGE, STUDIO]) {
			expect(types(fake, deviceId)).toEqual([
				"artifact",
				"apply",
				"set_secret",
				"start",
			]);
			expect(grants(fake, deviceId)).toHaveLength(1);
		}
		const [created] = sent(fake, EDGE, "apply");
		const config = (created?.[2] as { config: Record<string, unknown> }).config;
		expect(config.online_metadata_sha256).toBe(SHA);
		expect(config.deployment_id).toBe(plan.draft.deploymentId);
		expect(config.resource_grant).toMatchObject({ authz_version: 1 });
		const details = Object.values(sink.details ?? {});
		expect(details).toHaveLength(2);
		const tokens = details.map((detail) => detail.token ?? "");
		expect(tokens.every((token) => /^[\x21-\x7e]{32,}$/.test(token))).toBe(
			true,
		);
		expect(new Set(tokens).size).toBe(2);
		expect(JSON.stringify(fake.api.commands)).not.toContain(
			tokens[0] as string,
		);
		expect(JSON.stringify(fake.workspace.activity.list())).not.toContain(
			tokens[0] as string,
		);
		const [first, second] = state().rows;
		expect((first?.finishedAt ?? 0) <= (second?.startedAt ?? 0)).toBe(true);
		expect(fake.workspace.activity.runs()).toHaveLength(1);
	});

	test("a token-free service starts without generating, saving or showing an access token", async () => {
		const plan = visitorPlan([EDGE], {
			endpoint: {
				host: "127.0.0.1",
				port: 8080,
				token: "none",
				tokenValue: "",
			},
		});
		const { fake, sink, state } = await mountRun(plan);
		serveArtifacts(fake.agent(EDGE));
		run.setDeployRunExtras(plan.draft.deploymentId, {
			prepared: await visitorBundle(),
		});
		await start(sink);
		await until(
			() => state().status === "finished",
			"the token-free service to start",
		);
		expect(state().rows.map((row) => row.state)).toEqual(["done"]);
		expect(types(fake, EDGE)).toEqual(["artifact", "apply", "start"]);
		const config = (
			sent(fake, EDGE, "apply")[0]?.[2] as { config: Record<string, unknown> }
		).config;
		expect(config.hosting).toMatchObject({ authentication: "none" });
		expect(config.hosting).not.toHaveProperty("auth_secret");
		expect(Object.values(sink.details ?? {})).toHaveLength(1);
		for (const detail of Object.values(sink.details ?? {}))
			expect(detail.token).toBeUndefined();
	});

	test("a new offline service: the device checks the copy, the service is created with no secret in it, then its token is saved and it starts", async () => {
		const plan = planOf(CRM_PLAN_APP, {
			deviceIds: [STUDIO],
			eventId: "evt_crm_webhook",
		});
		const { fake, sink, state } = await mountRun(plan);
		serveArtifacts(fake.agent(STUDIO));
		const project = "app_crm_sync";
		const artifact = await prepareProjectArtifact(
			project,
			[{ path: `apps/${project}/manifest.app`, file: new Blob(["{}"]) }],
			undefined,
			undefined,
			"offline",
		);
		run.setDeployRunExtras(plan.draft.deploymentId, {
			prepared: { artifact, preparedAt: 0 },
		});
		const target = state().rows[0]?.target ?? "";

		await start(sink);
		await until(() => state().status === "finished", "the deploy to finish");

		const [row] = state().rows;
		expect(row?.state).toBe("done");
		expect(row?.phases).toEqual([
			"upload",
			"check_events",
			"install",
			"create",
			"secrets",
			"start",
		]);
		expect(types(fake, STUDIO)).toEqual([
			"artifact",
			"apply",
			"set_secret",
			"start",
		]);
		const checks = sent(fake, STUDIO, "artifact").filter(
			([, , command]) =>
				(command.request as { kind?: string }).kind === "describe",
		);
		expect(checks.length).toBeGreaterThan(0);
		const [created] = sent(fake, STUDIO, "apply");
		const config = (created?.[2] as { config: Record<string, unknown> }).config;
		expect(config.source).toBe("offline");
		expect(String(config.project_path)).toContain(
			`/private/projects/${project}/revisions/`,
		);
		expect(config.resource_grant ?? null).toBeNull();
		const token = sink.details?.[target]?.token ?? "";
		expect(token).toMatch(/^[\x21-\x7e]{32,}$/);
		expect(JSON.stringify(fake.api.commands)).not.toContain(token);
		expect(grants(fake, STUDIO)).toEqual([]);
	});

	test("a failed device holds the rest; Continue goes on and a retry reuses the approval the hub already has", async () => {
		const plan = visitorPlan([EDGE, STUDIO]);
		const { fake, sink, state } = await mountRun(plan);
		serveArtifacts(fake.agent(EDGE));
		serveArtifacts(fake.agent(STUDIO));
		run.setDeployRunExtras(plan.draft.deploymentId, {
			prepared: await visitorBundle(),
		});
		const [edge, studio] = state().rows.map((row) => row.target);
		const restore = fake
			.agent(EDGE)
			.reject("apply", "invalid", "The device refused these settings.");

		await start(sink);
		await until(() => state().status === "held", "the run to hold");
		expect(state().rows.map((row) => row.state)).toEqual(["failed", "held"]);
		expect(state().rows[0]?.error).toMatchObject({
			phase: "create",
			code: "invalid",
		});
		expect(state().holdBy).toBe(edge ?? "");
		expect(sink.details?.[edge ?? ""]?.kept).toBe("approval");
		expect(types(fake, STUDIO)).toEqual([]);
		expect(deployRunResult(state())).toBeNull();

		await act(async () => sink.handle?.resume());
		await until(() => state().status === "finished", "the rest to finish");
		expect(state().rows.map((row) => row.state)).toEqual(["failed", "done"]);
		expect(deployRunResult(state())?.outcome).toBe("partial");
		expect(deployRunResult(state())?.failed[0]?.target).toBe(edge ?? "");

		restore();
		await act(async () => sink.handle?.retry(edge ?? ""));
		await until(
			() => state().rows[0]?.state === "done" && state().status === "finished",
			"the retried device to finish",
		);
		expect(grants(fake, EDGE)).toHaveLength(1);
		expect(sent(fake, EDGE, "apply")).toHaveLength(2);
		expect(deployRunResult(state())?.outcome).toBe("all");
		expect(studio).toBeDefined();
	});

	test("a changed plan after a failed deploy: the approval the first attempt left is revoked before the new one is created", async () => {
		const first = visitorPlan([EDGE]);
		const view = await mountRun(first);
		const { fake } = view;
		serveArtifacts(fake.agent(EDGE));
		run.setDeployRunExtras(first.draft.deploymentId, {
			prepared: await visitorBundle(),
		});
		const restore = fake
			.agent(EDGE)
			.reject("apply", "invalid", "The device refused these settings.");
		await start(view.sink);
		await until(() => view.state().status === "finished", "the first attempt");
		expect(view.state().rows[0]?.state).toBe("failed");
		const [left] = fake.hub
			.resourcesOf(EDGE)
			.grants.filter((grant) => grant.placement_id === "check-in-page");
		expect(left?.status).toBe("active");
		expect(left?.deployment_id).toBe(first.draft.deploymentId);
		restore();

		// "Change and deploy again": the same service under a new deploy ID.
		const second = visitorPlan([EDGE]);
		expect(second.draft.deploymentId).not.toBe(first.draft.deploymentId);
		const again: Sink = {};
		await view.rerender(<Probe plan={second} options={{}} sink={again} />);
		run.setDeployRunExtras(second.draft.deploymentId, {
			prepared: await visitorBundle(),
		});
		await start(again);
		await until(
			() => again.handle?.state.status === "finished",
			"the second attempt",
		);

		expect(again.handle?.state.rows[0]?.state).toBe("done");
		const active = fake.hub
			.resourcesOf(EDGE)
			.grants.filter(
				(grant) =>
					grant.placement_id === "check-in-page" && grant.status === "active",
			);
		// The hub takes one active approval per service: exactly the new deploy's is left.
		expect(active.map((grant) => grant.deployment_id)).toEqual([
			second.draft.deploymentId,
		]);
		const writes = fake.api.calls
			.filter(([method]) => method === "POST" || method === "DELETE")
			.map(([method, path]) => `${method} ${path}`)
			.filter((line) => line.includes(`devices/${EDGE}/resource-grants`));
		expect(writes.at(-2)).toBe(
			`DELETE devices/${EDGE}/resource-grants/${left?.grant_id}`,
		);
		expect(writes.at(-1)).toBe(`POST devices/${EDGE}/resource-grants`);
		const created = sent(fake, EDGE, "apply").at(-1)?.[2] as {
			config: { deployment_id: string; resource_grant: { grant_id: string } };
		};
		expect(created.config.deployment_id).toBe(second.draft.deploymentId);
		expect(created.config.resource_grant.grant_id).toBe(active[0]?.grant_id);
	});

	test("an approval a service on the device still runs on is never revoked: the device is asked first", async () => {
		const first = visitorPlan([EDGE]);
		const view = await mountRun(first);
		const { fake } = view;
		serveArtifacts(fake.agent(EDGE));
		run.setDeployRunExtras(first.draft.deploymentId, {
			prepared: await visitorBundle(),
		});
		await start(view.sink);
		await until(() => view.state().status === "finished", "the first deploy");
		expect(view.state().rows[0]?.state).toBe("done");
		const running = fake.hub
			.resourcesOf(EDGE)
			.grants.find((grant) => grant.placement_id === "check-in-page");
		expect(running?.status).toBe("active");

		// A plan made from older facts still calls check-in-page a new service on this device.
		const stale = visitorPlan([EDGE]);
		const again: Sink = {};
		await view.rerender(<Probe plan={stale} options={{}} sink={again} />);
		run.setDeployRunExtras(stale.draft.deploymentId, {
			prepared: await visitorBundle(),
		});
		const applies = sent(fake, EDGE, "apply").length;
		await start(again);
		await until(
			() => again.handle?.state.status === "finished",
			"the second attempt",
		);

		expect(again.handle?.state.rows[0]?.error).toMatchObject({
			phase: "approve",
			code: "service_exists",
		});
		expect(fake.api.sent("DELETE", /resource-grants/)).toEqual([]);
		expect(running?.status).toBe("active");
		expect(grants(fake, EDGE)).toHaveLength(1);
		expect(sent(fake, EDGE, "apply")).toHaveLength(applies);
	});

	test("the hub refusing an approval is a refusal, not a lost answer", async () => {
		const plan = visitorPlan([EDGE]);
		const { fake, sink, state } = await mountRun(plan);
		serveArtifacts(fake.agent(EDGE));
		run.setDeployRunExtras(plan.draft.deploymentId, {
			prepared: await visitorBundle(),
		});
		fake.api.fail(
			{ method: "POST", path: /resource-grants$/ },
			new ApiResponseError({
				status: 403,
				code: "FORBIDDEN",
				message: "Online storage requires the current project owner's approval",
			}),
		);
		await start(sink);
		await until(() => state().status === "finished", "the refusal to show");
		expect(state().rows[0]?.error).toMatchObject({
			phase: "approve",
			code: "hub_refused",
		});
		expect(types(fake, EDGE)).toEqual([]);
	});

	test("a lost upload reply retains client transport diagnostics and the resumable transfer", async () => {
		const plan = visitorPlan([EDGE]);
		const { fake, sink, state } = await mountRun(plan);
		serveArtifacts(fake.agent(EDGE));
		run.setDeployRunExtras(plan.draft.deploymentId, {
			prepared: await visitorBundle(),
		});
		fake.agent(EDGE).loseReplyNext("artifact");

		await start(sink);
		await until(() => state().status === "finished", "the lost upload reply");
		expect(state().rows[0]?.error).toMatchObject({
			phase: "upload",
			code: "upload_unconfirmed",
		});
		const detail = state().rows[0]?.error?.detail ?? "";
		expect(detail).toContain("Client transport diagnostics:");
		expect(detail).toContain("Transport: websocket");
		expect(detail).toContain("Transport phase: wait_reply");
		expect(detail).toContain("Transport cause: connection_closed");
		expect(types(fake, EDGE)).toEqual(["artifact"]);
		expect(grants(fake, EDGE)).toHaveLength(1);
		const pending = pendingArtifactTransfers(
			EDGE,
			"app_visitor_checkin",
			fake.scope,
		);
		expect(pending.map((transfer) => transfer.confirmed)).toEqual([false]);
		for (const transfer of pending)
			forgetArtifactTransfer(EDGE, transfer.transfer_id, fake.scope);
	});

	test("an apply whose reply was lost is looked up and sent again under the same command id", async () => {
		const plan = visitorPlan([EDGE]);
		const { fake, sink, state } = await mountRun(plan);
		serveArtifacts(fake.agent(EDGE));
		run.setDeployRunExtras(plan.draft.deploymentId, {
			prepared: await visitorBundle(),
		});
		const target = state().rows[0]?.target ?? "";
		fake.agent(EDGE).dropNext("apply");

		await start(sink);
		await until(() => state().status === "finished", "the lost reply to fail");
		expect(state().rows[0]?.state).toBe("failed");
		expect(state().rows[0]?.error?.code).toBe("unconfirmed");
		const commandId = sink.details?.[target]?.operationId ?? "";
		expect(commandId).not.toBe("");
		const lookups = sent(fake, EDGE, "operation").length;

		await act(async () => sink.handle?.retry(target));
		await until(() => state().rows[0]?.state === "done", "the retry to finish");
		expect(sent(fake, EDGE, "operation").length).toBeGreaterThan(lookups);
		const [lost, again] = sent(fake, EDGE, "apply");
		expect(again?.[2]).toEqual(lost?.[2] as never);
		expect(fake.agent(EDGE).journal.get(commandId)?.type).toBe("apply");
		expect(types(fake, EDGE)).toEqual([
			"artifact",
			"apply",
			"apply",
			"set_secret",
			"start",
		]);
		expect(grants(fake, EDGE)).toHaveLength(1);
	});

	test("locking a device between two secrets sends nothing further; unlocking continues with the same commands", async () => {
		const plan = visitorPlan([EDGE], {
			secrets: { var_host_token: "host-directory-secret" },
		});
		const { fake, sink, state } = await mountRun(plan);
		serveArtifacts(fake.agent(EDGE));
		run.setDeployRunExtras(plan.draft.deploymentId, {
			prepared: await visitorBundle(),
		});
		const release = fake.agent(EDGE).hold("set_secret");

		await start(sink);
		await until(
			() => sent(fake, EDGE, "set_secret").length === 1,
			"the first secret to leave",
		);
		await act(async () => fake.workspace.keys.lock(EDGE));
		release();
		await until(() => state().rows[0]?.state === "blocked", "the row to wait");
		expect(state().rows[0]?.blocked).toBe("locked");
		expect(state().status).toBe("running");
		expect(sent(fake, EDGE, "set_secret")).toHaveLength(1);
		expect(sent(fake, EDGE, "start")).toHaveLength(0);

		await act(async () => {
			await fake.unlock(EDGE, { connectLive: true });
		});
		await until(() => state().status === "finished", "the run to finish");
		expect(state().rows[0]?.state).toBe("done");
		expect(sent(fake, EDGE, "apply")).toHaveLength(1);
		expect(sent(fake, EDGE, "set_secret")).toHaveLength(2);
		expect(sent(fake, EDGE, "start")).toHaveLength(1);
		expect(JSON.stringify(fake.api.commands)).not.toContain(
			"host-directory-secret",
		);
	});

	test("without a handed-over bundle the run prepares the version itself before any device starts", async () => {
		const plan = visitorPlan([EDGE]);
		const { fake, sink, state } = await mountRun(plan);
		serveArtifacts(fake.agent(EDGE));

		await start(sink);
		await until(() => state().status === "finished", "the run to finish");

		expect(state().shared?.state).toBe("done");
		expect(state().rows[0]?.state).toBe("done");
		expect(
			fake.api.sent("GET", "apps/app_visitor_checkin/device-metadata"),
		).toHaveLength(1);
		const [created] = sent(fake, EDGE, "apply");
		const config = (created?.[2] as { config: Record<string, unknown> }).config;
		expect(String(config.online_metadata_sha256)).toMatch(/^[a-f0-9]{64}$/);
	});

	test("the wizard's copy taken back mid-upload: the run prepares its own and goes on", async () => {
		const plan = visitorPlan([EDGE]);
		const { fake, sink, state } = await mountRun(plan);
		serveArtifacts(fake.agent(EDGE));
		const handed = await visitorBundle();
		run.setDeployRunExtras(plan.draft.deploymentId, { prepared: handed });
		const metadata = () =>
			fake.api.sent("GET", "apps/app_visitor_checkin/device-metadata").length;
		const release = fake.agent(EDGE).hold("artifact");

		await start(sink);
		await until(
			() => types(fake, EDGE).includes("artifact"),
			"the upload to begin",
		);
		expect(metadata()).toBe(0);
		// The wizard closes while the upload reads from its copy, and the upload breaks off.
		run.setDeployRunExtras(plan.draft.deploymentId, { prepared: null });
		await act(async () => {
			fake.agent(EDGE).disconnect();
			release();
		});
		await until(() => state().status === "finished", "the run to finish");

		expect(state().rows[0]?.state).toBe("done");
		expect(metadata()).toBe(1);
		expect(types(fake, EDGE)).toEqual([
			"artifact",
			"artifact",
			"apply",
			"set_secret",
			"start",
		]);
		// The upload begun from the wizard's copy stays remembered, unconfirmed, until it expires or is aborted.
		const leftover = pendingArtifactTransfers(
			EDGE,
			"app_visitor_checkin",
			fake.scope,
		);
		expect(leftover.map((transfer) => transfer.confirmed)).toEqual([false]);
		expect(leftover[0]?.manifest_sha256).toBe(
			handed.artifact.descriptor.manifest_sha256,
		);
		for (const transfer of leftover)
			forgetArtifactTransfer(EDGE, transfer.transfer_id, fake.scope);
	});

	test("a closed window sends nothing further; the reopened run reads how far the device got", async () => {
		const plan = visitorPlan([EDGE]);
		const first = await mountRun(plan);
		serveArtifacts(first.fake.agent(EDGE));
		run.setDeployRunExtras(plan.draft.deploymentId, {
			prepared: await visitorBundle(),
		});
		const release = first.fake.agent(EDGE).hold("apply");
		await start(first.sink);
		await until(
			() => sent(first.fake, EDGE, "apply").length === 1,
			"the apply to leave",
		);
		const { api } = first.fake;
		await first.unmount();
		release();
		await settle();
		expect(
			api.commands.filter((command) => command[1] === "set_secret"),
		).toEqual([]);
		expect(api.commands.filter((command) => command[1] === "start")).toEqual(
			[],
		);

		// A new fake workspace seeds its tray; the reopened window keeps the one it had.
		const trayKey = `${ACTIVITY_STORAGE_PREFIX}${accountStorageKey(api.scope)}`;
		const tray = globalThis.localStorage.getItem(trayKey) ?? "";
		const fake = await createFakeWorkspace(undefined, { api });
		globalThis.localStorage.setItem(trayKey, tray);
		fake.workspace.activity.dismiss("-");
		const resumeId = run.recallDeployRun(fake.workspace, plan);
		expect(resumeId).toBeDefined();
		const again = await mountRun(plan, { resumeId }, { fake });
		await until(
			() => again.state().status === "finished",
			"the reopened run to settle",
		);
		const [row] = again.state().rows;
		expect(again.state().id).toBe(resumeId ?? "");
		expect(row?.state).toBe("failed");
		expect(row?.error?.code).toBe("applied_only");
		expect(
			api.commands.filter((command) => command[1] === "apply"),
		).toHaveLength(1);
		expect(api.commands.filter((command) => command[1] === "start")).toEqual(
			[],
		);
	});
});

const NOTES_EVENT = {
	id: "evt_notes_http",
	name: "Notes page",
	event_type: "http",
	event_version: [1, 2, 0] as [number, number, number],
	board_version: [3, 0, 1] as [number, number, number],
	hosted: true,
	readiness_kind: "listener" as const,
	rollout_supported: true,
	eligible: true,
};

async function notesBundle(): Promise<DeployPrepared> {
	const project = "app_field_notes";
	const artifact = await prepareProjectArtifact(
		project,
		[
			{
				path: `apps/${project}/online-source.json`,
				file: new Blob(['{"version":2}']),
			},
		],
		undefined,
		undefined,
		"online",
	);
	return {
		artifact,
		approved: {
			app: { id: project } as never,
			file: {
				path: `apps/${project}/online-metadata.json`,
				file: new Blob(["{}"]),
			},
			sha256: SHA,
			catalog: {
				events: [NOTES_EVENT],
				variables: { evt_notes_http: [] },
			},
		},
		preparedAt: 0,
	};
}

/** The version-only update of every listed service, as App › Devices starts it. */
function notesUpdate(change: Partial<DeployDraft> = {}) {
	return planOf(
		APPS.app_field_notes,
		{ deviceIds: [STUDIO], mode: "update" },
		{
			targets: [
				{
					deviceId: STUDIO,
					choices: { main: { kind: "update", serviceId: "field-notes" } },
					serveBoth: [],
					over: {},
				},
			],
			...change,
		},
	);
}

describe("useDeployRun · updates", () => {
	test("a version-only update of a running online service is a safe update with the chosen timings", async () => {
		const plan = notesUpdate();
		expect(plan.draft.keepEvents).toBe(true);
		const { fake, sink, state } = await mountRun(plan);
		serveArtifacts(fake.agent(STUDIO));
		run.setDeployRunExtras(plan.draft.deploymentId, {
			prepared: await notesBundle(),
			timings: { stabilizeSeconds: 5, deadlineSeconds: 60 },
		});
		const target = state().rows[0]?.target ?? "";

		await start(sink);
		await until(
			() => state().status === "finished",
			"the update to finish",
			2000,
		);

		const [row] = state().rows;
		expect(row?.state).toBe("done");
		expect(row?.phases).toEqual([
			"upload",
			"install",
			"prepare_update",
			"check_new",
			"switch",
		]);
		expect(types(fake, STUDIO)).toEqual([
			"artifact",
			"stage_rollout",
			"activate_rollout",
		]);
		const [stage] = sent(fake, STUDIO, "stage_rollout");
		expect(stage?.[2]).toMatchObject({
			expected_revision: 9,
			stabilization_seconds: 5,
			deadline_seconds: 60,
		});
		expect(sink.details?.[target]).toMatchObject({
			strategy: "safe",
			reasons: [],
			fromSettings: 9,
			toSettings: 10,
			wasRunning: true,
		});
		expect(sink.details?.[target]?.stopped).toBeUndefined();
		expect(grants(fake, STUDIO)).toEqual([]);
		const [tray] = fake.workspace.activity.runs();
		const [item] = fake.workspace.activity.runItems(tray?.id ?? "");
		expect(item?.state).toBe("done");
		expect(item?.resume).toBeUndefined();
	});

	test("a safe update stages a new access token with the update: nothing is applied, set or started by hand", async () => {
		const token = "typed-access-token-of-32-chars-ok";
		const plan = notesUpdate({
			endpoint: { host: null, port: null, token: "own", tokenValue: token },
		});
		const { fake, sink, state } = await mountRun(plan);
		serveArtifacts(fake.agent(STUDIO));
		run.setDeployRunExtras(plan.draft.deploymentId, {
			prepared: await notesBundle(),
		});

		await start(sink);
		await until(
			() => state().status === "finished",
			"the update to finish",
			2000,
		);

		const [row] = state().rows;
		expect(row?.state).toBe("done");
		expect(row?.phases).toEqual([
			"upload",
			"install",
			"prepare_update",
			"secrets",
			"check_new",
			"switch",
		]);
		expect(types(fake, STUDIO)).toEqual([
			"artifact",
			"stage_rollout",
			"rollout_secret",
			"activate_rollout",
		]);
		// The token travels in its own command only, and the service keeps its approval.
		const [stage] = sent(fake, STUDIO, "stage_rollout");
		expect(JSON.stringify(stage?.[2])).not.toContain(token);
		expect(grants(fake, STUDIO)).toEqual([]);
	});

	test("a rollback is reported as one and a retry reads the service again", async () => {
		const plan = notesUpdate();
		const { fake, sink, state } = await mountRun(plan);
		const agent = fake.agent(STUDIO);
		serveArtifacts(agent);
		run.setDeployRunExtras(plan.draft.deploymentId, {
			prepared: await notesBundle(),
		});
		const target = state().rows[0]?.target ?? "";
		const restore = agent.handle("rollout", (command) => {
			const row = agent.rollouts.find(
				(value) => value.rollout_id === command.rollout_id,
			);
			return {
				state: "completed",
				result: {
					...row,
					state: "rolled_back",
					failure_code: "startup_failed",
				},
			};
		});

		await start(sink);
		await until(
			() => state().status === "finished",
			"the rollback to show",
			2000,
		);
		const [row] = state().rows;
		expect(row?.state).toBe("failed");
		expect(row?.error).toMatchObject({
			code: "rolled_back",
			detail: "startup_failed",
			rolledBack: true,
		});
		expect(deployRunResult(state())?.failed[0]?.rolledBack).toBe(true);
		const [tray] = fake.workspace.activity.runs();
		const [item] = fake.workspace.activity.runItems(tray?.id ?? "");
		expect(item?.detail?.code).toBe("rolled_back");
		const reads = sent(fake, STUDIO, "placement_configuration").length;

		restore();
		await act(async () => sink.handle?.retry(target));
		await until(
			() => sent(fake, STUDIO, "placement_configuration").length > reads,
			"the retry to read the service again",
			2000,
		);
	});

	test("an agent that names no update modes gets a quick update: nothing is staged", async () => {
		const plan = notesUpdate();
		const { fake, sink, state } = await mountRun(plan);
		const agent = fake.agent(STUDIO);
		serveArtifacts(agent);
		run.setDeployRunExtras(plan.draft.deploymentId, {
			prepared: await notesBundle(),
		});
		const target = state().rows[0]?.target ?? "";
		const call = fake.workspace.live.call(STUDIO);
		const current = await call({
			type: "placement_configuration",
			placement_id: "field-notes",
		});
		const { rollout_sources: _modes, ...older } = current.result;
		agent.handle("placement_configuration", () => ({
			state: "completed",
			result: older,
		}));

		await start(sink);
		await until(
			() => state().status === "finished",
			"the update to finish",
			2000,
		);

		expect(state().rows[0]?.state).toBe("done");
		expect(types(fake, STUDIO)).toEqual(["artifact", "apply", "start"]);
		expect(sink.details?.[target]).toMatchObject({
			strategy: "quick",
			reasons: [{ code: "agent_unsupported" }],
		});
		expect(state().rows[0]?.phases).toEqual([
			"upload",
			"install",
			"stop",
			"start",
		]);
	});

	test("turning write buffering off waits for an empty queue: nothing is applied while changes wait", async () => {
		const plan = notesUpdate({ writes: null });
		const { fake, sink, state } = await mountRun(plan);
		serveArtifacts(fake.agent(STUDIO));
		run.setDeployRunExtras(plan.draft.deploymentId, {
			prepared: await notesBundle(),
		});

		await start(sink);
		await until(() => state().status === "finished", "the run to stop", 2000);

		expect(state().rows[0]?.state).toBe("failed");
		expect(state().rows[0]?.error?.code).toBe("queue_not_empty");
		expect(types(fake, STUDIO)).toEqual(["artifact"]);
	});

	test("a secret the device refuses is named as such and nothing is sent again by itself", async () => {
		const plan = visitorPlan([EDGE]);
		const { fake, sink, state } = await mountRun(plan);
		serveArtifacts(fake.agent(EDGE));
		run.setDeployRunExtras(plan.draft.deploymentId, {
			prepared: await visitorBundle(),
		});
		fake
			.agent(EDGE)
			.reject("set_secret", "invalid", "The secret store refused this name.");

		await start(sink);
		await until(() => state().status === "finished", "the failure to show");

		expect(state().rows[0]?.error).toMatchObject({
			phase: "secrets",
			code: "secret_publication",
		});
		expect(sent(fake, EDGE, "start")).toEqual([]);
		expect(sent(fake, EDGE, "set_secret")).toHaveLength(1);
		expect(sink.details?.[state().rows[0]?.target ?? ""]?.kept).toBe(
			"approval",
		);
	});
});

describe("a deploy with a schedule (one place per schedule)", () => {
	const VISITOR = "app_visitor_checkin";
	const INVOICE = "app_invoice_ai";
	const REPORT = "evt_visitor_report";
	const RECONCILE = "evt_invoice_reconcile";
	const SERVICE = "daily-visitor-report";

	const reportPlan = () =>
		planOf(VISITOR_PLAN_APP, { deviceIds: [STUDIO], eventId: REPORT });

	async function runReport(mount: MountDevicesOptions = {}) {
		const plan = reportPlan();
		const mounted = await mountRun(plan, {}, mount);
		serveArtifacts(mounted.fake.agent(STUDIO));
		run.setDeployRunExtras(plan.draft.deploymentId, {
			prepared: await visitorBundle(),
		});
		return { ...mounted, plan };
	}

	test("the schedule is released to the service before the device gets its config, and the service then runs it", async () => {
		const { fake, sink, state } = await runReport();
		const agent = fake.agent(STUDIO);
		const release = agent.hold("apply");
		expect(state().rows[0]?.phases).toEqual([
			"schedules",
			"approve",
			"upload",
			"install",
			"create",
			"start",
		]);

		await start(sink);
		await until(
			() => sent(fake, STUDIO, "apply").length === 1,
			"the config to leave",
		);
		// The hub already holds the release when the device hears of the schedule.
		expect(fake.hub.schedules.listing(VISITOR)).toMatchObject([
			{
				event_id: REPORT,
				state: "released",
				device_id: STUDIO,
				placement_id: SERVICE,
			},
		]);
		await act(async () => release());
		await until(() => state().status === "finished", "the deploy to finish");

		expect(state().rows[0]?.state).toBe("done");
		expect(
			fake.api.sent("PUT", /device-schedules/).map(([, path]) => path),
		).toEqual([`apps/${VISITOR}/device-schedules/${REPORT}`]);
		expect(agent.placement(SERVICE)?.schedules).toMatchObject([
			{ event_id: REPORT, hold: null },
		]);
		expect(fake.hub.schedules.listing(VISITOR)).toMatchObject([
			{ event_id: REPORT, state: "device", device_id: STUDIO },
		]);
	});

	test("a person who may not edit the app's events: the target stops with that reason and the device gets nothing", async () => {
		const { fake, sink, state } = await runReport();
		fake.hub.schedules.canEditEvents = false;

		await start(sink);
		await until(() => state().status === "finished", "the refusal to show");

		expect(state().rows[0]?.state).toBe("failed");
		expect(state().rows[0]?.error).toMatchObject({
			phase: "schedules",
			code: "schedule_role",
			detail: "Daily visitor report",
		});
		expect(types(fake, STUDIO)).toEqual([]);
		expect(grants(fake, STUDIO)).toEqual([]);
		expect(fake.hub.schedules.listing(VISITOR)).toEqual([]);
	});

	test("an agent that is too old to run schedules never gets one: nothing is approved, uploaded or applied", async () => {
		const { fake, sink, state } = await runReport({
			agentFeatures: { placement_events: 1, placement_diagnostics: 1 },
		});

		await start(sink);
		await until(() => state().status === "finished", "the gate to show");

		expect(state().rows[0]?.error).toMatchObject({
			code: "agent_feature",
			detail: "scheduled_events",
		});
		expect(types(fake, STUDIO)).toEqual([]);
		expect(grants(fake, STUDIO)).toEqual([]);
		expect(fake.api.sent("PUT", /device-schedules/)).toEqual([]);
	});

	test("a hub that can't hand schedules to devices stops the target before the config is sent", async () => {
		const { fake, sink, state } = await runReport();
		fake.hub.capabilities.schedules = false;

		await start(sink);
		await until(() => state().status === "finished", "the refusal to show");

		expect(state().rows[0]?.error).toMatchObject({
			phase: "schedules",
			code: "schedule_hub",
		});
		expect(sent(fake, STUDIO, "apply")).toEqual([]);
	});

	test("a schedule another service runs: the target stops, and that service keeps it", async () => {
		const plan = planOf(APPS.app_invoice_ai, {
			deviceIds: [STUDIO],
			eventId: RECONCILE,
		});
		const { fake, sink, state } = await mountRun(plan);
		serveArtifacts(fake.agent(STUDIO));
		fake.hub.schedules.release(INVOICE, RECONCILE, EDGE, "invoice-extractor");
		fake.hub.schedules.claim(EDGE, "invoice-extractor", [RECONCILE]);

		await start(sink);
		await until(() => state().status === "finished", "the refusal to show");

		expect(state().rows[0]?.error).toMatchObject({
			phase: "schedules",
			code: "schedule_elsewhere",
			detail: "Nightly reconciliation",
		});
		expect(types(fake, STUDIO)).toEqual([]);
		expect(grants(fake, STUDIO)).toEqual([]);
		expect(fake.hub.schedules.listing(INVOICE)).toMatchObject([
			{ event_id: RECONCILE, state: "device", device_id: EDGE },
		]);
	});
});

describe("a deploy with a bot, an Endpoint or a form (R2 §6.2, §8.4)", () => {
	const SHOP = "app_shop_assistant";
	const TELEGRAM = "evt_shop_telegram";
	const FORM = "evt_shop_return";
	/** A piece of the token saved on the Telegram event's record. */
	const SAVED = "AAHfixture-token";
	const OLD_AGENT = {
		placement_events: 1,
		placement_diagnostics: 1,
		scheduled_events: 1,
	} as const;

	/** What the wizard plans: the draft with the token saved in Events as its default. */
	function shopPlan(eventId: string, change: Partial<DeployDraft> = {}) {
		deployments += 1;
		const app = APPS.app_shop_assistant;
		const draft = makePlan({
			scope: { kind: "app", appId: app.id },
			route: { appId: app.id, deviceIds: [EDGE], eventId },
			app,
			deploymentId: `dep-run-${deployments}`,
			now: NOW0,
		});
		const facts = {
			app,
			devices: DEVICES,
			platform: "desktop" as const,
			now: NOW0,
		};
		return resolvePlan(
			withSavedBotTokens({ ...draft, ...change }, facts),
			facts,
		);
	}

	const sessionText = () =>
		Array.from({ length: globalThis.sessionStorage.length }, (_, index) =>
			globalThis.sessionStorage.getItem(
				globalThis.sessionStorage.key(index) ?? "",
			),
		).join("\n");

	const metadataPaths = (fake: FakeWorkspace) =>
		fake.api.sent("GET", /device-metadata/).map(([, path]) => path);

	test("a bot is released before the device hears of it, and the token saved in Events travels only inside set_secret", async () => {
		const plan = shopPlan(TELEGRAM);
		const { fake, sink, state } = await mountRun(plan);
		const agent = fake.agent(EDGE);
		serveArtifacts(agent);
		const secrets: Record<string, unknown>[] = [];
		agent.handle("set_secret", (command) => {
			secrets.push(command);
			return {
				state: "completed",
				result: {
					placement_id: command.placement_id,
					name: command.name,
					secret: "completed",
				},
			};
		});
		const release = agent.hold("apply");
		expect(state().rows[0]?.phases).toEqual([
			"schedules",
			"approve",
			"upload",
			"install",
			"create",
			"secrets",
			"start",
		]);

		await start(sink);
		await until(
			() => sent(fake, EDGE, "apply").length === 1,
			"the config to leave",
		);
		// The hub holds the release before the device hears of the bot.
		expect(fake.hub.schedules.listing(SHOP)).toMatchObject([
			{
				event_id: TELEGRAM,
				state: "released",
				device_id: EDGE,
				placement_id: "shop-helper",
			},
		]);
		await act(async () => release());
		await until(() => state().status === "finished", "the deploy to finish");

		expect(state().rows[0]?.state).toBe("done");
		expect(metadataPaths(fake)).toEqual([
			`apps/${SHOP}/device-metadata?types=telegram`,
		]);
		const [apply] = sent(fake, EDGE, "apply");
		const config = (
			apply?.[2] as { config: { secret_overrides: Record<string, string> } }
		).config;
		expect(Object.keys(config.secret_overrides)).toEqual([
			`event.${TELEGRAM}.bot_token`,
		]);
		expect(JSON.stringify(secrets)).toContain(SAVED);
		const others = fake.api.commands.filter(
			([, type]) => type !== "set_secret",
		);
		expect(JSON.stringify(others)).not.toContain(SAVED);
		expect(JSON.stringify(fake.api.calls)).not.toContain(SAVED);
		expect(JSON.stringify(fake.workspace.activity.list())).not.toContain(SAVED);
		expect(sessionText()).not.toContain(SAVED);
	});

	test("an agent without the bot's flag gets nothing of it: no approval, upload, apply or staged update", async () => {
		const plan = shopPlan(TELEGRAM);
		const { fake, sink, state } = await mountRun(
			plan,
			{},
			{
				agentFeatures: OLD_AGENT,
			},
		);
		serveArtifacts(fake.agent(EDGE));

		await start(sink);
		await until(() => state().status === "finished", "the gate to show");

		expect(state().rows[0]?.error).toMatchObject({
			code: "agent_feature",
			detail: "telegram_bots",
		});
		expect(types(fake, EDGE)).toEqual([]);
		expect(grants(fake, EDGE)).toEqual([]);
		expect(fake.api.sent("PUT", /device-schedules/)).toEqual([]);
	});

	test("an http event whose route names no method needs the Endpoint flag: an older agent gets nothing", async () => {
		const app = {
			...APPS.app_invoice_ai,
			events: APPS.app_invoice_ai.events.map((event) =>
				event.id === "evt_extract_http"
					? { ...event, config: configBytes({ path: "/extract" }) }
					: event,
			),
		};
		const plan = planOf(app, {
			deviceIds: [STUDIO],
			eventId: "evt_extract_http",
		});
		const { fake, sink, state } = await mountRun(
			plan,
			{},
			{
				agentFeatures: OLD_AGENT,
			},
		);
		serveArtifacts(fake.agent(STUDIO));

		await start(sink);
		await until(() => state().status === "finished", "the gate to show");

		expect(state().rows[0]?.error).toMatchObject({
			code: "agent_feature",
			detail: "api_events",
		});
		expect(types(fake, STUDIO)).toEqual([]);
		expect(grants(fake, STUDIO)).toEqual([]);
	});

	test("a form's explicit Studio opt-in sends a listener and scoped access token", async () => {
		const plan = shopPlan(FORM, { hostOnDemand: true });
		const { fake, sink, state } = await mountRun(plan);
		serveArtifacts(fake.agent(EDGE));
		await start(sink);
		await until(() => state().status === "finished", "the hosted form");
		expect(state().rows[0]?.error).toBeUndefined();
		const detail = Object.values(sink.details ?? {})[0];
		expect(detail?.token?.length).toBeGreaterThanOrEqual(32);
		expect(sent(fake, EDGE, "set_secret")).toHaveLength(1);
		const applied = sent(fake, EDGE, "apply")[0];
		expect(JSON.stringify(applied)).toContain('"auth_secret":"service-access"');
		expect(JSON.stringify(applied)).not.toContain(
			detail?.token ?? "missing-token",
		);
		expect(sessionText()).not.toContain(detail?.token ?? "missing-token");
	});

	test("a run that prepares by itself names the new types: none for a Page, generic_form for a form", async () => {
		const page = visitorPlan([EDGE]);
		const first = await mountRun(page);
		serveArtifacts(first.fake.agent(EDGE));
		await start(first.sink);
		await until(() => first.state().status === "finished", "the Page");
		expect(metadataPaths(first.fake)).toEqual([
			"apps/app_visitor_checkin/device-metadata",
		]);
		// A deploy without a new type reads no placement list for it.
		expect(
			first.fake.api.sent("GET", /device-placements/).length,
		).toBeLessThanOrEqual(1);
		await cleanupDevices();

		const form = shopPlan(FORM);
		const second = await mountRun(form);
		serveArtifacts(second.fake.agent(EDGE));
		await start(second.sink);
		await until(() => second.state().status === "finished", "the form");
		expect(metadataPaths(second.fake)).toEqual([
			`apps/${SHOP}/device-metadata?types=generic_form`,
		]);
	});
});

describe("a run that prepares by itself: events that follow Latest", () => {
	const INVOICE = "app_invoice_ai";
	const REVIEW = "evt_invoice_review";

	const reviewPlan = () =>
		planOf(APPS.app_invoice_ai, { deviceIds: [STUDIO], eventId: REVIEW });

	test("flow edits become a version first, and the service is created at that version", async () => {
		const plan = reviewPlan();
		const { fake, sink, state } = await mountRun(plan);
		serveArtifacts(fake.agent(STUDIO));
		fake.hub.flows.edit(INVOICE, "flow_review");

		await start(sink);
		await until(() => state().status === "finished", "the deploy to finish");

		expect(state().rows[0]?.state).toBe("done");
		expect(fake.api.sent("POST", /version\/current/)).toHaveLength(1);
		expect(
			fake.api.sent("GET", /device-metadata/).map(([, path]) => path),
		).toEqual([`apps/${INVOICE}/device-metadata?latest=${REVIEW}`]);
		const [created] = sent(fake, STUDIO, "apply");
		const config = (
			created?.[2] as {
				config: { events: { event_id: string; board_version: number[] }[] };
			}
		).config;
		expect(config.events).toMatchObject([
			{ event_id: REVIEW, board_version: [0, 9, 3] },
		]);
	});

	test("a flow that can't become a version stops the run before anything reaches a device", async () => {
		const plan = reviewPlan();
		const { fake, sink, state } = await mountRun(plan);
		serveArtifacts(fake.agent(STUDIO));
		fake.hub.flows.edit(INVOICE, "flow_review");
		fake.hub.flows.canPublish = false;

		await start(sink);
		await until(() => state().status === "finished", "the refusal to show");

		expect(state().shared?.error?.code).toBe("flow_role");
		expect(types(fake, STUDIO)).toEqual([]);
		expect(fake.api.sent("GET", /device-metadata/)).toEqual([]);
	});
});
