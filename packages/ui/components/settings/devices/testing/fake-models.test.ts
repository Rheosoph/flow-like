import { describe, expect, test } from "bun:test";
import {
	MAC_MODEL_HOST_FEATURES,
	MODEL_HOST_FEATURES,
	PRE_MODEL_FEATURES,
	SAMPLE_DIGESTS,
	SAMPLE_JOB_IDS,
	SAMPLE_MODELS_HOUR,
	crowdedMacModels,
	emptyModels,
	gpuBoxModels,
	macMiniModels,
	overviewOf,
} from "../../../../lib/device-management/model/__fixtures__/sample-models";
import {
	type HostedModel,
	type ModelSpec,
	type ModelsRequest,
	hostedModelSchema,
	modelInstalledSchema,
	modelJobSchema,
	modelsCommand,
	probeModelSystem,
	readHostedModels,
	readModelJobs,
	readModelRecommendations,
	readModelStats,
	readModelsOverview,
	runtimeInstalledSchema,
} from "../../../../lib/device-management/models";
import type { ManagementCall } from "../../../../lib/device-management/telemetry";
import { FakeModelHost, isModelsRead, modelsFeature } from "./fake-models";

const NOW = SAMPLE_MODELS_HOUR + 120;

function host(sample = gpuBoxModels()) {
	const fake = new FakeModelHost(sample, () => NOW);
	const call: ManagementCall = async (command) => ({
		operation_id: "op",
		...fake.answer(command),
	});
	const send = (request: ModelsRequest) => fake.answer(modelsCommand(request));
	return { fake, call, send };
}

async function ok<T>(read: Promise<{ kind: string; data?: T }>): Promise<T> {
	const result = await read;
	if (result.kind !== "ok")
		throw new Error(`expected data, got ${result.kind}`);
	return result.data as T;
}

const model = (fake: FakeModelHost, id: string) =>
	fake.state.models.find((row) => row.id === id) as HostedModel;

describe("fixtures", () => {
	test("every sample answers every read in a shape the client accepts", async () => {
		for (const [sample, features] of [
			[gpuBoxModels(), MODEL_HOST_FEATURES],
			[macMiniModels(), MAC_MODEL_HOST_FEATURES],
			[crowdedMacModels(), MAC_MODEL_HOST_FEATURES],
			[emptyModels(), MODEL_HOST_FEATURES],
		] as const) {
			const { call } = host(sample);
			const overview = await ok(readModelsOverview(call, features));
			expect(overview).toEqual(overviewOf(sample, NOW));
			await ok(readHostedModels(call, features));
			await ok(readModelJobs(call, features));
			await ok(readModelRecommendations(call, features));
			await ok(probeModelSystem(call, features));
			await ok(
				readModelStats(call, features, {
					from: SAMPLE_MODELS_HOUR - 23 * 3600,
					to: SAMPLE_MODELS_HOUR + 3600,
					step: "hour",
				}),
			);
		}
	});

	test("the GPU box: a Vulkan chat model, an ONNX embedding model and Gemma downloading", async () => {
		const { call } = host();
		const overview = await ok(readModelsOverview(call, MODEL_HOST_FEATURES));
		expect(
			overview.models.map((row) => [row.id, row.engine, row.state]),
		).toEqual([
			["gemma-3-4b", "llamacpp", "acquiring"],
			["nomic-embed-v1.5", "onnx", "loaded"],
			["qwen3-8b", "llamacpp", "loaded"],
		]);
		expect(overview.system.gpus[0]?.name).toBe("NVIDIA GeForce RTX 4090");
		expect(overview.recommendations.map((row) => row.code)).toEqual([
			"kv_pressure",
			"requests_queued",
			"cpu_threads",
		]);
		const jobs = await ok(readModelJobs(call, MODEL_HOST_FEATURES));
		expect(jobs.jobs.map((job) => job.state)).toEqual(["fetching", "failed"]);
	});

	test("an agent from before model hosting carries none of its flags", () => {
		expect(
			Object.keys(PRE_MODEL_FEATURES).some((flag) => flag.startsWith("model_")),
		).toBe(false);
		expect(MODEL_HOST_FEATURES.model_host).toBe(1);
		expect(MAC_MODEL_HOST_FEATURES.model_runtime_mlx).toBe(1);
	});
});

