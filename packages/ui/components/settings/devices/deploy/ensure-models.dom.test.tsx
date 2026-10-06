import {
	afterAll,
	afterEach,
	beforeEach,
	describe,
	expect,
	test,
} from "bun:test";
import { sha256 } from "@noble/hashes/sha2";
import { act } from "react";
import type {
	DeployPlan,
	PlanDevice,
} from "../../../../lib/device-management/model/deploy-plan";
import type { DeployRunState } from "../../../../lib/device-management/model/deploy-run";
import type { ModelAssetStatus } from "../../../../lib/device-management/models";
import type { TunnelModelAssetPush } from "../../../../lib/device-management/tunnel-data";
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
const { ModelFilesBlock } = await import("./model-files-block");
const { ensureModelsSeams } = await import("./ensure-models");
const { modelPushSeams } = await import(
	"../../../../lib/device-management/model-push"
);
const { prepareProjectArtifact } = await import(
	"../../../../lib/device-management/artifacts"
);
const { makePlan, resolvePlan } = await import(
	"../../../../lib/device-management/model/deploy-plan"
);
const { NOW0, PLAN_DEVICES, VISITOR_CATALOG, VISITOR_PLAN_APP } = await import(
	"../../../../lib/device-management/model/__fixtures__/apps"
);
const { SAMPLE_IDS } = await import(
	"../../../../lib/device-management/model/__fixtures__/sample-fleet"
);

const EDGE = SAMPLE_IDS.edge;
const DEVICES: Record<string, PlanDevice> = {
	[EDGE]: { ...PLAN_DEVICES["edge-berlin-01"], id: EDGE },
};
const PROJECT = "app_visitor_checkin";
const HASH = "9a129038d9a00aed0cf6a7ea059ca50a813449061ab87848cf1a13eafdf33b2c";
const DIGEST = { algorithm: "blake3" as const, hex: HASH };
const JOB = "12345678-1234-4234-8234-123456789abc";
const SIZE = 4096;
const SOURCE = "https://cdn.flow-like.com/bits/qwen";
const encoder = new TextEncoder();
const hex = (bytes: Uint8Array) =>
	Array.from(bytes, (byte) => byte.toString(16).padStart(2, "0")).join("");

const seams = {
	ensure: { ...ensureModelsSeams },
	push: modelPushSeams.retryMs,
};
const originalFetch = globalThis.fetch;
beforeEach(() => {
	ensureModelsSeams.pollMs = 1;
	modelPushSeams.retryMs = [0, 0, 0];
});
afterEach(async () => {
	Object.assign(ensureModelsSeams, seams.ensure);
	modelPushSeams.retryMs = seams.push;
	globalThis.fetch = originalFetch;
	await cleanupDevices();
	await dom.cleanup();
});
afterAll(dom.restore);

let deployments = 0;

function modelPlan(): DeployPlan {
	deployments += 1;
	const draft = makePlan({
		scope: { kind: "app", appId: VISITOR_PLAN_APP.id },
		route: {
			appId: VISITOR_PLAN_APP.id,
			deviceIds: [EDGE],
			eventId: "evt_visitor_page",
		},
		app: VISITOR_PLAN_APP,
		deploymentId: `dep-models-${deployments}`,
		now: NOW0,
	});
	return resolvePlan(draft, {
		app: VISITOR_PLAN_APP,
		devices: DEVICES,
		platform: "desktop",
		now: NOW0,
	});
}

/** An online bundle whose one Bit's weights the device acquires itself (metadata v2). */
async function modelBundle(): Promise<DeployPrepared> {
	const metadata = encoder.encode(
		JSON.stringify({
			version: 2,
			bit: {
				id: "qwen",
				hub: "hub.test",
				hash: HASH,
				file_name: "qwen.gguf",
				size: SIZE,
			},
			dependencies: [],
			assets: [
				{
					bit_id: "qwen",
					descriptor: {
						digest: DIGEST,
						size: SIZE,
						file_name: "qwen.gguf",
						sources: [SOURCE],
					},
				},
			],
		}),
	);
	const artifact = await prepareProjectArtifact(
		PROJECT,
		[
			{
				path: `apps/${PROJECT}/online-source.json`,
				file: new Blob(['{"version":1}']),
			},
			{ path: "bits/metadata/qwen.json", file: new Blob([metadata]) },
		],
		undefined,
		{
			bit_pins: [{ bit_id: "qwen", metadata_sha256: hex(sha256(metadata)) }],
			package_pins: [],
		},
		"online",
	);
	return {
		artifact,
		approved: {
			app: { id: PROJECT } as never,
			file: {
				path: `apps/${PROJECT}/online-metadata.json`,
				file: new Blob(["{}"]),
			},
			sha256:
				"e647322ba8a0deb7e20632b5a4d4d61a356f13ee81ca306c8a9f58bf47d7e14d",
			catalog: VISITOR_CATALOG,
		},
		preparedAt: 0,
	};
}

