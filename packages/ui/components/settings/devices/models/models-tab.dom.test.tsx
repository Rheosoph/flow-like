import { afterAll, afterEach, describe, expect, spyOn, test } from "bun:test";
import { getI18n } from "@flow-like/locales";
import {
	allByRole,
	byRole,
	click,
	inPortal,
	installDom,
	queryByRole,
} from "../testing/dom-harness";

const dom = installDom();
const kit = await import("../device/device-test-kit");
const samples = await import(
	"../../../../lib/device-management/model/__fixtures__/sample-models"
);
const { SAMPLE_NOW } = await import(
	"../../../../lib/device-management/model/__fixtures__/sample-fleet"
);
const { createFakeWorkspace } = await import("../testing/fake-workspace");
const { mountDevices } = await import("../testing/mount-devices");
const { activityTitle } = await import("../shell/activity-tray");
const { DeviceModelsTab } = await import("./models-tab");
const { useModelsAction } = await import("./use-models-action");

const { IDS, MACHINE, commandTypes, lastNavigation, openDevice, primaries } =
	kit;
const {
	MAC_MODEL_HOST_FEATURES,
	MODEL_HOST_FEATURES,
	PRE_MODEL_FEATURES,
	SAMPLE_DIGESTS,
	SAMPLE_JOB_IDS,
	crowdedMacModels,
	emptyModels,
	gpuBoxModels,
	macMiniModels,
} = samples;

type DevicesT = Parameters<typeof activityTitle>[0];
const t = getI18n().getFixedT("en", "devices") as DevicesT;

afterEach(async () => {
	await kit.resetDevices();
	await dom.cleanup();
});
afterAll(dom.restore);

/** Model hosting's wire values and parameter keys (R3). */
const MODEL_WIRE =
	/kv_pressure|requests_queued|gpu_unused|cpu_threads|ctx_truncation|egress_blocked|llamacpp|always_on|on_demand|pinned_off|acquiring|awaiting_push|model_host|model_manage|model_use|install_runtime|kv_percent|q8_0|vulkan|aarch64|x86_64/;

const text = (root: ParentNode) =>
	(root.textContent ?? "").replace(/\s+/g, " ");

type Sample = ReturnType<typeof gpuBoxModels>;

function open(
	deviceId: string,
	sample: Sample,
	features = MODEL_HOST_FEATURES,
	extra: Parameters<typeof openDevice>[1] = {},
) {
	return openDevice(deviceId, {
		tab: "models",
		agentFeatures: { [deviceId]: features },
		modelHosts: { [deviceId]: sample },
		...extra,
	});
}

type View = Awaited<ReturnType<typeof open>>;

const tab = (view: View) =>
	view.container.querySelector("[data-models]") as HTMLElement;

const block = (view: View, id: string) =>
	view.container.querySelector(`#${id}`) as HTMLElement;

const row = (view: View, modelId: string) =>
	view.container.querySelector(`tr[data-model="${modelId}"]`) as HTMLElement;

const runtime = (view: View, id: string) =>
	view.container.querySelector(`[data-runtime="${id}"]`) as HTMLElement;

const job = (view: View, jobId: string) =>
	view.container.querySelector(`[data-job="${jobId}"]`) as HTMLElement;

/** The `models` requests the device received, oldest first. */
const modelRequests = (view: View) =>
	view.fake.api.commands
		.filter(([, type]) => type === "models")
		.map(([, , command]) => command.request as Record<string, unknown>);

const sentKinds = (view: View) =>
	modelRequests(view).map((request) => request.kind);

const lastOf = (view: View, kind: string) =>
	modelRequests(view)
		.filter((request) => request.kind === kind)
		.at(-1);

async function confirm(view: View) {
	const sheet = inPortal("alertdialog");
	const check = queryByRole("checkbox", undefined, sheet);
	if (check) await click(check);
	await click(sheet.querySelector("[data-confirm]") as HTMLElement);
	await view.settle();
}

/** A tokenizer the GPU box can't fetch itself and waits to be sent. */
const AWAITING_PUSH_JOB = {
	job_id: "5b1d2c3e-4f5a-4b6c-8d7e-9f0a1b2c3d4e",
	digest: SAMPLE_DIGESTS.nomicTokenizer,
	size: 1_000_000_000,
	file_name: "tokenizer.json",
	updated_at: SAMPLE_NOW - 30,
	state: "awaiting_push",
	bytes: 250_000_000,
} as const;

