import type {
	AdviceReason,
	KvCacheType,
	SettingsField,
} from "../../../../../lib/device-management/model/models/fit";
import type { ModelSettings } from "../../../../../lib/device-management/models";
import type { DevicesT } from "../../primitives/area-context";

/*
 * Words for the model settings (plan §3.7 settings sheet): each field, its
 * values and why a value is recommended. Wire values (`q8_0`, `auto`) never
 * reach the screen (R3). Literal `devices:`-prefixed keys for the extractor.
 */

type Copy = (t: DevicesT) => string;

const FIELD_LABELS: Record<SettingsField, Copy> = {
	ctx_per_slot: (t) =>
		t("devices:models.settings.field.ctx", "Context per slot"),
	parallel: (t) =>
		t("devices:models.settings.field.parallel", "Parallel requests"),
	kv_cache_type: (t) =>
		t("devices:models.settings.field.kvCache", "Context cache precision"),
	gpu_layers: (t) =>
		t("devices:models.settings.field.gpuLayers", "Layers on the GPU"),
	threads: (t) =>
		t("devices:models.settings.field.threads", "Processor threads"),
	flash_attn: (t) =>
		t("devices:models.settings.field.flashAttn", "Flash attention"),
};

export const fieldLabel = (t: DevicesT, field: SettingsField) =>
	FIELD_LABELS[field](t);

const FIELD_HINTS: Record<SettingsField, Copy> = {
	ctx_per_slot: (t) =>
		t(
			"devices:models.settings.hint.ctx",
			"How many tokens one request may hold: prompt, history and answer.",
		),
	parallel: (t) =>
		t(
			"devices:models.settings.hint.parallel",
			"Requests the model answers at the same time. Each one keeps its own context.",
		),
	kv_cache_type: (t) =>
		t(
			"devices:models.settings.hint.kvCache",
			"Lower precision halves the memory the context takes, at a small cost in quality.",
		),
	gpu_layers: (t) =>
		t(
			"devices:models.settings.hint.gpuLayers",
			"Layers on the GPU answer much faster than layers on the processor.",
		),
	threads: (t) =>
		t(
			"devices:models.settings.hint.threads",
			"Processor threads for the parts that run on the processor.",
		),
	flash_attn: (t) =>
		t(
			"devices:models.settings.hint.flashAttn",
			"A faster way to read long prompts that also takes less memory.",
		),
};

/** What a field does, under its label. */
export const fieldHint = (t: DevicesT, field: SettingsField) =>
	FIELD_HINTS[field](t);

const KV_LABELS: Record<KvCacheType, Copy> = {
	f16: (t) => t("devices:models.settings.kv.f16", "Full precision"),
	q8_0: (t) => t("devices:models.settings.kv.q8", "8-bit"),
	q4_0: (t) => t("devices:models.settings.kv.q4", "4-bit"),
};

export const kvLabel = (t: DevicesT, type: KvCacheType) => KV_LABELS[type](t);

type GpuLayers = NonNullable<ModelSettings["gpu_layers"]>;

/** `{count: n}` at or above this puts every layer on the GPU. */
export const ALL_LAYERS = 999;

export function gpuLayersLabel(t: DevicesT, value: GpuLayers) {
	if (value === "auto")
		return t("devices:models.settings.gpuLayers.auto", "As many as fit");
	if (value.count === 0)
		return t("devices:models.settings.gpuLayers.none", "None: processor only");
	if (value.count >= ALL_LAYERS)
		return t("devices:models.settings.gpuLayers.all", "All layers");
	return t("devices:models.settings.gpuLayers.count", {
		count: value.count,
		defaultValue_one: "{{count, number}} layer",
		defaultValue_other: "{{count, number}} layers",
	});
}

const VALUE_LABELS: Record<
	SettingsField,
	(t: DevicesT, settings: ModelSettings) => string
