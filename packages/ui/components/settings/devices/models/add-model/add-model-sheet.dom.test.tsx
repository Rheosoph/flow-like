import { afterAll, afterEach, describe, expect, test } from "bun:test";
import { act } from "react";
import type { HostedModel } from "../../../../../lib/device-management/models";
import type { IBit } from "../../../../../lib/schema";
import type { IBitState } from "../../../../../state/backend-state/bit-state";
import {
	allByRole,
	byRole,
	click,
	inPortal,
	installDom,
	queryByRole,
	settle,
	typeInto,
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
const { IBitTypes } = await import("../../../../../lib/schema");
const { useOverlayStore } = await import("../../workspace/overlay-store");
const { AddModelSheet } = await import("./add-model-sheet");
const { LFS, REVISIONS, SMALL_FILES, fakeHuggingFace, sampleRepos } =
	await import("./fake-hugging-face");

type FakeHuggingFace = ReturnType<typeof fakeHuggingFace>;

const {
	MAC_MODEL_HOST_FEATURES,
	MODEL_HOST_FEATURES,
	gpuBoxModels,
	macMiniModels,
	overviewOf,
} = samples;
const { edge, studio } = SAMPLE_IDS;
const GIB = 1024 ** 3;

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

const R3_WIRE =
	/llamacpp|q8_0|on_demand|always_on|engine_unavailable|no_fingerprint|too_large|"auto"/;

function hubBit(overrides: Partial<IBit> = {}): IBit {
	return {
		id: "qwen3-8b-hub",
		type: IBitTypes.Llm,
		authors: [],
		created: "",
		updated: "",
		dependencies: [],
		dependency_tree_hash: "",
		hash: "c".repeat(64),
		hub: "hub.flow-like.com",
		file_name: "Qwen3-8B-Q4_K_M.gguf",
		download_link: "https://cdn.flow-like.com/bits/qwen3-8b",
		size: 5_027_784_064,
		meta: {
			en: {
				name: "Qwen3 8B",
				description: "Alibaba's 8B chat model",
				tags: [],
				preview_media: [],
				created_at: { secs_since_epoch: 0, nanos_since_epoch: 0 },
				updated_at: { secs_since_epoch: 0, nanos_since_epoch: 0 },
			},
		},
		parameters: {
			context_length: 32_768,
			provider: {
				provider_name: "Local",
				model_id: "Qwen/Qwen3-8B-GGUF",
				version: REVISIONS.qwenGguf,
			},
		},
		...overrides,
	} as IBit;
}

const openAiBit = hubBit({
	id: "gpt",
	file_name: null,
	meta: { en: { ...hubBit().meta.en, name: "GPT-5" } as IBit["meta"][string] },
	parameters: { provider: { provider_name: "openai" } },
});

const ON_DEMAND = {
	mode: "on_demand",
	idle_unload_after_seconds: 900,
} as const;

/** "Qwen3-8B Q4_K_M", added from Hugging Face before. */
const earlierQwen: HostedModel = {
	id: "qwen3-8b-q4_k_m",
	display_name: "Qwen3-8B Q4_K_M",
	kind: "chat",
	engine: "llamacpp",
	assets: [{ algorithm: "sha256", hex: LFS.q4 }],
	settings: {},
	residency: ON_DEMAND,
	revision: 1,
	state: "stopped",
};

/** The GPU box with so many placement models before `extra` that the overview's one reply ends before it. */
function crowdedGpuBox(extra: HostedModel) {
	const sample = gpuBoxModels();
	for (let index = 0; index < 12; index++)
		sample.models.push({
			id: `auto-${String(index + 1).padStart(2, "0")}`,
			display_name: `Placement model ${index + 1}`,
			kind: "chat",
			engine: "llamacpp",
			assets: Array.from({ length: 12 }, (_, file) => ({
				algorithm: "sha256",
				hex: (index * 12 + file).toString(16).padStart(64, "b"),
			})),
			settings: {},
			residency: ON_DEMAND,
			revision: 1,
			state: "stopped",
		});
	sample.models.push(extra);
	const listed = overviewOf(sample);
	expect(listed.next).not.toBeNull();
	expect(listed.models.map((model) => model.id)).not.toContain(extra.id);
	return sample;
}

function fakeBits(hub: IBit[] = [hubBit(), openAiBit], mine: IBit[] = []) {
	return {
		searchBits: async (query: { offset?: number | null }) =>
			query.offset ? [] : hub,
		getPackFromBit: async (bit: IBit) => ({ bits: [bit] }),
		listCustomBits: async () => mine,
	};
}

interface MountOptions {
	deviceId?: string;
	sample?: ReturnType<typeof gpuBoxModels>;
	features?: typeof MODEL_HOST_FEATURES;
	hub?: FakeHuggingFace;
	bits?: ReturnType<typeof fakeBits>;
	unlock?: "none";
}

async function mountWizard(options: MountOptions = {}) {
	const deviceId = options.deviceId ?? edge;
	const hub = options.hub ?? fakeHuggingFace();
	const closed: number[] = [];
	const view = await mountDevices(
		<AddModelSheet
			deviceId={deviceId}
			scope={{ kind: "account" }}
			onNavigate={() => undefined}
			onClose={() => closed.push(1)}
			fetcher={hub.fetch}
		/>,
		{
			agentFeatures: { [deviceId]: options.features ?? MODEL_HOST_FEATURES },
			modelHosts: { [deviceId]: options.sample ?? gpuBoxModels() },
			backend: {
				bitState: (options.bits ?? fakeBits()) as unknown as IBitState,
			},
			...(options.unlock ? { unlock: options.unlock } : {}),
		},
	);
	await view.settle();
	return { view, hub, closed, deviceId };
}

type Wizard = Awaited<ReturnType<typeof mountWizard>>;

const sheet = () => inPortal("dialog");

async function until(done: () => boolean, what: string, rounds = 60) {
	for (let round = 0; round < rounds; round++) {
		if (done()) return;
		await settle();
	}
	throw new Error(`Timed out waiting for ${what}. Sheet: ${text(sheet())}`);
}

const step = () =>
	sheet().querySelector("[data-step]")?.getAttribute("data-step") ?? "done";

const next = () => byRole("button", /^(Continue|Add )/, sheet());

async function advance(to: string) {
	await click(next());
	await until(() => step() === to, `step ${to}`);
}

async function fromHuggingFace(reference: string) {
	await click(byRole("button", "Hugging Face", sheet()));
	await typeInto(byRole("textbox", "Repository", sheet()), reference);
	await click(next());
}

const installs = ({ view, deviceId }: Wizard) =>
	view.fake
		.agent(deviceId)
		.models.writes.filter((write) => write.kind === "install");

describe("add model", () => {
	test("opens from the overlay store and starts at the hub; Continue waits for a choice", async () => {
		await mountDevices(<div />, {
			overlays: true,
			agentFeatures: { [edge]: MODEL_HOST_FEATURES },
			modelHosts: { [edge]: gpuBoxModels() },
			backend: { bitState: fakeBits() as never },
		});
		await act(async () => {
			useOverlayStore.getState().openModelAdd(edge);
		});
		await until(() => queryByRole("dialog") !== null, "the sheet");
		expect(byRole("heading", undefined, sheet()).textContent).toBe(
			"Add a model to edge-berlin-01",
		);
		await until(() => text(sheet()).includes("Qwen3 8B"), "the hub list");
		expect(text(sheet())).not.toContain("GPT-5");
		expect(next().getAttribute("aria-disabled")).toBe("true");
		expect(text(sheet())).toContain("Choose a model first.");
	});

	test("a Hugging Face GGUF repository: versions with their fit, recommended settings, the consequences, one install", async () => {
		const wizard = await mountWizard();
		await fromHuggingFace("Qwen/Qwen3-8B-GGUF");
		await until(() => step() === "version", "the versions");
		const versions = allByRole("radio", undefined, sheet()).map((radio) =>
			radio.getAttribute("value"),
		);
		expect(versions).toEqual([
			"Qwen3-8B-Q4_K_M.gguf",
			"BF16/Qwen3-8B-BF16.gguf",
			"Qwen3-8B-Q8_0.gguf",
		]);
		const page = text(sheet());
		for (const part of [
			"Step 2 of 4 · Version",
			"Qwen3-8B",
			"License: apache-2.0",
			"Recommended",
			"Fits on the GPU",
			"4.7 GiB download",
			"of GPU memory",
			"Fast · about",
			"Every layer fits on NVIDIA GeForce RTX 4090.",
		])
			expect([part, page.includes(part)]).toEqual([part, true]);
		expect(page).not.toMatch(R3_WIRE);

		await advance("settings");
		const settings = sheet().querySelector(
			"[data-settings-form]",
		) as HTMLElement;
		expect(text(settings)).toContain("Context per slot");
		expect(text(settings)).toContain("8,192 tokens");
		expect(text(settings)).toContain("4 at a time");
		expect(text(settings)).toContain(
			"One per processor core: this device has 16.",
		);
		expect(settings.querySelectorAll("[data-recommended=other]")).toHaveLength(
			0,
		);

		await advance("review");
		const review = text(sheet());
		expect(review).toContain(
			"edge-berlin-01 downloads 4.7 GiB in 1 file from its sources and checks it against its fingerprint. Sources: huggingface.co.",
		);
		expect(review).toContain("Disk and memory");
		expect(review).toContain("Once loaded: About");
		expect(review).toContain("Remove the model to free its disk space.");
		expect(review).not.toMatch(R3_WIRE);

		await click(byRole("button", "Add Qwen3-8B Q4_K_M", sheet()));
		await until(() => step() === "done", "the install");
		expect(installs(wizard)).toEqual([
			{
				kind: "install",
				model_id: "qwen3-8b-q4_k_m",
				model: {
					display_name: "Qwen3-8B Q4_K_M",
					kind: "chat",
					engine: "llamacpp",
					assets: [
						{
							digest: { algorithm: "sha256", hex: LFS.q4 },
							size: 5_027_784_064,
							file_name: "Qwen3-8B-Q4_K_M.gguf",
							sources: [
								`https://huggingface.co/Qwen/Qwen3-8B-GGUF/resolve/${REVISIONS.qwenGguf}/Qwen3-8B-Q4_K_M.gguf?download=true`,
							],
						},
					],
				},
				settings: {
					ctx_per_slot: 8_192,
					parallel: 4,
					kv_cache_type: "f16",
					gpu_layers: "auto",
					threads: 16,
					flash_attn: true,
				},
				residency: { mode: "on_demand", idle_unload_after_seconds: 900 },
			},
		]);
		expect(text(sheet())).toContain("Qwen3-8B Q4_K_M is on edge-berlin-01");
		expect(text(sheet())).toContain(
			"The device downloads 1 file and checks it.",
		);
		const host = wizard.view.fake.agent(edge).models.state;
		expect(
			host.models.find((model) => model.id === "qwen3-8b-q4_k_m")?.state,
		).toBe("acquiring");
		await click(byRole("button", "Close", sheet()));
		expect(wizard.closed).toEqual([1]);
	});

	test("a changed setting is sent; Use recommended brings the recommendation back", async () => {
		const wizard = await mountWizard();
		await fromHuggingFace("Qwen/Qwen3-8B-GGUF");
		await until(() => step() === "version", "the versions");
		await advance("settings");
		const fit = () => sheet().querySelector("[data-fit]") as HTMLElement;
		const recommended = text(fit());
		const parallel = sheet().querySelector(
			"[data-setting=parallel]",
		) as HTMLElement;
		await click(byRole("combobox", undefined, parallel));
		await click(byRole("option", "2 at a time"));
		expect(text(parallel)).toContain("Recommended: 4 at a time.");
		expect(fit().tagName).toBe("OUTPUT");
		expect(text(fit())).not.toBe(recommended);
		const ctx = sheet().querySelector(
			"[data-setting=ctx_per_slot]",
		) as HTMLElement;
		await click(byRole("combobox", undefined, ctx));
		await click(byRole("option", "16,384 tokens"));
		await click(byRole("button", "Use recommended", ctx));
		expect(text(ctx)).toContain("Recommended.");
		await advance("review");
		await click(byRole("button", "Add Qwen3-8B Q4_K_M", sheet()));
		await until(() => step() === "done", "the install");
		expect(installs(wizard)[0]?.settings).toMatchObject({
			parallel: 2,
			ctx_per_slot: 8_192,
		});
	});

	test("MLX on a device without the MLX runtime can't be added; the reason says so", async () => {
		await mountWizard();
		await fromHuggingFace("mlx-community/Qwen3-4B-4bit");
		await until(() => step() === "version", "the versions");
		expect(text(sheet())).toContain("edge-berlin-01 can't run MLX models.");
		expect(next().getAttribute("aria-disabled")).toBe("true");
		expect(text(sheet())).toContain(
			"This version can't be added to the device.",
		);
	});

	test("MLX on the Mac mini: small files are fingerprinted on this computer before the install", async () => {
		const wizard = await mountWizard({
			deviceId: studio,
			sample: macMiniModels(),
			features: MAC_MODEL_HOST_FEATURES,
		});
		await fromHuggingFace("mlx-community/Qwen3-4B-4bit");
		await until(() => step() === "version", "the versions");
		expect(text(sheet())).toContain("of unified memory");
		await advance("settings");
		expect(
			[...sheet().querySelectorAll("[data-setting]")].map((row) =>
				row.getAttribute("data-setting"),
			),
		).toEqual(["ctx_per_slot", "kv_cache_type", "residency"]);
		await advance("review");
		await click(byRole("button", /^Add /, sheet()));
		await until(() => step() === "done", "the install");
		const sha256 = async (body: string) =>
			[
				...new Uint8Array(
					await crypto.subtle.digest("SHA-256", new TextEncoder().encode(body)),
				),
			]
				.map((byte) => byte.toString(16).padStart(2, "0"))
				.join("");
		const [install] = installs(wizard);
		const assets = (
			install?.model as { assets: { file_name: string; digest: unknown }[] }
		).assets;
		expect(assets.map((asset) => [asset.file_name, asset.digest])).toEqual([
			[
				"config.json",
				{ algorithm: "sha256", hex: await sha256(SMALL_FILES.mlxConfig) },
			],
			["model.safetensors", { algorithm: "sha256", hex: LFS.mlxWeights }],
			["tokenizer.json", { algorithm: "sha256", hex: LFS.mlxTokenizer }],
			[
				"tokenizer_config.json",
				{
					algorithm: "sha256",
					hex: await sha256(SMALL_FILES.mlxTokenizerConfig),
				},
			],
		]);
		expect(install?.model).toMatchObject({ engine: "mlx", kind: "chat" });
		expect(install?.settings).toEqual({
			ctx_per_slot: 8_192,
			kv_cache_type: "f16",
		});
		const tokenizerConfig = `https://huggingface.co/mlx-community/Qwen3-4B-4bit/resolve/${REVISIONS.mlx}/tokenizer_config.json?download=true`;
		expect(
			wizard.hub.requests.filter((url) => url === tokenizerConfig),
		).toHaveLength(2);
	});

	test("an oversized model file list disables Add before fingerprinting or sending", async () => {
		const repos = sampleRepos();
		const repo = repos["mlx-community/Qwen3-4B-4bit"];
		const { "model.safetensors": _weights, ...smallFiles } = repo.files;
		repo.files = smallFiles;
		for (let index = 0; index < 48; index++) {
			repo.files[
				`model-${String(index + 1).padStart(5, "0")}-${"shard".repeat(12)}.safetensors`
			] = {
				size: 1024 * 1024,
				lfs: index.toString(16).padStart(64, "0"),
			};
		}
		const wizard = await mountWizard({
			deviceId: studio,
			sample: macMiniModels(),
			features: MAC_MODEL_HOST_FEATURES,
			hub: fakeHuggingFace(repos),
		});
		await fromHuggingFace("mlx-community/Qwen3-4B-4bit");
		await until(() => step() === "version", "the versions");
		await advance("settings");
		await advance("review");
		expect(text(sheet())).toContain(
			"This model's file list exceeds the 16 KiB limit for adding it here.",
		);
		expect(next().getAttribute("aria-disabled")).toBe("true");
		const fetched = wizard.hub.requests.length;
		await click(next());
		expect(installs(wizard)).toEqual([]);
		expect(wizard.hub.requests).toHaveLength(fetched);
	});

	test("MLX on a Mac without a llama.cpp pack: the GPU shares RAM, so it fits with the standard context", async () => {
		const sample = macMiniModels();
		sample.system = {
			...sample.system,
			ram: { total: 16 * GIB, free: 10 * GIB },
			gpus: [
				{
					name: "Apple M4",
					backend: "metal",
					memory_total: null,
					memory_free: null,
				},
			],
		};
		sample.runtimes = sample.runtimes.filter(
			(runtime) => runtime.runtime === "mlx",
		);
		const wizard = await mountWizard({
			deviceId: studio,
			sample,
			features: MAC_MODEL_HOST_FEATURES,
		});
		await fromHuggingFace("mlx-community/Qwen3-4B-4bit");
		await until(() => step() === "version", "the versions");
		const page = text(sheet());
		expect(page).toContain("Fits on the GPU");
		expect(page).toContain("About 3.8 GiB of unified memory · 10.0 GiB free");
		expect(page).not.toContain("Too large");
		expect(page).not.toContain("counts the processor only");
		await advance("settings");
		await advance("review");
		await click(byRole("button", /^Add /, sheet()));
		await until(() => step() === "done", "the install");
		expect(installs(wizard)[0]?.settings).toEqual({
			ctx_per_slot: 8_192,
			kv_cache_type: "f16",
		});
	});

	test("a context limit beyond what a device takes is offered at the protocol's most", async () => {
		const repos = sampleRepos();
		const qwen = repos["Qwen/Qwen3-8B-GGUF"];
		if (!qwen) throw new Error("The fake hub has no Qwen3-8B GGUF.");
		qwen.info = {
			...qwen.info,
			gguf: { total: 8_190_735_360, context_length: 10_485_760 },
		};
		await mountWizard({ hub: fakeHuggingFace(repos) });
		await fromHuggingFace("Qwen/Qwen3-8B-GGUF");
		await until(() => step() === "version", "the versions");
		await advance("settings");
		const ctx = sheet().querySelector(
			"[data-setting=ctx_per_slot]",
		) as HTMLElement;
		await click(byRole("combobox", undefined, ctx));
		const options = allByRole("option").map((option) => option.textContent);
		expect(options.at(-1)).toBe("1,048,576 tokens");
		expect(options).not.toContain("10,485,760 tokens");
	});

	test("a hub model: the hub's blake3 digest, its CDN link first and the pinned Hugging Face file second", async () => {
		const wizard = await mountWizard();
		await until(() => text(sheet()).includes("Qwen3 8B"), "the hub list");
		await click(byRole("radio", /^Qwen3 8B/, sheet()));
		await advance("version");
		expect(text(sheet())).toContain("Q4_K_M");
		await advance("settings");
		await advance("review");
		await click(byRole("button", "Add Qwen3 8B Q4_K_M", sheet()));
		await until(() => step() === "done", "the install");
		expect(installs(wizard)[0]?.model).toMatchObject({
			display_name: "Qwen3 8B Q4_K_M",
			assets: [
				{
					digest: { algorithm: "blake3", hex: "c".repeat(64) },
					sources: [
						"https://cdn.flow-like.com/bits/qwen3-8b",
						`https://huggingface.co/Qwen/Qwen3-8B-GGUF/resolve/${REVISIONS.qwenGguf}/Qwen3-8B-Q4_K_M.gguf?download=true`,
					],
				},
			],
		});
	});

	test("the device already holds the files: nothing downloads, and the sheet says a second model gets made", async () => {
		const sample = gpuBoxModels();
		const qwen = sample.models.find((model) => model.id === "qwen3-8b");
		if (qwen) qwen.assets = [{ algorithm: "blake3", hex: "c".repeat(64) }];
		await mountWizard({ sample });
		await until(() => text(sheet()).includes("Qwen3 8B"), "the hub list");
		await click(byRole("radio", /^Qwen3 8B/, sheet()));
		await advance("version");
		expect(text(sheet())).toContain("Already on the device");
		await advance("settings");
		await advance("review");
		const review = text(sheet());
		expect(review).toContain(
			"edge-berlin-01 has every file of it already, so nothing is downloaded.",
		);
		expect(review).toContain(
			"edge-berlin-01 already hosts these files as Qwen3-8B Q4_K_M.",
		);
	});

	test("a repository that can't be read says why and stays at the first step", async () => {
		await mountWizard();
		await fromHuggingFace("nobody/nothing");
		await until(() => text(sheet()).includes("couldn't be read"), "the error");
		expect(text(sheet())).toContain(
			"The model couldn't be read: Hugging Face model was not found",
		);
		expect(step()).toBe("source");
	});

	test("the device refuses: its sentence shows at the review and nothing is added", async () => {
		const wizard = await mountWizard();
		wizard.view.fake
			.agent(edge)
			.models.refuse("install", "limit", "The model disk is full.");
		await fromHuggingFace("Qwen/Qwen3-8B-GGUF");
		await until(() => step() === "version", "the versions");
		await advance("settings");
		await advance("review");
		await click(byRole("button", "Add Qwen3-8B Q4_K_M", sheet()));
		await until(
			() => text(sheet()).includes("The model disk is full."),
			"the refusal",
		);
		expect(step()).toBe("review");
		expect(
			wizard.view.fake.agent(edge).models.state.models.map((model) => model.id),
		).not.toContain("qwen3-8b-q4_k_m");
	});

	test("a model past the overview's one reply keeps its id: the next free one is used", async () => {
		const wizard = await mountWizard({ sample: crowdedGpuBox(earlierQwen) });
		await until(() => text(sheet()).includes("Qwen3 8B"), "the hub list");
		await click(byRole("radio", /^Qwen3 8B/, sheet()));
		await advance("version");
		await advance("settings");
		await advance("review");
		await click(byRole("button", "Add Qwen3 8B Q4_K_M", sheet()));
		await until(() => step() === "done", "the install");
		expect(installs(wizard).map((write) => write.model_id)).toEqual([
			"qwen3-8b-q4_k_m-2",
		]);
	});

	test("files of a model past the overview's one reply are on the device, and that model is their twin", async () => {
		const wizard = await mountWizard({ sample: crowdedGpuBox(earlierQwen) });
		await until(() => step() === "source", "the wizard");
		await fromHuggingFace("Qwen/Qwen3-8B-GGUF");
		await until(() => step() === "version", "the versions");
		expect(text(sheet())).toContain("Already on the device");
		await advance("settings");
		await advance("review");
		const review = text(sheet());
		expect(review).toContain(
			"edge-berlin-01 has every file of it already, so nothing is downloaded.",
		);
		expect(review).toContain(
			"edge-berlin-01 already hosts these files as Qwen3-8B Q4_K_M.",
		);
		await click(byRole("button", "Add Qwen3-8B Q4_K_M", sheet()));
		await until(() => step() === "done", "the install");
		expect(installs(wizard).map((write) => write.model_id)).toEqual([
			"qwen3-8b-q4_k_m-2",
		]);
	});

	test("My Bits: a search reaches every Bit, and the field says when more match than are shown", async () => {
		const mine = Array.from({ length: 30 }, (_, index) =>
			hubBit({
				id: `my-model-${index + 1}`,
				hub: "",
				meta: {
					en: {
						...hubBit().meta.en,
						name: `My Model ${index + 1}`,
					} as IBit["meta"][string],
				},
			}),
		);
		await mountWizard({ bits: fakeBits([], mine) });
		await until(() => step() === "source", "the wizard");
		await click(byRole("button", "My Bits", sheet()));
		const shown = () =>
			allByRole("radio", undefined, sheet()).map((radio) =>
				radio.getAttribute("value"),
			);
		await until(() => shown().length === 25, "the first Bits");
		expect(text(sheet())).toContain(
			"25 of 30 shown. Search to find the others.",
		);
		const search = byRole("textbox", "Search your models", sheet());
		await typeInto(search, "model 30");
		expect(shown()).toEqual(["my-model-30"]);
		expect(text(sheet())).not.toContain("Search to find the others.");
		await click(byRole("radio", /^My Model 30/, sheet()));
		expect(next().getAttribute("aria-disabled")).toBeNull();
		await typeInto(search, "no such model");
		expect(text(sheet())).toContain("None of your models matches.");
	});

	test("closing the sheet while it fingerprints stops the downloads and sends nothing", async () => {
		const hub = fakeHuggingFace();
		const held: AbortSignal[] = [];
		const gate: { release?: () => void } = {};
		const released = new Promise<void>((resolve) => {
			gate.release = resolve;
		});
		const wizard = await mountWizard({
			deviceId: studio,
			sample: macMiniModels(),
			features: MAC_MODEL_HOST_FEATURES,
			hub: {
				...hub,
				fetch: async (input, init) => {
					if (init?.signal) {
						held.push(init.signal);
						await released;
					}
					return hub.fetch(input, init);
				},
			},
		});
		await fromHuggingFace("mlx-community/Qwen3-4B-4bit");
		await until(() => step() === "version", "the versions");
		await advance("settings");
		await advance("review");
		await click(byRole("button", /^Add /, sheet()));
		await until(() => held.length === 1, "the first fingerprint");
		expect(text(sheet())).toContain("Fingerprinting 0 of 2 small files");
		await wizard.view.rerender(<div />);
		expect(held[0]?.aborted).toBe(true);
		gate.release?.();
		for (let round = 0; round < 5; round++) await settle();
		expect(held).toHaveLength(1);
		expect(installs(wizard)).toEqual([]);
	});

	test("locked: the sheet says so instead of offering models", async () => {
		await mountWizard({ unlock: "none" });
		expect(text(sheet())).toContain("Unlock");
		expect(sheet().querySelector("[data-step]")).toBeNull();
	});
});