describe("populated", () => {
	test("the GPU box: one headline sentence, now-tier recommendations first, every block with its stamp", async () => {
		const view = await open(IDS.edge, gpuBoxModels());
		const page = text(tab(view));
		expect(page).toContain("Serving 2 of 3 models.");
		expect(page).toContain(
			"1.7M tokens and 4.4K requests in the last 24 hours · 6 errors · GPU memory 29 % used · 1 model downloading",
		);
		const attention = block(view, "models-attention");
		expect(text(attention)).toContain(
			"The context cache of Qwen3-8B Q4_K_M is 93 % full.",
		);
		expect(text(attention)).toContain("Broken now");
		expect(
			attention.querySelectorAll("[data-tier=now] [data-attention]"),
		).toHaveLength(2);
		for (const id of [
			"models-attention",
			"models-list",
			"models-downloads",
			"models-hardware",
		])
			expect([id, !!block(view, id).querySelector("[data-stamp]")]).toEqual([
				id,
				true,
			]);
		expect(primaries()).toBeLessThanOrEqual(1);
		expect(page).not.toMatch(MACHINE);
		expect(page).not.toMatch(MODEL_WIRE);
	});

	test("the table: residency, kind, engine on its backend, state, memory, slots, speed, requests and callers", async () => {
		const view = await open(IDS.edge, gpuBoxModels());
		expect(
			[...view.container.querySelectorAll("tr[data-model]")].map((node) =>
				node.getAttribute("data-model"),
			),
		).toEqual(["gemma-3-4b", "nomic-embed-v1.5", "qwen3-8b"]);
		const qwen = text(row(view, "qwen3-8b"));
		for (const part of [
			"Qwen3-8B Q4_K_M",
			"Always loaded",
			"Chat",
			"llama.cpp",
			"Vulkan",
			"Loaded",
			"6.4 GiB GPU",
			"921.6 MiB RAM",
			"3 of 4",
			"86",
			"1.1K",
			"You",
			"invoice-extractor",
			"One person",
		])
			expect([part, qwen.includes(part)]).toEqual([part, true]);
		const nomic = text(row(view, "nomic-embed-v1.5"));
		expect(nomic).toContain("Embedding");
		expect(nomic).toContain("ONNX Runtime");
		expect(nomic).toContain("Loads on demand, unloads after 15 min idle");
		expect(nomic).toContain("563.2 MiB RAM");
		expect(nomic).not.toContain("GPU");
		const gemma = text(row(view, "gemma-3-4b"));
		expect(gemma).toContain("Downloading");
		expect(gemma).toContain("Vision");
		expect(queryByRole("button", "Load…", row(view, "gemma-3-4b"))).toBeNull();
		expect(
			byRole("table", "Models on edge-berlin-01", view.container),
		).toBeTruthy();
	});

	test("each row: load or unload by residency, settings, use from my apps and remove", async () => {
		const view = await open(IDS.edge, gpuBoxModels());
		const enabled = (name: string, modelId: string) =>
			byRole("button", name, row(view, modelId)).getAttribute(
				"aria-disabled",
			) === null;
		expect(enabled("Load on demand…", "qwen3-8b")).toBe(true);
		expect(queryByRole("button", "Unload…", row(view, "qwen3-8b"))).toBeNull();
		expect(enabled("Unload…", "nomic-embed-v1.5")).toBe(true);
		for (const id of ["gemma-3-4b", "nomic-embed-v1.5", "qwen3-8b"])
			for (const name of ["Settings…", "Use from my apps…", "Remove…"])
				expect([id, name, enabled(name, id)]).toEqual([id, name, true]);
	});

	test("downloads: progress, rate, time left and source; a failed one says why and can't be cancelled", async () => {
		const view = await open(IDS.edge, gpuBoxModels());
		const fetching = text(job(view, SAMPLE_JOB_IDS.fetching));
		expect(fetching).toContain("gemma-3-4b-it-Q4_K_M.gguf");
		expect(fetching).toContain(
			"1.0 GiB of 2.3 GiB · 50.0 MiB/s · about 28 s left",
		);
		expect(fetching).toContain("from cdn.flow-like.com");
		expect(
			byRole("progressbar", undefined, job(view, SAMPLE_JOB_IDS.fetching)),
		).toBeTruthy();
		const blocked = job(view, SAMPLE_JOB_IDS.blocked);
		expect(text(blocked)).toContain("Failed");
		expect(text(blocked)).toContain(
			"The device couldn't reach any download source. Its network may block outbound connections.",
		);
		expect(queryByRole("button", "Cancel…", blocked)).toBeNull();
		expect(queryByRole("progressbar", undefined, blocked)).toBeNull();
		expect(
			[...block(view, "models-downloads").querySelectorAll("[data-job]")].map(
				(node) => node.getAttribute("data-job"),
			),
		).toEqual([SAMPLE_JOB_IDS.fetching, SAMPLE_JOB_IDS.blocked]);
		expect(text(block(view, "models-downloads"))).not.toMatch(MODEL_WIRE);
	});

	test("hardware: processor, memory, GPUs, the model disk and each runtime pack", async () => {
		const view = await open(IDS.edge, gpuBoxModels());
		const hardware = text(block(view, "models-hardware"));
		expect(hardware).toContain("AMD Ryzen 9 7950X 16-Core Processor");
		expect(hardware).toContain("16 cores · x86-64 · AVX2, AVX512F, FMA");
		expect(hardware).toContain("38.0 GiB free of 64.0 GiB");
		expect(hardware).toContain("NVIDIA GeForce RTX 4090 · Vulkan");
		expect(hardware).toContain("17.0 GiB free of 24.0 GiB");
		expect(hardware).toContain(
			"5.2 GiB of 1.2 TiB used by models · 1.3 TiB free on the disk",
		);
		const vulkan = text(runtime(view, "llamacpp-vulkan"));
		expect(vulkan).toContain("llama.cpp for Vulkan");
		expect(vulkan).toContain("Installed · 83.0 MiB");
		expect(
			byRole("button", "Remove…", runtime(view, "llamacpp-vulkan")),
		).toBeTruthy();
		const cpu = text(runtime(view, "llamacpp-cpu"));
		expect(cpu).toContain("llama.cpp for CPU");
		expect(cpu).toContain("Available · 16.0 MiB");
		expect(
			byRole("button", "Install…", runtime(view, "llamacpp-cpu")),
		).toBeTruthy();
	});

	test("the Mac mini: unified memory on Metal, MLX and a CPU-only embedding model, no downloads", async () => {
		const view = await open(
			IDS.studio,
			macMiniModels(),
			MAC_MODEL_HOST_FEATURES,
		);
		const page = text(tab(view));
		expect(page).toContain("Serving 2 models.");
		expect(page).toContain(
			"Apple M4 isn't used: models run on the CPU. A GPU runtime runs them faster.",
		);
		expect(text(row(view, "qwen3-4b-mlx"))).toContain("MLX");
		expect(text(row(view, "qwen3-4b-mlx"))).toContain("Metal");
		expect(text(row(view, "bge-small-en-v1.5"))).toContain("CPU");
		const hardware = text(block(view, "models-hardware"));
		expect(hardware).toContain("10 cores · ARM64 · NEON, DOTPROD, I8MM");
		expect(hardware).toContain("memory shared with the processor");
		expect(text(runtime(view, "mlx-metal"))).toContain("MLX for Metal");
		expect(text(runtime(view, "mlx-metal"))).toContain("Installed");
		expect(text(runtime(view, "llamacpp-metal"))).toContain("Available");
		expect(block(view, "models-downloads")).toBeNull();
		expect(page).not.toMatch(MODEL_WIRE);
	});
});

