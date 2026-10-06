import type {
	HostedModel,
	ModelBackend,
	ModelEngine,
	ModelJob,
	ModelKind,
	ModelRuntime,
	Residency,
} from "../../../../lib/device-management/models";
import { bytesText } from "../observe/observe-data";
import type { DevicesT } from "../primitives/area-context";
import type { ChipTone } from "../primitives/tone";
import type { HeadlineLead, ModelsHeadline } from "./models-view";

/*
 * Labels of the model host's wire values (R3: a wire value never reaches the
 * screen). Literal `devices:`-prefixed keys so the extractor files them under
 * `devices`; `satisfies` keeps every map exhaustive over its union.
 */

export function kindLabel(t: DevicesT, kind: ModelKind): string {
	const labels = {
		chat: t("devices:models.table.kind.chat", "Chat"),
		systemone: t("devices:models.table.kind.systemone", "Decisions"),
		vision: t("devices:models.table.kind.vision", "Vision"),
		embedding: t("devices:models.table.kind.embedding", "Embedding"),
	} satisfies Record<ModelKind, string>;
	return labels[kind];
}

export function engineLabel(t: DevicesT, engine: ModelEngine): string {
	const labels = {
		llamacpp: t("devices:models.table.engine.llamacpp", "llama.cpp"),
		mlx: t("devices:models.table.engine.mlx", "MLX"),
		onnx: t("devices:models.table.engine.onnx", "ONNX Runtime"),
	} satisfies Record<ModelEngine, string>;
	return labels[engine];
}

export function runtimeLabel(t: DevicesT, runtime: ModelRuntime): string {
	return engineLabel(t, runtime);
}

/** "llama.cpp for Vulkan": one runtime pack. */
export function runtimeName(
	t: DevicesT,
	pack: { runtime: ModelRuntime; backend: ModelBackend },
): string {
	return t(
		"devices:models.hardware.runtimeName",
		"{{runtime}} for {{backend}}",
		{
			runtime: runtimeLabel(t, pack.runtime),
			backend: backendLabel(t, pack.backend),
		},
	);
}

export function backendLabel(t: DevicesT, backend: ModelBackend): string {
	const labels = {
		cpu: t("devices:models.hardware.backend.cpu", "CPU"),
		vulkan: t("devices:models.hardware.backend.vulkan", "Vulkan"),
		metal: t("devices:models.hardware.backend.metal", "Metal"),
		cuda: t("devices:models.hardware.backend.cuda", "CUDA"),
	} satisfies Record<ModelBackend, string>;
	return labels[backend];
}

type ModelState = HostedModel["state"];

export const STATE_TONE: Record<ModelState, ChipTone> = {
	acquiring: "info",
	stopped: "outline",
	loading: "info",
	loaded: "good",
	unloading: "info",
	failed: "critical",
	unknown: "outline",
};

export function stateLabel(t: DevicesT, state: ModelState): string {
	const labels = {
		acquiring: t("devices:models.table.state.acquiring", "Downloading"),
		stopped: t("devices:models.table.state.stopped", "Not loaded"),
		loading: t("devices:models.table.state.loading", "Loading"),
		loaded: t("devices:models.table.state.loaded", "Loaded"),
		unloading: t("devices:models.table.state.unloading", "Unloading"),
		failed: t("devices:models.table.state.failed", "Failed"),
		unknown: t("devices:models.table.state.unknown", "State not known"),
	} satisfies Record<ModelState, string>;
	return labels[state];
}

type HostFailure = Extract<HostedModel, { state: "failed" }>["reason"];

export function failureLabel(t: DevicesT, reason: HostFailure): string {
	const labels = {
		asset_missing: t(
			"devices:models.table.failure.assetMissing",
			"A file is missing and couldn't be downloaded again.",
		),
		runtime_missing: t(
			"devices:models.table.failure.runtimeMissing",
			"No runtime for its engine is installed.",
		),
		insufficient_memory: t(
			"devices:models.table.failure.insufficientMemory",
			"Not enough memory, even after idle models were unloaded.",
		),
		engine_exited: t(
			"devices:models.table.failure.engineExited",
			"The engine stopped unexpectedly.",
		),
		health_timeout: t(
			"devices:models.table.failure.healthTimeout",
			"The engine didn't become ready in time.",
		),
		unknown: t(
			"devices:models.table.failure.unknown",
			"For a reason this version of Flow-Like doesn't know yet.",
		),
	} satisfies Record<HostFailure, string>;
	return labels[reason];
}

