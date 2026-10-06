import { afterAll, afterEach, describe, expect, test } from "bun:test";
import { act } from "react";
import {
	byRole,
	click,
	inPortal,
	installDom,
	queryByRole,
	settle,
} from "../../testing/dom-harness";

const dom = installDom();
const { cleanupDevices, mountDevices, preloadDevices } = await import(
	"../../testing/mount-devices"
);
await preloadDevices();
const samples = await import(
	"../../../../../lib/device-management/model/__fixtures__/sample-models"
);
const { SAMPLE_IDS } = await import(
	"../../../../../lib/device-management/model/__fixtures__/sample-fleet"
);
const { useOverlayStore } = await import("../../workspace/overlay-store");
const { ModelSettingsSheet } = await import("./model-settings-sheet");

const { MODEL_HOST_FEATURES, gpuBoxModels } = samples;
const { edge } = SAMPLE_IDS;

afterEach(async () => {
	await act(async () => {
		useOverlayStore.getState().close();
	});
	await cleanupDevices();
	await dom.cleanup();
});
afterAll(dom.restore);

const text = (root: ParentNode) =>
	(root.textContent ?? "").replace(/\s+/g, " ");

const sheet = () => inPortal("dialog");

async function until(done: () => boolean, what: string, rounds = 60) {
	for (let round = 0; round < rounds; round++) {
		if (done()) return;
		await settle();
	}
	throw new Error(`Timed out waiting for ${what}. Sheet: ${text(sheet())}`);
}

async function mountSheet(modelId: string, sample = gpuBoxModels()) {
	const closed: number[] = [];
	const view = await mountDevices(
		<ModelSettingsSheet
			deviceId={edge}
			modelId={modelId}
			scope={{ kind: "account" }}
			onNavigate={() => undefined}
			onClose={() => closed.push(1)}
		/>,
		{
			agentFeatures: { [edge]: MODEL_HOST_FEATURES },
			modelHosts: { [edge]: sample },
		},
	);
	await until(() => queryByRole("dialog") !== null, "the sheet");
	return { view, closed };
}

const row = (field: string) =>
	sheet().querySelector(`[data-setting=${field}]`) as HTMLElement;

async function choose(field: string, option: string) {
	await click(byRole("combobox", undefined, row(field)));
	await click(byRole("option", option));
}

/** The Review row of one field. */
const change = (label: string) =>
	[...sheet().querySelectorAll("li[data-k=changed]")]
		.map(text)
		.find((line) => line.includes(label)) ?? "";

const configures = (view: Awaited<ReturnType<typeof mountSheet>>["view"]) =>
	view.fake
		.agent(edge)
		.models.writes.filter((write) => write.kind === "configure");