describe("badge", () => {
	test("now-tier recommendations badge the tab once read, and the badge stays on other tabs", async () => {
		const view = await open(IDS.edge, gpuBoxModels());
		const models = byRole("tab", /^Models/, view.container);
		const count = models.querySelector("[data-count-tone]") as HTMLElement;
		expect(count.getAttribute("data-count-tone")).toBe("critical");
		expect(count.querySelector("[aria-hidden]")?.textContent).toBe("2");
		expect(text(models)).toContain("2 items need you");
		await click(byRole("tab", /^Overview/, view.container));
		await view.settle();
		expect(
			byRole("tab", /^Models/, view.container).querySelector(
				"[data-count-tone]",
			),
		).not.toBeNull();
	});

	test("a host with nothing urgent has no badge", async () => {
		const view = await open(
			IDS.studio,
			macMiniModels(),
			MAC_MODEL_HOST_FEATURES,
		);
		expect(row(view, "qwen3-4b-mlx")).not.toBeNull();
		expect(
			byRole("tab", /^Models/, view.container).querySelector(
				"[data-count-tone]",
			),
		).toBeNull();
	});

	test("the device page reads nothing about models until the tab opens", async () => {
		const view = await open(IDS.edge, gpuBoxModels(), MODEL_HOST_FEATURES, {
			tab: "overview",
		});
		expect(commandTypes(view)).not.toContain("models");
		expect(
			byRole("tab", /^Models/, view.container).querySelector(
				"[data-count-tone]",
			),
		).toBeNull();
	});
});

