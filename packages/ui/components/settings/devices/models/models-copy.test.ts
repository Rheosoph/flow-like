import { describe, expect, test } from "bun:test";
import { getI18n } from "@flow-like/locales";
import {
	type HostedModel,
	MODEL_BACKENDS,
	MODEL_ENGINES,
	MODEL_KINDS,
	MODEL_RUNTIMES,
	type ModelJob,
} from "../../../../lib/device-management/models";
import type { DevicesT } from "../primitives/area-context";
import {
	STATE_TONE,
	backendLabel,
	compactNumber,
	durationMs,
	engineLabel,
	etaText,
	failureLabel,
	headlineCopy,
	jobFailureText,
	jobStateLabel,
	kindLabel,
	rateText,
	residencyLabel,
	runtimeName,
	stateLabel,
} from "./models-copy";
import type { ModelsHeadline } from "./models-view";

const t = getI18n().getFixedT("en", "devices") as DevicesT;

/** A wire value, a key or an unfilled placeholder in a sentence (R3). */
const LEAK = /_|\{\{|undefined|NaN|devices:/;

type HostFailure = Extract<HostedModel, { state: "failed" }>["reason"];
type JobFailure = Extract<ModelJob, { state: "failed" }>;

const HOST_FAILURES = [
	"asset_missing",
	"runtime_missing",
	"insufficient_memory",
	"engine_exited",
	"health_timeout",
	"unknown",
] as const satisfies readonly HostFailure[];
const JOB_STATES = [
	"queued",
	"fetching",
	"verifying",
	"present",
	"awaiting_push",
	"failed",
] as const satisfies readonly ModelJob["state"][];
const JOB_FAILURES = [
	"egress_blocked",
	"http_status",
	"digest_mismatch",
	"size_mismatch",
	"disk_budget",
	"no_sources",
	"cancelled",
	"io",
	"unknown",
] as const satisfies readonly JobFailure["reason"][];

function expectLabels(labels: readonly string[], wire: readonly string[]) {
	expect(new Set(labels).size).toBe(labels.length);
	for (const [index, label] of labels.entries()) {
		expect([wire[index], LEAK.test(label)]).toEqual([wire[index], false]);
		expect(label).not.toBe(wire[index]);
	}
}

const failedJob = (
	reason: JobFailure["reason"],
	httpStatus?: number,
): JobFailure => ({
	job_id: "8d2e4f6a-1c3b-4d5e-8f7a-9b0c1d2e3f40",
	digest: { algorithm: "sha256", hex: "5".repeat(64) },
	size: 851_251_104,
	file_name: "mmproj-gemma-3-4b-it-f16.gguf",
	updated_at: 1_790_769_000,
	state: "failed",
	reason,
	...(httpStatus === undefined ? {} : { http_status: httpStatus }),
});

describe("labels", () => {
	test("every kind, engine, backend and model state has its own label", () => {
		expectLabels(
			MODEL_KINDS.map((kind) => kindLabel(t, kind)),
			MODEL_KINDS,
		);
		expectLabels(
			MODEL_ENGINES.map((engine) => engineLabel(t, engine)),
			MODEL_ENGINES,
		);
		expectLabels(
			MODEL_BACKENDS.map((backend) => backendLabel(t, backend)),
			MODEL_BACKENDS,
		);
		const states = Object.keys(STATE_TONE) as HostedModel["state"][];
		expectLabels(
			states.map((state) => stateLabel(t, state)),
			states,
		);
		expect(engineLabel(t, "onnx")).toBe("ONNX Runtime");
		expect(stateLabel(t, "acquiring")).toBe("Downloading");
	});

	test("a runtime pack is named by its engine and backend", () => {
		const names = MODEL_RUNTIMES.flatMap((runtime) =>
			MODEL_BACKENDS.map((backend) => runtimeName(t, { runtime, backend })),
		);
		expect(new Set(names).size).toBe(names.length);
		expect(runtimeName(t, { runtime: "llamacpp", backend: "vulkan" })).toBe(
			"llama.cpp for Vulkan",
		);
		expect(runtimeName(t, { runtime: "mlx", backend: "metal" })).toBe(
			"MLX for Metal",
		);
	});

	test("why a model failed to start, and how it stays loaded", () => {
		expectLabels(
			HOST_FAILURES.map((reason) => failureLabel(t, reason)),
			HOST_FAILURES,
		);
		expect(residencyLabel(t, { mode: "always_on" })).toBe("Always loaded");
		expect(residencyLabel(t, { mode: "pinned_off" })).toBe("Kept off");
		expect(
			residencyLabel(t, {
				mode: "on_demand",
				idle_unload_after_seconds: 3_600,
			}),
		).toBe("Loads on demand, unloads after 60 min idle");
	});

	test("downloads: every state and failure reads as a sentence; an HTTP error names its status", () => {
		expectLabels(
			JOB_STATES.map((state) => jobStateLabel(t, state)),
			JOB_STATES,
		);
		const texts = JOB_FAILURES.map((reason) =>
			jobFailureText(t, failedJob(reason)),
		);
		expectLabels(texts, JOB_FAILURES);
		expect(jobFailureText(t, failedJob("http_status", 403))).toBe(
			"The download source answered with error 403.",
		);
		expect(jobFailureText(t, failedJob("http_status"))).toBe(
			"The download source answered with an error.",
		);
	});
});

describe("headline", () => {
	const facts = (patch: Partial<ModelsHeadline>): ModelsHeadline => ({
		lead: { kind: "serving", loaded: 1, models: 1 },
		tokens24h: 0,
		requests24h: 0,
		errors24h: 0,
		downloading: 0,
		...patch,
	});
	const copy = (patch: Partial<ModelsHeadline>) =>
		headlineCopy(t, facts(patch), { device: "edge-berlin-01", locale: "en" });

	test("the lead: failed models first, then serving all or some, then idle", () => {
		expect(copy({ lead: { kind: "failed", failed: 1 } }).lead).toBe(
			"1 model on edge-berlin-01 failed to start.",
		);
		expect(copy({ lead: { kind: "failed", failed: 2 } }).lead).toBe(
			"2 models on edge-berlin-01 failed to start.",
		);
		expect(copy({}).lead).toBe("Serving 1 model.");
		expect(copy({ lead: { kind: "serving", loaded: 1, models: 3 } }).lead).toBe(
			"Serving 1 of 3 models.",
		);
		expect(copy({ lead: { kind: "idle", models: 1 } }).lead).toBe(
			"1 model, not loaded right now.",
		);
		expect(copy({ lead: { kind: "idle", models: 4 } }).lead).toBe(
			"4 models, none loaded right now.",
		);
	});

	test("the rest: the day's traffic, then errors, GPU memory and downloads only when there are any", () => {
		expect(copy({}).rest).toBe("No requests in the last 24 hours");
		expect(
			copy({
				tokens24h: 12_400,
				requests24h: 31,
				errors24h: 1,
				gpuMemoryPercent: 63,
				downloading: 2,
			}).rest,
		).toBe(
			"12.4K tokens and 31 requests in the last 24 hours · 1 error · GPU memory 63 % used · 2 models downloading",
		);
		expect(copy({ lead: { kind: "empty" } }).rest).toBe(
			"Add one to serve it to your apps and to the people you share the device with.",
		);
	});
});

describe("numbers", () => {
	test("counts are compact in the viewer's locale", () => {
		expect(compactNumber("en", 1_700_000)).toBe("1.7M");
		expect(compactNumber("en", 4_400)).toBe("4.4K");
		expect(compactNumber("en", 950)).toBe("950");
		expect(compactNumber("de", 1_700_000)).toContain("1,7");
	});

	test("rates are bytes per second", () => {
		expect(rateText(t, 52_428_800)).toBe("50.0 MiB/s");
	});

	test("durations switch to seconds from one second on", () => {
		expect(durationMs(t, 640)).toBe("640 ms");
		expect(durationMs(t, 999.6)).toBe("1 s");
		expect(durationMs(t, 1_800)).toBe("1.8 s");
		expect(durationMs(t, 12_345)).toBe("12.3 s");
	});

	test("time left rounds to seconds, minutes, then hours and minutes", () => {
		expect(etaText(t, 0.2)).toBe("about 1 s");
		expect(etaText(t, 28)).toBe("about 28 s");
		expect(etaText(t, 59.6)).toBe("about 1 min");
		expect(etaText(t, 90)).toBe("about 2 min");
		expect(etaText(t, 7_500)).toBe("about 2 h 5 min");
	});
});