> = {
	ctx_per_slot: (t, settings) =>
		t("devices:models.settings.value.tokens", {
			count: settings.ctx_per_slot ?? 0,
			defaultValue_one: "{{count, number}} token",
			defaultValue_other: "{{count, number}} tokens",
		}),
	parallel: (t, settings) =>
		t("devices:models.settings.value.slots", {
			count: settings.parallel ?? 1,
			defaultValue_one: "{{count, number}} at a time",
			defaultValue_other: "{{count, number}} at a time",
		}),
	kv_cache_type: (t, settings) => kvLabel(t, settings.kv_cache_type ?? "f16"),
	gpu_layers: (t, settings) => gpuLayersLabel(t, settings.gpu_layers ?? "auto"),
	threads: (t, settings) =>
		t("devices:models.settings.value.threads", {
			count: settings.threads ?? 1,
			defaultValue_one: "{{count, number}} thread",
			defaultValue_other: "{{count, number}} threads",
		}),
	flash_attn: (t, settings) =>
		settings.flash_attn === false
			? t("devices:models.settings.value.off", "Off")
			: t("devices:models.settings.value.on", "On"),
};

/** A field the model leaves out: the device picks its value when the model loads. */
export const deviceDefaultLabel = (t: DevicesT) =>
	t("devices:models.settings.value.deviceDefault", "Device default");

/** A field's value as the sheet names it. */
export const valueLabel = (
	t: DevicesT,
	field: SettingsField,
	settings: ModelSettings,
) =>
	settings[field] === undefined
		? deviceDefaultLabel(t)
		: VALUE_LABELS[field](t, settings);

/** Why flash attention can't be off. */
export const flashNeededText = (t: DevicesT) =>
	t(
		"devices:models.settings.flashNeeded",
		"An 8-bit or 4-bit context cache needs flash attention, so it can't be off.",
	);

export interface ReasonContext {
	/** The model's context limit, when known. */
	contextLength?: number;
	cores: number;
}

const REASONS: Record<
	AdviceReason,
	(t: DevicesT, context: ReasonContext) => string
> = {
	model_limit: (t, context) =>
		t("devices:models.settings.reason.modelLimit", {
			count: context.contextLength ?? 0,
			defaultValue_one: "The model handles at most {{count, number}} token.",
			defaultValue_other: "The model handles at most {{count, number}} tokens.",
		}),
	standard: (t) =>
		t(
			"devices:models.settings.reason.standard",
			"The engine's best-practice default.",
		),
	memory: (t) =>
		t(
			"devices:models.settings.reason.memory",
			"Lower, so the model fits where it runs fastest on this device.",
		),
	gpu_slots: (t) =>
		t(
			"devices:models.settings.reason.gpuSlots",
			"A GPU answers four requests at once at nearly the speed of one.",
		),
	cpu_slots: (t) =>
		t(
			"devices:models.settings.reason.cpuSlots",
			"On the processor, few requests at a time answer fastest.",
		),
	all_layers: (t) =>
		t(
			"devices:models.settings.reason.allLayers",
			"Every layer fits on the GPU.",
		),
	some_layers: (t) =>
		t(
			"devices:models.settings.reason.someLayers",
			"The GPU takes as many layers as fit; the rest run on the processor.",
		),
	no_gpu: (t) =>
		t(
			"devices:models.settings.reason.noGpu",
			"There is no GPU with room for it, so it runs on the processor.",
		),
	cores: (t, context) =>
		t("devices:models.settings.reason.cores", {
			count: context.cores,
			defaultValue_one:
				"One per processor core: this device has {{count, number}}.",
			defaultValue_other:
				"One per processor core: this device has {{count, number}}.",
		}),
	flash: (t) =>
		t(
			"devices:models.settings.reason.flash",
			"Faster on long prompts and lighter on memory.",
		),
	idle_unload: (t) =>
		t(
			"devices:models.settings.reason.idleUnload",
			"Frees the memory when nobody uses it; the next request loads it again.",
		),
};

/** Why the recommended value is what it is. */
export const adviceReason = (
	t: DevicesT,
	reason: AdviceReason,
	context: ReasonContext,
) => REASONS[reason](t, context);