describe("actions", () => {
	test("unload: a consequence preview, one unload command, a tray item that leads back, a fresh read", async () => {
		const view = await open(IDS.edge, gpuBoxModels());
		const reads = sentKinds(view).filter((kind) => kind === "overview").length;
		await click(byRole("button", "Unload…", row(view, "nomic-embed-v1.5")));
		const sheet = inPortal("alertdialog");
		expect(text(sheet)).toContain("Unload Nomic Embed v1.5?");
		expect(text(sheet)).toContain(
			"Nomic Embed v1.5 lets the requests in flight finish, then stops and frees its memory.",
		);
		expect(text(sheet)).toContain(
			"The next request loads it again and waits until it has started.",
		);
		await confirm(view);
		expect(
			modelRequests(view).filter((request) => request.kind === "unload"),
		).toEqual([{ kind: "unload", model_id: "nomic-embed-v1.5" }]);
		expect(
			sentKinds(view).filter((kind) => kind === "overview").length,
		).toBeGreaterThan(reads);
		expect(text(row(view, "nomic-embed-v1.5"))).toContain("Not loaded");
		const item = view.fake.workspace.activity
			.list()
			.find((entry) => entry.label.params?.request === "unload");
		expect(item?.href).toEqual({
			screen: "device",
			deviceId: IDS.edge,
			tab: "models",
		});
		expect(item ? activityTitle(t, item) : "").toBe("Unload model");
		expect(text(row(view, "nomic-embed-v1.5"))).toContain(
			"Unload Nomic Embed v1.5",
		);
	});

	test("a model kept loaded offers Load on demand, not an unload the device undoes within seconds", async () => {
		const view = await open(IDS.edge, gpuBoxModels());
		await click(byRole("button", "Load on demand…", row(view, "qwen3-8b")));
		const sheet = text(inPortal("alertdialog"));
		expect(sheet).toContain("Load Qwen3-8B Q4_K_M on demand?");
		expect(sheet).toContain(
			"Qwen3-8B Q4_K_M unloads after 15 min without requests and frees its memory. Nothing restarts now.",
		);
		expect(sheet).not.toContain("restarts with the new settings");
		await confirm(view);
		expect(lastOf(view, "configure")).toEqual({
			kind: "configure",
			model_id: "qwen3-8b",
			expected_revision: 3,
			settings: {
				ctx_per_slot: 8_192,
				parallel: 4,
				gpu_layers: "auto",
				flash_attn: true,
			},
			residency: { mode: "on_demand", idle_unload_after_seconds: 900 },
		});
		expect(sentKinds(view)).not.toContain("unload");
		const qwen = text(row(view, "qwen3-8b"));
		expect(qwen).toContain("Loads on demand, unloads after 15 min idle");
		expect(qwen).toContain("Loaded");
		expect(byRole("button", "Unload…", row(view, "qwen3-8b"))).toBeTruthy();
	});

	test("Use from my apps… opens the sheet that makes the model a Bit of this account", async () => {
		const view = await open(IDS.edge, gpuBoxModels());
		const writes = view.fake.api.writes().length;
		await click(byRole("button", "Use from my apps…", row(view, "qwen3-8b")));
		expect(text(inPortal("dialog"))).toContain(
			"Use Qwen3-8B Q4_K_M from your apps",
		);
		expect(view.fake.api.writes().length).toBe(writes);
	});

	test("load and remove send their kind with the model's revision; remove asks for an acknowledgement", async () => {
		const view = await open(IDS.edge, gpuBoxModels());
		await click(byRole("button", "Remove…", row(view, "gemma-3-4b")));
		const sheet = inPortal("alertdialog");
		const button = sheet.querySelector("[data-confirm]") as HTMLElement;
		expect(button.getAttribute("aria-disabled")).toBe("true");
		expect(text(sheet)).toContain("Let edge-berlin-01 delete its files");
		expect(text(sheet)).toContain(
			"Files no other model uses are deleted after a day, or sooner when the disk runs short.",
		);
		await confirm(view);
		expect(lastOf(view, "remove")).toEqual({
			kind: "remove",
			model_id: "gemma-3-4b",
			expected_revision: 1,
		});
		expect(row(view, "gemma-3-4b")).toBeNull();
		await click(byRole("button", "Unload…", row(view, "nomic-embed-v1.5")));
		expect(text(inPortal("alertdialog"))).toContain(
			"The next request loads it again and waits until it has started.",
		);
		await confirm(view);
		await click(byRole("button", "Load…", row(view, "nomic-embed-v1.5")));
		await confirm(view);
		expect(lastOf(view, "load")).toEqual({
			kind: "load",
			model_id: "nomic-embed-v1.5",
		});
		expect(text(row(view, "nomic-embed-v1.5"))).toContain("Loaded");
	});

	test("runtimes and downloads: install a pack, cancel a download", async () => {
		const view = await open(IDS.edge, gpuBoxModels());
		await click(byRole("button", "Install…", runtime(view, "llamacpp-cpu")));
		expect(text(inPortal("alertdialog"))).toContain(
			"Install llama.cpp for CPU?",
		);
		await confirm(view);
		expect(lastOf(view, "install_runtime")).toEqual({
			kind: "install_runtime",
			runtime: "llamacpp",
			backend: "cpu",
		});
		expect(text(block(view, "models-downloads"))).toContain(
			"llamacpp-b10809-cpu.tar.gz",
		);
		await click(
			byRole("button", "Cancel…", job(view, SAMPLE_JOB_IDS.fetching)),
		);
		await confirm(view);
		expect(lastOf(view, "cancel_job")).toEqual({
			kind: "cancel_job",
			job_id: SAMPLE_JOB_IDS.fetching,
		});
		expect(text(job(view, SAMPLE_JOB_IDS.fetching))).toContain("Cancelled.");
	});

	test("install runtime forwards a manifest only after confirmation on an agent advertising support", async () => {
		const sample = gpuBoxModels();
		sample.runtime_manifest_url =
			"https://releases.example.com/runtimes/x86_64-unknown-linux-gnu.jws";
		const manifest = "header.payload.signature";
		const view = await open(IDS.edge, sample, {
			...MODEL_HOST_FEATURES,
			model_runtime_manifest: 1,
		});
		const download = spyOn(globalThis, "fetch").mockResolvedValue(
			new Response(manifest),
		);
		try {
			await click(byRole("button", "Install…", runtime(view, "llamacpp-cpu")));
			expect(download).not.toHaveBeenCalled();
			await confirm(view);
			expect(download).toHaveBeenCalledTimes(1);
			expect(lastOf(view, "install_runtime")).toEqual({
				kind: "install_runtime",
				runtime: "llamacpp",
				backend: "cpu",
				manifest_jws: manifest,
			});
		} finally {
			download.mockRestore();
		}
	});

	test("a newer available build beside an installed runtime offers Update and preserves loaded models", async () => {
		const sample = gpuBoxModels();
		const installed = sample.runtimes.find((pack) => pack.installed);
		if (!installed) throw new Error("the sample has no installed runtime");
		sample.runtimes.push({
			...installed,
			build: "next-build",
			installed: false,
		});
		const view = await open(IDS.edge, sample);
		const update = view.container.querySelector(
			'[data-runtime-build="next-build"]',
		) as HTMLElement;
		expect(text(update)).toContain("Available");
		await click(byRole("button", "Update…", update));
		expect(text(inPortal("alertdialog"))).toContain(
			"Loaded models keep using their current build. Reload them to use the update; newly loaded models use it immediately.",
		);
		await confirm(view);
		expect(lastOf(view, "install_runtime")).toEqual({
			kind: "install_runtime",
			runtime: installed.runtime,
			backend: installed.backend,
		});
	});

	test("runtime update checks refresh device releases before reading the overview again", async () => {
		const sample = gpuBoxModels();
		sample.runtime_manifest_url =
			"https://releases.example.com/runtimes/test.jws";
		const view = await open(IDS.edge, sample, {
			...MODEL_HOST_FEATURES,
			model_runtime_updates: 1,
		});
		const before = modelRequests(view).length;
		await click(
			byRole(
				"button",
				"Check for runtime updates",
				block(view, "models-hardware"),
			),
		);
		await view.settle();
		const requests = modelRequests(view)
			.slice(before)
			.map((request) => request.kind);
		expect(requests[0]).toBe("probe");
		expect(requests).toContain("overview");
		expect(text(block(view, "models-hardware"))).toContain(
			"Runtime releases checked.",
		);
	});

	test("a runtime update check failure stays visible without claiming releases were checked", async () => {
		const sample = gpuBoxModels();
		sample.runtime_manifest_url =
			"https://releases.example.com/runtimes/test.jws";
		const view = await open(IDS.edge, sample, {
			...MODEL_HOST_FEATURES,
			model_runtime_updates: 1,
		});
		view.fake
			.agent(IDS.edge)
			.models.refuse(
				"probe",
				"failed",
				"Runtime release signature verification failed.",
			);
		await click(
			byRole(
				"button",
				"Check for runtime updates",
				block(view, "models-hardware"),
			),
		);
		await view.settle();
		expect(text(block(view, "models-hardware"))).toContain(
			"Runtime release signature verification failed.",
		);
		expect(text(block(view, "models-hardware"))).not.toContain(
			"Runtime releases checked.",
		);
	});

	test("older agents cannot claim to refresh runtime releases with a hardware-only probe", async () => {
		const view = await open(IDS.edge, gpuBoxModels());
		const button = byRole(
			"button",
			"Check for runtime updates",
			block(view, "models-hardware"),
		);
		expect(button.getAttribute("aria-disabled")).toBe("true");
		await click(button);
		expect(sentKinds(view)).not.toContain("probe");
	});

	test("a runtime check needs a configured release source", async () => {
		const view = await open(IDS.edge, gpuBoxModels(), {
			...MODEL_HOST_FEATURES,
			model_runtime_updates: 1,
		});
		const hardware = block(view, "models-hardware");
		const button = byRole("button", "Check for runtime updates", hardware);
		expect(button.getAttribute("aria-disabled")).toBe("true");
		expect(text(hardware)).toContain(
			"This device has no trusted runtime release source configured.",
		);
		await click(button);
		expect(sentKinds(view)).not.toContain("probe");
	});

	test("removing an installed runtime asks for an acknowledgement and sends its pack", async () => {
		const view = await open(
			IDS.studio,
			macMiniModels(),
			MAC_MODEL_HOST_FEATURES,
		);
		await click(byRole("button", "Remove…", runtime(view, "llamacpp-cpu")));
		const sheet = inPortal("alertdialog");
		expect(text(sheet)).toContain("Remove llama.cpp for CPU?");
		expect(text(sheet)).toContain("Models on this engine may stop working");
		await confirm(view);
		expect(lastOf(view, "remove_runtime")).toEqual({
			kind: "remove_runtime",
			runtime: "llamacpp",
			backend: "cpu",
		});
	});

	test("a refused change says why next to its model and changes nothing", async () => {
		const view = await open(IDS.edge, gpuBoxModels());
		view.fake
			.agent(IDS.edge)
			.models.refuse("unload", "busy", "Three requests are still running.");
		await click(byRole("button", "Unload…", row(view, "nomic-embed-v1.5")));
		await confirm(view);
		const nomic = text(row(view, "nomic-embed-v1.5"));
		expect(nomic).toContain("Three requests are still running.");
		expect(nomic).toContain("Loaded");
		expect(view.fake.agent(IDS.edge).models.writes).toEqual([]);
	});

	test("Settings… and Add model… open the overlays the add and settings flows fill", async () => {
		const view = await open(IDS.edge, gpuBoxModels());
		const writes = view.fake.api.writes().length;
		await click(byRole("button", "Settings…", row(view, "qwen3-8b")));
		expect(kit.useOverlayStore.getState().overlay).toEqual({
			kind: "model_settings",
			deviceId: IDS.edge,
			modelId: "qwen3-8b",
		});
		await click(byRole("button", "Add model…", block(view, "models-list")));
		expect(kit.useOverlayStore.getState().overlay).toEqual({
			kind: "model_add",
			deviceId: IDS.edge,
		});
		expect(view.fake.api.writes().length).toBe(writes);
	});
});

