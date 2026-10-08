import type {
	HostedModel,
	ModelSettings,
	ModelsOverview,
	Recommendation,
	Residency,
	RuntimeInfo,
} from "../../../../lib/device-management/models";
import { bytesText } from "../observe/observe-data";
import type { DevicesT } from "../primitives/area-context";
import { durationMs, runtimeName } from "./models-copy";
import type { ModelCommand, ModelsActions } from "./use-models-action";

/*
 * One sentence per recommendation code (plan §3.6): the device sends a code
 * and parameters, the copy lives here. A device may leave a parameter out, so
 * every sentence has a form without it. Where the device attached a fix, it
 * runs as the matching `useModelsAction` command (`recommendationFix`).
 */

export interface RecommendationContext {
	device: string;
	/** The display name of a hosted model; undefined when the overview doesn't list it. */
	modelName(modelId: string): string | undefined;
}

export interface RecommendationCopy {
	sentence: string;
	/** Device names in the sentence, set in mono (`AttentionEntry.names`). */
	names: string[];
}

type Params = NonNullable<Recommendation["params"]>;

const num = (params: Params, key: string): number | undefined => {
	const value = params[key];
	return typeof value === "number" ? value : undefined;
};

const str = (params: Params, key: string): string | undefined => {
	const value = params[key];
	return typeof value === "string" && value ? value : undefined;
};

interface Input {
	t: DevicesT;
	p: Params;
	/** The model's display name, its id when the overview doesn't list it, or "a model"; never first in a sentence. */
	model: string;
	/** The device named the model. */
	named: boolean;
	device: string;
	/** The runtime pack the attached fix installs ("llama.cpp for Vulkan"). */
	pack?: string;
}

type Sentence = (input: Input) => string;

const gpuUnused: Sentence = ({ t, p, model, named }) => {
	const gpu =
		str(p, "gpu") ?? t("devices:models.recommendations.theGpu", "The GPU");
	return named
		? t(
				"devices:models.recommendations.gpuUnused",
				"{{gpu}} isn't used: {{model}} runs on the CPU. A GPU runtime runs it faster.",
				{ gpu, model },
			)
		: t(
				"devices:models.recommendations.gpuUnusedAll",
				"{{gpu}} isn't used: models run on the CPU. A GPU runtime runs them faster.",
				{ gpu },
			);
};

const partialOffload: Sentence = ({ t, p, model }) => {
	const quant = str(p, "quant");
	const size = num(p, "size_bytes");
	return quant && size !== undefined
		? t(
				"devices:models.recommendations.partialOffload",
				"Only part of {{model}} fits in GPU memory, so it runs slower. {{quant}} ({{size}}) or a shorter context would fit.",
				{ model, quant, size: bytesText(size) },
			)
		: t(
				"devices:models.recommendations.partialOffloadPlain",
				"Only part of {{model}} fits in GPU memory, so it runs slower. A smaller quantization or a shorter context would fit.",
				{ model },
			);
};

const requestsQueued: Sentence = ({ t, p, model }) => {
	const deferred = num(p, "deferred");
	const wait = num(p, "queue_p95_ms");
	if (deferred !== undefined && wait !== undefined)
		return t("devices:models.recommendations.requestsQueued", {
			count: deferred,
			model,
			wait: durationMs(t, wait),
			defaultValue_one:
				"Requests wait for {{model}}: {{count, number}} is queued and the slowest wait {{wait}}. More parallel slots help.",
			defaultValue_other:
				"Requests wait for {{model}}: {{count, number}} are queued and the slowest wait {{wait}}. More parallel slots help.",
		});
	return wait === undefined
		? t(
				"devices:models.recommendations.requestsQueuedPlain",
				"Requests wait for {{model}}. More parallel slots help.",
				{ model },
			)
		: t(
				"devices:models.recommendations.requestsQueuedWait",
				"Requests wait for {{model}}: the slowest wait {{wait}}. More parallel slots help.",
				{ model, wait: durationMs(t, wait) },
			);
};