describe("model settings", () => {
	test("opens from the overlay store with every llama.cpp setting, its value and why", async () => {
		await mountDevices(<div />, {
			overlays: true,
			agentFeatures: { [edge]: MODEL_HOST_FEATURES },
			modelHosts: { [edge]: gpuBoxModels() },
		});
		await act(async () => {
			useOverlayStore.getState().openModelSettings(edge, "qwen3-8b");
		});
		await until(() => text(sheet()).includes("Context per slot"), "the form");
		expect(byRole("heading", undefined, sheet()).textContent).toBe(
			"Settings of Qwen3-8B Q4_K_M",
		);
		expect(
			[...sheet().querySelectorAll("[data-setting]")].map((node) =>
				node.getAttribute("data-setting"),
			),
		).toEqual([
			"ctx_per_slot",
			"parallel",
			"kv_cache_type",
			"gpu_layers",
			"threads",
			"flash_attn",
			"residency",
		]);
		const page = text(sheet());
		expect(page).toContain(
			"uses 6.4 GiB of GPU memory and 921.6 MiB of RAM now",
		);
		expect(text(row("ctx_per_slot"))).toContain("8,192 tokens");
		expect(text(row("threads"))).toContain("16 threads");
		expect(text(row("threads"))).toContain(
			"One per processor core: this device has 16.",
		);
		expect(page).not.toMatch(/q8_0|f16|on_demand|always_on|llamacpp/);
	});

	test("the device's own fix is the recommendation: its value, its sentence, Use recommended", async () => {
		await mountSheet("qwen3-8b");
		const kv = text(row("kv_cache_type"));
		expect(kv).toContain("Device default");
		expect(kv).toContain("Recommended: 8-bit.");
		expect(kv).toContain("The context cache of Qwen3-8B Q4_K_M is 93 % full.");
		await click(byRole("button", "Use recommended", row("kv_cache_type")));
		expect(text(row("kv_cache_type"))).toContain("Recommended.");
	});

	test("Review shows each change and what happens; Apply sends Configure with the revision it opened with", async () => {
		const { view, closed } = await mountSheet("qwen3-8b");
		await choose("parallel", "6 at a time");
		await click(byRole("button", "Use recommended", row("kv_cache_type")));
		await click(byRole("button", "Review changes", sheet()));
		const review = text(sheet());
		expect(review).toContain("Review changes to Qwen3-8B Q4_K_M");
		expect(review).toContain("Parallel requests");
		expect(review).toContain("4 at a time");
		expect(review).toContain("6 at a time");
		expect(review).toContain("Context cache precision");
		expect(review).toContain(
			"Qwen3-8B Q4_K_M lets the requests in flight finish, then restarts with the new settings.",
		);
		expect(review).toContain("New requests wait until it has restarted.");
		await click(byRole("button", "Apply", sheet()));
		await until(() => closed.length === 1, "the sheet to close");
		expect(configures(view)).toEqual([
			{
				kind: "configure",
				model_id: "qwen3-8b",
				expected_revision: 3,
				settings: {
					ctx_per_slot: 8_192,
					parallel: 6,
					gpu_layers: "auto",
					flash_attn: true,
					kv_cache_type: "q8_0",
				},
				residency: { mode: "always_on" },
			},
		]);
	});

	test("a change made meanwhile on the device: the sheet says so, loads it and applies only this sheet's change on top", async () => {
		const { view, closed } = await mountSheet("qwen3-8b");
		const host = view.fake.agent(edge).models;
		const qwen = host.state.models.find((model) => model.id === "qwen3-8b");
		if (!qwen) throw new Error("The GPU box sample has no qwen3-8b.");
		qwen.revision = 4;
		qwen.settings = { ...qwen.settings, parallel: 8 };
		await click(byRole("radio", /^Loads on demand/, row("residency")));
		await click(byRole("button", "Review changes", sheet()));
		await click(byRole("button", "Apply", sheet()));
		await until(
			() => text(sheet()).includes("These settings changed on the device"),
			"the conflict",
		);
		expect(closed).toEqual([]);
		expect(
			byRole("button", "Apply", sheet()).getAttribute("aria-disabled"),
		).toBe("true");
		await click(byRole("button", "Load the new settings", sheet()));
		await until(
			() => !text(sheet()).includes("These settings changed on the device"),
			"the reload",
		);
		expect(text(sheet())).not.toContain("Parallel requests");
		await click(byRole("button", "Apply", sheet()));
		await until(() => closed.length === 1, "the sheet to close");
		expect(configures(view).map((write) => write.expected_revision)).toEqual([
			3, 4,
		]);
		expect(configures(view).at(-1)).toMatchObject({
			settings: { parallel: 8 },
			residency: { mode: "on_demand", idle_unload_after_seconds: 900 },
		});
	});

	test("a field the model leaves out shows the device default; setting it is a change, and so is going back", async () => {
		const { view, closed } = await mountSheet("qwen3-8b");
		expect(byRole("combobox", undefined, row("threads")).textContent).toBe(
			"Device default",
		);
		expect(text(row("threads"))).toContain("Recommended: 16 threads.");
		await choose("threads", "16 threads");
		expect(text(sheet())).not.toContain("Nothing changed yet.");
		await choose("flash_attn", "Device default");
		await click(byRole("button", "Review changes", sheet()));
		expect(change("Processor threads")).toMatch(/Device default.*16 threads/);
		expect(change("Flash attention")).toMatch(/On.*Device default/);
		await click(byRole("button", "Apply", sheet()));
		await until(() => closed.length === 1, "the sheet to close");
		expect(configures(view).at(-1)?.settings).toEqual({
			ctx_per_slot: 8_192,
			parallel: 4,
			gpu_layers: "auto",
			threads: 16,
		});
	});

	test("an 8-bit or 4-bit cache keeps flash attention on", async () => {
		const { view, closed } = await mountSheet("qwen3-8b");
		await choose("flash_attn", "Off");
		await choose("kv_cache_type", "8-bit");
		const flash = row("flash_attn");
		expect(byRole("combobox", undefined, flash).textContent).toBe("On");
		expect(text(flash)).toContain(
			"An 8-bit or 4-bit context cache needs flash attention, so it can't be off.",
		);
		await click(byRole("combobox", undefined, flash));
		expect(byRole("option", "Off").getAttribute("aria-disabled")).toBe("true");
		await click(byRole("option", "On"));
		await click(byRole("button", "Review changes", sheet()));
		await click(byRole("button", "Apply", sheet()));
		await until(() => closed.length === 1, "the sheet to close");
		expect(configures(view).at(-1)?.settings).toMatchObject({
			kv_cache_type: "q8_0",
			flash_attn: true,
		});
	});

	test("an ONNX model shows only when it runs: its worker reads no settings, and Configure keeps what it stores", async () => {
		const { view, closed } = await mountSheet("nomic-embed-v1.5");
		expect(
			[...sheet().querySelectorAll("[data-setting]")].map((node) =>
				node.getAttribute("data-setting"),
			),
		).toEqual(["residency"]);
		expect(text(sheet())).toContain(
			"Embedding · ONNX Runtime · uses 563.2 MiB of RAM now",
		);
		await click(byRole("radio", /^Always loaded/, row("residency")));
		await click(byRole("button", "Review changes", sheet()));
		await click(byRole("button", "Apply", sheet()));
		await until(() => closed.length === 1, "the sheet to close");
		expect(configures(view).at(-1)).toMatchObject({
			settings: { threads: 8 },
			residency: { mode: "always_on" },
		});
	});

	test("nothing changed: Review stays off and says why", async () => {
		const { view } = await mountSheet("qwen3-8b");
		expect(text(sheet())).toContain("Nothing changed yet.");
		const review = byRole("button", "Review changes", sheet());
		expect(review.getAttribute("aria-disabled")).toBe("true");
		await click(review);
		expect(text(sheet())).toContain("Step 1 of 2 · Change");
		expect(configures(view)).toEqual([]);
	});

	test("a model that is gone says so", async () => {
		await mountSheet("removed-model");
		await until(() => text(sheet()).includes("no longer"), "the gone state");
		expect(text(sheet())).toContain(
			"This model is no longer on edge-berlin-01.",
		);
	});
});
