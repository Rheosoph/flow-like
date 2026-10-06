import {
	afterAll,
	afterEach,
	beforeEach,
	describe,
	expect,
	test,
} from "bun:test";
import type {
	ModelAssetStatus,
	ModelJob,
} from "../../../../lib/device-management/models";
import type { TunnelModelAssetPush } from "../../../../lib/device-management/tunnel-data";
import { byRole, click, installDom } from "../testing/dom-harness";

const dom = installDom();
const kit = await import("../device/device-test-kit");
const samples = await import(
	"../../../../lib/device-management/model/__fixtures__/sample-models"
);
const { modelPushSeams, rememberPushableAssets } = await import(
	"../../../../lib/device-management/model-push"
);
const { DeviceTunnelDataClient } = await import(
	"../../../../lib/device-management/tunnel-data"
);
const { createFakeWorkspace } = await import("../testing/fake-workspace");
const { modelsKeys } = await import("./use-models");

const { IDS, MACHINE } = kit;
const { MODEL_HOST_FEATURES, SAMPLE_JOB_IDS, gpuBoxModels } = samples;
const SIZE = 4096;
const SOURCE =
	"https://huggingface.co/google/gemma-3-4b-it-GGUF/resolve/main/mmproj.gguf";
const originalFetch = globalThis.fetch;
const originalPush = DeviceTunnelDataClient.prototype.pushModelAsset;
const retries = modelPushSeams.retryMs;

beforeEach(() => {
	modelPushSeams.retryMs = [0, 0, 0];
});
afterEach(async () => {
	modelPushSeams.retryMs = retries;
	globalThis.fetch = originalFetch;
	DeviceTunnelDataClient.prototype.pushModelAsset = originalPush;
	await kit.resetDevices();
	await dom.cleanup();
});
afterAll(dom.restore);

/** The GPU box with its blocked projector download made small enough to send in a test. */
function smallSample(hex: string, sources?: string[]) {
	const sample = gpuBoxModels();
	sample.jobs = sample.jobs.map((job) =>
		job.job_id === SAMPLE_JOB_IDS.blocked
			? {
					...job,
					size: SIZE,
					digest: { algorithm: "sha256", hex },
					...(sources ? { sources } : {}),
				}
			: job,
	);
	return sample;
}

type Fake = Awaited<ReturnType<typeof createFakeWorkspace>>;

function fakeFor(hex: string, sources?: string[]): Promise<Fake> {
	return createFakeWorkspace(undefined, {
		platform: "web",
		agentFeatures: { [IDS.edge]: MODEL_HOST_FEATURES },
		modelHosts: { [IDS.edge]: smallSample(hex, sources) },
	});
}

const open = (fake: Fake) => kit.openDevice(IDS.edge, { tab: "models", fake });

type View = Awaited<ReturnType<typeof open>>;

const blocked = (view: View) =>
	view.container.querySelector(
		`[data-job="${SAMPLE_JOB_IDS.blocked}"]`,
	) as HTMLElement;

const text = (root: ParentNode) =>
	(root.textContent ?? "").replace(/\s+/g, " ");

function remember(hex: string) {
	rememberPushableAssets([
		{
			pin: "gemma",
			bitId: "gemma-projector",
			bitHash: "projector",
			descriptor: {
				digest: { algorithm: "sha256", hex },
				size: SIZE,
				file_name: "mmproj-gemma-3-4b-it-f16.gguf",
				sources: [SOURCE],
			},
		},
	]);
}