const kvPressure: Sentence = ({ t, p, model }) => {
	const percent = num(p, "kv_percent");
	return percent === undefined
		? t(
				"devices:models.recommendations.kvPressurePlain",
				"The context cache of {{model}} is almost full. A shorter context per slot, an 8-bit cache or fewer slots frees memory.",
				{ model },
			)
		: t(
				"devices:models.recommendations.kvPressure",
				"The context cache of {{model}} is {{percent, number}} % full. A shorter context per slot, an 8-bit cache or fewer slots frees memory.",
				{ model, percent },
			);
};

const ctxTruncation: Sentence = ({ t, p, model }) => {
	const percent = num(p, "percent");
	const suggested = num(p, "suggested_ctx");
	const extra = num(p, "extra_bytes");
	if (percent !== undefined && suggested !== undefined && extra !== undefined)
		return t(
			"devices:models.recommendations.ctxTruncation",
			"{{percent, number}} % of prompts to {{model}} come close to its context limit. {{suggested, number}} tokens per slot needs {{extra}} more memory.",
			{ model, percent, suggested, extra: bytesText(extra) },
		);
	return t(
		"devices:models.recommendations.ctxTruncationPlain",
		"Prompts to {{model}} come close to its context limit. A longer context per slot needs more memory.",
		{ model },
	);
};

const memoryPressure: Sentence = ({ t, p, device }) => {
	const percent = num(p, "memory_percent");
	return percent === undefined
		? t(
				"devices:models.recommendations.memoryPressurePlain",
				"{{device}} is low on memory with models loaded. Load models on demand or unload one.",
				{ device },
			)
		: t(
				"devices:models.recommendations.memoryPressure",
				"{{device}} uses {{percent, number}} % of its memory with models loaded. Load models on demand or unload one.",
				{ device, percent },
			);
};

const idleResident: Sentence = ({ t, p, model }) => {
	const hours = num(p, "idle_hours");
	return hours === undefined
		? t(
				"devices:models.recommendations.idleResidentPlain",
				"Nothing uses {{model}}, but it stays loaded. Loading it on demand frees its memory.",
				{ model },
			)
		: t(
				"devices:models.recommendations.idleResident",
				"Nothing used {{model}} for {{hours, number}} hours, but it stays loaded. Loading it on demand frees its memory.",
				{ model, hours },
			);
};

const slowTtft: Sentence = ({ t, p, model }) => {
	const ttft = num(p, "ttft_p95_ms");
	return ttft === undefined
		? t(
				"devices:models.recommendations.slowTtftPlain",
				"Answers from {{model}} are slow to start because its prompt cache misses. Reusing the cache or a fixed system prompt helps.",
				{ model },
			)
		: t(
				"devices:models.recommendations.slowTtft",
				"Answers from {{model}} take up to {{ttft}} to start because its prompt cache misses. Reusing the cache or a fixed system prompt helps.",
				{ model, ttft: durationMs(t, ttft) },
			);
};

const cpuThreads: Sentence = ({ t, p, model }) => {
	const threads = num(p, "threads");
	const cores = num(p, "physical_cores");
	return threads !== undefined && cores !== undefined
		? t(
				"devices:models.recommendations.cpuThreads",
				"The engine of {{model}} uses {{threads, number}} threads on {{cores, number}} physical cores. One thread per core is faster.",
				{ model, threads, cores },
			)
		: t(
				"devices:models.recommendations.cpuThreadsPlain",
				"The engine of {{model}} uses another number of threads than the device has physical cores. One thread per core is faster.",
				{ model },
			);
};

const diskLow: Sentence = ({ t, p }) => {
	const free = num(p, "free_bytes");
	return free === undefined
		? t(
				"devices:models.recommendations.diskLowPlain",
				"The model disk is almost full. Remove models you don't use or give it more room.",
			)
		: t(
				"devices:models.recommendations.diskLow",
				"The model disk has {{free}} left. Remove models you don't use or give it more room.",
				{ free: bytesText(free) },
			);
};