describe("reads", () => {
	test("pages carry their cursor; the limit is checked", async () => {
		const { call, send } = host();
		const first = await ok(
			readHostedModels(call, MODEL_HOST_FEATURES, { limit: 2 }),
		);
		expect(first.models.map((row) => row.id)).toEqual([
			"gemma-3-4b",
			"nomic-embed-v1.5",
		]);
		expect(first.next).toBe("nomic-embed-v1.5");
		const second = await ok(
			readHostedModels(call, MODEL_HOST_FEATURES, {
				after: first.next ?? "",
				limit: 2,
			}),
		);
		expect(second).toEqual({
			models: [model(host().fake, "qwen3-8b")],
			next: null,
		});
		const recommendations = await ok(
			readModelRecommendations(call, MODEL_HOST_FEATURES, { limit: 1 }),
		);
		expect(recommendations.next).toBe("1");
		const rest = await ok(
			readModelRecommendations(call, MODEL_HOST_FEATURES, {
				after: recommendations.next ?? "",
			}),
		);
		expect(rest.recommendations.map((row) => row.code)).toEqual([
			"requests_queued",
			"cpu_threads",
		]);
		expect(send({ kind: "jobs", limit: 0 })).toMatchObject({
			state: "rejected",
			result: { code: "invalid" },
		});
	});

	test("an overview carries the models that fit one reply and names the last; pages go on after it", async () => {
		const sample = crowdedMacModels();
		const { call } = host(sample);
		const overview = await ok(
			readModelsOverview(call, MAC_MODEL_HOST_FEATURES),
		);
		expect(overview.models.length).toBeLessThan(sample.models.length);
		expect(overview.next).toBe(overview.models.at(-1)?.id ?? "");
		const rest = await ok(
			readHostedModels(call, MAC_MODEL_HOST_FEATURES, {
				after: overview.next ?? "",
				limit: 32,
			}),
		);
		expect(rest.next).toBeNull();
		expect([...overview.models, ...rest.models].map((row) => row.id)).toEqual(
			sample.models.map((row) => row.id).sort(),
		);
	});

	test("statistics follow the requested range and step", async () => {
		const { call } = host();
		const minutes = await ok(
			readModelStats(call, MODEL_HOST_FEATURES, {
				modelId: "qwen3-8b",
				from: SAMPLE_MODELS_HOUR,
				to: SAMPLE_MODELS_HOUR + 600,
				step: "minute",
			}),
		);
		expect(minutes.series.requests).toHaveLength(10);
		expect(minutes.series.requests[0]).toBe(0);
		const older = await ok(
			readModelStats(call, MODEL_HOST_FEATURES, {
				from: SAMPLE_MODELS_HOUR - 30 * 86_400,
				to: SAMPLE_MODELS_HOUR - 29 * 86_400,
				step: "hour",
			}),
		);
		expect(older.series.ttft_p95_ms.every((value) => value === null)).toBe(
			true,
		);
		expect(older.consumers).toEqual([]);
	});

	test("downloads and the deploy step need only the model store", () => {
		expect(modelsFeature(modelsCommand({ kind: "jobs" }))).toBe("model_store");
		expect(
			modelsFeature(
				modelsCommand({ kind: "cancel_job", job_id: SAMPLE_JOB_IDS.fetching }),
			),
		).toBe("model_store");
		expect(modelsFeature(modelsCommand({ kind: "overview" }))).toBe(
			"model_host",
		);
		expect(isModelsRead(modelsCommand({ kind: "probe" }))).toBe(true);
		expect(
			isModelsRead(modelsCommand({ kind: "load", model_id: "qwen3-8b" })),
		).toBe(false);
	});
});