/** A device that takes what it is sent; `verify` holds its answer after the last byte. */
function device(fake: Fake, hex: string) {
	const calls: { offset: number; bytes: number }[] = [];
	const digest = { algorithm: "sha256" as const, hex };
	let verified = () => {};
	const verifying = { held: false, aborted: false };
	const push = async (
		input: TunnelModelAssetPush,
	): Promise<ModelAssetStatus> => {
		if (!input.file) {
			calls.push({ offset: input.offset, bytes: 0 });
			return { digest, job_id: input.jobId, state: "awaiting_push", bytes: 0 };
		}
		let bytes = 0;
		for (let offset = input.offset; offset < input.file.size; offset += 1024) {
			const end = Math.min(input.file.size, offset + 1024);
			bytes += (await input.file.slice(offset, end).arrayBuffer()).byteLength;
			input.onProgress?.(end);
		}
		calls.push({ offset: input.offset, bytes });
		if (verifying.held)
			await new Promise<void>((resolve, reject) => {
				verified = resolve;
				input.signal?.addEventListener(
					"abort",
					() => {
						verifying.aborted = true;
						reject(input.signal?.reason);
					},
					{ once: true },
				);
			});
		const host = fake.agent(IDS.edge).models;
		host.state.jobs = host.state.jobs.filter(
			(job) => job.job_id !== input.jobId,
		);
		return { digest, state: "present" };
	};
	return { push, calls, verifying, verify: () => verified() };
}

/** The session manager's push replaced by the device's, as the old DOM tests did. */
function patchLive(fake: Fake, push: ReturnType<typeof device>["push"]) {
	const live = fake.workspace.live as unknown as Record<string, unknown>;
	live.pushModelAsset = (_deviceId: string, input: TunnelModelAssetPush) =>
		push(input);
}

async function until(view: View, done: () => boolean, what: string) {
	for (let round = 0; round < 200; round++) {
		if (done()) return;
		await view.settle();
	}
	throw new Error(`Timed out waiting for ${what}.`);
}

const sendButton = (root: HTMLElement) =>
	[...root.querySelectorAll("button")].find((button) =>
		button.textContent?.startsWith("Send from this computer"),
	);

const pushItems = (fake: Fake) =>
	fake.workspace.activity
		.list()
		.filter((item) => item.label.params?.request === "push");

/** The device reports the blocked download as `verifying`, and the tab reads it again. */
async function deviceVerifies(view: View, fake: Fake) {
	const host = fake.agent(IDS.edge).models;
	host.state.jobs = host.state.jobs.map((job) => {
		if (job.job_id !== SAMPLE_JOB_IDS.blocked) return job;
		const {
			reason: _reason,
			http_status: _status,
			...rest
		} = job as ModelJob & Record<string, unknown>;
		return { ...(rest as ModelJob), state: "verifying" } as ModelJob;
	});
	await fake.queryClient.invalidateQueries({
		queryKey: modelsKeys.jobs(fake.workspace.scopeKey, IDS.edge),
	});
	await view.settle();
}

