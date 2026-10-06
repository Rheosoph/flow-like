import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import type { AgentFeatures } from "./model/types";
import {
	MODEL_STATS_MAX_POINTS,
	hostedModelSchema,
	modelAssetSummarySchema,
	modelInstalledSchema,
	modelsCommand,
	probeModelSystem,
	readHostedModels,
	readModelJobs,
	readModelRecommendations,
	readModelStats,
	readModelsOverview,
} from "./models";
import type { ManagementCall } from "./telemetry";
import type { ManagementResponse } from "./types";
import { LiveCallError } from "./workspace/errors";

const fixture = JSON.parse(
	readFileSync(
		new URL(
			"../../../device-protocol/fixtures/models-v1.json",
			import.meta.url,
		),
		"utf8",
	),
);
const HOST: AgentFeatures = { model_store: 1, model_host: 1 };
const clone = <T>(value: T): T => structuredClone(value);

function completed(result: unknown): ManagementResponse {
	return {
		operation_id: "op",
		state: "completed",
		result: result as Record<string, unknown>,
	};
}
function rejected(code: string): ManagementResponse {
	return {
		operation_id: "op",
		state: "rejected",
		result: { code, error: "Refused.", retryable: false },
	};
}
function recorder(reply: () => ManagementResponse) {
	const sent: Record<string, unknown>[] = [];
	const call: ManagementCall = async (command) => {
		sent.push(command);
		return reply();
	};
	return { call, sent };
}
async function data<T>(
	read: Promise<{ kind: "ok"; data: T } | { kind: "unsupported" }>,
) {
	const result = await read;
	if (result.kind !== "ok")
		throw new Error(`expected data, got ${result.kind}`);
	return result.data;
}