async function until(view: View, done: () => boolean, what: string) {
	for (let round = 0; round < 60; round++) {
		if (done()) return;
		await view.settle();
	}
	throw new Error(`Timed out waiting for ${what}: ${text(tab(view))}`);
}

const chartColumns = (view: View, title: string) =>
	[...block(view, "models-stats").querySelectorAll("figure[data-chart]")]
		.find(
			(figure) =>
				figure.querySelector("figcaption span")?.textContent === title,
		)
		?.querySelectorAll("[data-column]").length ?? 0;

const follows = (first: Element, second: Element) =>
	(first.compareDocumentPosition(second) & Node.DOCUMENT_POSITION_FOLLOWING) !==
	0;

describe("usage and every model", () => {
	test("Usage and performance follows the models, with its stamp and the playground; 90 days arrive whole", async () => {
		const view = await open(IDS.edge, gpuBoxModels());
		await until(view, () => chartColumns(view, "Tokens") === 24, "the charts");
		const stats = block(view, "models-stats");
		expect(text(stats)).toContain("Usage and performance");
		expect(stats.querySelector("[data-stamp]")).not.toBeNull();
		expect(byRole("button", "Try a model…", stats)).toBeTruthy();
		expect(follows(block(view, "models-list"), stats)).toBe(true);
		expect(follows(stats, block(view, "models-downloads"))).toBe(true);
		await click(byRole("button", "90 d", stats));
		await until(view, () => chartColumns(view, "Tokens") === 90, "90 days");
		expect(text(block(view, "models-stats"))).not.toContain(
			"didn't return its usage statistics",
		);
	});

	test("models past the overview's one reply are read page by page: every row, the count and their names", async () => {
		const sample = crowdedMacModels();
		const view = await open(IDS.studio, sample, MAC_MODEL_HOST_FEATURES);
		await until(
			view,
			() =>
				view.container.querySelectorAll("tr[data-model]").length ===
				sample.models.length,
			"every model",
		);
		expect(lastOf(view, "models")?.after).toBeString();
		expect(
			[...view.container.querySelectorAll("tr[data-model]")].map((node) =>
				node.getAttribute("data-model"),
			),
		).toEqual(sample.models.map((model) => model.id).sort());
		expect(text(block(view, "models-list"))).toContain(
			String(sample.models.length),
		);
		expect(text(row(view, "qwen3-4b-mlx"))).toContain("Qwen3-4B MLX 4-bit");
		const attention = text(block(view, "models-attention"));
		expect(attention).toContain("prompts to Qwen3-4B MLX 4-bit come close");
		expect(attention).not.toContain("qwen3-4b-mlx");
	});

	test("Settings… of a model past the overview's one reply opens that model's settings", async () => {
		const view = await open(
			IDS.studio,
			crowdedMacModels(),
			MAC_MODEL_HOST_FEATURES,
			{ overlays: true },
		);
		const listed = () => row(view, "qwen3-4b-mlx") !== null;
		await until(view, listed, "the last model");
		await click(byRole("button", "Settings…", row(view, "qwen3-4b-mlx")));
		const opened = () =>
			text(document.body).includes("Settings of Qwen3-4B MLX 4-bit");
		await until(view, opened, "its settings");
		expect(text(document.body)).not.toContain("is no longer on");
	});
});