describe("Send from this computer", () => {
	test("a download the device can't finish is sent from its known source and then leaves the list", async () => {
		const hex = "5".repeat(64);
		remember(hex);
		const fake = await fakeFor(hex);
		const target = device(fake, hex);
		patchLive(fake, target.push);
		const view = await open(fake);
		const asked: string[] = [];
		globalThis.fetch = (async (input: RequestInfo | URL) => {
			asked.push(String(input));
			return new Response(new Uint8Array(SIZE).fill(1));
		}) as unknown as typeof fetch;

		const button = sendButton(blocked(view));
		expect(button?.textContent).toBe("Send from this computer (4.0 KiB)");
		await click(button as HTMLElement);
		await until(view, () => target.calls.length === 2, "the push to finish");
		await until(
			view,
			() => !blocked(view),
			"the finished download to leave the list",
		);
		expect(target.calls).toEqual([
			{ offset: 0, bytes: 0 },
			{ offset: 0, bytes: SIZE },
		]);
		expect(asked).toEqual([SOURCE, SOURCE]);
		expect(pushItems(fake).map((item) => item.state)).toEqual(["done"]);
		expect(MACHINE.test(text(view.container))).toBe(false);
	});

	test("a push in progress shows its progress and can be stopped", async () => {
		const hex = "6".repeat(64);
		remember(hex);
		const fake = await fakeFor(hex);
		const live = fake.workspace.live as unknown as Record<string, unknown>;
		live.pushModelAsset = async (
			_deviceId: string,
			input: TunnelModelAssetPush,
		): Promise<ModelAssetStatus> => {
			const digest = { algorithm: "sha256" as const, hex };
			if (!input.file)
				return {
					digest,
					job_id: input.jobId,
					state: "awaiting_push",
					bytes: 0,
				};
			return new Promise((_, reject) =>
				input.signal?.addEventListener(
					"abort",
					() => reject(input.signal?.reason),
					{ once: true },
				),
			);
		};
		const view = await open(fake);
		globalThis.fetch = (async () =>
			new Response(new Uint8Array(SIZE))) as unknown as typeof fetch;
		await click(sendButton(blocked(view)) as HTMLElement);
		await until(
			view,
			() => text(blocked(view)).includes("Stop sending"),
			"the progress",
		);
		expect(text(blocked(view))).toContain("Sending from this computer");
		expect(text(blocked(view))).toContain("0 B of 4.0 KiB");
		expect(pushItems(fake).map((item) => item.state)).toEqual(["active"]);
		const stop = [...blocked(view).querySelectorAll("button")].find((button) =>
			button.textContent?.includes("Stop sending"),
		);
		await click(stop as HTMLElement);
		await until(
			view,
			() => text(blocked(view)).includes("Stopped."),
			"the stop",
		);
		expect(text(blocked(view))).toContain(
			"Stopped. The device keeps what it received; sending again continues from there.",
		);
		expect(pushItems(fake).map((item) => item.state)).toEqual(["failed"]);
	});

	test("a push goes on while the device verifies the file and after its tab closes", async () => {
		const hex = "7".repeat(64);
		remember(hex);
		const fake = await fakeFor(hex);
		const target = device(fake, hex);
		target.verifying.held = true;
		patchLive(fake, target.push);
		const view = await open(fake);
		globalThis.fetch = (async () =>
			new Response(new Uint8Array(SIZE).fill(2))) as unknown as typeof fetch;

		await click(sendButton(blocked(view)) as HTMLElement);
		await until(
			view,
			() =>
				text(blocked(view)).includes(
					"All of it is sent. The device is checking the file.",
				),
			"the last byte",
		);
		expect(text(blocked(view))).not.toContain("Stop sending");
		await deviceVerifies(view, fake);
		expect(text(blocked(view))).toContain("Verifying");
		expect(text(blocked(view))).toContain(
			"All of it is sent. The device is checking the file.",
		);
		expect(target.verifying.aborted).toBe(false);

		await click(byRole("tab", /^Overview/, view.container));
		await view.settle();
		expect(blocked(view)).toBeNull();
		expect(target.verifying.aborted).toBe(false);
		expect(pushItems(fake).map((item) => item.state)).toEqual(["active"]);

		target.verify();
		await until(
			view,
			() => pushItems(fake)[0]?.state === "done",
			"the device's answer",
		);
		expect(target.verifying.aborted).toBe(false);
	});

	test("the live session sends the file itself, with the source the device names", async () => {
		const hex = "8".repeat(64);
		const fake = await fakeFor(hex, [SOURCE]);
		const target = device(fake, hex);
		DeviceTunnelDataClient.prototype.pushModelAsset = (input) =>
			target.push(input);
		const view = await open(fake);
		const asked: string[] = [];
		globalThis.fetch = (async (input: RequestInfo | URL) => {
			asked.push(String(input));
			return new Response(new Uint8Array(SIZE).fill(3));
		}) as unknown as typeof fetch;

		await click(sendButton(blocked(view)) as HTMLElement);
		await until(
			view,
			() => !blocked(view),
			"the sent download to leave the list",
		);
		expect(target.calls).toEqual([
			{ offset: 0, bytes: 0 },
			{ offset: 0, bytes: SIZE },
		]);
		expect(asked).toEqual([SOURCE, SOURCE]);
	});

	test("a file nobody names a source for says what to do instead; a running download shows nothing", async () => {
		const fake = await fakeFor("9".repeat(64));
		const view = await open(fake);
		expect(sendButton(blocked(view))).toBeUndefined();
		expect(text(blocked(view))).toContain(
			"The device lists no download address for this file. The desktop app can send it from its own copy of the model.",
		);
		const fetching = view.container.querySelector(
			`[data-job="${SAMPLE_JOB_IDS.fetching}"]`,
		) as HTMLElement;
		expect(fetching.querySelector("[data-send-model]")).toBeNull();
		expect(text(fetching)).not.toContain("download address");
	});
});