describe("writes", () => {
	test("install adds the model and one download per missing file", () => {
		const { fake, send } = host(emptyModels());
		const reply = send({
			kind: "install",
			model_id: "phi-4-mini",
			model: {
				display_name: "Phi-4 mini Q4_K_M",
				kind: "chat",
				engine: "llamacpp",
				assets: [
					{
						digest: SAMPLE_DIGESTS.qwen4,
						size: 2_400_000_000,
						file_name: "phi-4-mini-Q4_K_M.gguf",
						sources: [
							"https://huggingface.co/microsoft/phi-4/resolve/a/b.gguf",
						],
					},
				],
			},
		});
		const installed = modelInstalledSchema.parse(reply.result);
		expect(installed.model.state).toBe("acquiring");
		expect(installed.assets).toMatchObject({ total: 1, present: 0 });
		expect(fake.state.jobs).toHaveLength(1);
		expect(fake.state.jobs[0]).toMatchObject({
			state: "fetching",
			source_host: "huggingface.co",
		});
		expect(fake.writes.map((write) => write.kind)).toEqual(["install"]);
		fake.finishJobs();
		expect(model(fake, "phi-4-mini").state).toBe("stopped");
	});

	test("installing the same spec again answers the model and restarts its cancelled download; another spec under its id is refused", () => {
		const { fake, send } = host(emptyModels());
		const spec: ModelSpec = {
			display_name: "Phi-4 mini Q4_K_M",
			kind: "chat",
			engine: "llamacpp",
			assets: [
				{
					digest: SAMPLE_DIGESTS.qwen4,
					size: 2_400_000_000,
					file_name: "phi-4-mini-Q4_K_M.gguf",
					sources: ["https://huggingface.co/microsoft/phi-4/resolve/a/b.gguf"],
				},
			],
		};
		const install = (model: ModelSpec): ModelsRequest => ({
			kind: "install",
			model_id: "phi-4-mini",
			model,
		});
		send(install(spec));
		const job = fake.state.jobs[0]?.job_id ?? "";
		send({ kind: "cancel_job", job_id: job });
		expect(fake.state.jobs[0]).toMatchObject({ state: "failed" });
		const again = modelInstalledSchema.parse(send(install(spec)).result);
		expect(again.model.id).toBe("phi-4-mini");
		expect(again.assets).toMatchObject({ total: 1, present: 0 });
		expect(fake.state.jobs).toHaveLength(1);
		expect(fake.state.jobs[0]).toMatchObject({
			job_id: job,
			state: "fetching",
		});
		expect(
			send(install({ ...spec, display_name: "Phi-4 mini Q8_0" })),
		).toMatchObject({
			state: "rejected",
			result: { code: "revision_conflict" },
		});
	});

	test("a load answers loading and is done by the next read; a model kept off is refused like the agent does", async () => {
		const { fake, send, call } = host();
		send({ kind: "unload", model_id: "nomic-embed-v1.5" });
		const loading = hostedModelSchema.parse(
			send({ kind: "load", model_id: "nomic-embed-v1.5" }).result,
		);
		expect(loading.state).toBe("loading");
		await ok(readModelsOverview(call, MODEL_HOST_FEATURES));
		expect(model(fake, "nomic-embed-v1.5").state).toBe("loaded");
		const off = {
			kind: "configure",
			model_id: "nomic-embed-v1.5",
			expected_revision: 2,
			settings: { threads: 8 },
			residency: { mode: "pinned_off" },
		} as const;
		expect(hostedModelSchema.parse(send(off).result).state).toBe("stopped");
		expect(send({ kind: "load", model_id: "nomic-embed-v1.5" })).toMatchObject({
			state: "rejected",
			result: { code: "invalid" },
		});
		expect(model(fake, "nomic-embed-v1.5").state).toBe("stopped");
	});

	test("a model kept loaded starts loading again right after its unload, as the device's supervisor does", async () => {
		const { fake, send, call } = host();
		const answer = hostedModelSchema.parse(
			send({ kind: "unload", model_id: "qwen3-8b" }).result,
		);
		expect(answer.state).toBe("stopped");
		expect(model(fake, "qwen3-8b").state).toBe("loading");
		await ok(readHostedModels(call, MODEL_HOST_FEATURES));
		expect(model(fake, "qwen3-8b").state).toBe("loaded");
		send({ kind: "unload", model_id: "nomic-embed-v1.5" });
		expect(model(fake, "nomic-embed-v1.5").state).toBe("stopped");
	});

	test("new settings stop a loaded engine, which loads again when it is kept loaded; residency alone restarts nothing; a stale revision is refused", () => {
		const { fake, send } = host();
		const configure = {
			kind: "configure",
			model_id: "qwen3-8b",
			expected_revision: 3,
			settings: { parallel: 6 },
			residency: { mode: "always_on" },
		} as const;
		const restarted = hostedModelSchema.parse(send(configure).result);
		expect(restarted).toMatchObject({ revision: 4, state: "stopped" });
		expect(send(configure)).toMatchObject({
			state: "rejected",
			result: { code: "revision_conflict" },
		});
		expect(model(fake, "qwen3-8b").settings).toEqual({ parallel: 6 });
		fake.finishLoads();
		expect(model(fake, "qwen3-8b")).toMatchObject({
			state: "loaded",
			slots: 6,
		});
		const nomic = model(fake, "nomic-embed-v1.5");
		const onDemand = {
			kind: "configure",
			model_id: nomic.id,
			expected_revision: nomic.revision,
			settings: { threads: 16 },
			residency: nomic.residency,
		} as const;
		expect(hostedModelSchema.parse(send(onDemand).result).state).toBe(
			"stopped",
		);
		fake.finishLoads();
		expect(model(fake, nomic.id).state).toBe("stopped");
		const residency = {
			kind: "configure",
			model_id: "qwen3-8b",
			expected_revision: 4,
			settings: { parallel: 6 },
			residency: { mode: "on_demand", idle_unload_after_seconds: 900 },
		} as const;
		expect(hostedModelSchema.parse(send(residency).result).state).toBe(
			"loaded",
		);
	});

	test("remove drops the model, its downloads and its recommendations", () => {
		const { fake, send } = host();
		expect(
			send({ kind: "remove", model_id: "gemma-3-4b", expected_revision: 1 }),
		).toEqual({ state: "completed", result: { model_id: "gemma-3-4b" } });
		expect(fake.state.models.map((row) => row.id)).toEqual([
			"qwen3-8b",
			"nomic-embed-v1.5",
		]);
		expect(fake.state.jobs).toEqual([]);
		send({ kind: "remove", model_id: "qwen3-8b", expected_revision: 3 });
		expect(fake.state.recommendations.map((row) => row.code)).toEqual([
			"cpu_threads",
		]);
		const gone: ModelsRequest[] = [
			{ kind: "remove", model_id: "qwen3-8b", expected_revision: 3 },
			{ kind: "load", model_id: "qwen3-8b" },
			{ kind: "unload", model_id: "qwen3-8b" },
		];
		for (const request of gone)
			expect(send(request)).toMatchObject({
				state: "rejected",
				result: { code: "revision_conflict" },
			});
	});

	test("runtimes install through a download; a pack a loaded model runs from is refused like the agent's", () => {
		const { fake, send } = host();
		const reply = runtimeInstalledSchema.parse(
			send({ kind: "install_runtime", runtime: "llamacpp", backend: "cpu" })
				.result,
		);
		expect(reply.asset.state).toBe("fetching");
		fake.finishJobs();
		expect(
			fake.state.runtimes.find((row) => row.backend === "cpu")?.installed,
		).toBe(true);
		expect(
			send({ kind: "install_runtime", runtime: "mlx", backend: "cpu" }),
		).toMatchObject({ state: "rejected" });
		expect(
			send({ kind: "remove_runtime", runtime: "llamacpp", backend: "vulkan" }),
		).toMatchObject({
			state: "rejected",
			result: { code: "revision_conflict" },
		});
		expect(
			send({ kind: "remove_runtime", runtime: "llamacpp", backend: "cpu" }),
		).toMatchObject({ state: "completed" });
		send({ kind: "unload", model_id: "qwen3-8b" });
		expect(
			send({ kind: "remove_runtime", runtime: "llamacpp", backend: "vulkan" }),
		).toMatchObject({ state: "completed" });
	});

	test("a download is cancelled once; refusals hold until restored", () => {
		const { send, fake } = host();
		const cancelled = modelJobSchema.parse(
			send({ kind: "cancel_job", job_id: SAMPLE_JOB_IDS.fetching }).result,
		);
		expect(cancelled).toMatchObject({ state: "failed", reason: "cancelled" });
		expect(
			send({
				kind: "cancel_job",
				job_id: "00000000-0000-4000-8000-000000000000",
			}),
		).toMatchObject({
			state: "rejected",
			result: { code: "revision_conflict" },
		});
		const restore = fake.refuse("unload", "busy", "The engine is busy.");
		expect(send({ kind: "unload", model_id: "qwen3-8b" })).toEqual({
			state: "rejected",
			result: { code: "busy", error: "The engine is busy.", retryable: false },
		});
		restore();
		expect(send({ kind: "unload", model_id: "qwen3-8b" }).state).toBe(
			"completed",
		);
		expect(
			send({
				kind: "ensure",
				project_id: "app_invoice_ai",
				pins: [{ bit_id: "bit-1", metadata_sha256: "a".repeat(64) }],
			}),
		).toEqual({
			state: "completed",
			result: { total: 1, present: 1, pending: [] },
		});
	});
});