describe("downloads slot (lane C1)", () => {
	test("the slot is asked under every download; it shows what this computer sends", async () => {
		const sample = gpuBoxModels();
		sample.jobs.push({ ...AWAITING_PUSH_JOB });
		const view = await mountDevices(
			<DeviceModelsTab
				deviceId={IDS.edge}
				scope={{ kind: "account" }}
				route={{ screen: "device", deviceId: IDS.edge, tab: "models" }}
				sendFromThisComputer={(stuck) => (
					<button type="button" data-send={stuck.job_id}>
						{stuck.file_name}
					</button>
				)}
			/>,
			{
				search: `device=${IDS.edge}&tab=models`,
				agentFeatures: { [IDS.edge]: MODEL_HOST_FEATURES },
				modelHosts: { [IDS.edge]: sample },
			},
		);
		await view.settle();
		const slots = [...view.container.querySelectorAll("[data-send]")].map(
			(node) => node.getAttribute("data-send"),
		);
		expect(slots.sort()).toEqual(
			[
				SAMPLE_JOB_IDS.fetching,
				SAMPLE_JOB_IDS.blocked,
				AWAITING_PUSH_JOB.job_id,
			].sort(),
		);
		const waiting = text(job(view, AWAITING_PUSH_JOB.job_id));
		expect(waiting).toContain("Waiting for a computer to send it");
		expect(waiting).toContain("238.4 MiB of 953.7 MiB");
		expect(
			byRole("button", "Cancel…", job(view, AWAITING_PUSH_JOB.job_id)),
		).toBeTruthy();
	});

	test("without the slot nothing renders under a failed download", async () => {
		const view = await open(IDS.edge, gpuBoxModels());
		expect(text(job(view, SAMPLE_JOB_IDS.blocked))).not.toContain(
			"Send from this computer",
		);
		expect(view.container.querySelector("[data-send]")).toBeNull();
	});
});

interface ProbeProps {
	deviceId: string;
	onDone(outcome: unknown): void;
	run(actions: ReturnType<typeof useModelsAction>): Promise<unknown>;
}

function ActionProbe({ deviceId, onDone, run }: Readonly<ProbeProps>) {
	const actions = useModelsAction(deviceId);
	return (
		<button type="button" onClick={() => void run(actions).then(onDone)}>
			probe
		</button>
	);
}

async function probe(run: ProbeProps["run"]) {
	const outcomes: unknown[] = [];
	const view = await mountDevices(
		<ActionProbe
			deviceId={IDS.edge}
			run={run}
			onDone={(outcome) => outcomes.push(outcome)}
		/>,
		{
			agentFeatures: { [IDS.edge]: MODEL_HOST_FEATURES },
			modelHosts: { [IDS.edge]: emptyModels() },
		},
	);
	await click(byRole("button", "probe", view.container));
	for (let round = 0; round < 5 && !outcomes.length; round++)
		await view.settle();
	return { view, outcome: outcomes[0] as { status: string; result?: unknown } };
}

describe("useModelsAction (lanes C1 and D2)", () => {
	test("install sends the model spec once, without a sheet when the screen confirmed, and the model starts downloading", async () => {
		const { view, outcome } = await probe((actions) =>
			actions
				.install({
					modelId: "qwen3-8b",
					model: {
						display_name: "Qwen3-8B Q4_K_M",
						kind: "chat",
						engine: "llamacpp",
						assets: [
							{
								digest: SAMPLE_DIGESTS.qwen8,
								size: 5_027_784_064,
								file_name: "Qwen3-8B-Q4_K_M.gguf",
								sources: ["https://huggingface.co/Qwen/Qwen3-8B-GGUF"],
							},
						],
					},
				})
				.run({ confirmed: true }),
		);
		expect(outcome.status).toBe("done");
		expect(lastOf(view, "install")).toMatchObject({
			kind: "install",
			model_id: "qwen3-8b",
			model: { display_name: "Qwen3-8B Q4_K_M", engine: "llamacpp" },
		});
		expect(document.querySelector("[role=alertdialog]")).toBeNull();
		const host = view.fake.agent(IDS.edge).models.state;
		expect(host.models.map((model) => [model.id, model.state])).toEqual([
			["qwen3-8b", "acquiring"],
		]);
		expect(host.jobs.map((entry) => entry.source_host)).toEqual([
			"huggingface.co",
		]);
	});

	test("ensure is the deploy step: it names the project and its pins and parses the asset summary", async () => {
		const pins = [{ bit_id: "bit_qwen", metadata_sha256: "a".repeat(64) }];
		const { view, outcome } = await probe((actions) =>
			actions
				.ensure({ projectId: "app_invoice_ai", pins })
				.run({ confirmed: true }),
		);
		expect(outcome).toEqual({
			status: "done",
			result: { total: 1, present: 1, pending: [] },
		});
		expect(lastOf(view, "ensure")).toEqual({
			kind: "ensure",
			project_id: "app_invoice_ai",
			pins,
		});
	});
});