export function residencyLabel(t: DevicesT, residency: Residency): string {
	if (residency.mode === "always_on")
		return t("devices:models.table.residency.alwaysOn", "Always loaded");
	if (residency.mode === "pinned_off")
		return t("devices:models.table.residency.pinnedOff", "Kept off");
	return t(
		"devices:models.table.residency.onDemand",
		"Loads on demand, unloads after {{minutes, number}} min idle",
		{ minutes: Math.round(residency.idle_unload_after_seconds / 60) },
	);
}

type JobState = ModelJob["state"];

export function jobStateLabel(t: DevicesT, state: JobState): string {
	const labels = {
		queued: t("devices:models.downloads.state.queued", "Queued"),
		fetching: t("devices:models.downloads.state.fetching", "Downloading"),
		verifying: t("devices:models.downloads.state.verifying", "Verifying"),
		present: t("devices:models.downloads.state.present", "Done"),
		awaiting_push: t(
			"devices:models.downloads.state.awaitingPush",
			"Waiting for a computer to send it",
		),
		failed: t("devices:models.downloads.state.failed", "Failed"),
	} satisfies Record<JobState, string>;
	return labels[state];
}

type JobFailure = Extract<ModelJob, { state: "failed" }>;

export function jobFailureText(t: DevicesT, job: JobFailure): string {
	const texts = {
		egress_blocked: () =>
			t(
				"devices:models.downloads.failure.egressBlocked",
				"The device couldn't reach any download source. Its network may block outbound connections.",
			),
		http_status: () =>
			job.http_status === undefined
				? t(
						"devices:models.downloads.failure.httpStatus",
						"The download source answered with an error.",
					)
				: t(
						"devices:models.downloads.failure.httpStatusCode",
						"The download source answered with error {{status}}.",
						{ status: job.http_status },
					),
		digest_mismatch: () =>
			t(
				"devices:models.downloads.failure.digestMismatch",
				"The file didn't match its fingerprint, so the device discarded it.",
			),
		size_mismatch: () =>
			t(
				"devices:models.downloads.failure.sizeMismatch",
				"The file had another size than expected, so the device discarded it.",
			),
		disk_budget: () =>
			t(
				"devices:models.downloads.failure.diskBudget",
				"The model disk has no room left for it.",
			),
		no_sources: () =>
			t(
				"devices:models.downloads.failure.noSources",
				"No download source is known for this file.",
			),
		cancelled: () =>
			t("devices:models.downloads.failure.cancelled", "Cancelled."),
		io: () =>
			t(
				"devices:models.downloads.failure.io",
				"The device couldn't write the file to its disk.",
			),
		unknown: () =>
			t(
				"devices:models.downloads.failure.unknown",
				"The download failed for a reason this version of Flow-Like doesn't know yet.",
			),
	} satisfies Record<JobFailure["reason"], () => string>;
	return texts[job.reason]();
}

/* Headline. */

export interface HeadlineCopy {
	lead: string;
	rest?: string;
}

function headlineLead(t: DevicesT, lead: HeadlineLead, device: string) {
	if (lead.kind === "empty")
		return t(
			"devices:models.overview.headline.empty",
			"{{device}} hosts no models yet.",
			{
				device,
			},
		);
	if (lead.kind === "failed")
		return t("devices:models.overview.headline.failed", {
			count: lead.failed,
			device,
			defaultValue_one:
				"{{count, number}} model on {{device}} failed to start.",
			defaultValue_other:
				"{{count, number}} models on {{device}} failed to start.",
		});
	if (lead.kind === "idle")
		return t("devices:models.overview.headline.idle", {
			count: lead.models,
			defaultValue_one: "{{count, number}} model, not loaded right now.",
			defaultValue_other: "{{count, number}} models, none loaded right now.",
		});
	return lead.loaded === lead.models
		? t("devices:models.overview.headline.serving", {
				count: lead.models,
				defaultValue_one: "Serving {{count, number}} model.",
				defaultValue_other: "Serving {{count, number}} models.",
			})
		: t(
				"devices:models.overview.headline.servingSome",
				"Serving {{loaded, number}} of {{models, number}} models.",
				{ loaded: lead.loaded, models: lead.models },
			);
}