/** The device sends one per outdated pack; the pack its fix updates tells them apart. */
const runtimeOutdated: Sentence = ({ t, p, device, pack }) => {
	const latest = str(p, "latest");
	if (pack)
		return latest
			? t(
					"devices:models.recommendations.runtimeOutdatedPack",
					"A newer build of {{pack}}, {{latest}}, is available for {{device}}.",
					{ pack, latest, device },
				)
			: t(
					"devices:models.recommendations.runtimeOutdatedPackPlain",
					"A newer build of {{pack}} is available for {{device}}.",
					{ pack, device },
				);
	return latest
		? t(
				"devices:models.recommendations.runtimeOutdated",
				"A newer model runtime, {{latest}}, is available for {{device}}.",
				{ latest, device },
			)
		: t(
				"devices:models.recommendations.runtimeOutdatedPlain",
				"A newer model runtime is available for {{device}}.",
				{ device },
			);
};

const containerGpuHidden: Sentence = ({ t, device }) =>
	t(
		"devices:models.recommendations.containerGpuHidden",
		"{{device}} runs in a container that can't see the host's GPU. Start the container with GPU access to use it.",
		{ device },
	);

const SENTENCES = {
	gpu_unused: gpuUnused,
	partial_offload: partialOffload,
	requests_queued: requestsQueued,
	kv_pressure: kvPressure,
	ctx_truncation: ctxTruncation,
	memory_pressure: memoryPressure,
	idle_resident: idleResident,
	slow_ttft: slowTtft,
	cpu_threads: cpuThreads,
	disk_low: diskLow,
	runtime_outdated: runtimeOutdated,
	container_gpu_hidden: containerGpuHidden,
} satisfies Record<Recommendation["code"], Sentence>;

/** The runtime pack an `install_runtime` fix names; none for any other fix. */
function fixPack(
	recommendation: Recommendation,
): Pick<RuntimeInfo, "runtime" | "backend"> | undefined {
	const fix = recommendation.fix;
	return fix?.kind === "install_runtime"
		? { runtime: fix.runtime, backend: fix.backend }
		: undefined;
}

export function recommendationCopy(
	t: DevicesT,
	recommendation: Recommendation,
	context: RecommendationContext,
): RecommendationCopy {
	const id = recommendation.model_id;
	const model = id
		? (context.modelName(id) ?? id)
		: t("devices:models.recommendations.anyModel", "a model");
	const pack = fixPack(recommendation);
	const sentence = SENTENCES[recommendation.code]({
		t,
		p: recommendation.params ?? {},
		model,
		named: id !== undefined,
		device: context.device,
		...(pack ? { pack: runtimeName(t, pack) } : {}),
	});
	return { sentence, names: [context.device] };
}

/** A stable id for one recommendation: its code, the model it is about and the pack its fix installs. */
export function recommendationId(recommendation: Recommendation): string {
	const pack = fixPack(recommendation);
	const of = recommendation.model_id ?? "device";
	return pack
		? `${recommendation.code}:${of}:${pack.runtime}-${pack.backend}`
		: `${recommendation.code}:${of}`;
}

/** `recommendationId` of each entry, numbered where the device repeats one, so list keys stay unique. */
export function recommendationIds(
	recommendations: readonly Recommendation[],
): string[] {
	const seen = new Map<string, number>();
	return recommendations.map((recommendation) => {
		const id = recommendationId(recommendation);
		const count = (seen.get(id) ?? 0) + 1;
		seen.set(id, count);
		return count > 1 ? `${id}:${count}` : id;
	});
}

/* Fixes. */

type Fix = NonNullable<Recommendation["fix"]>;
type ConfigureFix = Extract<Fix, { kind: "configure" }>;
type RuntimeFix = Extract<Fix, { kind: "install_runtime" }>;

export interface RecommendationFix {
	/** The control: what the fix changes ("Use an 8-bit cache…"). */
	label: string;
	/** The `useModelsAction` command; it confirms and reports like every model change. */
	command: ModelCommand<unknown>;
}

