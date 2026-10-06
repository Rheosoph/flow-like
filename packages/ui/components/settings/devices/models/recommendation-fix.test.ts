import { describe, expect, test } from "bun:test";
import { getI18n } from "@flow-like/locales";
import {
	gpuBoxModels,
	macMiniModels,
	overviewOf,
} from "../../../../lib/device-management/model/__fixtures__/sample-models";
import type {
	HostedModel,
	ModelSettings,
	ModelsOverview,
	Recommendation,
	Residency,
	RuntimeInfo,
} from "../../../../lib/device-management/models";
import type { DevicesT } from "../primitives/area-context";
import { recommendationFix } from "./recommendation-copy";
import type { ModelCommand, ModelsActions } from "./use-models-action";

const t = getI18n().getFixedT("en", "devices") as DevicesT;

/** R3: wire values and placeholders never reach a label. */
const WIRE =
	/q8_0|q4_0|f16|kv_cache_type|ctx_per_slot|gpu_layers|on_demand|always_on|pinned_off|llamacpp|install_runtime|\{\{|undefined|NaN/;

interface Sent {
	kind: string;
	args: unknown[];
}

/** Records which `useModelsAction` command a fix builds. */
function recordingActions() {
	const sent: Sent[] = [];
	const command =
		(kind: string) =>
		(...args: unknown[]) => {
			sent.push({ kind, args });
			return { request: kind } as unknown as ModelCommand<unknown>;
		};
	const actions = {
		install: command("install"),
		configure: command("configure"),
		load: command("load"),
		unload: command("unload"),
		remove: command("remove"),
		installRuntime: command("install_runtime"),
		removeRuntime: command("remove_runtime"),
		cancelJob: command("cancel_job"),
		ensure: command("ensure"),
	} as unknown as ModelsActions;
	return { sent, actions };
}

const gpuBox = () => overviewOf(gpuBoxModels());
const macMini = () => overviewOf(macMiniModels());

const modelOf = (overview: ModelsOverview, id: string) =>
	overview.models.find((model) => model.id === id) as HostedModel;

function fixOf(overview: ModelsOverview, recommendation: Recommendation) {
	const { sent, actions } = recordingActions();
	const fix = recommendationFix(t, recommendation, overview, actions);
	return { fix, sent };
}

/** A configure fix for `model` that wants `settings` and `residency`. */
function configure(
	model: HostedModel,
	change: { settings?: ModelSettings; residency?: Residency },
	code: Recommendation["code"] = "kv_pressure",
	named = true,
): Recommendation {
	return {
		code,
		tier: "soon",
		...(named ? { model_id: model.id } : {}),
		fix: {
			kind: "configure",
			model_id: model.id,
			expected_revision: model.revision,
			settings: change.settings ?? model.settings,
			residency: change.residency ?? model.residency,
		},
	};
}

describe("the device's own fixes", () => {
	test("every sample recommendation gets the button that names its change", () => {
		const labels = [gpuBox(), macMini()].flatMap((overview) =>
			overview.recommendations.map(
				(recommendation) => fixOf(overview, recommendation).fix?.label,
			),
		);
		expect(labels).toEqual([
			"Use an 8-bit cache…",
			"Use 6 slots…",
			"Apply the recommended settings…",
			"Install llama.cpp for Metal…",
			"Use 16,384 tokens per slot…",
		]);
		for (const label of labels) expect(label).not.toMatch(WIRE);
	});

	test("a configure fix runs `configure` on the listed model with the fix's settings and residency", () => {
		const overview = gpuBox();
		const recommendation = overview.recommendations[0] as Recommendation;
		const { fix, sent } = fixOf(overview, recommendation);
		expect(fix?.command).toMatchObject({ request: "configure" });
		expect(sent).toHaveLength(1);
		const [model, change] = sent[0]?.args as [
			HostedModel,
			{ settings: ModelSettings; residency: Residency },
		];
		expect(model.id).toBe("qwen3-8b");
		expect(model.revision).toBe(3);
		expect(change.settings.kv_cache_type).toBe("q8_0");
		expect(change.settings.parallel).toBe(4);
		expect(change.residency).toEqual({ mode: "always_on" });
	});

	test("a runtime fix installs the listed pack, or updates an installed one", () => {
		const overview = macMini();
		const gpu = overview.recommendations[0] as Recommendation;
		const { fix, sent } = fixOf(overview, gpu);
		expect(fix?.command).toMatchObject({ request: "install_runtime" });
		expect(sent[0]?.args[0]).toMatchObject({
			runtime: "llamacpp",
			backend: "metal",
		} satisfies Partial<RuntimeInfo>);
		const outdated: Recommendation = {
			code: "runtime_outdated",
			tier: "later",
			params: { latest: "b10900" },
			fix: { kind: "install_runtime", runtime: "llamacpp", backend: "vulkan" },
		};
		expect(fixOf(gpuBox(), outdated).fix?.label).toBe(
			"Update llama.cpp for Vulkan…",
		);
	});
});

describe("no button", () => {
	test("without a fix, for a model the overview doesn't list, or for a runtime it doesn't offer", () => {
		const overview = gpuBox();
		expect(
			fixOf(overview, { code: "disk_low", tier: "soon" }).fix,
		).toBeUndefined();
		const ghost: Recommendation = {
			code: "idle_resident",
			tier: "later",
			model_id: "ghost",
			fix: { kind: "unload", model_id: "ghost" },
		};
		expect(fixOf(overview, ghost).fix).toBeUndefined();
		const metal: Recommendation = {
			code: "gpu_unused",
			tier: "soon",
			fix: { kind: "install_runtime", runtime: "llamacpp", backend: "metal" },
		};
		expect(fixOf(overview, metal).fix).toBeUndefined();
	});

	test("when the model changed since the device made the fix", () => {
		const overview = gpuBox();
		const model = modelOf(overview, "qwen3-8b");
		const stale = configure(
			{ ...model, revision: model.revision - 1 },
			{ settings: { ...model.settings, parallel: 6 } },
		);
		expect(fixOf(overview, stale).fix).toBeUndefined();
	});

	test("when nothing would change, or there is nothing to unload", () => {
		const overview = gpuBox();
		const model = modelOf(overview, "qwen3-8b");
		expect(fixOf(overview, configure(model, {})).fix).toBeUndefined();
		const gemma = modelOf(overview, "gemma-3-4b");
		const unload: Recommendation = {
			code: "memory_pressure",
			tier: "now",
			fix: { kind: "unload", model_id: gemma.id },
		};
		expect(fixOf(overview, unload).fix).toBeUndefined();
	});
});

describe("labels", () => {
	const overview = gpuBox();
	const qwen = modelOf(overview, "qwen3-8b");
	const label = (settings: ModelSettings) =>
		fixOf(
			overview,
			configure(qwen, { settings: { ...qwen.settings, ...settings } }),
		).fix?.label;

	test("one changed setting by name", () => {
		expect(label({ kv_cache_type: "q4_0" })).toBe("Use a 4-bit cache…");
		expect(label({ kv_cache_type: "f16" })).toBe("Use a 16-bit cache…");
		expect(label({ parallel: 1 })).toBe("Use 1 slot…");
		expect(label({ threads: 1 })).toBe("Use 1 thread…");
		expect(label({ gpu_layers: { count: 0 } })).toBe(
			"Run on the processor only…",
		);
		expect(label({ gpu_layers: { count: 20 } })).toBe(
			"Put 20 layers on the GPU…",
		);
		expect(label({ flash_attn: false })).toBe("Turn off flash attention…");
		const sample = gpuBoxModels();
		sample.models = sample.models.map((model) =>
			model.id === qwen.id
				? {
						...model,
						settings: { ...model.settings, gpu_layers: { count: 10 } },
					}
				: model,
		);
		const pinned = overviewOf(sample);
		const fitted = configure(modelOf(pinned, qwen.id), {
			settings: { ...modelOf(pinned, qwen.id).settings, gpu_layers: "auto" },
		});
		expect(fixOf(pinned, fitted).fix?.label).toBe("Fit the layers to the GPU…");
	});

	test("residency names the model, so a device-wide recommendation still says which", () => {
		const onDemand = configure(
			qwen,
			{ residency: { mode: "on_demand", idle_unload_after_seconds: 900 } },
			"memory_pressure",
			false,
		);
		expect(fixOf(overview, onDemand).fix?.label).toBe(
			"Load Qwen3-8B Q4_K_M on demand…",
		);
		const off = configure(qwen, { residency: { mode: "pinned_off" } });
		expect(fixOf(overview, off).fix?.label).toBe("Keep Qwen3-8B Q4_K_M off…");
		const nomic = modelOf(overview, "nomic-embed-v1.5");
		const on = configure(nomic, { residency: { mode: "always_on" } });
		expect(fixOf(overview, on).fix?.label).toBe(
			"Keep Nomic Embed v1.5 loaded…",
		);
	});

	test("several changes, a dropped field or a model the sentence doesn't name read as the recommended settings", () => {
		expect(label({ parallel: 6, kv_cache_type: "q8_0" })).toBe(
			"Apply the recommended settings…",
		);
		expect(
			fixOf(
				overview,
				configure(qwen, { settings: { gpu_layers: { count: 1 }, threads: 8 } }),
			).fix?.label,
		).toBe("Apply the recommended settings…");
		const { flash_attn: _flash, ...dropped } = qwen.settings;
		expect(
			fixOf(overview, configure(qwen, { settings: dropped })).fix?.label,
		).toBe("Apply the recommended settings…");
		const unnamed = configure(
			qwen,
			{ settings: { ...qwen.settings, parallel: 2 } },
			"memory_pressure",
			false,
		);
		expect(fixOf(overview, unnamed).fix?.label).toBe(
			"Apply the recommended settings to Qwen3-8B Q4_K_M…",
		);
	});

	test("an unload names its model", () => {
		const unload: Recommendation = {
			code: "memory_pressure",
			tier: "now",
			params: { memory_percent: 94 },
			fix: { kind: "unload", model_id: "nomic-embed-v1.5" },
		};
		const { fix, sent } = fixOf(overview, unload);
		expect(fix?.label).toBe("Unload Nomic Embed v1.5…");
		expect(sent[0]?.kind).toBe("unload");
	});
});