/** The device takes a bundle at once. */
function serveArtifacts(agent: FakeAgent) {
	agent.handle("artifact", (command, { operationId }) => {
		const descriptor = (
			command.request as { descriptor: Record<string, unknown> }
		).descriptor as {
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
	});
}

/** The device's download of the weights, as the test moves it on. */
interface Download {
	state: Record<string, unknown> | "present";
}

function serveModels(agent: FakeAgent, download: Download) {
	const kinds: string[] = [];
	agent.handle("models", (command) => {
		const { kind } = command.request as { kind: string };
		kinds.push(kind);
		const present = download.state === "present";
		if (kind === "ensure")
			return {
				state: "completed",
				result: { total: 1, present: present ? 1 : 0, pending: [] },
			};
		const jobs =
			download.state === "present"
				? []
				: [
						{
							job_id: JOB,
							digest: DIGEST,
							size: SIZE,
							file_name: "qwen.gguf",
							updated_at: 1,
							...download.state,
						},
					];
		return { state: "completed", result: { jobs, next: null } };
	});
	return kinds;
}

/** Rejects once `signal` aborts, as the data client does: a cancelled push stops at once. */
function abortOf(signal: AbortSignal | undefined) {
	const aborted = new Promise<never>((_, reject) =>
		signal?.addEventListener("abort", () => reject(signal.reason), {
			once: true,
		}),
	);
	void aborted.catch(() => {});
	return aborted;
}

interface PushControl {
	calls: { offset: number; bytes: number }[];
	/** Resolves the next data push once set; a push waits for it. */
	gate?: Promise<void>;
}

/** The session manager's push once live.ts offers it: a device that keeps what it gets. */
function servePush(fake: FakeWorkspace, download: Download): PushControl {
	const control: PushControl = { calls: [] };
	const live = fake.workspace.live as unknown as Record<string, unknown>;
	live.pushModelAsset = async (
		_deviceId: string,
		input: TunnelModelAssetPush,
	): Promise<ModelAssetStatus> => {
		if (!input.file) {
			control.calls.push({ offset: input.offset, bytes: 0 });
			return { digest: DIGEST, job_id: JOB, state: "awaiting_push", bytes: 0 };
		}
		const aborted = abortOf(input.signal);
		let bytes = 0;
		for (let offset = input.offset; offset < input.file.size; offset += 1024) {
			await Promise.race([control.gate ?? Promise.resolve(), aborted]);
			input.signal?.throwIfAborted();
			const end = Math.min(input.file.size, offset + 1024);
			bytes += (await input.file.slice(offset, end).arrayBuffer()).byteLength;
			input.onProgress?.(end);
		}
		control.calls.push({ offset: input.offset, bytes });
		download.state = "present";
		return { digest: DIGEST, state: "present" };
	};
	return control;
}

interface Sink {
	handle?: Run.DeployRunHandle;
	details?: Run.DeployRunDetails;
}

function Probe({ plan, sink }: Readonly<{ plan: DeployPlan; sink: Sink }>) {
	sink.handle = run.useDeployRun(plan, {
		title: run.deployRunTitleRef(plan),
		oneAtATime: true,
		stopOnFail: true,
	});
	sink.details = run.useDeployRunDetails(plan.draft.deploymentId);
	return (
		<ModelFilesBlock
			deploymentId={plan.draft.deploymentId}
			state={sink.handle.state}
			details={sink.details}
		/>
	);
}

async function mountRun(options: MountDevicesOptions = {}) {
	const plan = modelPlan();
	const sink: Sink = {};
	const mounted = await mountDevices(<Probe plan={plan} sink={sink} />, {
		platform: "web",
		...options,
	});
	run.setDeployRunExtras(plan.draft.deploymentId, {
		prepared: await modelBundle(),
	});
	const agent = mounted.fake.agent(EDGE);
	agent.features = { ...agent.features, model_store: 1 };
	await act(async () => mounted.fake.workspace.live.refreshInspection(EDGE));
	// The DOM install brings its own fetch; the model's source answers here.
	globalThis.fetch = (async () =>
		new Response(new Uint8Array(SIZE).fill(7))) as unknown as typeof fetch;
	const state = () => sink.handle?.state as DeployRunState;
	const target = `${EDGE}/${plan.targets[0]?.services[0]?.serviceId ?? ""}`;
	return { ...mounted, plan, sink, state, target };
}

async function until(done: () => boolean, what: string, rounds = 400) {
	for (let round = 0; round < rounds; round++) {
		if (done()) return;
		await settle();
	}
	throw new Error(`Timed out waiting for ${what}.`);
}

/** What the deploy sends, in order: the upload by its `begin`, model checks, apply and start. */
const commandTypes = (fake: FakeWorkspace) =>
	fake.api.commands
		.filter((command) => command[0] === EDGE)
		.map(([, type, command]) => {
			const kind = (command.request as { kind?: string } | undefined)?.kind;
			return type === "models" || type === "artifact"
				? `${type}:${kind}`
				: type;
		})
		.filter((type) =>
			["artifact:begin", "apply", "start", "models:ensure"].includes(type),
		);

const text = () => document.body.textContent?.replace(/\s+/g, " ") ?? "";

async function start(sink: Sink) {
	await act(async () => {
		sink.handle?.start();
	});
}

describe("ensure models in a deploy", () => {
	test("the device downloads the version's model file itself, and the version is applied once it holds it", async () => {
		const download: Download = {
			state: {
				state: "fetching",
				source_index: 0,
				bytes: 2048,
				source_host: "cdn.flow-like.com",
				bytes_per_second: 1024,
			},
		};
		const { fake, sink, state, target } = await mountRun();
		serveArtifacts(fake.agent(EDGE));
		const kinds = serveModels(fake.agent(EDGE), download);

		await start(sink);
		await until(
			() => sink.details?.[target]?.models?.[0]?.state === "downloading",
			"the download to show",
		);
		expect(text()).toContain("Model files");
		expect(text()).toContain("Device is downloading");
		expect(text()).toContain("from cdn.flow-like.com");
		expect(commandTypes(fake)).toEqual(["artifact:begin", "models:ensure"]);

		download.state = "present";
		await until(() => state().status === "finished", "the deploy to finish");
		expect(state().rows[0]?.state).toBe("done");
		expect(commandTypes(fake)).toEqual([
			"artifact:begin",
			"models:ensure",
			"models:ensure",
			"apply",
			"start",
		]);
		expect(kinds).toContain("jobs");
		expect(sink.details?.[target]?.models?.map((file) => file.state)).toEqual([
			"present",
		]);
		expect(text()).not.toContain("Model files");
	});

	test("a file the device can't download is sent from this computer, visibly, and then applied", async () => {
		const download: Download = {
			state: { state: "failed", reason: "egress_blocked" },
		};
		const { fake, sink, state, target } = await mountRun();
		serveArtifacts(fake.agent(EDGE));
		serveModels(fake.agent(EDGE), download);
		const push = servePush(fake, download);
		let release!: () => void;
		push.gate = new Promise((resolve) => {
			release = resolve;
		});
		const asked: string[] = [];
		globalThis.fetch = (async (input: RequestInfo | URL) => {
			asked.push(String(input));
			return new Response(new Uint8Array(SIZE).fill(7));
		}) as unknown as typeof fetch;

		await start(sink);
		await until(
			() => sink.details?.[target]?.models?.[0]?.state === "sending",
			"the push to start",
		);
		expect(text()).toContain("Stop sending");
		await act(async () => release());
		await until(() => state().status === "finished", "the deploy to finish");

		expect(state().rows[0]?.state).toBe("done");
		expect(push.calls).toEqual([
			{ offset: 0, bytes: 0 },
			{ offset: 0, bytes: SIZE },
		]);
		expect(asked).toEqual([SOURCE, SOURCE]);
		expect(commandTypes(fake).at(-2)).toBe("apply");
	});

	test("stopping the push fails the device with that reason; a retry sends the file again", async () => {
		const download: Download = {
			state: { state: "failed", reason: "egress_blocked" },
		};
		const { fake, sink, state, target } = await mountRun();
		serveArtifacts(fake.agent(EDGE));
		serveModels(fake.agent(EDGE), download);
		const push = servePush(fake, download);
		push.gate = new Promise(() => {});

		await start(sink);
		await until(() => text().includes("Stop sending"), "the stop button");
		const stop = [...document.querySelectorAll("button")].find((button) =>
			button.textContent?.includes("Stop sending"),
		);
		await act(async () => stop?.click());
		await until(() => state().status === "finished", "the stop to land");
		const [row] = state().rows;
		expect(row?.state).toBe("failed");
		expect(row?.error).toMatchObject({
			phase: "install",
			code: "model_push_cancelled",
			detail: "qwen.gguf",
		});
		expect(commandTypes(fake)).not.toContain("apply");

		push.gate = undefined;
		await act(async () => sink.handle?.retry(target));
		await until(() => state().rows[0]?.state === "done", "the retry to finish");
		expect(push.calls.at(-1)).toEqual({ offset: 0, bytes: SIZE });
	});

	test("when no source delivers the file, the device keeps its job and nothing is applied", async () => {
		const download: Download = {
			state: { state: "failed", reason: "egress_blocked" },
		};
		const { fake, sink, state, target } = await mountRun();
		serveArtifacts(fake.agent(EDGE));
		serveModels(fake.agent(EDGE), download);
		const push = servePush(fake, download);
		globalThis.fetch = (async () =>
			new Response("", { status: 404 })) as unknown as typeof fetch;

		await start(sink);
		await until(() => state().status === "finished", "the failure");
		expect(state().rows[0]?.error).toMatchObject({
			code: "model_push",
			detail: "qwen.gguf",
		});
		expect(sink.details?.[target]?.models?.[0]).toMatchObject({
			state: "failed",
			error:
				"None of the sources of qwen.gguf delivered it (cdn.flow-like.com: HTTP 404).",
		});
		expect(push.calls).toEqual([]);
		expect(text()).toContain("Sending it from this computer failed");
		expect(commandTypes(fake)).not.toContain("apply");
	});

	test("after a lock and an unlock the deploy follows the device's download again before its flags are read again", async () => {
		const download: Download = {
			state: { state: "fetching", source_index: 0, bytes: 2048 },
		};
		const { fake, sink, state, target } = await mountRun();
		serveArtifacts(fake.agent(EDGE));
		const kinds = serveModels(fake.agent(EDGE), download);
		const rowState = () => state().rows[0]?.state;
		const fileState = () => sink.details?.[target]?.models?.[0]?.state;
		const askedSince = (count: number, kind: string) =>
			kinds.slice(count).includes(kind);

		await start(sink);
		await until(() => fileState() === "downloading", "the download to show");
		await act(async () => fake.workspace.keys.lock(EDGE));
		await until(() => rowState() === "blocked", "the row to wait");
		expect(fake.workspace.live.inspection(EDGE)).toBeUndefined();
		const release = fake.agent(EDGE).hold("inspect_page");
		const before = kinds.length;

		await act(async () => fake.unlock(EDGE));
		await until(
			() => askedSince(before, "ensure"),
			"the device to be asked again",
		);
		await act(async () => release());
		await until(
			() => askedSince(before, "jobs"),
			"the download to be read again",
		);
		expect(rowState()).toBe("active");
		download.state = "present";
		await until(() => state().status === "finished", "the deploy to finish");
		expect(rowState()).toBe("done");
	});

	test("an agent without a model store gets no v2 bundle", async () => {
		const { fake, sink, state } = await mountRun();
		const agent = fake.agent(EDGE);
		const { model_store: _store, model_host: _host, ...older } = agent.features;
		agent.features = older;
		await act(async () => fake.workspace.live.refreshInspection(EDGE));
		serveArtifacts(agent);

		await start(sink);
		await until(() => state().status === "finished", "the refusal");
		expect(state().rows[0]?.error).toMatchObject({
			code: "agent_feature",
			detail: "model_store",
		});
		expect(commandTypes(fake)).toEqual([]);
	});
});