const SETTING_FIELDS = [
	"ctx_per_slot",
	"parallel",
	"kv_cache_type",
	"gpu_layers",
	"threads",
	"flash_attn",
] as const satisfies readonly (keyof ModelSettings)[];

type SettingField = (typeof SETTING_FIELDS)[number];

/** Fields the fix changes; one it leaves out goes back to the device's recommended value. */
function settingChanges(current: ModelSettings, wanted: ModelSettings) {
	return SETTING_FIELDS.filter(
		(field) => JSON.stringify(current[field]) !== JSON.stringify(wanted[field]),
	);
}

const sameResidency = (a: Residency, b: Residency) =>
	a.mode === b.mode &&
	(a.mode !== "on_demand" ||
		b.mode !== "on_demand" ||
		a.idle_unload_after_seconds === b.idle_unload_after_seconds);

function cacheLabel(
	t: DevicesT,
	type: NonNullable<ModelSettings["kv_cache_type"]>,
) {
	const labels = {
		q8_0: t(
			"devices:models.recommendations.fix.cacheQ8",
			"Use an 8-bit cache…",
		),
		q4_0: t("devices:models.recommendations.fix.cacheQ4", "Use a 4-bit cache…"),
		f16: t(
			"devices:models.recommendations.fix.cacheF16",
			"Use a 16-bit cache…",
		),
	} satisfies Record<typeof type, string>;
	return labels[type];
}

function gpuLayersLabel(
	t: DevicesT,
	layers: NonNullable<ModelSettings["gpu_layers"]>,
) {
	if (layers === "auto")
		return t(
			"devices:models.recommendations.fix.gpuAuto",
			"Fit the layers to the GPU…",
		);
	if (layers.count === 0)
		return t(
			"devices:models.recommendations.fix.cpuOnly",
			"Run on the processor only…",
		);
	return t("devices:models.recommendations.fix.gpuLayers", {
		count: layers.count,
		defaultValue_one: "Put {{count, number}} layer on the GPU…",
		defaultValue_other: "Put {{count, number}} layers on the GPU…",
	});
}

/** The label of a change to one field; `undefined` when the fix drops the field. */
function settingLabel(t: DevicesT, wanted: ModelSettings, field: SettingField) {
	const { ctx_per_slot, parallel, kv_cache_type, gpu_layers, threads } = wanted;
	const labels = {
		ctx_per_slot: () =>
			ctx_per_slot === undefined
				? undefined
				: t(
						"devices:models.recommendations.fix.context",
						"Use {{tokens, number}} tokens per slot…",
						{ tokens: ctx_per_slot },
					),
		parallel: () =>
			parallel === undefined
				? undefined
				: t("devices:models.recommendations.fix.slots", {
						count: parallel,
						defaultValue_one: "Use {{count, number}} slot…",
						defaultValue_other: "Use {{count, number}} slots…",
					}),
		kv_cache_type: () =>
			kv_cache_type === undefined ? undefined : cacheLabel(t, kv_cache_type),
		gpu_layers: () =>
			gpu_layers === undefined ? undefined : gpuLayersLabel(t, gpu_layers),
		threads: () =>
			threads === undefined
				? undefined
				: t("devices:models.recommendations.fix.threads", {
						count: threads,
						defaultValue_one: "Use {{count, number}} thread…",
						defaultValue_other: "Use {{count, number}} threads…",
					}),
		flash_attn: () => {
			if (wanted.flash_attn === undefined) return undefined;
			return wanted.flash_attn
				? t(
						"devices:models.recommendations.fix.flashOn",
						"Turn on flash attention…",
					)
				: t(
						"devices:models.recommendations.fix.flashOff",
						"Turn off flash attention…",
					);
		},
	} satisfies Record<SettingField, () => string | undefined>;
	return labels[field]();
}