function headlineFacts(t: DevicesT, facts: ModelsHeadline, locale: string) {
	const parts = [
		facts.requests24h
			? t(
					"devices:models.overview.headline.traffic",
					"{{tokens}} tokens and {{requests}} requests in the last 24 hours",
					{
						tokens: compactNumber(locale, facts.tokens24h),
						requests: compactNumber(locale, facts.requests24h),
					},
				)
			: t(
					"devices:models.overview.headline.noTraffic",
					"No requests in the last 24 hours",
				),
	];
	if (facts.errors24h)
		parts.push(
			t("devices:models.overview.headline.errors", {
				count: facts.errors24h,
				defaultValue_one: "{{count, number}} error",
				defaultValue_other: "{{count, number}} errors",
			}),
		);
	if (facts.gpuMemoryPercent !== undefined)
		parts.push(
			t(
				"devices:models.overview.headline.gpuMemory",
				"GPU memory {{percent, number}} % used",
				{ percent: facts.gpuMemoryPercent },
			),
		);
	if (facts.downloading)
		parts.push(
			t("devices:models.overview.headline.downloading", {
				count: facts.downloading,
				defaultValue_one: "{{count, number}} model downloading",
				defaultValue_other: "{{count, number}} models downloading",
			}),
		);
	return parts.join(" · ");
}

/** "Serving 2 of 3 models." · "1.7M tokens and 4.4K requests in the last 24 hours · GPU memory 29 % used". */
export function headlineCopy(
	t: DevicesT,
	facts: ModelsHeadline,
	context: { device: string; locale: string },
): HeadlineCopy {
	const lead = headlineLead(t, facts.lead, context.device);
	if (facts.lead.kind === "empty")
		return {
			lead,
			rest: t(
				"devices:models.overview.headline.emptyRest",
				"Add one to serve it to your apps and to the people you share the device with.",
			),
		};
	return { lead, rest: headlineFacts(t, facts, context.locale) };
}

/* Numbers. */

const compactFormats = new Map<string, Intl.NumberFormat>();

/** "1.2M": token and request counts of a day. */
export function compactNumber(locale: string, value: number): string {
	let format = compactFormats.get(locale);
	if (!format) {
		format = new Intl.NumberFormat(locale, {
			notation: "compact",
			maximumFractionDigits: 1,
		});
		compactFormats.set(locale, format);
	}
	return format.format(value);
}

export function rateText(t: DevicesT, bytesPerSecond: number): string {
	return t("devices:models.downloads.rate", "{{size}}/s", {
		size: bytesText(bytesPerSecond),
	});
}

/** "1.8 s" or "640 ms". */
export function durationMs(t: DevicesT, ms: number): string {
	const whole = Math.round(ms);
	return whole >= 1000
		? t("devices:models.overview.seconds", "{{value, number}} s", {
				value: Math.round(whole / 100) / 10,
			})
		: t("devices:models.overview.milliseconds", "{{value, number}} ms", {
				value: whole,
			});
}

/** What is left of a download: "about 30 s", "about 4 min", "about 2 h 5 min". */
export function etaText(t: DevicesT, seconds: number): string {
	const whole = Math.max(1, Math.round(seconds));
	if (whole < 60)
		return t("devices:models.downloads.etaSeconds", "about {{value}} s", {
			value: whole,
		});
	const minutes = Math.round(whole / 60);
	if (minutes < 60)
		return t("devices:models.downloads.etaMinutes", "about {{value}} min", {
			value: minutes,
		});
	return t(
		"devices:models.downloads.etaHours",
		"about {{hours}} h {{minutes}} min",
		{ hours: Math.floor(minutes / 60), minutes: minutes % 60 },
	);
}