describe("models replies", () => {
	test("the shared protocol fixture parses with flat states", async () => {
		const { call, sent } = recorder(() => completed(fixture.overview));
		const overview = await data(readModelsOverview(call, HOST));
		expect(sent).toEqual([{ type: "models", request: { kind: "overview" } }]);
		const [qwen, nomic] = overview.models;
		expect(qwen.state === "loaded" ? qwen.slots_busy : -1).toBe(1);
		expect(qwen.settings.gpu_layers).toEqual({ count: 37 });
		expect(nomic.state).toBe("acquiring");
		expect(overview.system.gpus[1].memory_total).toBeNull();
		expect(overview.recommendations[0].fix).toEqual(
			fixture.overview.recommendations[0].fix,
		);
		const installed = modelInstalledSchema.parse(fixture.installed);
		expect(installed.assets.pending[0]).toMatchObject({
			state: "awaiting_push",
			bytes: 0,
			job_id: "6f1c2a3b-4d5e-4f60-8a7b-9c0d1e2f3a4b",
		});
	});

	test("agents without the flag are never asked, and refusals read as unsupported", async () => {
		const unasked = recorder(() => completed(fixture.overview));
		expect(await readModelsOverview(unasked.call, {})).toEqual({
			kind: "unsupported",
			feature: "model_host",
		});
		expect(await readModelJobs(unasked.call, { model_host: 1 })).toEqual({
			kind: "unsupported",
			feature: "model_store",
		});
		expect(unasked.sent).toEqual([]);

		const older = recorder(() => rejected("unsupported"));
		expect(await readModelsOverview(older.call, HOST)).toEqual({
			kind: "unsupported",
			feature: "model_host",
		});
		const denied = recorder(() => rejected("unauthorized"));
		await expect(readModelsOverview(denied.call, HOST)).rejects.toBeInstanceOf(
			LiveCallError,
		);
		const refusal = await readModelsOverview(denied.call, HOST).catch(
			(error: LiveCallError) => error,
		);
		expect(refusal).toMatchObject({
			liveError: {
				step: "reading_services",
				code: "rejected",
				rejection: { code: "unauthorized", error: "Refused." },
			},
		});
	});

	test("invalid answers name what is wrong", async () => {
		const busy = clone(fixture.overview);
		busy.models[0].slots_busy = 5;
		const { call } = recorder(() => completed(busy));
		await expect(readModelsOverview(call, HOST)).rejects.toThrow(
			/invalid model overview .*busy slots exceed slots/,
		);
		const stateless = clone(fixture.overview.models[1]);
		stateless.state = undefined;
		expect(hostedModelSchema.safeParse(stateless).success).toBe(false);
		const present = clone(fixture.installed.assets);
		present.pending[0] = { ...present.pending[0], state: "present" };
		expect(modelAssetSummarySchema.safeParse(present).success).toBe(false);
	});

	test("pages send their cursor and limit and refuse longer answers", async () => {
		const { call, sent } = recorder(() => completed(fixture.jobs));
		const jobs = await data(
			readModelJobs(call, HOST, { after: "job-0", limit: 2 }),
		);
		expect(sent[0]).toEqual(
			modelsCommand({ kind: "jobs", after: "job-0", limit: 2 }),
		);
		expect(jobs.jobs.map((job) => job.state)).toEqual(["fetching", "failed"]);
		expect(jobs.next).toBe("7a2b3c4d-5e6f-4a1b-8c2d-3e4f5a6b7c8d");
		await expect(readModelJobs(call, HOST, { limit: 1 })).rejects.toThrow(
			/invalid model download list/,
		);
		expect(() => readHostedModels(call, HOST, { limit: 33 })).toThrow(
			RangeError,
		);
		const models = recorder(() =>
			completed({ models: fixture.overview.models, next: null }),
		);
		expect(
			(await data(readHostedModels(models.call, HOST))).models,
		).toHaveLength(2);
		expect(models.sent[0]).toEqual(
			modelsCommand({ kind: "models", after: null, limit: 8 }),
		);
	});

	test("recommendation fixes are bounded writes passed through unchanged", async () => {
		const page = {
			recommendations: clone(fixture.overview.recommendations),
			next: null,
		};
		page.recommendations[0].fix.future_field = true;
		const { call } = recorder(() => completed(page));
		const read = await data(readModelRecommendations(call, HOST));
		expect(read.recommendations[0].fix).toMatchObject({
			kind: "configure",
			future_field: true,
		});
		expect(read.recommendations[1].fix?.kind).toBe("install_runtime");
		page.recommendations[0].fix = {
			kind: "remove",
			model_id: "qwen3-8b",
			expected_revision: 3,
		};
		const unsent = await data(readModelRecommendations(call, HOST));
		expect(unsent.recommendations[0].code).toBe("kv_pressure");
		expect(unsent.recommendations[0].fix).toBeUndefined();
		expect(unsent.recommendations[1].fix?.kind).toBe("install_runtime");
	});

	test("what a newer agent added is left out or reads as unknown; the rest of the answer stays", async () => {
		const overview = clone(fixture.overview);
		overview.recommendations.unshift({ code: "kv_offload", tier: "now" });
		overview.runtimes.push({ ...overview.runtimes[0], backend: "rocm" });
		overview.system.gpus.push({ ...overview.system.gpus[0], backend: "rocm" });
		overview.models[0] = {
			...overview.models[0],
			state: "updating",
			ram_bytes: undefined,
			vram_bytes: undefined,
			slots: undefined,
			slots_busy: undefined,
		};
		overview.models[1] = {
			...overview.models[1],
			state: "failed",
			reason: "gpu_lost",
		};
		const read = await data(
			readModelsOverview(recorder(() => completed(overview)).call, HOST),
		);
		expect(read.recommendations.map((item) => item.code)).toEqual([
			"kv_pressure",
			"gpu_unused",
		]);
		expect(read.runtimes).toHaveLength(fixture.overview.runtimes.length);
		expect(read.system.gpus).toHaveLength(fixture.overview.system.gpus.length);
		expect(read.models.map((model) => model.state)).toEqual([
			"unknown",
			"failed",
		]);
		const failed = read.models[1];
		expect(failed.state === "failed" ? failed.reason : "").toBe("unknown");

		const jobs = clone(fixture.jobs);
		jobs.jobs[0].state = "throttled";
		jobs.jobs[1] = { ...jobs.jobs[1], reason: "quota" };
		const page = await data(
			readModelJobs(recorder(() => completed(jobs)).call, HOST),
		);
		expect(page.jobs.map((job) => job.state)).toEqual(["failed"]);
		const job = page.jobs[0];
		expect(job.state === "failed" ? job.reason : "").toBe("unknown");
		const summary = clone(fixture.installed.assets);
		summary.pending[0].state = "throttled";
		expect(modelAssetSummarySchema.parse(summary).pending).toEqual([]);
	});

	test("stats ranges are checked before sending and echoed back", async () => {
		const input = {
			modelId: "qwen3-8b",
			from: fixture.stats.from,
			to: fixture.stats.from + 2 * 3_600,
			step: "hour" as const,
		};
		const { call, sent } = recorder(() => completed(fixture.stats));
		const stats = await data(readModelStats(call, HOST, input));
		expect(sent[0]).toEqual(
			modelsCommand({
				kind: "stats",
				model_id: "qwen3-8b",
				from: input.from,
				to: input.to,
				step: "hour",
			}),
		);
		expect(stats.series.ttft_p95_ms).toEqual([620, null]);
		expect(stats.consumers[1].consumer).toEqual({
			kind: "grant",
			grant_id: "grant-1",
		});
		await expect(
			readModelStats(call, HOST, { ...input, to: input.to + 3_600 }),
		).rejects.toThrow(/invalid usage statistics/);
		await expect(
			readModelStats(call, HOST, { ...input, modelId: undefined }),
		).rejects.toThrow(/range differs from the request/);
		for (const bad of [
			{ ...input, from: input.from + 60 },
			{ ...input, to: input.from },
			{ ...input, to: input.from + (MODEL_STATS_MAX_POINTS + 1) * 3_600 },
		])
			expect(() => readModelStats(call, HOST, bad)).toThrow(RangeError);
	});

	test("probe returns the hardware facts", async () => {
		const { call, sent } = recorder(() => completed(fixture.overview.system));
		const system = await data(probeModelSystem(call, HOST));
		expect(sent[0]).toEqual(modelsCommand({ kind: "probe" }));
		expect(system.cpu.physical_cores).toBe(16);
		const swapped = clone(fixture.overview.system);
		swapped.ram.free = swapped.ram.total + 1;
		const invalid = recorder(() => completed(swapped));
		await expect(probeModelSystem(invalid.call, HOST)).rejects.toThrow(
			/free exceeds total/,
		);
	});
});