function residencyLabel(t: DevicesT, residency: Residency, model: string) {
	const labels = {
		on_demand: () =>
			t(
				"devices:models.recommendations.fix.onDemand",
				"Load {{model}} on demand…",
				{ model },
			),
		always_on: () =>
			t(
				"devices:models.recommendations.fix.alwaysOn",
				"Keep {{model}} loaded…",
				{ model },
			),
		pinned_off: () =>
			t("devices:models.recommendations.fix.pinnedOff", "Keep {{model}} off…", {
				model,
			}),
	} satisfies Record<Residency["mode"], () => string>;
	return labels[residency.mode]();
}

/**
 * What a `configure` fix changes, as its button: one setting or the residency
 * by name, several as "the recommended settings". A recommendation that
 * doesn't name the model (memory pressure) gets labels that do.
 */
function configureLabel(
	t: DevicesT,
	model: HostedModel,
	fix: ConfigureFix,
	named: boolean,
) {
	const fields = settingChanges(model.settings, fix.settings);
	const residency = !sameResidency(model.residency, fix.residency);
	if (!fields.length)
		return residency
			? residencyLabel(t, fix.residency, model.display_name)
			: undefined;
	const single = fields.length === 1 && !residency && named;
	const one = single ? settingLabel(t, fix.settings, fields[0]) : undefined;
	return one ?? settingsLabel(t, model.display_name, named);
}

function settingsLabel(t: DevicesT, model: string, named: boolean) {
	return named
		? t(
				"devices:models.recommendations.fix.settings",
				"Apply the recommended settings…",
			)
		: t(
				"devices:models.recommendations.fix.settingsFor",
				"Apply the recommended settings to {{model}}…",
				{ model },
			);
}

const UNLOADABLE = new Set<HostedModel["state"]>(["loaded", "loading"]);

function modelFix(
	t: DevicesT,
	fix: Exclude<Fix, RuntimeFix>,
	model: HostedModel,
	named: boolean,
	actions: ModelsActions,
): RecommendationFix | undefined {
	if (fix.kind === "unload")
		return UNLOADABLE.has(model.state)
			? {
					label: t(
						"devices:models.recommendations.fix.unload",
						"Unload {{model}}…",
						{
							model: model.display_name,
						},
					),
					command: actions.unload(model),
				}
			: undefined;
	if (fix.expected_revision !== model.revision) return undefined;
	const label = configureLabel(t, model, fix, named);
	return label
		? {
				label,
				command: actions.configure(model, {
					settings: fix.settings,
					residency: fix.residency,
				}),
			}
		: undefined;
}

function runtimeFix(
	t: DevicesT,
	fix: RuntimeFix,
	overview: ModelsOverview,
	actions: ModelsActions,
): RecommendationFix | undefined {
	const matching = overview.runtimes.filter(
		(row) => row.runtime === fix.runtime && row.backend === fix.backend,
	);
	const runtime = matching.find((row) => row.installed) ?? matching[0];
	if (!runtime) return undefined;
	const name = runtimeName(t, runtime);
	return {
		label: runtime.installed
			? t("devices:models.recommendations.fix.update", "Update {{runtime}}…", {
					runtime: name,
				})
			: t(
					"devices:models.recommendations.fix.install",
					"Install {{runtime}}…",
					{
						runtime: name,
					},
				),
		command: actions.installRuntime(runtime),
	};
}

/**
 * The one-click fix the device attached to a recommendation, as the matching
 * command of `useModelsAction` (plan §3.6). None when the overview doesn't
 * list its model or runtime, when the model changed since the device computed
 * it (the fix names the revision it was made for), or when nothing would
 * change.
 */
export function recommendationFix(
	t: DevicesT,
	recommendation: Recommendation,
	overview: ModelsOverview,
	actions: ModelsActions,
): RecommendationFix | undefined {
	const fix = recommendation.fix;
	if (!fix) return undefined;
	if (fix.kind === "install_runtime")
		return runtimeFix(t, fix as RuntimeFix, overview, actions);
	const model = overview.models.find((entry) => entry.id === fix.model_id);
	if (!model) return undefined;
	return modelFix(
		t,
		fix as Exclude<Fix, RuntimeFix>,
		model,
		recommendation.model_id === model.id,
		actions,
	);
}