describe("states", () => {
	test("no models: the headline says so and Add model… is the one way on", async () => {
		const view = await open(IDS.edge, emptyModels());
		const page = text(tab(view));
		expect(page).toContain("edge-berlin-01 hosts no models yet.");
		expect(page).toContain("No models on edge-berlin-01 yet");
		expect(block(view, "models-attention")).toBeNull();
		expect(block(view, "models-downloads")).toBeNull();
		expect(text(block(view, "models-hardware"))).toContain(
			"No GPU found. Models run on the processor.",
		);
		const add = allByRole("button", "Add model…", tab(view));
		expect(add).toHaveLength(1);
		await click(add[0] as HTMLElement);
		expect(kit.useOverlayStore.getState().overlay).toEqual({
			kind: "model_add",
			deviceId: IDS.edge,
		});
		expect(primaries()).toBeLessThanOrEqual(1);
	});

	test("locked: nothing is read; Unlock… asks for the password, connects and offers to keep the keys for model access", async () => {
		const view = await open(IDS.edge, gpuBoxModels(), MODEL_HOST_FEATURES, {
			unlock: "none",
			overlays: true,
		});
		const page = text(tab(view));
		expect(page).toContain(
			"Models and their usage are read from edge-berlin-01 with your keys.",
		);
		expect(commandTypes(view)).not.toContain("models");
		await click(byRole("button", "Unlock…", tab(view)));
		expect(kit.useOverlayStore.getState().overlay).toEqual({
			kind: "unlock",
			deviceId: IDS.edge,
			connectLive: true,
			forModels: true,
		});
		await view.settle();
		expect(
			byRole(
				"checkbox",
				/^Keep unlocked for model access/,
				inPortal("dialog"),
			).getAttribute("aria-checked"),
		).toBe("true");
	});

	test("locking clears what was read", async () => {
		const view = await open(IDS.edge, gpuBoxModels());
		expect(row(view, "qwen3-8b")).not.toBeNull();
		await click(byRole("button", "Lock", view.container));
		await view.settle();
		expect(row(view, "qwen3-8b")).toBeNull();
		expect(text(tab(view))).not.toContain("Qwen3");
		expect(text(tab(view))).toContain("Locked");
	});

	test("Lock all devices clears every unlocked device and the model data", async () => {
		const view = await open(IDS.edge, gpuBoxModels(), MODEL_HOST_FEATURES, {
			unlock: [IDS.edge, IDS.studio],
		});
		const keys = view.fake.workspace.keys;
		expect(keys.snapshot(IDS.studio).state).toBe("unlocked");
		await click(byRole("button", "Lock all devices", tab(view)));
		await view.settle();
		for (const deviceId of [IDS.edge, IDS.studio])
			expect(keys.snapshot(deviceId).state).toBe("locked");
		expect(row(view, "qwen3-8b")).toBeNull();
		expect(
			byRole("button", "Lock all devices", tab(view)).getAttribute(
				"aria-disabled",
			),
		).toBe("true");
	});

	test("no live session: what is missing, and Connect live", async () => {
		const fake = await createFakeWorkspace(undefined, {
			agentFeatures: { [IDS.edge]: MODEL_HOST_FEATURES },
			modelHosts: { [IDS.edge]: gpuBoxModels() },
		});
		fake.workspace.live.close(IDS.edge);
		await fake.settle();
		const view = await openDevice(IDS.edge, { fake, tab: "models" });
		const page = text(tab(view));
		expect(page).toContain("Needs a live connection");
		expect(page).toContain(
			"Models, downloads and runtimes are read from edge-berlin-01 over a live connection.",
		);
		expect(commandTypes(view)).not.toContain("models");
		expect(byRole("button", "Connect live", tab(view))).toBeTruthy();
		expect(row(view, "qwen3-8b")).toBeNull();
	});

	test("an older agent: never asked, and Update agent… leads to the agent update", async () => {
		const view = await open(IDS.edge, gpuBoxModels(), PRE_MODEL_FEATURES);
		const page = text(tab(view));
		expect(page).toContain(
			"The agent on edge-berlin-01 can't host models yet.",
		);
		expect(commandTypes(view)).not.toContain("models");
		await click(byRole("button", "Update agent…", tab(view)));
		expect(lastNavigation(view)?.[1]).toContain("tab=settings");
		expect(page).not.toMatch(MODEL_WIRE);
	});

	test("an agent that refuses models as unsupported reads like an older one", async () => {
		const fake = await createFakeWorkspace(undefined, {
			agentFeatures: { [IDS.edge]: MODEL_HOST_FEATURES },
		});
		fake
			.agent(IDS.edge)
			.reject("models", "unsupported", "Unsupported command.");
		const view = await openDevice(IDS.edge, { fake, tab: "models" });
		expect(text(tab(view))).toContain(
			"The agent on edge-berlin-01 can't host models yet.",
		);
	});

	test("a failed read says why and tries again", async () => {
		const fake = await createFakeWorkspace(undefined, {
			agentFeatures: { [IDS.edge]: MODEL_HOST_FEATURES },
			modelHosts: { [IDS.edge]: gpuBoxModels() },
		});
		const restore = fake
			.agent(IDS.edge)
			.reject("models", "failed", "The model host is starting.");
		const view = await openDevice(IDS.edge, { fake, tab: "models" });
		expect(text(tab(view))).toContain(
			"edge-berlin-01 didn't return its models",
		);
		expect(text(tab(view))).toContain(
			"The device couldn't complete the request.",
		);
		restore();
		await click(byRole("button", "Try again", tab(view)));
		await view.settle();
		expect(row(view, "qwen3-8b")).not.toBeNull();
	});

	test("shared without model permissions: no access, with the way to ask", async () => {
		const view = await openDevice(IDS.lab, {
			tab: "models",
			agentFeatures: { [IDS.lab]: MODEL_HOST_FEATURES },
		});
		const notice = tab(view).querySelector("[data-gate=noaccess]");
		expect(notice).not.toBeNull();
		expect(byRole("button", "Ask the owner", tab(view))).toBeTruthy();
		expect(commandTypes(view)).not.toContain("models");
	});
});

type Fake = Awaited<ReturnType<typeof createFakeWorkspace>>;

/** The model host's counters as the agent adds them to a model reader's encrypted device metrics. */
const REPORTED = {
	models: 3,
	loaded: 1,
	failed: 1,
	requests_24h: 4_400,
	tokens_24h: 1_700_000,
	errors_24h: 6,
	store_bytes: 5_575_494_884,
	store_budget_bytes: 1_278_000_000_000,
};

/** The device publishes encrypted metrics carrying `models`, taken `ageS` seconds ago. */
async function reportModels(
	fake: Fake,
	deviceId: string,
	models: typeof REPORTED,
	ageS = 20,
) {
	const streams = fake.hub.streams.get(deviceId) ?? [];
	const status = streams.find((stream) => stream.kind === "status");
	if (!status)
		throw new Error("The seed has no status stream for this device.");
	fake.hub.streams.set(deviceId, [
		...streams.filter((stream) => stream.kind !== "metrics"),
		{
			kind: "metrics",
			scope: { kind: "device" },
			grantId: status.grantId,
			sequence: status.sequence,
			bootId: status.bootId,
			observedAt: SAMPLE_NOW - ageS,
			payload: { metrics: { records: [], models } },
		},
	]);
	await fake.workspace.fleet.refresh(deviceId);
}

const modelHost = () =>
	createFakeWorkspace(undefined, {
		agentFeatures: { [IDS.edge]: MODEL_HOST_FEATURES },
		modelHosts: { [IDS.edge]: gpuBoxModels() },
	});

const badgeOf = (view: View) =>
	byRole("tab", /^Models/, view.container)
		.querySelector("[data-count-tone]")
		?.querySelector("[aria-hidden]")?.textContent;

describe("what the device does and reports", () => {
	test("a model kept off offers no Load…, which the device would refuse; Settings… changes how it runs", async () => {
		const sample = gpuBoxModels();
		sample.models = sample.models.map((model) =>
			model.id === "nomic-embed-v1.5"
				? {
						id: model.id,
						display_name: model.display_name,
						kind: model.kind,
						engine: model.engine,
						assets: model.assets,
						settings: model.settings,
						residency: { mode: "pinned_off" },
						revision: model.revision,
						state: "stopped",
					}
				: model,
		);
		const view = await open(IDS.edge, sample);
		const nomic = row(view, "nomic-embed-v1.5");
		expect(text(nomic)).toContain("Kept off");
		expect(text(nomic)).toContain("Not loaded");
		expect(queryByRole("button", "Load…", nomic)).toBeNull();
		expect(queryByRole("button", "Unload…", nomic)).toBeNull();
		expect(byRole("button", "Settings…", nomic)).toBeTruthy();
	});

	test("device-wide recommendations show before the first model is added", async () => {
		const sample = emptyModels();
		sample.recommendations = [
			{ code: "container_gpu_hidden", tier: "soon" },
			{
				code: "disk_low",
				tier: "soon",
				params: { free_bytes: 12_000_000_000 },
			},
		];
		const view = await open(IDS.edge, sample);
		const attention = text(block(view, "models-attention"));
		expect(attention).toContain(
			"edge-berlin-01 runs in a container that can't see the host's GPU.",
		);
		expect(attention).toContain("The model disk has 11.2 GiB left.");
		expect(text(tab(view))).toContain("No models on edge-berlin-01 yet");
	});

	test("every recommendation is listed and every now-tier one counted, past the overview's top three", async () => {
		const sample = gpuBoxModels();
		sample.recommendations.push(
			{
				code: "memory_pressure",
				tier: "now",
				model_id: "qwen3-8b",
				params: { memory_percent: 94 },
			},
			{
				code: "slow_ttft",
				tier: "now",
				model_id: "nomic-embed-v1.5",
				params: { ttft_p95_ms: 2_400 },
			},
			{
				code: "disk_low",
				tier: "soon",
				params: { free_bytes: 12_000_000_000 },
			},
		);
		const view = await open(IDS.edge, sample);
		const listed = () =>
			block(view, "models-attention").querySelectorAll("[data-attention]")
				.length;
		await until(view, () => listed() === 6, "every recommendation");
		expect(lastOf(view, "recommendations")).toMatchObject({
			kind: "recommendations",
			limit: 32,
		});
		const attention = text(block(view, "models-attention"));
		expect(attention).toContain(
			"Answers from Nomic Embed v1.5 take up to 2.4 s",
		);
		expect(attention).toContain("The model disk has 11.2 GiB left.");
		expect(badgeOf(view)).toBe("4");
	});

	test("without a live session the last reported counts show with their age; failed models badge the tab before it opens", async () => {
		const fake = await modelHost();
		await reportModels(fake, IDS.edge, REPORTED);
		fake.workspace.live.close(IDS.edge);
		await fake.settle();
		const view = await openDevice(IDS.edge, { fake, tab: "overview" });
		expect(badgeOf(view)).toBe("1");
		await click(byRole("tab", /^Models/, view.container));
		await view.settle();
		expect(text(tab(view))).toContain("Needs a live connection");
		const reported = block(view, "models-snapshot");
		expect(text(reported)).toContain(
			"1 model on edge-berlin-01 failed to start.",
		);
		expect(text(reported)).toContain(
			"1.7M tokens and 4.4K requests in the last 24 hours · 6 errors",
		);
		const stamp = reported.querySelector<HTMLElement>("[data-stamp]");
		expect(`${stamp?.dataset.src}:${stamp?.dataset.age}`).toBe("snap:current");
		expect(commandTypes(view)).not.toContain("models");
	});

	test("locking keeps the counts alone, stamped locked; the models themselves are cleared", async () => {
		const fake = await modelHost();
		await reportModels(fake, IDS.edge, REPORTED);
		const view = await openDevice(IDS.edge, { fake, tab: "models" });
		expect(row(view, "qwen3-8b")).not.toBeNull();
		expect(block(view, "models-snapshot")).toBeNull();
		await click(byRole("button", "Lock", view.container));
		await view.settle();
		expect(row(view, "qwen3-8b")).toBeNull();
		expect(text(tab(view))).not.toContain("Qwen3");
		const kept = block(view, "models-snapshot");
		expect(text(kept)).toContain("1 model on edge-berlin-01 failed to start.");
		const stamp = kept.querySelector<HTMLElement>("[data-stamp]");
		expect(`${stamp?.dataset.src}:${stamp?.dataset.age}`).toBe("snap:locked");
		expect(badgeOf(view)).toBeUndefined();
	});
});
